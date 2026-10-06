//! The Varyk packages of the build in a package's tables (M5b2 spec 2.1
//! to 2.4, 4.2): every checked package's modules, structs, enums, and
//! functions enter as imported items (M3 spec 4.1), each with the package
//! it is declared in, and the package's `[dependencies]` become the third
//! place the first name of a path is looked up.

use std::collections::HashMap;

use varyk_syntax::{FileId, Ident, Path, PathStart, SourceFile, Span};

use crate::diagnostics::{Diagnostic, codes};
use crate::hir::HirProgram;
use crate::packages::DepTarget;
use crate::types::Ty;

use super::{
    Callee, Dep, DepKind, Dependencies, EnumDef, EnumId, FieldDef, HttpItems, ImportedFnId,
    ImportedSig, LookupError, Module, ModuleId, PackageId, PackageItem, Packages, Scope, StructDef,
    StructId, Symbols, Unusable, UserType, VariantDef, VariantFieldsDef, is_std_module,
    reserved_type_name,
};

impl Symbols {
    /// The dependency of the package keyed `name` (`-` read as `_`).
    pub(crate) fn dep(&self, name: &str) -> Option<&Dep> {
        self.deps.iter().find(|dep| dep.name == name)
    }

    /// Step 3 of M5b2 spec 2.1 for `first`, the first name of a path that
    /// is neither a module declared here nor a `use` alias: the root of
    /// the Varyk package the package's dependency of that name is,
    /// `Dependency` for a dependency Varyk code cannot name, `Unknown`
    /// for a name that is no dependency or one that cannot be a name.
    pub(crate) fn dependency_root(&self, first: &Ident) -> Result<ModuleId, LookupError> {
        let index = self
            .deps
            .iter()
            .position(|dep| dep.name == first.name)
            .ok_or(LookupError::Unknown)?;
        // A standard name comes first, whatever the dependency is.
        if unnameable(&first.name).is_some() {
            return Err(LookupError::Unknown);
        }
        let dep = self.deps.get(index).ok_or(LookupError::Unknown)?;
        match dep.kind {
            DepKind::Varyk(root) => Ok(root),
            DepKind::Rust | DepKind::Optional => Err(LookupError::Dependency {
                span: first.span,
                dep: index as u32,
            }),
        }
    }

    /// Whether this package lists the Varyk package `id` in its
    /// `[dependencies]`.
    fn lists(&self, id: PackageId) -> bool {
        self.deps.iter().any(|dep| match dep.kind {
            DepKind::Varyk(root) => self.scopes[root.0 as usize].package == Some(id),
            DepKind::Rust | DepKind::Optional => false,
        })
    }

    /// The first struct or enum in `ty`, itself included, declared in a
    /// package this one does not list: its name and where it is declared.
    fn unlisted_type(&self, ty: &Ty) -> Option<(&str, &PackageItem)> {
        match ty {
            Ty::Struct(id) => {
                let def = &self.structs[id.0 as usize];
                def.package
                    .as_ref()
                    .filter(|item| !self.lists(item.package))
                    .map(|item| (def.name.as_str(), item))
            }
            Ty::Enum(id) => {
                let def = &self.enums[id.0 as usize];
                def.package
                    .as_ref()
                    .filter(|item| !self.lists(item.package))
                    .map(|item| (def.name.as_str(), item))
            }
            Ty::Option(inner)
            | Ty::Vec(inner)
            | Ty::Chain(inner)
            | Ty::Task(inner)
            | Ty::Shared(inner) => self.unlisted_type(inner),
            Ty::Result(a, b) | Ty::HashMap(a, b) => {
                self.unlisted_type(a).or_else(|| self.unlisted_type(b))
            }
            Ty::Bool | Ty::Int(_) | Ty::Float(_) | Ty::String | Ty::Error | Ty::Unit => None,
        }
    }

    /// V0115 at `span`, for a value of type `ty`, when `ty` names a struct
    /// or enum of a package this one does not list, or of another version
    /// of one it does (M5b2 spec 2.4): the generated Rust may write the
    /// type, which this package's crate could not name.
    pub(crate) fn unlisted_package(&self, ty: &Ty, span: Span) -> Option<Diagnostic> {
        let (name, item) = self.unlisted_type(ty)?;
        let package = &item.name;
        let listing = self.listings.get(item.package.0);
        let other = self.deps.iter().find_map(|dep| match dep.kind {
            DepKind::Varyk(root) if dep.package == *package => self.scopes[root.0 as usize].package,
            _ => None,
        });
        let version = |id: PackageId| {
            self.listings
                .get(id.0)
                .map_or_else(String::new, |listing| format!(" {}", listing.version))
        };
        let whose = match other {
            Some(_) => format!("the package `{package}`{}", version(item.package)),
            None => format!("the package `{package}`"),
        };
        let what = if matches!(ty, Ty::Struct(_) | Ty::Enum(_)) {
            format!("this is a `{name}` from {whose}")
        } else {
            let full = crate::types::derives::ty_name(&self.structs, &self.enums, ty);
            format!("this `{full}` holds a `{name}` from {whose}")
        };
        let mut diagnostic = Diagnostic::new(
            codes::V0115,
            span,
            format!("{what}, which this package does not list"),
        );
        match other {
            Some(listed) => {
                diagnostic = diagnostic.with_note(format!(
                    "this package lists `{package}`{}, another version of it; two versions of a \
                     package are two packages, and a `{name}` of one is not a `{name}` of the \
                     other",
                    version(listed)
                ));
                if let Some(listing) = listing {
                    diagnostic = diagnostic.with_note(format!(
                        "to use this `{name}`, make the line of `{package}` in `Cargo.toml` this \
                         one: `{}`",
                        listing.line
                    ));
                }
            }
            None => {
                let add = listing.map_or_else(
                    || "add it under `[dependencies]` there".to_string(),
                    |listing| {
                        format!(
                            "add this line under `[dependencies]` there: `{}`",
                            listing.line
                        )
                    },
                );
                diagnostic = diagnostic.with_note(format!(
                    "a package can use only the packages its `Cargo.toml` lists; {add}"
                ));
            }
        }
        Some(diagnostic)
    }

    /// The diagnostic of [`LookupError::Dependency`]: V0401 at `span` for
    /// an `optional` dependency `dep`, V0110 for a Rust crate (M5b2 spec
    /// 2.2, M3 spec 1).
    pub(crate) fn dependency_error(&self, span: Span, dep: u32) -> Diagnostic {
        let Some(dep) = self.deps.get(dep as usize) else {
            return Diagnostic::new(codes::V0100, span, "cannot find this dependency");
        };
        let name = &dep.name;
        match dep.kind {
            DepKind::Optional => Diagnostic::new(
                codes::V0401,
                span,
                format!(
                    "`{name}` is an optional dependency, and an optional dependency cannot be \
                     named yet"
                ),
            )
            .with_note(
                "cargo leaves an optional dependency out of the build unless a feature turns it \
                 on, so Varyk cannot tell what it is; to use it from Varyk code, remove \
                 `optional = true` from its line in `Cargo.toml`",
            ),
            DepKind::Rust | DepKind::Varyk(_) => Diagnostic::new(
                codes::V0110,
                span,
                format!(
                    "`{name}` is a library written in Rust, which Varyk code cannot use directly"
                ),
            )
            .with_note(
                "Varyk code does not use crates directly; call it from a `.rs` module in this \
                 package",
            ),
        }
    }
}

/// Why a dependency keyed `name` cannot be named in Varyk code, if it
/// cannot: it is a standard module or type's name, or starts with
/// `varyk_` (M5b2 spec 2.1), each of which comes first wherever it is
/// written. (A keyword cannot start a path at all.)
fn unnameable(name: &str) -> Option<String> {
    if name.starts_with("varyk_") {
        Some("names starting with `varyk_` are kept for Varyk".to_string())
    } else if is_std_module(name) {
        Some(format!("`{name}` is a standard module, which comes first"))
    } else if reserved_type_name(name, Span::new(FileId(0), 0, 0)).is_some() {
        Some(format!(
            "`{name}` is a standard type's name, which comes first"
        ))
    } else {
        None
    }
}

/// The package's dependencies as Varyk code names them, from its
/// manifest's keys and what the graph resolved them to; `roots` holds the
/// root module of each imported Varyk package, by graph index.
pub(super) fn deps(
    dependencies: Option<Dependencies<'_>>,
    packages: Packages<'_>,
    roots: &HashMap<usize, ModuleId>,
) -> Vec<Dep> {
    let Some(dependencies) = dependencies else {
        return Vec::new();
    };
    dependencies
        .crates
        .iter()
        .map(|key| {
            let name = key.replace('-', "_");
            let resolved = packages
                .keys
                .iter()
                .find(|(resolved, _)| *resolved == name)
                .map(|(_, target)| *target);
            let (kind, package) = if dependencies.optional.contains(key) {
                (DepKind::Optional, key.clone())
            } else {
                match resolved {
                    Some(DepTarget::Varyk(index)) => {
                        let package = packages
                            .checked
                            .iter()
                            .find(|checked| checked.graph_index == index)
                            .map_or_else(|| key.clone(), |checked| checked.name.clone());
                        let kind = roots
                            .get(&index)
                            .map_or(DepKind::Rust, |&root| DepKind::Varyk(root));
                        (kind, package)
                    }
                    Some(DepTarget::Rust) | None => (DepKind::Rust, key.clone()),
                }
            };
            Dep {
                name,
                package,
                kind,
            }
        })
        .collect()
}

/// Imports every package of `packages.checked` into `symbols`: its
/// modules as scopes after this package's, and its structs, enums, and
/// functions (but its tests, which are not visible, M5b2 spec 2.3) as
/// imported items carrying their package. A struct or enum a package has
/// from another is the one imported from that other, so each has one id
/// however it is reached (spec 4.2). Returns each package's root module,
/// by its index in the graph.
pub(super) fn register(symbols: &mut Symbols, packages: Packages<'_>) -> HashMap<usize, ModuleId> {
    let mut roots = HashMap::new();
    // Every struct and enum imported so far, by where it is declared and
    // its name, which a module's types never share.
    let mut types: HashMap<(PackageItem, String), UserType> = HashMap::new();
    for checked in packages.checked {
        let package = PackageId(checked.graph_index);
        let program = &checked.program;
        let base = symbols.scopes.len() as u32;
        let module = |id: ModuleId| ModuleId(base + id.0);
        // Named, in messages and in the paths written for it, by this
        // package's key for it, else by its own name.
        let name = packages
            .keys
            .iter()
            .find(|(_, target)| *target == DepTarget::Varyk(checked.graph_index))
            .map_or_else(|| checked.name.replace('-', "_"), |(key, _)| key.clone());
        let path = |id: ModuleId| package_path(program, id);
        for hir in &program.modules {
            let inside = path(hir.id);
            let scope_name = std::iter::once(name.clone())
                .chain(inside.iter().cloned())
                .collect::<Vec<_>>()
                .join("::");
            symbols.scopes.push(Scope {
                name: scope_name,
                parent: hir.parent.map(module),
                is_pub: hir.is_pub,
                decl: hir.decl,
                package: Some(package),
                ..Scope::default()
            });
        }
        for hir in &program.modules {
            if let Some(parent) = hir.parent {
                symbols.scopes[module(parent).0 as usize]
                    .children
                    .insert(hir.name.clone(), module(hir.id));
            }
        }
        roots.insert(checked.graph_index, module(program.entry));

        // Ids first, so field and payload types can name any of them.
        let item = |declared: &Option<PackageItem>, id: ModuleId| {
            declared.clone().unwrap_or_else(|| PackageItem {
                package,
                name: checked.name.clone(),
                path: path(id),
            })
        };
        let mut struct_ids = Vec::with_capacity(program.structs.len());
        let mut new_structs = Vec::new();
        for def in &program.structs {
            let at = item(&def.package, def.module);
            if let Some(UserType::Struct(id)) = types.get(&(at.clone(), def.name.clone())) {
                struct_ids.push(*id);
                continue;
            }
            // Declared in this package (or, which cannot happen, in one
            // not imported: then kept at its root).
            let home = if def.package.is_none() {
                module(def.module)
            } else {
                module(program.entry)
            };
            let id = StructId(symbols.structs.len() as u32);
            symbols.structs.push(StructDef {
                name: def.name.clone(),
                module: home,
                is_pub: def.is_pub,
                fields: Vec::new(),
                imported: def.imported,
                derives: def.imported.then_some(def.derives),
                package: Some(at.clone()),
                span: def.span,
            });
            if def.package.is_none() {
                symbols.scopes[home.0 as usize]
                    .types
                    .insert(def.name.clone(), UserType::Struct(id));
            }
            types.insert((at, def.name.clone()), UserType::Struct(id));
            struct_ids.push(id);
            new_structs.push((def, id));
        }
        let mut enum_ids = Vec::with_capacity(program.enums.len());
        let mut new_enums = Vec::new();
        for def in &program.enums {
            let at = item(&def.package, def.module);
            if let Some(UserType::Enum(id)) = types.get(&(at.clone(), def.name.clone())) {
                enum_ids.push(*id);
                continue;
            }
            let home = if def.package.is_none() {
                module(def.module)
            } else {
                module(program.entry)
            };
            let id = EnumId(symbols.enums.len() as u32);
            symbols.enums.push(EnumDef {
                name: def.name.clone(),
                module: home,
                is_pub: def.is_pub,
                variants: Vec::new(),
                imported: def.imported,
                derives: def.imported.then_some(def.derives),
                drops: def.drops.clone(),
                opaque: def.opaque.clone(),
                package: Some(at.clone()),
                span: def.span,
            });
            if def.package.is_none() {
                symbols.scopes[home.0 as usize]
                    .types
                    .insert(def.name.clone(), UserType::Enum(id));
            }
            types.insert((at, def.name.clone()), UserType::Enum(id));
            enum_ids.push(id);
            new_enums.push((def, id));
        }

        let ids = Ids {
            structs: &struct_ids,
            enums: &enum_ids,
        };
        for (def, id) in new_structs {
            let fields = def
                .fields
                .iter()
                .map(|field| FieldDef {
                    name: field.name.clone(),
                    ty: ids.ty(&field.ty),
                    is_pub: field.is_pub,
                    unusable: field.unusable.as_ref().map(|unusable| Unusable {
                        rust_ty: unusable.rust_ty.clone(),
                        note: unusable.note.clone(),
                        within: unusable.within.map(module),
                    }),
                    attrs: field.attrs.clone(),
                    span: field.span,
                })
                .collect();
            symbols.structs[id.0 as usize].fields = fields;
        }
        for (def, id) in new_enums {
            let variants = def
                .variants
                .iter()
                .map(|variant| VariantDef {
                    name: variant.name.clone(),
                    fields: match &variant.fields {
                        VariantFieldsDef::Tuple(types) => {
                            VariantFieldsDef::Tuple(types.iter().map(|ty| ids.ty(ty)).collect())
                        }
                        VariantFieldsDef::Named(fields) => VariantFieldsDef::Named(
                            fields
                                .iter()
                                .map(|(name, ty)| (name.clone(), ids.ty(ty)))
                                .collect(),
                        ),
                    },
                    rename: variant.rename.clone(),
                })
                .collect();
            symbols.enums[id.0 as usize].variants = variants;
        }

        for function in program
            .functions
            .iter()
            .filter(|function| !function.is_test)
        {
            let owner = function.owner.map(|owner| ids.owner(owner));
            let receiver = function
                .params
                .first()
                .filter(|param| owner.is_some() && param.name == "self");
            let params = function
                .params
                .iter()
                .skip(usize::from(receiver.is_some()))
                .map(|param| (ids.ty(&param.ty), param.mode))
                .collect();
            let sig = ImportedSig {
                name: function.name.clone(),
                module: module(function.module),
                owner,
                self_mode: receiver.map(|param| param.mode),
                params,
                literal: Vec::new(),
                serialize_param: None,
                variadic: false,
                ret: ids.ty(&function.ret),
                result_hole: None,
                ret_root: function.ret_root.map(|local| local.0 as usize),
                signature: String::new(),
                callable: true,
                note: None,
                within: None,
                redefined_in: None,
                is_async: function.is_async,
                names_std: false,
                private: (!function.is_pub).then_some(function.span),
                package: Some(item(&None, function.module)),
            };
            add_fn(symbols, sig);
        }
        // The functions of the package's own `.rs` modules; those it has
        // from another package are that package's.
        for sig in program.imported.iter().filter(|sig| sig.package.is_none()) {
            let sig = ImportedSig {
                module: module(sig.module),
                owner: sig.owner.map(|owner| ids.owner(owner)),
                params: sig
                    .params
                    .iter()
                    .map(|(ty, mode)| (ids.ty(ty), *mode))
                    .collect(),
                ret: ids.ty(&sig.ret),
                within: sig.within.map(module),
                package: Some(item(&None, sig.module)),
                ..sig.clone()
            };
            add_fn(symbols, sig);
        }
        // Its `pub use` lines, each the item of its own module under the
        // re-exporting module's name too (milestone 5b3 spec 2.5); a
        // `pub use` always names an item of the package, by its own path.
        for hir in &program.modules {
            for reexport in hir.uses.iter().filter(|use_| use_.is_pub) {
                let mut segments: Vec<&str> = reexport.path.split("::").skip(1).collect();
                let Some(name) = segments.pop() else {
                    continue;
                };
                let Some(declared) = program
                    .modules
                    .iter()
                    .find(|other| path(other.id) == segments)
                else {
                    continue;
                };
                let Some(scope) = symbols.scopes.get(module(declared.id).0 as usize) else {
                    continue;
                };
                let callee = scope.fns.get(name).copied();
                let found = scope.types.get(name).copied();
                let Some(scope) = symbols.scopes.get_mut(module(hir.id).0 as usize) else {
                    continue;
                };
                if let Some(callee) = callee {
                    scope.reexport_fns.insert(name.to_string(), callee);
                }
                if let Some(found) = found {
                    scope.reexport_types.insert(name.to_string(), found);
                }
            }
        }
    }
    roots
}

/// The `[package]` name whose `App` is the route table (milestone 5b4
/// spec 2.1).
const HTTP_PACKAGE: &str = "varyk-http";

/// The structs a `varyk-http` package names at its root for this compiler
/// (milestone 5b4 spec 6.1).
const HTTP_ITEMS: [&str; 3] = ["App", "Request", "Response"];

/// Marks the route table (milestone 5b4 spec 2.1, 7.2): for each package
/// of the build named `varyk-http`, and for the package being checked
/// when it is named so, its `App`, `Request`, and `Response`, looked up
/// as structs at its root, declared there or brought there by `pub use`,
/// enter `symbols.http`. A package this one lists that lacks any of them
/// is V0407, at its `Cargo.toml`; one it does not list cannot be named
/// here, so it is left as it is, and so is the package's own root, whose
/// `App`, when it has none, is no route table to call.
pub(super) fn mark_http(
    symbols: &mut Symbols,
    packages: Packages<'_>,
    sources: &[SourceFile],
) -> Vec<Diagnostic> {
    if packages.name == Some(HTTP_PACKAGE) {
        // The package's own root is the first module.
        if let Some(Ok(items)) = symbols.scopes.first().map(http_items) {
            symbols.http.push(items);
        }
    }
    let mut diagnostics = Vec::new();
    for checked in packages
        .checked
        .iter()
        .filter(|checked| checked.name == HTTP_PACKAGE)
    {
        let package = PackageId(checked.graph_index);
        let root = symbols
            .scopes
            .iter()
            .find(|scope| scope.package == Some(package) && scope.parent.is_none());
        let missing = match root.map(http_items) {
            Some(Ok(items)) => {
                symbols.http.push(items);
                continue;
            }
            Some(Err(missing)) => missing,
            None => HTTP_ITEMS.iter().map(|name| format!("`{name}`")).collect(),
        };
        let listed = packages
            .keys
            .iter()
            .any(|(_, target)| *target == DepTarget::Varyk(checked.graph_index));
        if listed {
            diagnostics.push(http_mismatch(checked, &missing, sources));
        }
    }
    diagnostics
}

/// The `varyk-http` items of the package whose root module is `scope`, or
/// the names, quoted, of the structs of [`HTTP_ITEMS`] it lacks.
fn http_items(scope: &Scope) -> Result<HttpItems, Vec<String>> {
    let found: Vec<Option<StructId>> = HTTP_ITEMS
        .iter()
        .map(
            |name| match scope.types.get(*name).or(scope.reexport_types.get(*name)) {
                Some(UserType::Struct(id)) => Some(*id),
                _ => None,
            },
        )
        .collect();
    let [Some(app), Some(request), Some(response)] = found[..] else {
        return Err(HTTP_ITEMS
            .iter()
            .zip(&found)
            .filter(|(_, found)| found.is_none())
            .map(|(name, _)| format!("`{name}`"))
            .collect());
    };
    let mut root: Vec<(String, StructId)> = scope
        .types
        .iter()
        .chain(&scope.reexport_types)
        .filter_map(|(name, found)| match found {
            UserType::Struct(id) => Some((name.clone(), *id)),
            UserType::Enum(_) => None,
        })
        .collect();
    root.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(HttpItems {
        app,
        request,
        response,
        root,
    })
}

/// V0407 for `checked`, a `varyk-http` without the structs `missing`, at
/// the first line of its `Cargo.toml`.
fn http_mismatch(
    checked: &crate::CheckedPackage,
    missing: &[String],
    sources: &[SourceFile],
) -> Diagnostic {
    let file = checked.manifest_file;
    let end = sources
        .get(file.0 as usize)
        .and_then(|source| source.text.lines().next())
        .map_or(0, str::len);
    let missing = match missing {
        [one] => format!("struct {one}"),
        [first, last] => format!("structs {first} or {last}"),
        _ => format!("structs {}", missing.join(", ")),
    };
    Diagnostic::new(
        codes::V0407,
        Span::new(file, 0, end as u32),
        "this `varyk-http` does not match this `varyk`",
    )
    .with_note(format!(
        "`varyk-http` {} has no {missing} at its root, which `varyk` {} writes calls to",
        checked.version,
        env!("CARGO_PKG_VERSION")
    ))
    .with_note(
        "the version table in `varyk-http`'s README says which `varyk-http` goes with each \
         `varyk`; use the one it names for this `varyk`",
    )
}

/// Adds `sig` to `symbols`: a free function to its module's functions, a
/// method or associated function to its type's.
fn add_fn(symbols: &mut Symbols, sig: ImportedSig) {
    let id = Callee::Imported(ImportedFnId(symbols.imported.len() as u32));
    match sig.owner {
        None => {
            symbols.scopes[sig.module.0 as usize]
                .fns
                .entry(sig.name.clone())
                .or_insert(id);
        }
        Some(owner) => {
            symbols
                .members
                .entry((owner, sig.name.clone()))
                .or_insert(id);
        }
    }
    symbols.imported.push(sig);
}

/// The modules from a package's root down to its module `id`:
/// `["length"]` for `crate::length`.
fn package_path(program: &HirProgram, id: ModuleId) -> Vec<String> {
    program
        .module_path(id)
        .split("::")
        .skip(1)
        .map(str::to_string)
        .collect()
}

/// A checked package's struct and enum ids in the tables it is imported
/// into, by its own ids.
struct Ids<'a> {
    structs: &'a [StructId],
    enums: &'a [EnumId],
}

impl Ids<'_> {
    /// `ty`, a type of the package, in the tables it is imported into.
    fn ty(&self, ty: &Ty) -> Ty {
        let inner = |ty: &Ty| Box::new(self.ty(ty));
        match ty {
            Ty::Struct(id) => Ty::Struct(self.structs[id.0 as usize]),
            Ty::Enum(id) => Ty::Enum(self.enums[id.0 as usize]),
            Ty::Option(t) => Ty::Option(inner(t)),
            Ty::Result(ok, err) => Ty::Result(inner(ok), inner(err)),
            Ty::Vec(t) => Ty::Vec(inner(t)),
            Ty::HashMap(key, value) => Ty::HashMap(inner(key), inner(value)),
            Ty::Chain(t) => Ty::Chain(inner(t)),
            Ty::Task(t) => Ty::Task(inner(t)),
            Ty::Shared(t) => Ty::Shared(inner(t)),
            Ty::Bool | Ty::Int(_) | Ty::Float(_) | Ty::String | Ty::Error | Ty::Unit => ty.clone(),
        }
    }

    fn owner(&self, owner: UserType) -> UserType {
        match owner {
            UserType::Struct(id) => UserType::Struct(self.structs[id.0 as usize]),
            UserType::Enum(id) => UserType::Enum(self.enums[id.0 as usize]),
        }
    }
}

/// Adds the help of M5b2 spec 2.1 to each V0100, V0111, and V0113 whose
/// path (or name) starts at the name of a dependency that the path does
/// not reach there: the line that renames the dependency in `Cargo.toml`.
pub(crate) fn rename_help(
    symbols: &Symbols,
    modules: &[Module],
    sources: &[SourceFile],
    diagnostics: &mut [Diagnostic],
) {
    for diagnostic in diagnostics.iter_mut() {
        if ![codes::V0100, codes::V0111, codes::V0113].contains(&diagnostic.code) {
            continue;
        }
        let span = diagnostic.span;
        let Some(text) = sources
            .get(span.file.0 as usize)
            .and_then(|source| source.text.get(span.start as usize..span.end as usize))
        else {
            continue;
        };
        // A single name is never a package (M5b2 spec 2.1), so a V0100 for
        // one is about something else.
        if diagnostic.code == codes::V0100 && !text.contains("::") {
            continue;
        }
        let first = text.split("::").next().unwrap_or(text);
        let Some(dep) = symbols.dep(first.trim()) else {
            continue;
        };
        let DepKind::Varyk(root) = dep.kind else {
            continue;
        };
        let Some(from) = modules.iter().find(|module| module.file == span.file) else {
            continue;
        };
        let name = &dep.name;
        let path = Path {
            leading: PathStart::None,
            segments: vec![Ident {
                name: name.clone(),
                span,
            }],
            span,
        };
        // A `use` starting at a module declared elsewhere in the package
        // is that module's V0111, even where the path would reach the
        // package.
        let elsewhere = diagnostic.code == codes::V0111 && symbols.module_named(name);
        let why = match symbols.module_at(from.id, &path) {
            Ok(module) if module == root && !elsewhere => continue,
            Ok(_) => format!("a module of this package is called `{name}`, and it comes first"),
            Err(_) if elsewhere => {
                format!("a module of this package is called `{name}`, and it comes first")
            }
            Err(_) => match unnameable(name) {
                Some(why) => why,
                None => continue,
            },
        };
        diagnostic.notes.push(format!(
            "`{name}` is also a dependency in `Cargo.toml`, but Varyk code cannot name it: {why}; \
             to use it, give it another name there, as in \
             `{name}_package = {{ package = \"{}\", .. }}`",
            dep.package
        ));
    }
}
