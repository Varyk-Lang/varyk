//! The resolver: loads the entry file and its `mod` files, collects every
//! module's symbols, resolves signature and field types, and enforces
//! visibility (spec sections 4.1 and 4.5).
//!
//! [`resolve`] owns lexing and parsing of every file, entry included, with
//! the per-file stop rule: a file with syntax errors contributes only those
//! errors, and resolution stops after module loading if any file had them.
//! Expression paths and locals are resolved later, during lowering in the
//! type checker, through [`Symbols::lookup_fn`], [`Symbols::lookup_struct`], and
//! [`Symbols::resolve_type`].

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fs;
use std::path::Path;

use varyk_syntax::{
    FileId, FixIt, Function, Item, ModDecl, Program, SourceFile, Span, TypeExpr, parse_source,
};

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
    /// Index of the declaring [`Item::Function`] in the module's
    /// `Program::items`; see [`Resolved::fn_decl`].
    pub item: usize,
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

/// A resolved function reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Callee {
    Varyk(FnId),
    Imported(ImportedFnId),
}

/// Why a lookup failed. The type checker turns these into diagnostics at the use
/// site: `Unknown` is V0100, `NotVisible` is V0105.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookupError {
    /// No such module, or no such item in it.
    Unknown,
    /// The item exists in another module but is not `pub`. Holds the span
    /// of its declaration, whose start is where a `pub ` fix-it inserts.
    NotVisible(Span),
}

/// Every module's functions, imported functions, and structs.
#[derive(Debug, Clone, Default)]
pub struct Symbols {
    pub fns: Vec<FnSig>,
    pub imported: Vec<ImportedSig>,
    pub structs: Vec<StructDef>,
    /// Per-module name tables, indexed by `ModuleId`.
    scopes: Vec<Scope>,
}

#[derive(Debug, Clone, Default)]
struct Scope {
    name: String,
    fns: HashMap<String, Callee>,
    structs: HashMap<String, StructId>,
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
                return Err(LookupError::NotVisible(sig.span));
            }
        }
        Ok(callee)
    }

    /// Looks up a struct by name in module `from`. Struct names are never
    /// module-qualified, so there is no visibility rule to apply.
    pub fn lookup_struct(&self, from: ModuleId, name: &str) -> Option<StructId> {
        self.scopes[from.0 as usize].structs.get(name).copied()
    }

    /// Resolves a written type as seen from module `from`: a primitive
    /// name, `string`, or a struct visible from `from`. `String` and `str`
    /// are V0107 with a fix-it to `string`; anything else is V0101.
    #[expect(
        clippy::result_large_err,
        reason = "a single diagnostic on a cold path; the type checker pushes it straight into its list"
    )]
    pub fn resolve_type(&self, ty: &TypeExpr, from: ModuleId) -> Result<Ty, Diagnostic> {
        let name = ty.name.name.as_str();
        let span = ty.name.span;
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
        self.lookup_struct(from, name)
            .map(Ty::Struct)
            .ok_or_else(|| Diagnostic::new(codes::V0101, span, format!("unknown type `{name}`")))
    }

    /// V0105 when a `pub` item's resolved type `ty`, written at `span`, is
    /// a non-`pub` struct: every caller in another module would be rejected
    /// by rustc. Resolved types only name structs of the item's own module.
    fn private_in_public(&self, ty: Ty, span: Span) -> Option<Diagnostic> {
        let Ty::Struct(id) = ty else {
            return None;
        };
        let def = &self.structs[id.0 as usize];
        if def.is_pub {
            return None;
        }
        let diagnostic = Diagnostic::new(
            codes::V0105,
            span,
            format!(
                "`{}` is used by a public item but is not public; add `pub` to the struct",
                def.name
            ),
        );
        Some(declared_without_pub(diagnostic, def.span, "struct"))
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

/// Labels the `keyword` (`fn` or `struct`) of the non-`pub` declaration at
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
        let Item::Function(function) = &program.items[sig.item] else {
            unreachable!("FnSig::item always indexes a function item");
        };
        function
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

/// V0103 at `span` for a struct or module named after a built-in type:
/// the generated Rust spells `string` as `String` and the primitives by
/// their own names, so a user item with one of those names would capture
/// them in rustc (`mod String;` makes every `String` a module).
fn reserved_type_name(name: &str, span: Span) -> Option<Diagnostic> {
    let taken =
        Ty::from_primitive_name(name).is_some() || matches!(name, "string" | "String" | "str");
    taken.then(|| {
        Diagnostic::new(
            codes::V0103,
            span,
            format!("the name `{name}` is already taken by a built-in type"),
        )
    })
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
/// structs declared later or in any order), then field and signature types.
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

    // Pass 1: names. Duplicates still get a table entry (so their types
    // are checked too) but not a scope entry.
    for module in modules {
        let ModuleKind::Varyk(program) = &module.kind else {
            continue;
        };
        let is_entry = module.id == ModuleId(0);
        let scope = &mut symbols.scopes[module.id.0 as usize];
        let mut first_fn: HashMap<&str, Span> = HashMap::new();
        let mut first_struct: HashMap<&str, Span> = HashMap::new();
        if is_entry {
            // Modules and structs share Rust's type namespace, so a struct
            // named like a loaded module is a duplicate (rustc E0428).
            for item in &program.items {
                let Item::Mod(decl) = item else {
                    continue;
                };
                if modules[1..].iter().any(|m| m.name == decl.name.name) {
                    first_struct
                        .entry(&decl.name.name)
                        .or_insert(decl.name.span);
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
                    let id = FnId(symbols.fns.len() as u32);
                    symbols.fns.push(FnSig {
                        name: name.name.clone(),
                        module: module.id,
                        is_pub: function.is_pub,
                        params: Vec::new(),
                        ret: Ty::Unit,
                        item: index,
                        span: function.span,
                    });
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
                    if let Some(diagnostic) = reserved_type_name(&name.name, name.span) {
                        diagnostics.push(diagnostic);
                    }
                    let id = StructId(symbols.structs.len() as u32);
                    symbols.structs.push(StructDef {
                        name: name.name.clone(),
                        module: module.id,
                        is_pub: decl.is_pub,
                        fields: Vec::new(),
                        span: decl.span,
                    });
                    match first_struct.entry(&name.name) {
                        Entry::Occupied(first) => {
                            diagnostics.push(duplicate(&name.name, name.span, *first.get()));
                        }
                        Entry::Vacant(slot) => {
                            slot.insert(name.span);
                            scope.structs.insert(name.name.clone(), id);
                        }
                    }
                }
                Item::Mod(decl) if !is_entry => {
                    diagnostics.push(Diagnostic::new(
                        codes::V0001,
                        decl.span,
                        "nested modules are not supported in milestone 1; declare every `mod` in the entry file",
                    ));
                }
                Item::Mod(_) => {}
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
    let (mut next_fn, mut next_struct) = (0, 0);
    // The span of each resolved field, parallel to `StructDef::fields`,
    // for the recursive-struct check below.
    let mut field_spans: Vec<Vec<Span>> = Vec::new();
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
                Item::Struct(decl) => {
                    let mut seen: HashMap<&str, Span> = HashMap::new();
                    let mut fields = Vec::new();
                    let mut spans = Vec::new();
                    for field in &decl.fields {
                        let name = &field.name;
                        if let Some(&first) = seen.get(name.name.as_str()) {
                            diagnostics.push(duplicate(&name.name, name.span, first));
                        }
                        seen.entry(&name.name).or_insert(name.span);
                        match symbols.resolve_type(&field.ty, module.id) {
                            Ok(ty) => {
                                if decl.is_pub {
                                    if let Some(diagnostic) =
                                        symbols.private_in_public(ty, field.ty.name.span)
                                    {
                                        diagnostics.push(diagnostic);
                                    }
                                }
                                fields.push((name.name.clone(), ty));
                                spans.push(field.span);
                            }
                            Err(diagnostic) => diagnostics.push(diagnostic),
                        }
                    }
                    symbols.structs[next_struct].fields = fields;
                    field_spans.push(spans);
                    next_struct += 1;
                }
                Item::Mod(_) => {}
            }
        }
    }

    check_recursive_structs(&symbols, &field_spans, diagnostics);
    symbols
}

/// V0109 for every struct that contains itself, directly or through other
/// structs' fields (rustc E0072). A depth-first walk over struct-typed
/// fields reports each cycle once, at the field that closes it.
fn check_recursive_structs(
    symbols: &Symbols,
    field_spans: &[Vec<Span>],
    diagnostics: &mut Vec<Diagnostic>,
) {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Unvisited,
        OnStack,
        Done,
    }

    fn visit(
        id: usize,
        symbols: &Symbols,
        field_spans: &[Vec<Span>],
        state: &mut [State],
        stack: &mut Vec<usize>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        state[id] = State::OnStack;
        stack.push(id);
        for (index, (_, ty)) in symbols.structs[id].fields.iter().enumerate() {
            let Ty::Struct(target) = ty else {
                continue;
            };
            let target = target.0 as usize;
            match state[target] {
                State::Unvisited => {
                    visit(target, symbols, field_spans, state, stack, diagnostics);
                }
                State::OnStack => {
                    let start = stack
                        .iter()
                        .position(|&s| s == target)
                        .expect("an on-stack struct is on the stack");
                    let cycle: Vec<&str> = stack[start..]
                        .iter()
                        .chain(std::iter::once(&target))
                        .map(|&s| symbols.structs[s].name.as_str())
                        .collect();
                    diagnostics.push(
                        Diagnostic::new(
                            codes::V0109,
                            field_spans[id][index],
                            "a struct cannot contain itself",
                        )
                        .with_note(format!("the cycle is {}", cycle.join(" -> "))),
                    );
                }
                State::Done => {}
            }
        }
        stack.pop();
        state[id] = State::Done;
    }

    let mut state = vec![State::Unvisited; symbols.structs.len()];
    let mut stack = Vec::new();
    for id in 0..symbols.structs.len() {
        if state[id] == State::Unvisited {
            visit(
                id,
                symbols,
                field_spans,
                &mut state,
                &mut stack,
                diagnostics,
            );
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
                    if let Some(diagnostic) = symbols.private_in_public(ty, param.ty.name.span) {
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
                    if let Some(diagnostic) = symbols.private_in_public(ty, written.name.span) {
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
    use crate::types::IntKind;

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
        assert_eq!(hidden, Err(LookupError::NotVisible(expected)));

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
}
