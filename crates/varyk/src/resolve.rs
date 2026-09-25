//! The resolver: loads the entry file and its `mod` files, collects every
//! module's symbols, resolves signature and field types, and enforces
//! visibility (spec sections 4.1 and 4.5).
//!
//! [`resolve`] owns lexing and parsing of every file, entry included, with
//! the per-file stop rule: a file with syntax errors contributes only those
//! errors, and resolution stops after module loading if any file had them.
//! Expression paths and locals are resolved later, during lowering in the
//! type checker, through [`Symbols::lookup_fn`], [`Symbols::lookup_type`],
//! [`Symbols::lookup_member`], and [`Symbols::resolve_type`].

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fs;
use std::path::Path;

use varyk_syntax::{
    FileId, FixIt, Function, ImplBlock, Item, ModDecl, Program, SelfMode, SourceFile, Span,
    TypeExpr, parse_source,
};

use crate::builtins::BuiltinId;
use crate::diagnostics::{Diagnostic, codes};
use crate::interop::{ImportedFn, RustTy, import_rust_module};
use crate::types::{FloatKind, IntKind, ParamMode, Ty};

/// The name given to the entry module. It can never collide with a `mod`
/// name (`crate` is a keyword), and module-qualified lookups never find the
/// entry module: milestone-1 paths name only the entry's `mod`s.
pub const ENTRY_MODULE_NAME: &str = "crate";

/// Index into [`Resolved::modules`]; the entry module is always `ModuleId(0)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModuleId(pub u32);

/// Index into [`Symbols::fns`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FnId(pub u32);

/// Index into [`Symbols::imported`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImportedFnId(pub u32);

/// Index into [`Symbols::structs`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StructId(pub u32);

/// Index into [`Symbols::enums`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EnumId(pub u32);

/// A user-declared type: what a type name, a struct literal's path, and
/// an `impl` block resolve to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UserType {
    Struct(StructId),
    Enum(EnumId),
}

impl UserType {
    pub fn ty(self) -> Ty {
        match self {
            UserType::Struct(id) => Ty::Struct(id),
            UserType::Enum(id) => Ty::Enum(id),
        }
    }
}

/// What a module was loaded from.
#[derive(Debug, Clone, PartialEq)]
pub enum ModuleKind {
    /// A `.vr` file (or the entry file), parsed.
    Varyk(Program),
    /// A `.rs` file's source text, copied verbatim into the generated crate.
    Rust(String),
}

/// One loaded module: the entry file or a file named by a `mod` in it.
#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub id: ModuleId,
    /// The `mod` name, or [`ENTRY_MODULE_NAME`] for the entry module.
    pub name: String,
    pub kind: ModuleKind,
    pub file: FileId,
}

/// A Varyk function's resolved signature.
#[derive(Debug, Clone, PartialEq)]
pub struct FnSig {
    pub name: String,
    pub module: ModuleId,
    pub is_pub: bool,
    /// Name, type, and mode of each parameter (spec 4.2).
    pub params: Vec<(String, Ty, ParamMode)>,
    /// [`Ty::Unit`] when no return type is written.
    pub ret: Ty,
    /// The type of the `impl` block declaring the function; `None` for a
    /// free function (and for one in an `impl` block naming no type of
    /// its file, which is an error).
    pub owner: Option<UserType>,
    /// A method's receiver mode (spec 2.5): `self` is a shared borrow,
    /// `mut self` a mutable one. `None` for a free or associated
    /// function. The receiver is not in `params`.
    pub self_mode: Option<ParamMode>,
    /// Index of the declaring [`Item::Function`] or [`Item::Impl`] in the
    /// module's `Program::items`; see [`Resolved::fn_decl`].
    pub item: usize,
    /// For a function of an `impl` block, its index in
    /// [`ImplBlock::functions`].
    pub member: Option<usize>,
    /// The whole declaration, starting at `fn` (or `pub`).
    pub span: Span,
}

/// A `pub fn` imported from a `.rs` module, mapped per the spec 4.5 table.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedSig {
    pub name: String,
    pub module: ModuleId,
    /// Empty when not `callable`: an uncallable signature is never
    /// type-checked.
    pub params: Vec<(Ty, ParamMode)>,
    /// [`Ty::Unit`] when not `callable`.
    pub ret: Ty,
    /// The original Rust signature text, for the V0108 diagnostic.
    pub signature: String,
    pub callable: bool,
}

/// A struct's resolved definition.
#[derive(Debug, Clone, PartialEq)]
pub struct StructDef {
    pub name: String,
    pub module: ModuleId,
    pub is_pub: bool,
    pub fields: Vec<(String, Ty)>,
    /// The whole declaration, starting at `struct` (or `pub`).
    pub span: Span,
}

/// An enum's resolved definition (spec 2.2).
#[derive(Debug, Clone, PartialEq)]
pub struct EnumDef {
    pub name: String,
    pub module: ModuleId,
    pub is_pub: bool,
    /// Each variant's name and payload types, in declaration order; a
    /// unit variant has no payload.
    pub variants: Vec<(String, Vec<Ty>)>,
    /// The whole declaration, starting at `enum` (or `pub`).
    pub span: Span,
}

impl EnumDef {
    /// The index of the variant called `name`.
    pub fn variant(&self, name: &str) -> Option<usize> {
        self.variants
            .iter()
            .position(|(variant, _)| variant == name)
    }
}

/// A resolved function reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Callee {
    Varyk(FnId),
    Imported(ImportedFnId),
    /// An associated function of the built-in table: `Vec::new`.
    Builtin(BuiltinId),
}

/// Why a lookup failed. The type checker turns these into diagnostics at the use
/// site: `Unknown` is V0100, `NotVisible` is V0105.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookupError {
    /// No such module, or no such item in it.
    Unknown,
    /// The item exists in another module but is not `pub`. Holds the span
    /// of its declaration, whose start is where a `pub ` fix-it inserts,
    /// and the keyword it starts with (`fn`, `struct`, or `enum`).
    NotVisible { decl: Span, keyword: &'static str },
}

/// Every module's functions, imported functions, structs, and enums, and
/// every type's methods and associated functions.
#[derive(Debug, Clone, Default)]
pub struct Symbols {
    pub fns: Vec<FnSig>,
    pub imported: Vec<ImportedSig>,
    pub structs: Vec<StructDef>,
    pub enums: Vec<EnumDef>,
    /// The functions of every type's `impl` blocks, by type then name.
    members: HashMap<(UserType, String), FnId>,
    /// Per-module name tables, indexed by `ModuleId`.
    scopes: Vec<Scope>,
}

#[derive(Debug, Clone, Default)]
struct Scope {
    name: String,
    fns: HashMap<String, Callee>,
    /// Structs and enums, which share Rust's type namespace.
    types: HashMap<String, UserType>,
}

impl Symbols {
    /// Looks up `name` (or `module::name`) as seen from module `from`. An
    /// item in another module must be `pub`; imported Rust functions are
    /// always `pub`.
    pub fn lookup_fn(
        &self,
        from: ModuleId,
        module: Option<&str>,
        name: &str,
    ) -> Result<Callee, LookupError> {
        let target = self.target_module(from, module)?;
        let callee = *self.scopes[target.0 as usize]
            .fns
            .get(name)
            .ok_or(LookupError::Unknown)?;
        if let Callee::Varyk(id) = callee {
            let sig = &self.fns[id.0 as usize];
            if target != from && !sig.is_pub {
                return Err(fn_not_visible(sig));
            }
        }
        Ok(callee)
    }

    /// Looks up a struct or enum `name` (or `module::name`) as seen from
    /// module `from` (spec 2.10). A type in another module must be `pub`.
    pub fn lookup_type(
        &self,
        from: ModuleId,
        module: Option<&str>,
        name: &str,
    ) -> Result<UserType, LookupError> {
        let target = self.target_module(from, module)?;
        let found = *self.scopes[target.0 as usize]
            .types
            .get(name)
            .ok_or(LookupError::Unknown)?;
        let (is_pub, decl, keyword) = match found {
            UserType::Struct(id) => {
                let def = &self.structs[id.0 as usize];
                (def.is_pub, def.span, "struct")
            }
            UserType::Enum(id) => {
                let def = &self.enums[id.0 as usize];
                (def.is_pub, def.span, "enum")
            }
        };
        if target != from && !is_pub {
            return Err(LookupError::NotVisible { decl, keyword });
        }
        Ok(found)
    }

    /// Looks up a struct by name in module `from` itself.
    pub fn lookup_struct(&self, from: ModuleId, name: &str) -> Option<StructId> {
        match self.scopes[from.0 as usize].types.get(name) {
            Some(UserType::Struct(id)) => Some(*id),
            _ => None,
        }
    }

    /// Looks up a method or associated function `name` of type `owner` as
    /// seen from module `from` (spec 2.5): one declared in another module
    /// must be `pub`.
    pub fn lookup_member(
        &self,
        from: ModuleId,
        owner: UserType,
        name: &str,
    ) -> Result<FnId, LookupError> {
        let id = *self
            .members
            .get(&(owner, name.to_string()))
            .ok_or(LookupError::Unknown)?;
        let sig = &self.fns[id.0 as usize];
        if sig.module != from && !sig.is_pub {
            return Err(fn_not_visible(sig));
        }
        Ok(id)
    }

    /// Resolves a written type as seen from module `from`: a primitive
    /// name, `string`, a struct or enum visible from `from` (through a
    /// module, `m::User`, when it is `pub`), or `Option`, `Result`, or
    /// `Vec` with the right number of type arguments (spec 2.6). `String`
    /// and `str` are V0107 with a fix-it to `string`; an unknown name or a
    /// wrong argument count is V0101.
    #[expect(
        clippy::result_large_err,
        reason = "a single diagnostic on a cold path; the type checker pushes it straight into its list"
    )]
    pub fn resolve_type(&self, ty: &TypeExpr, from: ModuleId) -> Result<Ty, Diagnostic> {
        let name = ty.name.name.as_str();
        let span = ty.name.span;
        if let Some((arity, shape)) = generic_shape(name).filter(|_| ty.module.is_none()) {
            if ty.args.len() != arity {
                return Err(Diagnostic::new(
                    codes::V0101,
                    ty.span,
                    format!(
                        "`{name}` takes {} type{}, written `{shape}`",
                        if arity == 1 { "one" } else { "two" },
                        if arity == 1 { "" } else { "s" },
                    ),
                ));
            }
            let mut args = Vec::new();
            for arg in &ty.args {
                args.push(self.resolve_type(arg, from)?);
            }
            let mut args = args.into_iter().map(Box::new);
            let mut next = || args.next().expect("the argument count was checked");
            return Ok(match name {
                "Option" => Ty::Option(next()),
                "Vec" => Ty::Vec(next()),
                _ => Ty::Result(next(), next()),
            });
        }
        if !ty.args.is_empty() {
            return Err(Diagnostic::new(
                codes::V0001,
                ty.span,
                format!("`{name}` takes no type arguments; only `Option`, `Result`, and `Vec` do"),
            ));
        }
        if let Some(module) = &ty.module {
            let path = format!("{}::{name}", module.name);
            return match self.lookup_type(from, Some(&module.name), name) {
                Ok(found) => Ok(found.ty()),
                Err(LookupError::Unknown) => Err(Diagnostic::new(
                    codes::V0101,
                    ty.span,
                    format!("unknown type `{path}`"),
                )),
                Err(LookupError::NotVisible { decl, keyword }) => {
                    Err(not_visible("type", &path, ty.span, decl, keyword))
                }
            };
        }
        if let Some(prim) = Ty::from_primitive_name(name) {
            return Ok(prim);
        }
        if name == "String" || name == "str" {
            return Err(Diagnostic::new(
                codes::V0107,
                span,
                format!("`{name}` is not a Varyk type; Varyk's string type is `string`"),
            )
            .with_fix_it(FixIt {
                span,
                replacement: "string".to_string(),
            }));
        }
        self.lookup_type(from, None, name)
            .map(UserType::ty)
            .map_err(|_| {
                let mut diagnostic =
                    Diagnostic::new(codes::V0101, span, format!("unknown type `{name}`"));
                if let Some(note) = self.did_you_mean(from, name) {
                    diagnostic = diagnostic.with_note(note);
                }
                diagnostic
            })
    }

    /// Modules other than `from` that declare a type or a function named
    /// `name`, for [`Self::did_you_mean`].
    fn modules_declaring(&self, from: ModuleId, name: &str) -> Vec<&str> {
        self.scopes
            .iter()
            .enumerate()
            .filter(|&(id, scope)| {
                id as u32 != from.0
                    && (scope.types.contains_key(name) || scope.fns.contains_key(name))
            })
            .map(|(_, scope)| scope.name.as_str())
            .collect()
    }

    /// A "did you mean `module::name`?" note naming up to two other
    /// modules that declare a type or a function called `name`: `None`
    /// when none do. For an unknown type (V0101) or an unknown value or
    /// associated call whose leading segment was taken for a module
    /// (V0100), written bare instead of through its module.
    pub fn did_you_mean(&self, from: ModuleId, name: &str) -> Option<String> {
        let modules = self.modules_declaring(from, name);
        if modules.is_empty() {
            return None;
        }
        let paths: Vec<String> = modules
            .iter()
            .take(2)
            .map(|module| format!("`{module}::{name}`"))
            .collect();
        Some(format!("did you mean {}?", paths.join(" or ")))
    }

    /// V0105 when a `pub` item's resolved type `ty`, written at `span`,
    /// names a non-`pub` struct or enum, itself or inside `Option`,
    /// `Result`, or `Vec`: every caller in another module would be
    /// rejected by rustc. A type of another module is always `pub` here,
    /// since resolving it required that.
    fn private_in_public(&self, ty: &Ty, span: Span) -> Option<Diagnostic> {
        let (name, is_pub, decl, keyword) = match ty {
            Ty::Option(inner) | Ty::Vec(inner) => return self.private_in_public(inner, span),
            Ty::Result(ok, err) => {
                return self
                    .private_in_public(ok, span)
                    .or_else(|| self.private_in_public(err, span));
            }
            Ty::Struct(id) => {
                let def = &self.structs[id.0 as usize];
                (&def.name, def.is_pub, def.span, "struct")
            }
            Ty::Enum(id) => {
                let def = &self.enums[id.0 as usize];
                (&def.name, def.is_pub, def.span, "enum")
            }
            _ => return None,
        };
        if is_pub {
            return None;
        }
        let diagnostic = Diagnostic::new(
            codes::V0105,
            span,
            format!(
                "`{name}` is used by a public item but is not public; add `pub` to the {keyword}"
            ),
        );
        Some(declared_without_pub(diagnostic, decl, keyword))
    }

    /// `from` itself for an unqualified name; otherwise the non-entry
    /// module called `module`.
    fn target_module(&self, from: ModuleId, module: Option<&str>) -> Result<ModuleId, LookupError> {
        let Some(module) = module else {
            return Ok(from);
        };
        self.scopes
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, scope)| scope.name == module)
            .map(|(index, _)| ModuleId(index as u32))
            .ok_or(LookupError::Unknown)
    }
}

/// A function in another module that is not `pub`.
fn fn_not_visible(sig: &FnSig) -> LookupError {
    LookupError::NotVisible {
        decl: sig.span,
        keyword: "fn",
    }
}

/// The argument count and written shape of `Option`, `Result`, and `Vec`;
/// `None` for any other name.
fn generic_shape(name: &str) -> Option<(usize, &'static str)> {
    match name {
        "Option" => Some((1, "Option<T>")),
        "Vec" => Some((1, "Vec<T>")),
        "Result" => Some((2, "Result<T, E>")),
        _ => None,
    }
}

/// V0105 at `span` for `what` (`function`, `type`, ...) `path`, used
/// outside its module although its declaration at `decl`, starting with
/// `keyword`, is not `pub`.
pub(crate) fn not_visible(
    what: &str,
    path: &str,
    span: Span,
    decl: Span,
    keyword: &str,
) -> Diagnostic {
    let diagnostic = Diagnostic::new(
        codes::V0105,
        span,
        format!(
            "{what} `{path}` is not marked `pub`, so it can only be used inside its own module"
        ),
    );
    declared_without_pub(diagnostic, decl, keyword)
}

/// Labels the `keyword` (`fn`, `struct`, or `enum`) of the non-`pub` declaration at
/// `decl`, rather than underlining its whole body, and adds a `pub ` fix-it.
pub(crate) fn declared_without_pub(
    diagnostic: Diagnostic,
    decl: Span,
    keyword: &str,
) -> Diagnostic {
    let keyword_span = Span::new(decl.file, decl.start, decl.start + keyword.len() as u32);
    diagnostic
        .with_label(keyword_span, "declared here without `pub`")
        .with_fix_it(FixIt {
            span: Span::new(decl.file, decl.start, decl.start),
            replacement: "pub ".to_string(),
        })
}

/// The resolver's output: every loaded module and every module's symbols.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub modules: Vec<Module>,
    pub symbols: Symbols,
    pub entry: ModuleId,
}

impl Resolved {
    /// The AST of a Varyk function.
    pub fn fn_decl(&self, id: FnId) -> &Function {
        let sig = &self.symbols.fns[id.0 as usize];
        let ModuleKind::Varyk(program) = &self.modules[sig.module.0 as usize].kind else {
            unreachable!("a Varyk function always lives in a Varyk module");
        };
        match (&program.items[sig.item], sig.member) {
            (Item::Function(function), None) => function,
            (Item::Impl(block), Some(member)) => &block.functions[member],
            _ => unreachable!("FnSig::item indexes a function, or an impl block with `member`"),
        }
    }
}

/// Resolves the program rooted at `entry`.
///
/// Pushes `entry` onto `sources` (the caller builds it with
/// `FileId(sources.len())`, so its id is its index), then every module
/// file, each with the next `FileId`. Returns every diagnostic collected,
/// or the resolved modules and symbols if there were none.
pub fn resolve(
    entry: SourceFile,
    sources: &mut Vec<SourceFile>,
) -> Result<Resolved, Vec<Diagnostic>> {
    let entry_file = entry.id;
    let entry_path = entry.path.clone();
    sources.push(entry);
    let program = parse(sources.last().expect("just pushed"))?;
    let mod_decls: Vec<ModDecl> = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Mod(decl) => Some(decl.clone()),
            _ => None,
        })
        .collect();

    let mut diagnostics = Vec::new();
    let mut modules = vec![Module {
        id: ModuleId(0),
        name: ENTRY_MODULE_NAME.to_string(),
        kind: ModuleKind::Varyk(program),
        file: entry_file,
    }];
    let mut imports = Vec::new();
    let dir = entry_path.parent().unwrap_or(Path::new(""));
    if !load_modules(
        &mod_decls,
        dir,
        sources,
        &mut modules,
        &mut imports,
        &mut diagnostics,
    ) {
        return Err(diagnostics);
    }

    let symbols = collect_symbols(&modules, imports, &mut diagnostics);
    check_entry_main(&modules[0], &mut diagnostics);

    if diagnostics.is_empty() {
        Ok(Resolved {
            modules,
            symbols,
            entry: ModuleId(0),
        })
    } else {
        Err(diagnostics)
    }
}

/// Lexes and parses `file`, converting any syntax errors.
fn parse(file: &SourceFile) -> Result<Program, Vec<Diagnostic>> {
    let (program, errors) = parse_source(file);
    if errors.is_empty() {
        Ok(program)
    } else {
        Err(errors.into_iter().map(Diagnostic::from).collect())
    }
}

/// Loads the file behind every `mod` declared in the entry program, appending to
/// `modules` (and, for `.rs` modules, their imported functions to
/// `imports`). Returns false if any module file had syntax errors, which
/// stops resolution.
fn load_modules(
    mod_decls: &[ModDecl],
    dir: &Path,
    sources: &mut Vec<SourceFile>,
    modules: &mut Vec<Module>,
    imports: &mut Vec<(ModuleId, Vec<ImportedFn>)>,
    diagnostics: &mut Vec<Diagnostic>,
) -> bool {
    let mut syntax_ok = true;
    let mut declared: HashMap<&str, Span> = HashMap::new();

    for decl in mod_decls {
        let name = decl.name.name.as_str();
        // `main.rs` is the crate root and `lib.rs` would become a second
        // crate of its own under cargo.
        if name == "main" || name == "lib" {
            diagnostics.push(Diagnostic::new(
                codes::V0104,
                decl.span,
                format!("`{name}` is not a valid module name"),
            ));
            continue;
        }
        if let Some(diagnostic) = reserved_type_name(name, decl.name.span) {
            diagnostics.push(diagnostic);
            continue;
        }
        if let Some(&first) = declared.get(name) {
            diagnostics.push(duplicate(name, decl.name.span, first));
            continue;
        }
        declared.insert(name, decl.name.span);

        let vr = dir.join(format!("{name}.vr"));
        let rs = dir.join(format!("{name}.rs"));
        let path = match (vr.is_file(), rs.is_file()) {
            (true, true) => {
                diagnostics.push(Diagnostic::new(
                    codes::V0104,
                    decl.span,
                    format!(
                        "module `{name}` is ambiguous: both `{name}.vr` and `{name}.rs` exist next to the entry file"
                    ),
                ));
                continue;
            }
            (false, false) => {
                diagnostics.push(Diagnostic::new(
                    codes::V0104,
                    decl.span,
                    format!(
                        "no file for module `{name}`: expected `{name}.vr` or `{name}.rs` next to the entry file"
                    ),
                ));
                continue;
            }
            (true, false) => vr,
            (false, true) => rs,
        };

        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    codes::V0104,
                    decl.span,
                    format!("cannot read `{}`: {err}", path.display()),
                ));
                continue;
            }
        };
        let file = FileId(sources.len() as u32);
        sources.push(SourceFile::new(file, path.clone(), text));
        let source = sources.last().expect("just pushed");
        let id = ModuleId(modules.len() as u32);

        let kind = if path.extension().is_some_and(|ext| ext == "vr") {
            match parse(source) {
                Ok(program) => ModuleKind::Varyk(program),
                Err(errors) => {
                    diagnostics.extend(errors);
                    syntax_ok = false;
                    continue;
                }
            }
        } else {
            match import_rust_module(&source.text) {
                Ok(fns) => {
                    imports.push((id, fns));
                    ModuleKind::Rust(source.text.clone())
                }
                Err(message) => {
                    diagnostics.push(Diagnostic::new(
                        codes::V0104,
                        decl.span,
                        format!("cannot parse `{}`: {message}", path.display()),
                    ));
                    continue;
                }
            }
        };
        modules.push(Module {
            id,
            name: name.to_string(),
            kind,
            file,
        });
    }
    syntax_ok
}

/// V0103 at `span` for a struct, enum, or module named after a built-in
/// type or one of the reserved names of spec 2.1: these share Rust's type
/// namespace with the primitives, `String`, `Option`, `Result`, and `Vec`,
/// so a user item with one of those names would capture them in rustc
/// (`mod String;` makes every `String` a module).
pub(crate) fn reserved_type_name(name: &str, span: Span) -> Option<Diagnostic> {
    let primitive = Ty::from_primitive_name(name).is_some() || matches!(name, "string" | "str");
    if primitive {
        return Some(taken(name, "a built-in type", span));
    }
    reserved_value_name(name, span)
}

/// V0103 at `span` for a function, method, variant, field, parameter, or
/// `let` named with one of the reserved names of spec 2.1 (`Some`, `None`,
/// `Ok`, `Err`, `Option`, `Result`, `Vec`, `String`): the generated Rust
/// would hide Rust's own (`let None = x;` is a pattern). The primitive
/// names stay usable here; Rust keeps values and types apart.
pub(crate) fn reserved_value_name(name: &str, span: Span) -> Option<Diagnostic> {
    let owner = match name {
        "Some" | "None" => "`Option`",
        "Ok" | "Err" => "`Result`",
        "Option" | "Result" | "Vec" | "String" => "a built-in type",
        _ => return None,
    };
    Some(taken(name, owner, span))
}

fn taken(name: &str, owner: &str, span: Span) -> Diagnostic {
    Diagnostic::new(
        codes::V0103,
        span,
        format!("the name `{name}` is already taken by {owner}"),
    )
}

/// V0103 at `span`, labelling the first definition at `first`.
fn duplicate(name: &str, span: Span, first: Span) -> Diagnostic {
    Diagnostic::new(
        codes::V0103,
        span,
        format!("the name `{name}` is defined more than once"),
    )
    .with_label(first, format!("`{name}` first defined here"))
}

/// Builds the symbol tables: first every name (so types can refer to
/// types declared later or in any order), then every `impl` block's
/// functions attached to its type, then field, payload, and signature
/// types.
fn collect_symbols(
    modules: &[Module],
    imports: Vec<(ModuleId, Vec<ImportedFn>)>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Symbols {
    let mut symbols = Symbols {
        scopes: modules
            .iter()
            .map(|module| Scope {
                name: module.name.clone(),
                ..Scope::default()
            })
            .collect(),
        ..Symbols::default()
    };
    // Every `impl` block with its module and the id of its first function.
    let mut impls: Vec<(ModuleId, &ImplBlock, u32)> = Vec::new();

    // Pass 1: names. Duplicates still get a table entry (so their types
    // are checked too) but not a scope entry.
    for module in modules {
        let ModuleKind::Varyk(program) = &module.kind else {
            continue;
        };
        let is_entry = module.id == ModuleId(0);
        let scope = &mut symbols.scopes[module.id.0 as usize];
        let mut first_fn: HashMap<&str, Span> = HashMap::new();
        let mut first_type: HashMap<&str, Span> = HashMap::new();
        if is_entry {
            // Modules share Rust's type namespace with structs and enums,
            // so a type named like a loaded module is a duplicate (rustc
            // E0428).
            for item in &program.items {
                let Item::Mod(decl) = item else {
                    continue;
                };
                if modules[1..].iter().any(|m| m.name == decl.name.name) {
                    first_type.entry(&decl.name.name).or_insert(decl.name.span);
                }
            }
        }

        for (index, item) in program.items.iter().enumerate() {
            match item {
                Item::Function(function) => {
                    let name = &function.name;
                    if !is_entry && name.name == "main" {
                        diagnostics.push(Diagnostic::new(
                            codes::V0106,
                            header(function),
                            "a module must not define `main`",
                        ));
                    }
                    let id = push_fn(&mut symbols.fns, function, module.id, index, None);
                    diagnostics.extend(reserved_value_name(&name.name, name.span));
                    match first_fn.entry(&name.name) {
                        Entry::Occupied(first) => {
                            diagnostics.push(duplicate(&name.name, name.span, *first.get()));
                        }
                        Entry::Vacant(slot) => {
                            slot.insert(name.span);
                            scope.fns.insert(name.name.clone(), Callee::Varyk(id));
                        }
                    }
                }
                Item::Struct(decl) => {
                    let name = &decl.name;
                    // Reported, but still registered: the later passes walk
                    // the struct declarations in the same order by id.
                    diagnostics.extend(reserved_type_name(&name.name, name.span));
                    let id = StructId(symbols.structs.len() as u32);
                    symbols.structs.push(StructDef {
                        name: name.name.clone(),
                        module: module.id,
                        is_pub: decl.is_pub,
                        fields: Vec::new(),
                        span: decl.span,
                    });
                    match first_type.entry(&name.name) {
                        Entry::Occupied(first) => {
                            diagnostics.push(duplicate(&name.name, name.span, *first.get()));
                        }
                        Entry::Vacant(slot) => {
                            slot.insert(name.span);
                            scope.types.insert(name.name.clone(), UserType::Struct(id));
                        }
                    }
                }
                Item::Enum(decl) => {
                    let name = &decl.name;
                    diagnostics.extend(reserved_type_name(&name.name, name.span));
                    let id = EnumId(symbols.enums.len() as u32);
                    // Variant names are known now; pass 2 fills in their
                    // payload types (and pass 1b already needs the names).
                    let variants = decl
                        .variants
                        .iter()
                        .map(|variant| (variant.name.name.clone(), Vec::new()))
                        .collect();
                    symbols.enums.push(EnumDef {
                        name: name.name.clone(),
                        module: module.id,
                        is_pub: decl.is_pub,
                        variants,
                        span: decl.span,
                    });
                    match first_type.entry(&name.name) {
                        Entry::Occupied(first) => {
                            diagnostics.push(duplicate(&name.name, name.span, *first.get()));
                        }
                        Entry::Vacant(slot) => {
                            slot.insert(name.span);
                            scope.types.insert(name.name.clone(), UserType::Enum(id));
                        }
                    }
                }
                Item::Impl(block) => {
                    let first = symbols.fns.len() as u32;
                    for (member, function) in block.functions.iter().enumerate() {
                        let name = &function.name;
                        diagnostics.extend(reserved_value_name(&name.name, name.span));
                        push_fn(&mut symbols.fns, function, module.id, index, Some(member));
                    }
                    impls.push((module.id, block, first));
                }
                Item::Mod(decl) if !is_entry => {
                    diagnostics.push(Diagnostic::new(
                        codes::V0001,
                        decl.span,
                        "nested modules are not supported yet; declare every `mod` in the entry file",
                    ));
                }
                Item::Mod(_) => {}
            }
        }
    }

    // Pass 1b: every `impl` block's functions join their type's table.
    // Several blocks for one type share it.
    let mut first_member: HashMap<(UserType, &str), Span> = HashMap::new();
    for (module, block, first) in impls {
        let type_name = &block.type_name;
        let Some(&owner) = symbols.scopes[module.0 as usize].types.get(&type_name.name) else {
            diagnostics.push(Diagnostic::new(
                codes::V0001,
                type_name.span,
                format!(
                    "an `impl` block must name a struct or enum declared in the same file, and this file declares no `{}`",
                    type_name.name
                ),
            ));
            continue;
        };
        for (offset, function) in block.functions.iter().enumerate() {
            let id = FnId(first + offset as u32);
            symbols.fns[id.0 as usize].owner = Some(owner);
            let name = &function.name;
            // `E::Make` must mean one thing: a function named after one of
            // the enum's own variants would be unreachable behind it.
            if let UserType::Enum(enum_id) = owner {
                let def = &symbols.enums[enum_id.0 as usize];
                if def.variant(&name.name).is_some() {
                    let owner_text = format!("a variant of `{}`", def.name);
                    diagnostics.push(taken(&name.name, &owner_text, name.span));
                    continue;
                }
            }
            match first_member.entry((owner, &name.name)) {
                Entry::Occupied(first) => {
                    diagnostics.push(duplicate(&name.name, name.span, *first.get()));
                }
                Entry::Vacant(slot) => {
                    slot.insert(name.span);
                    symbols.members.insert((owner, name.name.clone()), id);
                }
            }
        }
    }

    for (module, fns) in imports {
        let scope = &mut symbols.scopes[module.0 as usize];
        for imported in fns {
            let id = ImportedFnId(symbols.imported.len() as u32);
            scope
                .fns
                .entry(imported.name.clone())
                .or_insert(Callee::Imported(id));
            symbols.imported.push(import_sig(module, imported));
        }
    }

    // Pass 2: types, in declaration order. Tables were filled in the same
    // order, so a running index finds each item's entry.
    let (mut next_fn, mut next_struct, mut next_enum) = (0, 0, 0);
    // Every resolved field or payload type with the span it was written
    // at, per struct and per enum, for the recursive-type check below.
    let mut struct_parts: Vec<Vec<(Ty, Span)>> = Vec::new();
    let mut enum_parts: Vec<Vec<(Ty, Span)>> = Vec::new();
    for module in modules {
        let ModuleKind::Varyk(program) = &module.kind else {
            continue;
        };
        for item in &program.items {
            match item {
                Item::Function(function) => {
                    let (params, ret) =
                        resolve_signature(&symbols, function, module.id, diagnostics);
                    let sig = &mut symbols.fns[next_fn];
                    sig.params = params;
                    sig.ret = ret;
                    next_fn += 1;
                }
                Item::Impl(block) => {
                    for function in &block.functions {
                        let (params, ret) =
                            resolve_signature(&symbols, function, module.id, diagnostics);
                        let sig = &mut symbols.fns[next_fn];
                        sig.params = params;
                        sig.ret = ret;
                        next_fn += 1;
                    }
                }
                Item::Struct(decl) => {
                    let mut seen: HashMap<&str, Span> = HashMap::new();
                    let mut fields = Vec::new();
                    let mut parts = Vec::new();
                    for field in &decl.fields {
                        let name = &field.name;
                        diagnostics.extend(reserved_value_name(&name.name, name.span));
                        if let Some(&first) = seen.get(name.name.as_str()) {
                            diagnostics.push(duplicate(&name.name, name.span, first));
                        }
                        seen.entry(&name.name).or_insert(name.span);
                        match symbols.resolve_type(&field.ty, module.id) {
                            Ok(ty) => {
                                if decl.is_pub {
                                    diagnostics
                                        .extend(symbols.private_in_public(&ty, field.ty.span));
                                }
                                parts.push((ty.clone(), field.span));
                                fields.push((name.name.clone(), ty));
                            }
                            Err(diagnostic) => diagnostics.push(diagnostic),
                        }
                    }
                    symbols.structs[next_struct].fields = fields;
                    struct_parts.push(parts);
                    next_struct += 1;
                }
                Item::Enum(decl) => {
                    let mut seen: HashMap<&str, Span> = HashMap::new();
                    let mut variants = Vec::new();
                    let mut parts = Vec::new();
                    for variant in &decl.variants {
                        let name = &variant.name;
                        diagnostics.extend(reserved_value_name(&name.name, name.span));
                        if let Some(&first) = seen.get(name.name.as_str()) {
                            diagnostics.push(duplicate(&name.name, name.span, first));
                        }
                        seen.entry(&name.name).or_insert(name.span);
                        let mut payload = Vec::new();
                        for written in &variant.fields {
                            match symbols.resolve_type(written, module.id) {
                                Ok(ty) => {
                                    if decl.is_pub {
                                        diagnostics
                                            .extend(symbols.private_in_public(&ty, written.span));
                                    }
                                    parts.push((ty.clone(), written.span));
                                    payload.push(ty);
                                }
                                Err(diagnostic) => diagnostics.push(diagnostic),
                            }
                        }
                        variants.push((name.name.clone(), payload));
                    }
                    symbols.enums[next_enum].variants = variants;
                    enum_parts.push(parts);
                    next_enum += 1;
                }
                Item::Mod(_) => {}
            }
        }
    }
    struct_parts.extend(enum_parts);
    check_recursive_types(&symbols, &struct_parts, diagnostics);
    symbols
}

/// Appends the signature shell of `function`, declared by item `item` of
/// `module` (at `member` inside an `impl` block); pass 2 fills in its
/// types and pass 1b its owner.
fn push_fn(
    fns: &mut Vec<FnSig>,
    function: &Function,
    module: ModuleId,
    item: usize,
    member: Option<usize>,
) -> FnId {
    let id = FnId(fns.len() as u32);
    fns.push(FnSig {
        name: function.name.name.clone(),
        module,
        is_pub: function.is_pub,
        params: Vec::new(),
        ret: Ty::Unit,
        owner: None,
        self_mode: match function.self_mode {
            SelfMode::None => None,
            SelfMode::Shared => Some(ParamMode::SharedBorrow),
            SelfMode::Mutable => Some(ParamMode::MutableBorrow),
        },
        item,
        member,
        span: function.span,
    });
    id
}

/// V0109 for every struct or enum that contains itself, directly or
/// through other structs' fields and enums' payloads, `Option` and
/// `Result` arguments included, since Rust lays all of them out inline
/// (rustc E0072); a `Vec` holds its elements elsewhere and breaks the
/// cycle (spec 2.2). `parts[node]` lists a node's field or payload types
/// with their spans, structs first (node `id`) and then enums (node
/// `structs.len() + id`). A depth-first walk reports each cycle once, at
/// the part that closes it.
fn check_recursive_types(
    symbols: &Symbols,
    parts: &[Vec<(Ty, Span)>],
    diagnostics: &mut Vec<Diagnostic>,
) {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Unvisited,
        OnStack,
        Done,
    }

    struct Walk<'a> {
        symbols: &'a Symbols,
        parts: &'a [Vec<(Ty, Span)>],
        state: Vec<State>,
        stack: Vec<usize>,
        diagnostics: &'a mut Vec<Diagnostic>,
    }

    impl Walk<'_> {
        fn name(&self, node: usize) -> &str {
            let structs = self.symbols.structs.len();
            if node < structs {
                &self.symbols.structs[node].name
            } else {
                &self.symbols.enums[node - structs].name
            }
        }

        /// The nodes `ty` lays out inline.
        fn inline(&self, ty: &Ty, out: &mut Vec<usize>) {
            match ty {
                Ty::Struct(id) => out.push(id.0 as usize),
                Ty::Enum(id) => out.push(self.symbols.structs.len() + id.0 as usize),
                Ty::Option(inner) => self.inline(inner, out),
                Ty::Result(ok, err) => {
                    self.inline(ok, out);
                    self.inline(err, out);
                }
                _ => {}
            }
        }

        fn visit(&mut self, node: usize) {
            self.state[node] = State::OnStack;
            self.stack.push(node);
            let parts = self.parts;
            for (ty, span) in &parts[node] {
                let mut targets = Vec::new();
                self.inline(ty, &mut targets);
                targets.dedup();
                for target in targets {
                    match self.state[target] {
                        State::Unvisited => self.visit(target),
                        State::OnStack => self.report(target, *span),
                        State::Done => {}
                    }
                }
            }
            self.stack.pop();
            self.state[node] = State::Done;
        }

        fn report(&mut self, target: usize, span: Span) {
            let start = self
                .stack
                .iter()
                .position(|&s| s == target)
                .expect("an on-stack type is on the stack");
            let cycle: Vec<&str> = self.stack[start..]
                .iter()
                .chain(std::iter::once(&target))
                .map(|&node| self.name(node))
                .collect();
            let message = if target < self.symbols.structs.len() {
                "a struct cannot contain itself"
            } else {
                "an enum cannot contain itself"
            };
            let diagnostic = Diagnostic::new(codes::V0109, span, message)
                .with_note(format!("the cycle is {}", cycle.join(" -> ")));
            self.diagnostics.push(diagnostic);
        }
    }

    let mut walk = Walk {
        symbols,
        parts,
        state: vec![State::Unvisited; parts.len()],
        stack: Vec::new(),
        diagnostics,
    };
    for node in 0..parts.len() {
        if walk.state[node] == State::Unvisited {
            walk.visit(node);
        }
    }
}

/// Resolves a Varyk function's parameter and return types. Parameter
/// modes follow spec 4.2: `mut` is a mutable borrow; otherwise a Copy type
/// is owned and anything else a shared borrow.
fn resolve_signature(
    symbols: &Symbols,
    function: &Function,
    module: ModuleId,
    diagnostics: &mut Vec<Diagnostic>,
) -> (Vec<(String, Ty, ParamMode)>, Ty) {
    let mut seen: HashMap<&str, Span> = HashMap::new();
    let mut params = Vec::new();
    for param in &function.params {
        let name = &param.name;
        if let Some(&first) = seen.get(name.name.as_str()) {
            diagnostics.push(duplicate(&name.name, name.span, first));
        }
        seen.entry(&name.name).or_insert(name.span);
        match symbols.resolve_type(&param.ty, module) {
            Ok(ty) => {
                if function.is_pub {
                    if let Some(diagnostic) = symbols.private_in_public(&ty, param.ty.span) {
                        diagnostics.push(diagnostic);
                    }
                }
                let mode = if param.mutable {
                    ParamMode::MutableBorrow
                } else if ty.is_copy() {
                    ParamMode::Owned
                } else {
                    ParamMode::SharedBorrow
                };
                params.push((name.name.clone(), ty, mode));
            }
            Err(diagnostic) => diagnostics.push(diagnostic),
        }
    }
    let ret = match &function.return_type {
        None => Ty::Unit,
        Some(written) => match symbols.resolve_type(written, module) {
            Ok(ty) => {
                if function.is_pub {
                    if let Some(diagnostic) = symbols.private_in_public(&ty, written.span) {
                        diagnostics.push(diagnostic);
                    }
                }
                ty
            }
            Err(diagnostic) => {
                diagnostics.push(diagnostic);
                Ty::Unit
            }
        },
    };
    (params, ret)
}

/// The entry module must define `fn main()` with no parameters and no
/// return type (spec 4.1).
fn check_entry_main(entry: &Module, diagnostics: &mut Vec<Diagnostic>) {
    let ModuleKind::Varyk(program) = &entry.kind else {
        return;
    };
    let main = program.items.iter().find_map(|item| match item {
        Item::Function(function) if function.name.name == "main" => Some(function),
        _ => None,
    });
    match main {
        None => diagnostics.push(Diagnostic::new(
            codes::V0106,
            Span::new(entry.file, 0, 0),
            "the entry file must define `fn main()`",
        )),
        Some(function) if !function.params.is_empty() || function.return_type.is_some() => {
            diagnostics.push(Diagnostic::new(
                codes::V0106,
                header(function),
                "`main` must take no parameters and return nothing",
            ));
        }
        Some(_) => {}
    }
}

/// From the start of the declaration through the function's name
/// (`fn main`, or `pub fn main`).
fn header(function: &Function) -> Span {
    Span::new(
        function.span.file,
        function.span.start,
        function.name.span.end,
    )
}

/// Maps an imported Rust signature per the spec 4.5 table. A signature
/// with any unmapped type is not callable, and keeps no types.
fn import_sig(module: ModuleId, imported: ImportedFn) -> ImportedSig {
    let params: Option<Vec<_>> = imported.params.iter().map(map_param).collect();
    let ret = map_return(&imported.ret);
    let (params, ret, callable) = match (params, ret) {
        (Some(params), Some(ret)) => (params, ret, true),
        _ => (Vec::new(), Ty::Unit, false),
    };
    ImportedSig {
        name: imported.name,
        module,
        params,
        ret,
        signature: imported.signature,
        callable,
    }
}

fn map_param(ty: &RustTy) -> Option<(Ty, ParamMode)> {
    Some(match ty {
        RustTy::Str => (Ty::String, ParamMode::SharedBorrow),
        RustTy::String => (Ty::String, ParamMode::Owned),
        RustTy::RefMutString => (Ty::String, ParamMode::MutableBorrow),
        RustTy::Ref(inner) => (map_primitive(inner)?, ParamMode::SharedBorrow),
        RustTy::RefMut(inner) => (map_primitive(inner)?, ParamMode::MutableBorrow),
        other => (map_primitive(other)?, ParamMode::Owned),
    })
}

fn map_return(ty: &RustTy) -> Option<Ty> {
    match ty {
        RustTy::Unit => Some(Ty::Unit),
        RustTy::String => Some(Ty::String),
        other => map_primitive(other),
    }
}

fn map_primitive(ty: &RustTy) -> Option<Ty> {
    Some(match ty {
        RustTy::Bool => Ty::Bool,
        RustTy::I8 => Ty::Int(IntKind::I8),
        RustTy::I16 => Ty::Int(IntKind::I16),
        RustTy::I32 => Ty::Int(IntKind::I32),
        RustTy::I64 => Ty::Int(IntKind::I64),
        RustTy::U8 => Ty::Int(IntKind::U8),
        RustTy::U16 => Ty::Int(IntKind::U16),
        RustTy::U32 => Ty::Int(IntKind::U32),
        RustTy::U64 => Ty::Int(IntKind::U64),
        RustTy::Usize => Ty::Int(IntKind::Usize),
        RustTy::F32 => Ty::Float(FloatKind::F32),
        RustTy::F64 => Ty::Float(FloatKind::F64),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::diagnostics::codes;
    use crate::types::{FloatKind, IntKind};

    /// Resolves a single-file program from an inline string, with a dummy
    /// path (no `mod` declarations may appear).
    fn resolve_str(text: &str) -> (Result<Resolved, Vec<Diagnostic>>, Vec<SourceFile>) {
        let mut sources = Vec::new();
        let entry = SourceFile::new(FileId(0), "dummy/test.vr", text);
        let result = resolve(entry, &mut sources);
        (result, sources)
    }

    fn errors_str(text: &str) -> Vec<Diagnostic> {
        match resolve_str(text).0 {
            Ok(_) => panic!("expected diagnostics for:\n{text}"),
            Err(diagnostics) => diagnostics,
        }
    }

    /// Resolves `<rel>` relative to the workspace root, the way the CLI
    /// would.
    fn resolve_path(rel: &str) -> (Result<Resolved, Vec<Diagnostic>>, Vec<SourceFile>) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(rel);
        let text = std::fs::read_to_string(&path).expect("fixture should exist");
        let mut sources = Vec::new();
        let entry = SourceFile::new(FileId(0), path, text);
        let result = resolve(entry, &mut sources);
        (result, sources)
    }

    fn fixture(case: &str) -> (Vec<Diagnostic>, Vec<SourceFile>) {
        let (result, sources) = resolve_path(&format!(
            "crates/varyk/tests/fixtures/resolve/{case}/main.vr"
        ));
        match result {
            Ok(_) => panic!("expected diagnostics for fixture {case}"),
            Err(diagnostics) => (diagnostics, sources),
        }
    }

    /// The span of the first occurrence of `needle` in `sources[file]`.
    fn span_of(sources: &[SourceFile], file: u32, needle: &str) -> Span {
        let text = &sources[file as usize].text;
        let start = text
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} not in file"));
        Span::new(FileId(file), start as u32, (start + needle.len()) as u32)
    }

    fn only(diagnostics: &[Diagnostic]) -> &Diagnostic {
        assert_eq!(
            diagnostics.len(),
            1,
            "expected one diagnostic, got {diagnostics:#?}"
        );
        &diagnostics[0]
    }

    fn module_id(resolved: &Resolved, name: &str) -> ModuleId {
        resolved
            .modules
            .iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("no module {name}"))
            .id
    }

    // --- Module loading -------------------------------------------------

    #[test]
    fn mod_math_loads_math_vr() {
        let (result, sources) = resolve_path("examples/modules/main.vr");
        let resolved = result.expect("examples/modules should resolve");
        assert_eq!(sources.len(), 2);
        assert_eq!(resolved.modules.len(), 2);
        let math = &resolved.modules[module_id(&resolved, "math").0 as usize];
        assert!(matches!(math.kind, ModuleKind::Varyk(_)));
        assert_eq!(math.file, FileId(1));
        assert!(sources[1].path.ends_with("math.vr"));

        let callee = resolved
            .symbols
            .lookup_fn(resolved.entry, Some("math"), "square")
            .expect("math::square should be visible");
        let Callee::Varyk(id) = callee else {
            panic!("expected a Varyk fn, got {callee:?}");
        };
        let sig = &resolved.symbols.fns[id.0 as usize];
        assert_eq!(sig.name, "square");
        assert!(sig.is_pub);
        assert_eq!(
            sig.params,
            vec![("x".to_string(), Ty::Int(IntKind::I32), ParamMode::Owned)]
        );
        assert_eq!(sig.ret, Ty::Int(IntKind::I32));
        assert_eq!(resolved.fn_decl(id).name.name, "square");
    }

    #[test]
    fn mod_greet_loads_greet_rs_through_the_importer() {
        let (result, sources) = resolve_path("examples/interop/main.vr");
        let resolved = result.expect("examples/interop should resolve");
        assert_eq!(sources.len(), 2);
        let greet = &resolved.modules[module_id(&resolved, "greet").0 as usize];
        assert!(matches!(greet.kind, ModuleKind::Rust(_)));

        let callee = resolved
            .symbols
            .lookup_fn(resolved.entry, Some("greet"), "hello")
            .expect("greet::hello should be visible");
        let Callee::Imported(id) = callee else {
            panic!("expected an imported fn, got {callee:?}");
        };
        let sig = &resolved.symbols.imported[id.0 as usize];
        assert!(sig.callable);
        assert_eq!(sig.params, vec![(Ty::String, ParamMode::SharedBorrow)]);
        assert_eq!(sig.ret, Ty::String);
        assert!(sig.signature.contains("hello"));
    }

    #[test]
    fn both_module_files_present_is_v0104_naming_both() {
        let (diagnostics, sources) = fixture("both_files");
        let d = only(&diagnostics);
        assert_eq!(d.code, codes::V0104);
        assert_eq!(d.span, span_of(&sources, 0, "mod util;"));
        assert!(
            d.message.contains("util.vr") && d.message.contains("util.rs"),
            "{}",
            d.message
        );
    }

    #[test]
    fn missing_module_file_is_v0104_at_the_mod() {
        let (diagnostics, sources) = fixture("missing_file");
        let d = only(&diagnostics);
        assert_eq!(d.code, codes::V0104);
        assert_eq!(d.span, span_of(&sources, 0, "mod nothing;"));
    }

    #[test]
    fn unparseable_rs_module_is_v0104_at_the_mod() {
        let (diagnostics, sources) = fixture("bad_rs");
        let d = only(&diagnostics);
        assert_eq!(d.code, codes::V0104);
        assert_eq!(d.span, span_of(&sources, 0, "mod broken;"));
        // The file was still pushed so a diagnostic could cite it.
        assert_eq!(sources.len(), 2);
    }

    #[test]
    fn mod_main_is_v0104_at_the_mod() {
        let (diagnostics, sources) = fixture("mod_main");
        let d = only(&diagnostics);
        assert_eq!(d.code, codes::V0104);
        assert_eq!(d.span, span_of(&sources, 0, "mod main;"));
    }

    #[test]
    fn mod_lib_is_v0104_at_the_mod() {
        let (diagnostics, sources) = fixture("mod_lib");
        let d = only(&diagnostics);
        assert_eq!(d.code, codes::V0104);
        assert_eq!(d.span, span_of(&sources, 0, "mod lib;"));
    }

    #[test]
    fn mod_inside_a_vr_module_is_v0001_at_that_mod() {
        let (diagnostics, sources) = fixture("nested_mod");
        let d = only(&diagnostics);
        assert_eq!(d.code, codes::V0001);
        assert_eq!(d.span, span_of(&sources, 1, "mod inner;"));
    }

    #[test]
    fn module_defining_main_is_v0106() {
        let (diagnostics, sources) = fixture("module_main");
        let d = only(&diagnostics);
        assert_eq!(d.code, codes::V0106);
        assert_eq!(d.span.file, FileId(1));
        assert_eq!(d.span.start, span_of(&sources, 1, "fn main").start);
    }

    // --- Visibility ---------------------------------------------------------

    #[test]
    fn non_pub_struct_in_a_pub_signature_or_pub_struct_field_is_v0105() {
        let (result, sources) =
            resolve_path("crates/varyk/tests/fixtures/resolve/private_in_public/main.vr");
        let diagnostics = result.expect_err("private types in public items should fail");
        let spans: Vec<Span> = diagnostics.iter().map(|d| d.span).collect();
        let at = |needle: &str| {
            let span = span_of(&sources, 1, needle);
            Span::new(span.file, span.end - "Hidden".len() as u32, span.end)
        };
        assert_eq!(
            spans,
            vec![
                at("pub fn take(h: Hidden"),
                at("make() -> Hidden"),
                at("Shown {\n    h: Hidden")
            ]
        );
        for d in &diagnostics {
            assert_eq!(d.code, codes::V0105);
            assert_eq!(
                d.message,
                "`Hidden` is used by a public item but is not public; add `pub` to the struct"
            );
            assert_eq!(d.fix_it.as_ref().expect("fix-it").replacement, "pub ");
        }
    }

    #[test]
    fn lookup_enforces_pub_across_modules() {
        let (result, sources) =
            resolve_path("crates/varyk/tests/fixtures/resolve/visibility/main.vr");
        let resolved = result.expect("visibility fixture should resolve");
        let symbols = &resolved.symbols;
        let math = module_id(&resolved, "math");

        let hidden = symbols.lookup_fn(resolved.entry, Some("math"), "hidden");
        let expected = span_of(&sources, 1, "fn hidden() {}");
        assert_eq!(
            hidden,
            Err(LookupError::NotVisible {
                decl: expected,
                keyword: "fn"
            })
        );

        let shown = symbols
            .lookup_fn(resolved.entry, Some("math"), "shown")
            .expect("pub fn should be visible");
        let Callee::Varyk(id) = shown else { panic!() };
        assert_eq!(
            symbols.fns[id.0 as usize].params,
            vec![
                (
                    "n".to_string(),
                    Ty::Int(IntKind::I32),
                    ParamMode::MutableBorrow
                ),
                ("s".to_string(), Ty::String, ParamMode::SharedBorrow),
            ]
        );

        // Within its own module, a non-`pub` item is visible.
        assert!(symbols.lookup_fn(math, None, "hidden").is_ok());
        let holder = symbols.lookup_struct(math, "Holder").unwrap();
        let secret = symbols.lookup_struct(math, "Secret").unwrap();
        assert_eq!(
            symbols.structs[holder.0 as usize].fields,
            vec![("s".to_string(), Ty::Struct(secret))]
        );

        assert_eq!(
            symbols.lookup_fn(resolved.entry, Some("nope"), "shown"),
            Err(LookupError::Unknown)
        );
        assert_eq!(
            symbols.lookup_fn(resolved.entry, Some("math"), "nope"),
            Err(LookupError::Unknown)
        );
        assert_eq!(
            symbols.lookup_fn(resolved.entry, None, "shown"),
            Err(LookupError::Unknown)
        );
        assert!(symbols.lookup_fn(resolved.entry, None, "main").is_ok());
    }

    // --- Entry `main` -------------------------------------------------------

    #[test]
    fn entry_without_main_is_v0106_at_the_start_of_the_file() {
        let d = errors_str("fn helper() {}\n");
        let d = only(&d);
        assert_eq!(d.code, codes::V0106);
        assert_eq!(d.span, Span::new(FileId(0), 0, 0));
    }

    #[test]
    fn main_with_parameters_is_v0106() {
        let d = errors_str("fn main(x: i32) {}\n");
        let d = only(&d);
        assert_eq!(d.code, codes::V0106);
        assert_eq!(d.span.start, 0);
    }

    #[test]
    fn main_returning_a_value_is_v0106() {
        let d = errors_str("fn main() -> i32 { 0 }\n");
        assert_eq!(only(&d).code, codes::V0106);
    }

    // --- Types and duplicates -----------------------------------------------

    #[test]
    fn unknown_type_in_signature_or_field_is_v0101() {
        let text = "struct S {\n    a: Nope,\n}\n\nfn f(x: Missing) -> Gone {}\n\nfn main() {}\n";
        let (result, sources) = resolve_str(text);
        let diagnostics = result.expect_err("should fail");
        assert_eq!(diagnostics.len(), 3, "{diagnostics:#?}");
        for (d, name) in diagnostics.iter().zip(["Nope", "Missing", "Gone"]) {
            assert_eq!(d.code, codes::V0101);
            assert_eq!(d.span, span_of(&sources, 0, name));
        }
    }

    #[test]
    fn unknown_type_matching_another_modules_type_is_v0101_with_a_did_you_mean_note() {
        let (diagnostics, sources) = fixture("unknown_type_other_module");
        let d = only(&diagnostics);
        assert_eq!(d.code, codes::V0101);
        let at = span_of(&sources, 0, "Task)");
        assert_eq!(d.span, Span::new(at.file, at.start, at.start + 4));
        assert!(
            d.notes.iter().any(|n| n == "did you mean `m::Task`?"),
            "{d:#?}"
        );
    }

    #[test]
    fn duplicate_function_is_v0103_labelling_the_first() {
        let text = "fn f() {}\nfn f() {}\nfn main() {}\n";
        let diagnostics = errors_str(text);
        let d = only(&diagnostics);
        assert_eq!(d.code, codes::V0103);
        assert_eq!(d.span, Span::new(FileId(0), 13, 14));
        assert_eq!(d.labels.len(), 1);
        assert_eq!(d.labels[0].span, Span::new(FileId(0), 3, 4));
    }

    #[test]
    fn enum_function_named_after_a_variant_is_v0103() {
        let text = "enum E {\n    Make(i32),\n    Nope,\n}\nimpl E {\n    fn Make(x: i32) -> E {\n        E::Nope\n    }\n}\nfn main() {}\n";
        let diagnostics = errors_str(text);
        let d = only(&diagnostics);
        assert_eq!(d.code, codes::V0103);
        assert!(d.message.contains("a variant of `E`"), "{}", d.message);
        assert_eq!(&text[d.span.start as usize..d.span.end as usize], "Make");
    }

    #[test]
    fn duplicate_struct_is_v0103() {
        let d = errors_str("struct S {}\nstruct S {}\nfn main() {}\n");
        assert_eq!(only(&d).code, codes::V0103);
    }

    #[test]
    fn struct_named_like_a_module_is_v0103_labelling_the_mod() {
        let (diagnostics, sources) = fixture("mod_struct_clash");
        let d = only(&diagnostics);
        assert_eq!(d.code, codes::V0103);
        let at_struct = span_of(&sources, 0, "struct foo");
        assert_eq!(
            d.span,
            Span::new(FileId(0), at_struct.start + 7, at_struct.end)
        );
        assert_eq!(d.labels.len(), 1);
        let at_mod = span_of(&sources, 0, "mod foo");
        assert_eq!(
            d.labels[0].span,
            Span::new(FileId(0), at_mod.start + 4, at_mod.end)
        );
    }

    #[test]
    fn indirectly_recursive_structs_are_one_v0109_at_the_closing_field() {
        let text = "struct A {\n    b: B,\n}\nstruct B {\n    a: A,\n}\nfn main() {}\n";
        let (result, sources) = resolve_str(text);
        let diagnostics = result.expect_err("should fail");
        let d = only(&diagnostics);
        assert_eq!(d.code, codes::V0109);
        assert_eq!(d.span, span_of(&sources, 0, "a: A"));
        assert_eq!(d.notes, vec!["the cycle is A -> B -> A".to_string()]);
    }

    #[test]
    fn a_struct_field_of_another_struct_is_not_recursive() {
        let text = "struct A {\n    b: B,\n}\nstruct B {\n    x: i32,\n}\nfn main() {}\n";
        assert!(resolve_str(text).0.is_ok());
    }

    #[test]
    fn capital_string_and_str_are_v0107_with_a_fix_it_to_string() {
        let text = "fn f(a: String, b: str) {}\nfn main() {}\n";
        let (result, sources) = resolve_str(text);
        let diagnostics = result.expect_err("should fail");
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        for (d, name) in diagnostics.iter().zip(["String", "str"]) {
            assert_eq!(d.code, codes::V0107);
            let span = span_of(&sources, 0, &format!(": {name}"));
            let span = Span::new(span.file, span.start + 2, span.end);
            assert_eq!(d.span, span);
            let fix = d.fix_it.as_ref().expect("fix-it");
            assert_eq!(fix.span, span);
            assert_eq!(fix.replacement, "string");
        }
    }

    #[test]
    fn param_modes_follow_copy_and_mut() {
        let text = "struct U {\n    n: i32,\n}\nfn f(a: bool, b: U, mut c: U, mut d: f64) {}\nfn main() {}\n";
        let (result, _) = resolve_str(text);
        let resolved = result.expect("should resolve");
        let Ok(Callee::Varyk(id)) = resolved.symbols.lookup_fn(resolved.entry, None, "f") else {
            panic!()
        };
        let modes: Vec<ParamMode> = resolved.symbols.fns[id.0 as usize]
            .params
            .iter()
            .map(|p| p.2)
            .collect();
        assert_eq!(
            modes,
            vec![
                ParamMode::Owned,
                ParamMode::SharedBorrow,
                ParamMode::MutableBorrow,
                ParamMode::MutableBorrow,
            ]
        );
    }

    #[test]
    fn syntax_errors_stop_resolution() {
        let d = errors_str("fn main( {}\n");
        assert!(d.iter().all(|d| d.code != codes::V0106), "{d:#?}");
        assert!(!d.is_empty());
    }

    // --- Enums, impl blocks, and generic types ------------------------------

    fn resolved(text: &str) -> Resolved {
        match resolve_str(text).0 {
            Ok(resolved) => resolved,
            Err(diagnostics) => panic!("expected {text:?} to resolve, got {diagnostics:#?}"),
        }
    }

    fn member<'a>(resolved: &'a Resolved, owner: UserType, name: &str) -> (FnId, &'a FnSig) {
        let id = resolved
            .symbols
            .lookup_member(resolved.entry, owner, name)
            .unwrap_or_else(|e| panic!("no member {name}: {e:?}"));
        (id, &resolved.symbols.fns[id.0 as usize])
    }

    const I32: Ty = Ty::Int(IntKind::I32);
    const F64: Ty = Ty::Float(FloatKind::F64);

    #[test]
    fn enum_and_impl_register() {
        let r = resolved(
            "enum Shape {\n    Circle(f64),\n    Rect(f64, string),\n    Point,\n}\n\nstruct Counter {\n    count: i32,\n}\n\nimpl Counter {\n    fn new() -> Counter {\n        Counter { count: 0 }\n    }\n\n    fn add(mut self, by: i32) {\n        self.count = self.count + by;\n    }\n\n    fn value(self) -> i32 {\n        self.count\n    }\n}\n\nfn main() {}\n",
        );
        let symbols = &r.symbols;
        let Ok(UserType::Enum(shape)) = symbols.lookup_type(r.entry, None, "Shape") else {
            panic!("Shape should be an enum");
        };
        let def = &symbols.enums[shape.0 as usize];
        assert_eq!(
            def.variants,
            vec![
                ("Circle".to_string(), vec![F64]),
                ("Rect".to_string(), vec![F64, Ty::String]),
                ("Point".to_string(), vec![]),
            ]
        );
        assert_eq!(def.variant("Point"), Some(2));
        assert_eq!(def.variant("Square"), None);

        let counter = symbols
            .lookup_type(r.entry, None, "Counter")
            .expect("Counter");
        let (_, new) = member(&r, counter, "new");
        assert_eq!(
            (new.owner, new.self_mode, &new.ret),
            (Some(counter), None, &counter.ty())
        );
        let (add_id, add) = member(&r, counter, "add");
        assert_eq!(add.self_mode, Some(ParamMode::MutableBorrow));
        assert_eq!(add.params, vec![("by".to_string(), I32, ParamMode::Owned)]);
        assert_eq!(r.fn_decl(add_id).name.name, "add");
        let (_, value) = member(&r, counter, "value");
        assert_eq!(
            (value.self_mode, &value.ret),
            (Some(ParamMode::SharedBorrow), &I32)
        );

        // Methods and associated functions are not free functions.
        assert_eq!(
            symbols.lookup_fn(r.entry, None, "new"),
            Err(LookupError::Unknown)
        );
        assert_eq!(
            symbols.lookup_member(r.entry, counter, "nope"),
            Err(LookupError::Unknown)
        );
    }

    #[test]
    fn two_impl_blocks_for_one_type_merge() {
        let r = resolved(
            "struct C {\n    n: i32,\n}\nimpl C {\n    fn a(self) {}\n}\nimpl C {\n    fn b(self) {}\n}\nfn main() {}\n",
        );
        let c = r.symbols.lookup_type(r.entry, None, "C").expect("C");
        assert_eq!(member(&r, c, "a").1.owner, Some(c));
        assert_eq!(member(&r, c, "b").1.owner, Some(c));
    }

    #[test]
    fn a_method_defined_twice_across_impl_blocks_is_v0103() {
        let text = "struct C {\n    n: i32,\n}\nimpl C {\n    fn a(self) {}\n}\nimpl C {\n    fn a() {}\n}\nfn main() {}\n";
        let (result, sources) = resolve_str(text);
        let diagnostics = result.expect_err("should fail");
        let d = only(&diagnostics);
        assert_eq!(d.code, codes::V0103);
        let second = span_of(&sources, 0, "a() {}");
        assert_eq!(d.span, Span::new(FileId(0), second.start, second.start + 1));
        let first = span_of(&sources, 0, "a(self)");
        assert_eq!(
            d.labels[0].span,
            Span::new(FileId(0), first.start, first.start + 1)
        );
    }

    #[test]
    fn an_impl_on_an_enum_registers() {
        let r = resolved(
            "enum Shape {\n    Point,\n}\nimpl Shape {\n    fn name(self) -> string {\n        \"point\"\n    }\n}\nfn main() {}\n",
        );
        let shape = r
            .symbols
            .lookup_type(r.entry, None, "Shape")
            .expect("Shape");
        assert!(matches!(shape, UserType::Enum(_)));
        let (_, name) = member(&r, shape, "name");
        assert_eq!(
            (name.owner, name.self_mode, &name.ret),
            (Some(shape), Some(ParamMode::SharedBorrow), &Ty::String)
        );
    }

    #[test]
    fn an_impl_for_a_type_not_in_the_file_is_v0001() {
        let text = "impl Nope {\n    fn f() {}\n}\nfn main() {}\n";
        let (result, sources) = resolve_str(text);
        let diagnostics = result.expect_err("should fail");
        let d = only(&diagnostics);
        assert_eq!(d.code, codes::V0001);
        assert_eq!(d.span, span_of(&sources, 0, "Nope"));

        let (diagnostics, sources) = fixture("impl_other_module");
        let d = only(&diagnostics);
        assert_eq!(d.code, codes::V0001);
        assert_eq!(d.span, span_of(&sources, 0, "Task"));
    }

    #[test]
    fn an_enum_containing_itself_through_option_is_v0109_and_through_vec_is_not() {
        let text = "enum L {\n    C(Option<L>),\n    End,\n}\nfn main() {}\n";
        let (result, sources) = resolve_str(text);
        let diagnostics = result.expect_err("should fail");
        let d = only(&diagnostics);
        assert_eq!(d.code, codes::V0109);
        assert_eq!(d.span, span_of(&sources, 0, "Option<L>"));
        assert_eq!(d.message, "an enum cannot contain itself");
        assert_eq!(d.notes, vec!["the cycle is L -> L".to_string()]);

        resolved("enum T {\n    C(Vec<T>),\n    Leaf,\n}\nfn main() {}\n");
    }

    #[test]
    fn a_struct_and_an_enum_containing_each_other_through_result_are_one_v0109() {
        let text =
            "struct S {\n    r: Result<i32, E>,\n}\nenum E {\n    Wrap(S),\n}\nfn main() {}\n";
        let (result, sources) = resolve_str(text);
        let diagnostics = result.expect_err("should fail");
        let d = only(&diagnostics);
        assert_eq!(d.code, codes::V0109);
        let payload = span_of(&sources, 0, "S),");
        assert_eq!(
            d.span,
            Span::new(payload.file, payload.start, payload.start + 1)
        );
        assert_eq!(d.notes, vec!["the cycle is S -> E -> S".to_string()]);
    }

    #[test]
    fn reserved_names_in_each_declaration_position_are_v0103() {
        let text = "mod String;\nfn Some() {}\nenum Option {\n    A,\n}\nenum E {\n    Ok,\n}\nstruct None {}\nstruct F {\n    Vec: i32,\n}\nimpl F {\n    fn Err(self) {}\n}\nfn main() {}\n";
        let (result, sources) = resolve_str(text);
        let diagnostics = result.expect_err("should fail");
        let mut spans: Vec<Span> = diagnostics.iter().map(|d| d.span).collect();
        spans.sort_by_key(|span| span.start);
        let expected: Vec<Span> = ["String", "Some", "Option", "Ok", "None", "Vec", "Err"]
            .iter()
            .map(|name| span_of(&sources, 0, name))
            .collect();
        assert_eq!(spans, expected, "{diagnostics:#?}");
        assert!(diagnostics.iter().all(|d| d.code == codes::V0103));
        let some = diagnostics
            .iter()
            .find(|d| d.span == expected[1])
            .expect("Some");
        assert_eq!(some.message, "the name `Some` is already taken by `Option`");
        let option = diagnostics
            .iter()
            .find(|d| d.span == expected[2])
            .expect("Option");
        assert_eq!(
            option.message,
            "the name `Option` is already taken by a built-in type"
        );
    }

    #[test]
    fn option_result_and_vec_resolve_with_the_right_number_of_type_arguments() {
        let r = resolved(
            "struct U {\n    n: i32,\n}\nfn f(x: Vec<Option<Result<U, string>>>, n: usize) -> Option<usize> {\n    None\n}\nfn main() {}\n",
        );
        let Ok(Callee::Varyk(id)) = r.symbols.lookup_fn(r.entry, None, "f") else {
            panic!()
        };
        let sig = &r.symbols.fns[id.0 as usize];
        let u = Ty::Struct(r.symbols.lookup_struct(r.entry, "U").expect("U"));
        let usize = Ty::Int(IntKind::Usize);
        let x = Ty::Vec(Box::new(Ty::Option(Box::new(Ty::Result(
            Box::new(u),
            Box::new(Ty::String),
        )))));
        assert_eq!(
            sig.params,
            vec![
                ("x".to_string(), x, ParamMode::SharedBorrow),
                ("n".to_string(), usize.clone(), ParamMode::Owned),
            ]
        );
        assert_eq!(sig.ret, Ty::Option(Box::new(usize)));
    }

    #[test]
    fn option_result_or_vec_with_the_wrong_number_of_type_arguments_is_v0101() {
        let text = "fn f(a: Result<i32>, b: Option<i32, i32>, c: Vec) {}\nfn main() {}\n";
        let (result, sources) = resolve_str(text);
        let diagnostics = result.expect_err("should fail");
        assert_eq!(diagnostics.len(), 3, "{diagnostics:#?}");
        let expected = [
            (
                "Result<i32>",
                "`Result` takes two types, written `Result<T, E>`",
            ),
            (
                "Option<i32, i32>",
                "`Option` takes one type, written `Option<T>`",
            ),
            ("Vec)", "`Vec` takes one type, written `Vec<T>`"),
        ];
        for (d, (needle, message)) in diagnostics.iter().zip(expected) {
            assert_eq!(d.code, codes::V0101);
            let span = span_of(&sources, 0, needle);
            let len = needle.trim_end_matches(')').len() as u32;
            assert_eq!(d.span, Span::new(span.file, span.start, span.start + len));
            assert_eq!(d.message, message);
        }
    }

    #[test]
    fn a_module_type_in_a_signature_resolves_and_needs_pub() {
        let (result, _) = resolve_path("crates/varyk/tests/fixtures/resolve/module_types/main.vr");
        let r = result.expect("module_types should resolve");
        let symbols = &r.symbols;
        let task = symbols
            .lookup_type(r.entry, Some("m"), "Task")
            .expect("m::Task");
        let shape = symbols
            .lookup_type(r.entry, Some("m"), "Shape")
            .expect("m::Shape");
        let Ok(Callee::Varyk(take)) = symbols.lookup_fn(r.entry, None, "take") else {
            panic!()
        };
        let types: Vec<Ty> = symbols.fns[take.0 as usize]
            .params
            .iter()
            .map(|p| p.1.clone())
            .collect();
        assert_eq!(types, vec![task.ty(), shape.ty()]);
        let UserType::Enum(shape_id) = shape else {
            panic!("m::Shape is an enum")
        };
        assert_eq!(
            symbols.enums[shape_id.0 as usize].variant("Circle"),
            Some(1)
        );

        // `m::Task::new` is `pub`; `secret` is not.
        let (_, new) = member(&r, task, "new");
        assert_eq!(new.name, "new");
        assert!(matches!(
            symbols.lookup_member(r.entry, task, "secret"),
            Err(LookupError::NotVisible { keyword: "fn", .. })
        ));
        let m = module_id(&r, "m");
        assert!(symbols.lookup_member(m, task, "secret").is_ok());
        assert!(matches!(
            symbols.lookup_type(r.entry, Some("m"), "Hidden"),
            Err(LookupError::NotVisible {
                keyword: "struct",
                ..
            })
        ));

        let (diagnostics, sources) = fixture("module_type_private");
        let d = only(&diagnostics);
        assert_eq!(d.code, codes::V0105);
        assert_eq!(d.span, span_of(&sources, 0, "m::Hidden"));
        let fix = d.fix_it.as_ref().expect("fix-it");
        assert_eq!(fix.span.file, FileId(1));
        assert_eq!(fix.span.start, span_of(&sources, 1, "struct Hidden").start);
        assert_eq!(fix.replacement, "pub ");
    }

    #[test]
    fn a_private_type_inside_a_public_enum_or_generic_type_is_v0105() {
        let text = "struct Hidden {}\npub enum E {\n    A(Hidden),\n}\npub fn f(x: Option<Hidden>) {}\nfn main() {}\n";
        let (result, sources) = resolve_str(text);
        let diagnostics = result.expect_err("should fail");
        let spans: Vec<Span> = diagnostics.iter().map(|d| d.span).collect();
        let hidden = span_of(&sources, 0, "Hidden)");
        let hidden = Span::new(hidden.file, hidden.start, hidden.start + 6);
        assert_eq!(spans, vec![hidden, span_of(&sources, 0, "Option<Hidden>")]);
        assert!(diagnostics.iter().all(|d| d.code == codes::V0105));
    }

    #[test]
    fn usize_imports_and_resolves() {
        let fns = import_rust_module("pub fn f(n: usize, r: &usize, m: &mut usize) -> usize { n }")
            .expect("parses");
        let sig = import_sig(ModuleId(1), fns.into_iter().next().expect("one fn"));
        let usize = Ty::Int(IntKind::Usize);
        assert!(sig.callable);
        assert_eq!(
            sig.params,
            vec![
                (usize.clone(), ParamMode::Owned),
                (usize.clone(), ParamMode::SharedBorrow),
                (usize.clone(), ParamMode::MutableBorrow),
            ]
        );
        assert_eq!(sig.ret, usize);
    }
}
