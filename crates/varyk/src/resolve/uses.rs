//! `use` declarations (spec 3.3): resolving what a `use` path names, the
//! V0110/V0111/V0103/V0105 diagnostics that can stop it, and registering
//! the local alias it introduces into `Scope.use_types` or
//! `Scope.use_fns` (split by namespace, spec 2.1, so a type alias and a
//! function alias of the same name coexist as they do in Rust).
//!
//! [`register`] runs once every module's own fns, types, children, and
//! `.rs` imports are known (`collect_symbols`'s "pass 1c", after imports
//! and before pass 2), so a `use` can name anything in the crate
//! regardless of file order; [`Symbols::lookup_fn`], [`Symbols::lookup_type`],
//! and [`Symbols::module_at`] then consult the alias it left behind for
//! the first segment of a later path.
//!
//! A `pub use` (milestone 5b3 spec 2.5) is also entered into its module's
//! `reexport_fns` or `reexport_types`, which a path ending at that module
//! consults after the module's own items; [`reexport_problem`] holds its
//! V0001 and V0105.

use std::collections::HashMap;

use varyk_syntax::{Ident, Item, Path, PathStart, Span, UseDecl};

use crate::diagnostics::{Diagnostic, codes};

use super::visibility::{check_visible, not_visible};
use super::{
    Callee, ENTRY_MODULE_NAME, LookupError, Module, ModuleId, ModuleKind, Symbols, UserType,
    duplicate, is_std_module, no_parent, path_text, reserved_type_name, reserved_value_name,
    split_last, std_fn_taken, std_module_taken, varyk_prefix_taken,
};

/// What a `use` name resolves to (spec 3.3): consulted by lookups before
/// the fns and types the file declares itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UseTarget {
    Module(ModuleId),
    Type(UserType),
    Fn(Callee),
}

impl Symbols {
    /// `module`'s path from the crate root (spec 3.1): `crate` for the
    /// root, `crate::shop::cart` for a nested one.
    fn module_path(&self, module: ModuleId) -> String {
        let scope = &self.scopes[module.0 as usize];
        // A module of another package: from the dependency's name, which
        // a leading `::` keeps clear of a module of this package (M5b2
        // spec 5).
        if scope.package.is_some() {
            return format!("::{}", scope.name);
        }
        if module == ModuleId(0) {
            scope.name.clone()
        } else {
            format!("{ENTRY_MODULE_NAME}::{}", scope.name)
        }
    }

    /// The canonical `crate::`-rooted path of the `use` named `name` in
    /// `module` (spec 3.3): the same path the backend writes for the item
    /// elsewhere, regardless of how the user wrote it. `None` when
    /// `module` has no `use` called `name` (it did not resolve, so
    /// `register` never stored it). `name` is looked up in both
    /// namespaces; when two declarations share a local name across
    /// namespaces (spec 2.1), this cannot tell them apart and returns
    /// whichever the type namespace holds — emission uses
    /// [`Self::use_target_path_at`] instead, which can.
    pub fn use_target_path(&self, module: ModuleId, name: &str) -> Option<String> {
        let scope = &self.scopes[module.0 as usize];
        let target = *scope
            .use_types
            .get(name)
            .or_else(|| scope.use_fns.get(name))?;
        Some(self.canonical_use_path(target))
    }

    /// The canonical `crate::`-rooted path of `module`'s `order`-th `use`
    /// declaration, in source order (spec 3.3): what `use` emission reads,
    /// since two declarations of the same module can introduce the same
    /// local name in different namespaces (spec 2.1, `use crate::a::Foo;`
    /// a type and `use crate::b::Foo;` a function) and a lookup by name
    /// alone cannot tell which declaration is being emitted. `register`
    /// records one entry here per `Item::Use`, in the same order, only
    /// ever for a `use` that resolved; `None` otherwise (which cannot
    /// happen for a module that reached `typecheck`, since `resolve`
    /// fails the whole program first if any `use` did not).
    pub fn use_target_path_at(&self, module: ModuleId, order: usize) -> Option<String> {
        let target = *self.scopes[module.0 as usize].use_order.get(order)?;
        Some(self.canonical_use_path(target))
    }

    fn canonical_use_path(&self, target: UseTarget) -> String {
        match target {
            UseTarget::Module(id) => self.module_path(id),
            UseTarget::Type(UserType::Struct(id)) => {
                let def = &self.structs[id.0 as usize];
                format!("{}::{}", self.module_path(def.module), def.name)
            }
            UseTarget::Type(UserType::Enum(id)) => {
                let def = &self.enums[id.0 as usize];
                format!("{}::{}", self.module_path(def.module), def.name)
            }
            UseTarget::Fn(Callee::Varyk(id)) => {
                let sig = &self.fns[id.0 as usize];
                format!("{}::{}", self.module_path(sig.module), sig.name)
            }
            UseTarget::Fn(Callee::Imported(id)) => {
                let imported = &self.imported[id.0 as usize];
                format!("{}::{}", self.module_path(imported.module), imported.name)
            }
            UseTarget::Fn(Callee::Builtin(_)) => {
                unreachable!("a `use` path never resolves to a built-in")
            }
        }
    }
}

/// Registers every module's `use` declarations into its `Scope.use_types`
/// or `Scope.use_fns` (spec 3.3): resolves what each names, rejects the
/// introduced local name exactly as any other declaration would (a
/// reserved or built-in name is V0103, spec 2.1; a clash with an item or
/// another `use` of this file is V0103 too), and pushes a diagnostic
/// instead for a path `resolve_use_path` cannot follow.
pub(super) fn register(
    symbols: &mut Symbols,
    modules: &[Module],
    diagnostics: &mut Vec<Diagnostic>,
) {
    seed_reexports(symbols, modules);
    for module in modules {
        let ModuleKind::Varyk(program) = &module.kind else {
            continue;
        };
        // Clashes between two `use` lines of this file: the item
        // namespaces themselves (`fns`, `types`, `children`) are read
        // straight off `symbols`, which `register` never mutates for this
        // module until an alias has cleared every check.
        let mut seen_fns: HashMap<&str, Span> = HashMap::new();
        let mut seen_types: HashMap<&str, Span> = HashMap::new();
        // Every name a `use` of this file introduces, with its path: a
        // `use` path cannot start at one, in any order.
        let introduced: HashMap<&str, &Path> = program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Use(decl) => Some((use_local(decl).name.as_str(), &decl.path)),
                _ => None,
            })
            .collect();
        for item in &program.items {
            let Item::Use(decl) = item else { continue };
            if let Some(diagnostic) = std_module_use(decl) {
                diagnostics.push(diagnostic);
                continue;
            }
            if let Some(diagnostic) = through_alias(symbols, module.id, decl, &introduced) {
                diagnostics.push(diagnostic);
                continue;
            }
            let local = use_local(decl);
            let targets = match resolve_use_path(symbols, module.id, decl, &introduced) {
                Ok(targets) => targets,
                Err(diagnostic) => {
                    diagnostics.push(diagnostic);
                    continue;
                }
            };
            // rustc drops a `#[test]` item outside a test build, so a `use`
            // of one would have nothing to point at (spec 2.7).
            let test_fn = targets.iter().any(|target| match target {
                UseTarget::Fn(Callee::Varyk(id)) => symbols.fns[id.0 as usize].is_test,
                _ => false,
            });
            if test_fn {
                let path = path_text(&decl.path);
                diagnostics.push(
                    Diagnostic::new(
                        codes::V0114,
                        decl.path.span,
                        format!("`{path}` is a test and cannot be imported"),
                    )
                    .with_note(
                        "`varyk test` runs each test on its own; put what the tests share in \
                         a function without `#[test]` and import that",
                    ),
                );
                continue;
            }
            if decl.is_pub {
                if let Some(diagnostic) = reexport_problem(symbols, decl, &targets) {
                    diagnostics.push(diagnostic);
                    continue;
                }
            }
            // The emitted `use` imports every namespace the name occupies
            // in its module (a function and a struct may share a name),
            // so each is an alias here, checked like any declaration: it
            // must not capture a reserved or built-in name (`Vec`,
            // `Option`, a primitive, ...), the same rule a struct, enum,
            // or function name follows, and for the same reason (the
            // generated Rust would hide Rust's own), nor clash with an
            // item or another `use` of this file (spec 2.1).
            let problem = targets.iter().find_map(|&target| {
                let reserved = match target {
                    UseTarget::Fn(_) => reserved_value_name(&local.name, local.span)
                        .or_else(|| std_fn_taken(&local.name, local.span)),
                    UseTarget::Module(_) => reserved_type_name(&local.name, local.span)
                        .or_else(|| std_module_taken(&local.name, local.span)),
                    UseTarget::Type(_) => reserved_type_name(&local.name, local.span)
                        .or_else(|| std_module_taken(&local.name, local.span)),
                }
                .or_else(|| varyk_prefix_taken(&local.name, local.span));
                if reserved.is_some() {
                    return reserved;
                }
                let scope = &symbols.scopes[module.id.0 as usize];
                let clash = match target {
                    UseTarget::Fn(_) => scope
                        .fns
                        .get(local.name.as_str())
                        .and_then(|&callee| callee_span(symbols, callee))
                        .or_else(|| seen_fns.get(local.name.as_str()).copied()),
                    UseTarget::Module(_) | UseTarget::Type(_) => scope
                        .types
                        .get(local.name.as_str())
                        .map(|&found| user_type_span(symbols, found))
                        .or_else(|| {
                            scope
                                .children
                                .get(local.name.as_str())
                                .map(|&child| mod_decl_span(symbols, child))
                        })
                        .or_else(|| seen_types.get(local.name.as_str()).copied()),
                };
                clash.map(|first| duplicate(&local.name, local.span, first))
            });
            if let Some(diagnostic) = problem {
                diagnostics.push(diagnostic);
                continue;
            }
            let module_scope = &mut symbols.scopes[module.id.0 as usize];
            for &target in &targets {
                match target {
                    UseTarget::Fn(_) => {
                        seen_fns.insert(&local.name, local.span);
                        module_scope.use_fns.insert(local.name.clone(), target);
                    }
                    UseTarget::Module(_) | UseTarget::Type(_) => {
                        seen_types.insert(&local.name, local.span);
                        module_scope.use_types.insert(local.name.clone(), target);
                    }
                }
            }
            // Recorded again, in source order, for `use_target_path_at`:
            // `use_types`/`use_fns` alone cannot tell two declarations
            // that share a local name across namespaces apart.
            // Every target shares one canonical path, so the first will do.
            module_scope.use_order.push(targets[0]);
        }
    }
}

/// Enters every `pub use` of every module that resolves into its module's
/// re-exports (milestone 5b3 spec 2.5) before any `use` is registered, so
/// a path through one resolves whichever file comes first: a `pub use` of
/// a `pub use` waits for the one it names, round after round, until a
/// round enters nothing. Nothing is reported here; [`register`] reports
/// every `use` that does not resolve, and the checks of
/// [`reexport_problem`], whose failing `pub use` it enters nothing for.
fn seed_reexports(symbols: &mut Symbols, modules: &[Module]) {
    let mut files = Vec::new();
    let mut pending = Vec::new();
    for module in modules {
        let ModuleKind::Varyk(program) = &module.kind else {
            continue;
        };
        let introduced: HashMap<&str, &Path> = program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Use(decl) => Some((use_local(decl).name.as_str(), &decl.path)),
                _ => None,
            })
            .collect();
        for item in &program.items {
            if let Item::Use(decl) = item {
                if decl.is_pub {
                    pending.push((module.id, decl, files.len()));
                }
            }
        }
        files.push(introduced);
    }
    loop {
        let before = pending.len();
        pending.retain(|&(module, decl, file)| {
            let Some(introduced) = files.get(file) else {
                return false;
            };
            if std_module_use(decl).is_some()
                || through_alias(symbols, module, decl, introduced).is_some()
            {
                return false;
            }
            let Ok(targets) = resolve_use_path(symbols, module, decl, introduced) else {
                return true;
            };
            if reexport_problem(symbols, decl, &targets).is_none() {
                add_reexports(symbols, module, &use_local(decl).name, &targets);
            }
            false
        });
        if pending.len() == before {
            break;
        }
    }
}

/// Enters `targets`, what a `pub use` of `module` named `name` resolved
/// to, into the module's re-exports.
fn add_reexports(symbols: &mut Symbols, module: ModuleId, name: &str, targets: &[UseTarget]) {
    let Some(scope) = symbols.scopes.get_mut(module.0 as usize) else {
        return;
    };
    for &target in targets {
        match target {
            UseTarget::Fn(callee) => {
                scope.reexport_fns.insert(name.to_string(), callee);
            }
            UseTarget::Type(found) => {
                scope.reexport_types.insert(name.to_string(), found);
            }
            UseTarget::Module(_) => {}
        }
    }
}

/// Why the `pub use` `decl`, resolved to `targets`, cannot re-export
/// them (milestone 5b3 spec 2.5): V0001 for a module or an item of
/// another package; V0105 for an item without `pub`, or one in a module
/// that is not `pub` all the way from the root, since the generated Rust
/// names it by its own path. `None` when it can.
fn reexport_problem(
    symbols: &Symbols,
    decl: &UseDecl,
    targets: &[UseTarget],
) -> Option<Diagnostic> {
    let path = path_text(&decl.path);
    let span = decl.path.span;
    let not_yet = |what: &str, note: String| {
        Diagnostic::new(
            codes::V0001,
            span,
            format!("`pub use` of {what} is not supported yet"),
        )
        .with_note(note)
    };
    let other_package = || {
        not_yet(
            "an item from another package",
            format!(
                "a package can re-export only its own items; a package that uses this one can \
                 name `{path}` itself by listing that package in its `Cargo.toml`"
            ),
        )
    };
    if targets
        .iter()
        .any(|target| matches!(target, UseTarget::Module(_)))
    {
        return Some(not_yet(
            "a module",
            format!(
                "re-export each item of `{path}` with its own `pub use`, or write `{path}::..` \
                 where it is used"
            ),
        ));
    }
    for &target in targets {
        // The item's module, and its declaration without `pub`, if so.
        let (module, private) = match target {
            UseTarget::Fn(Callee::Varyk(id)) => {
                let sig = symbols.fns.get(id.0 as usize)?;
                (
                    sig.module,
                    (!sig.is_pub).then_some(("function", sig.span, "fn")),
                )
            }
            UseTarget::Fn(Callee::Imported(id)) => {
                let sig = symbols.imported.get(id.0 as usize)?;
                if sig.package.is_some() {
                    return Some(other_package());
                }
                (sig.module, sig.private.map(|decl| ("function", decl, "fn")))
            }
            UseTarget::Type(UserType::Struct(id)) => {
                let def = symbols.structs.get(id.0 as usize)?;
                if def.package.is_some() {
                    return Some(other_package());
                }
                (
                    def.module,
                    (!def.is_pub).then_some(("struct", def.span, "struct")),
                )
            }
            UseTarget::Type(UserType::Enum(id)) => {
                let def = symbols.enums.get(id.0 as usize)?;
                if def.package.is_some() {
                    return Some(other_package());
                }
                (
                    def.module,
                    (!def.is_pub).then_some(("enum", def.span, "enum")),
                )
            }
            UseTarget::Fn(Callee::Builtin(_)) | UseTarget::Module(_) => continue,
        };
        if let Some((what, decl, keyword)) = private {
            return Some(not_visible(what, &path, span, decl, keyword));
        }
        if let Some(diagnostic) = symbols.reexport_through_private(module, &path, span) {
            return Some(diagnostic);
        }
    }
    None
}

/// V0113 for a `use` whose path starts at `json`, `env`, or `log` (M5a
/// spec 2.10): a standard module is reached only by its path at the call.
fn std_module_use(decl: &UseDecl) -> Option<Diagnostic> {
    let first = decl.path.segments.first()?;
    if decl.path.leading != PathStart::None || !is_std_module(&first.name) {
        return None;
    }
    let name = &first.name;
    Some(
        Diagnostic::new(
            codes::V0113,
            decl.path.span,
            format!("`{name}` is a standard module and cannot be brought in with `use`"),
        )
        .with_note(format!(
            "write the path where it is used, as in `{name}::..`, and remove this `use`"
        )),
    )
}

/// The local name a `use` introduces: its alias, else its last segment.
fn use_local(decl: &UseDecl) -> &Ident {
    decl.alias.as_ref().unwrap_or_else(|| {
        decl.path
            .segments
            .last()
            .expect("a `use` path has at least one segment")
    })
}

/// V0111 when `decl`'s path starts at a name another `use` of this file
/// introduces (`introduced`, with that `use`'s path) rather than at a
/// module declared here: a `use` path does not follow another `use`, in
/// either order, whether written bare, after `self::`, or, in the entry
/// file, after `crate::`. When that `use` names a module, the message
/// spells the path meant; when it names a type, a variant or function of
/// which `decl` goes on to, it says to write that through the type, as
/// for any `use` of a variant or a function of a type.
fn through_alias(
    symbols: &Symbols,
    from: ModuleId,
    decl: &UseDecl,
    introduced: &HashMap<&str, &Path>,
) -> Option<Diagnostic> {
    let written = match decl.path.leading {
        PathStart::None | PathStart::SelfMod => true,
        PathStart::Crate => from == ModuleId(0),
        PathStart::Super => false,
    };
    if !written {
        return None;
    }
    let first = decl.path.segments.first()?;
    if symbols.child(from, &first.name).is_some() {
        return None;
    }
    let target = *introduced.get(first.name.as_str())?;
    if std::ptr::eq(target, &decl.path) {
        return None;
    }
    let rest = &decl.path.segments[1..];
    if symbols.module_at(from, target).is_err() {
        // A type: going on to one of its variants or functions is the
        // usual V0111 about those, the type already in scope.
        let found = split_last(target)
            .and_then(|(sub, name)| symbols.lookup_type(from, sub.as_ref(), &name.name).ok());
        if let (Some(found), [member]) = (found, rest) {
            let prefix = Path {
                leading: PathStart::None,
                segments: vec![first.clone()],
                span: first.span,
            };
            if let UserType::Enum(id) = found {
                if symbols.enums[id.0 as usize].variant(&member.name).is_some() {
                    return Some(variant_error(&prefix, first, member, true));
                }
            }
            if symbols.members.contains_key(&(found, member.name.clone())) {
                return Some(member_error(&prefix, first, member, true));
            }
        }
        return Some(made_by_another_use(first, String::new()));
    }
    let mut meant = path_text(target);
    for segment in rest {
        meant.push_str("::");
        meant.push_str(&segment.name);
    }
    if let Some(alias) = &decl.alias {
        meant.push_str(&format!(" as {}", alias.name));
    }
    Some(made_by_another_use(
        first,
        format!("; write `use {meant};`"),
    ))
}

/// The V0111 of [`through_alias`] at `first`, a name another `use` made,
/// ending with `advice`.
fn made_by_another_use(first: &Ident, advice: String) -> Diagnostic {
    Diagnostic::new(
        codes::V0111,
        first.span,
        format!(
            "`{}` is a name made by another `use`; a `use` path starts with `crate::`, \
             `self::`, `super::`, or a module declared in this file{advice}",
            first.name
        ),
    )
}

fn callee_span(symbols: &Symbols, callee: Callee) -> Option<Span> {
    match callee {
        Callee::Varyk(id) => Some(symbols.fns[id.0 as usize].span),
        // Neither ever comes from an item declared in the same `.vr` file
        // a `use` alias could clash with: an imported fn belongs to a
        // `.rs` module (which has no `use`), and a built-in is never a
        // module's own item.
        Callee::Imported(_) | Callee::Builtin(_) => None,
    }
}

fn user_type_span(symbols: &Symbols, found: UserType) -> Span {
    match found {
        UserType::Struct(id) => symbols.structs[id.0 as usize].span,
        UserType::Enum(id) => symbols.enums[id.0 as usize].span,
    }
}

fn mod_decl_span(symbols: &Symbols, module: ModuleId) -> Span {
    symbols.scopes[module.0 as usize]
        .decl
        .expect("every module but the root has a `mod` declaration")
}

/// Resolves what a `use` path (spec 3.3) names, in every namespace: a
/// module when the whole path is one (`use shop::cart;`), else a type in
/// the module its segments before the last name; and a function of that
/// name there too, since the emitted Rust `use` imports both. `Err` for
/// anything the spec disallows: V0110 or V0111 for a leading name that is
/// not a module of this file, V0111 for a path ending at an enum variant,
/// V0100 for an otherwise-unknown module or name, V0105 for a private
/// item or module on the way to it (through `check_visible`, the only
/// place that is decided).
#[expect(
    clippy::result_large_err,
    reason = "a single diagnostic on a cold path, as `Symbols::resolve_type`"
)]
fn resolve_use_path(
    symbols: &Symbols,
    from: ModuleId,
    decl: &UseDecl,
    introduced: &HashMap<&str, &Path>,
) -> Result<Vec<UseTarget>, Diagnostic> {
    let path = &decl.path;
    if let Some(diagnostic) = dependency_use(symbols, from, decl) {
        return Err(diagnostic);
    }
    let (prefix, last) = split_last(path).expect("a `use` path has at least one segment");
    // The whole path might just name a module: try that first, since a
    // module and a type inside its parent never share a name (spec 3.1's
    // shared type namespace), so this can never mask the type case below.
    if let Ok(module) = symbols.module_at(from, path) {
        let mut targets = vec![module_target(symbols, from, module, path)?];
        if let Ok(callee) = symbols.lookup_fn(from, prefix.as_ref(), &last.name) {
            targets.push(UseTarget::Fn(callee));
        }
        return Ok(targets);
    }
    if let Some(prefix) = &prefix {
        match symbols.module_at(from, prefix) {
            Ok(_) => {}
            Err(LookupError::NoParent { span }) => return Err(no_parent(span)),
            Err(LookupError::Dependency { span, dep }) => {
                return Err(symbols.dependency_error(span, dep));
            }
            Err(LookupError::Unknown) => {
                return Err(unresolved_leading(
                    symbols,
                    from,
                    prefix,
                    last,
                    decl.alias.as_ref(),
                    introduced,
                ));
            }
            Err(_) => {
                unreachable!("`module_at` returns only `Unknown`, `NoParent`, or `Dependency`")
            }
        }
    }
    item_target(symbols, from, prefix.as_ref(), last, path).map_err(|error| {
        // A bare `use shop;` of a module declared in another file.
        if path.leading == PathStart::None && prefix.is_none() && symbols.module_named(&last.name) {
            module_elsewhere(symbols, last, &[], decl.alias.as_ref())
        } else {
            error
        }
    })
}

/// `path` names module `module` in full (`use shop::cart;`,
/// `use crate::shop;`): visible exactly when [`check_visible`] with no
/// item-level check says so, since a module's own `pub` is already part
/// of the chain it walks.
#[expect(
    clippy::result_large_err,
    reason = "a single diagnostic on a cold path, as `Symbols::resolve_type`"
)]
fn module_target(
    symbols: &Symbols,
    from: ModuleId,
    module: ModuleId,
    path: &Path,
) -> Result<UseTarget, Diagnostic> {
    match check_visible(symbols, from, module, None) {
        None => Ok(UseTarget::Module(module)),
        Some(LookupError::PrivateModule { module }) => {
            Err(symbols.private_module(module, "module", &path_text(path), path.span))
        }
        Some(_) => unreachable!("`check_visible` with no item check returns only `PrivateModule`"),
    }
}

/// `last`, the segment after `prefix` (the module `prefix` names, `None`
/// for `from` itself), looked up as a function and as a type: whatever
/// [`Symbols::lookup_fn`] and [`Symbols::lookup_type`] find, each with
/// its own visibility check, which fails the `use` only when neither is
/// found (Rust imports the namespaces it can see). Neither is a variant or an associated
/// function of a type (spec 3.3 stops the search at a module's own fns
/// and types).
#[expect(
    clippy::result_large_err,
    reason = "a single diagnostic on a cold path, as `Symbols::resolve_type`"
)]
fn item_target(
    symbols: &Symbols,
    from: ModuleId,
    prefix: Option<&Path>,
    last: &Ident,
    path: &Path,
) -> Result<Vec<UseTarget>, Diagnostic> {
    let full = path_text(path);
    let mut targets = Vec::new();
    let mut error = None;
    match symbols.lookup_fn(from, prefix, &last.name) {
        Ok(callee) => targets.push(UseTarget::Fn(callee)),
        Err(LookupError::Unknown) => {}
        Err(found) => {
            error = Some(use_lookup_error(
                symbols, found, "function", &full, path.span,
            ));
        }
    }
    match symbols.lookup_type(from, prefix, &last.name) {
        Ok(found) => targets.push(UseTarget::Type(found)),
        // A `.rs` struct or enum Varyk did not import (M3 spec 4.1, 4.3):
        // V0101 saying why.
        Err(LookupError::Unknown) if targets.is_empty() && error.is_none() => {
            return Err(symbols
                .skipped_item(from, prefix, &last.name, &full, path.span)
                .unwrap_or_else(|| {
                    Diagnostic::new(codes::V0100, path.span, format!("cannot find `{full}`"))
                }));
        }
        Err(LookupError::Unknown) => {}
        Err(found) => {
            error.get_or_insert_with(|| use_lookup_error(symbols, found, "type", &full, path.span));
        }
    }
    match (targets.is_empty(), error) {
        (true, Some(error)) => Err(error),
        _ => Ok(targets),
    }
}

fn use_lookup_error(
    symbols: &Symbols,
    error: LookupError,
    what: &str,
    path: &str,
    span: Span,
) -> Diagnostic {
    match error {
        LookupError::NotVisible { decl, keyword } => not_visible(what, path, span, decl, keyword),
        LookupError::PrivateModule { module } => symbols.private_module(module, what, path, span),
        LookupError::NoParent { span } => no_parent(span),
        LookupError::Dependency { span, dep } => symbols.dependency_error(span, dep),
        LookupError::Unknown => unreachable!("the caller handles `Unknown` itself"),
    }
}

/// `prefix` (the segments of a `use` path before `last`) failed to
/// resolve as a module chain from `from`: decides which of the spec 3.3
/// diagnostics that is. Its last segment might actually be a type, with
/// `last` one of its variants or associated functions (a `use` cannot
/// end there, V0111); a
/// bare first segment that is not a module declared in this file goes to
/// [`leading_name_error`]; anything else is the ordinary "cannot find
/// module".
fn unresolved_leading(
    symbols: &Symbols,
    from: ModuleId,
    prefix: &Path,
    last: &Ident,
    alias: Option<&Ident>,
    introduced: &HashMap<&str, &Path>,
) -> Diagnostic {
    if let Some((sub, name)) = split_last(prefix) {
        match symbols.lookup_type(from, sub.as_ref(), &name.name) {
            Ok(found @ UserType::Enum(id))
                if symbols.enums[id.0 as usize].variant(&last.name).is_some() =>
            {
                let in_scope = in_scope(symbols, from, name, found, introduced);
                return variant_error(prefix, name, last, in_scope);
            }
            Ok(found) if symbols.members.contains_key(&(found, last.name.clone())) => {
                let in_scope = in_scope(symbols, from, name, found, introduced);
                return member_error(prefix, name, last, in_scope);
            }
            _ => {}
        }
    }
    if prefix.leading == PathStart::None {
        let first = &prefix.segments[0];
        if symbols.child(from, &first.name).is_none() {
            return leading_name_error(symbols, from, prefix, first, last, alias);
        }
    }
    super::resolve_path(symbols, from, prefix)
        .expect_err("`module_at` already failed on this prefix")
}

/// V0111 at `type_name::variant`: a `use` cannot end at an enum variant
/// (spec 3.3); it names the enum, and a path or a pattern names the
/// variant through it. `prefix` is the path to the enum.
fn variant_error(prefix: &Path, type_name: &Ident, variant: &Ident, in_scope: bool) -> Diagnostic {
    Diagnostic::new(
        codes::V0111,
        Span::new(prefix.span.file, type_name.span.start, variant.span.end),
        format!(
            "`use` cannot name a variant; {}`{}::{}`",
            use_the_type_first(prefix, type_name, in_scope),
            type_name.name,
            variant.name
        ),
    )
}

/// V0111 at `type_name::member`: a `use` cannot reach inside a type to
/// its method or associated function (spec 3.3); it names the type, and a
/// call names the function through it. `prefix` is the path to the type.
fn member_error(prefix: &Path, type_name: &Ident, member: &Ident, in_scope: bool) -> Diagnostic {
    Diagnostic::new(
        codes::V0111,
        Span::new(prefix.span.file, type_name.span.start, member.span.end),
        format!(
            "`use` cannot name a function of a type; {}`{}::{}()`",
            use_the_type_first(prefix, type_name, in_scope),
            type_name.name,
            member.name
        ),
    )
}

/// Whether `type_name` already names `found` in module `from`: the
/// module declares it, or a `use` of its file (`introduced`, each name
/// with its `use` path, in any order) brings it in.
fn in_scope(
    symbols: &Symbols,
    from: ModuleId,
    type_name: &Ident,
    found: UserType,
    introduced: &HashMap<&str, &Path>,
) -> bool {
    symbols.scopes[from.0 as usize].types.get(&type_name.name) == Some(&found)
        || introduced
            .get(type_name.name.as_str())
            .and_then(|path| split_last(path))
            .is_some_and(|(sub, name)| {
                symbols.lookup_type(from, sub.as_ref(), &name.name) == Ok(found)
            })
}

/// The start of the advice of [`variant_error`] and [`member_error`]: a
/// `use` of the type at `prefix` and then its path, or just the path when
/// the type is already in scope (`in_scope`, or written as a bare name).
fn use_the_type_first(prefix: &Path, type_name: &Ident, in_scope: bool) -> String {
    if in_scope || prefix.leading == PathStart::None && prefix.segments.len() == 1 {
        format!("`{}` can already be used here, so write ", type_name.name)
    } else {
        format!("write `use {};` and then ", path_text(prefix))
    }
}

/// V0111 at `name`, the bare start of a `use` path and a module declared
/// elsewhere in the package: the note spells the `use` from `crate::`,
/// with the segments after `name` (`rest`) and the `as` name, when there
/// is one module of that name.
fn module_elsewhere(
    symbols: &Symbols,
    name: &Ident,
    rest: &[&Ident],
    alias: Option<&Ident>,
) -> Diagnostic {
    let fix = match symbols.modules_named(&name.name)[..] {
        [module] => {
            let mut path = format!("crate::{module}");
            for segment in rest {
                path.push_str("::");
                path.push_str(&segment.name);
            }
            if let Some(alias) = alias {
                path.push_str(&format!(" as {}", alias.name));
            }
            format!("write `use {path};`")
        }
        _ => "write its path from `crate::`".to_string(),
    };
    Diagnostic::new(
        codes::V0111,
        name.span,
        format!(
            "no module `{}` is declared in this file; a `use` path starts with `crate::`, \
             `self::`, `super::`, or a module declared in this file",
            name.name
        ),
    )
    .with_note(format!(
        "`{}` is declared elsewhere in this package; {fix}",
        name.name
    ))
}

/// Whether `name` is a crate this compiler recognizes by name: the ones
/// the standard library is split into, and the package's own
/// `[dependencies]`, as Rust code names them. An unrecognized name is not
/// assumed to be a crate (spec 3.3's V0110 is for a *known* crate only,
/// so a typo of a local module name is not misreported as one).
fn is_known_crate_name(symbols: &Symbols, name: &str) -> bool {
    matches!(name, "std" | "core" | "alloc") || symbols.dep(name).is_some()
}

/// The diagnostic for a `use` whose path starts at a dependency of the
/// package, written bare and not a module of this file (M5b2 spec 2.1,
/// 2.2): V0110 or V0401 for one Varyk code cannot name, and V0111 for
/// a Varyk package's name alone, which is already in scope; and the V0111
/// of a module declared elsewhere in the package, which comes first.
/// `None` for anything else.
fn dependency_use(symbols: &Symbols, from: ModuleId, decl: &UseDecl) -> Option<Diagnostic> {
    let first = decl.path.segments.first()?;
    if decl.path.leading != PathStart::None
        || symbols.child(from, &first.name).is_some()
        || symbols.dep(&first.name).is_none()
    {
        return None;
    }
    if symbols.module_named(&first.name) {
        let rest: Vec<&Ident> = decl.path.segments[1..].iter().collect();
        return Some(module_elsewhere(symbols, first, &rest, decl.alias.as_ref()));
    }
    match symbols.dependency_root(first) {
        Err(LookupError::Dependency { span, dep }) => Some(symbols.dependency_error(span, dep)),
        Ok(_) if decl.path.segments.len() == 1 => Some(
            Diagnostic::new(
                codes::V0111,
                decl.path.span,
                format!(
                    "`{}` is a package this package depends on, so its name can already be \
                     used here",
                    first.name
                ),
            )
            .with_note(format!(
                "write paths that start with `{}::` where they are needed, and remove this \
                 `use`; to call the package by another name, rename it in `Cargo.toml`",
                first.name
            )),
        ),
        _ => None,
    }
}

/// V0111, V0110, or the ordinary "cannot find module" at `name`, a bare
/// leading segment of `prefix` that is not a module declared in this
/// file (spec 3.3): V0111 when `name` is a module declared elsewhere in
/// the crate ("write `crate::`"); V0110, with the facade note, when
/// `name` is a crate this compiler recognizes; otherwise the same
/// "cannot find module" diagnostic an ordinary path gets, with a note in
/// case `name` was meant to be a crate this compiler does not yet know
/// by name.
fn leading_name_error(
    symbols: &Symbols,
    from: ModuleId,
    prefix: &Path,
    name: &Ident,
    last: &Ident,
    alias: Option<&Ident>,
) -> Diagnostic {
    if symbols.module_named(&name.name) {
        let mut rest: Vec<&Ident> = prefix.segments[1..].iter().collect();
        rest.push(last);
        return module_elsewhere(symbols, name, &rest, alias);
    }
    if is_known_crate_name(symbols, &name.name) {
        return Diagnostic::new(
            codes::V0110,
            name.span,
            format!("`use` cannot name the crate `{}`", name.name),
        )
        .with_note(
            "Varyk code does not use crates directly; call it from a `.rs` module in this package",
        );
    }
    let diagnostic = super::resolve_path(symbols, from, prefix)
        .expect_err("`module_at` already failed on this prefix");
    let path = format!("{}::{}", path_text(prefix), last.name);
    match symbols.not_added(from, &path) {
        Some(note) => diagnostic.with_note(note),
        None => diagnostic.with_note(format!(
            "if `{}` is a crate: Varyk code does not use crates directly; call it from a `.rs` \
             module in this package",
            name.name
        )),
    }
}
