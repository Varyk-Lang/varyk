//! The type checker (spec sections 4.1, 4.2, 4.5): one pass that lowers
//! each function's AST to typed HIR while checking it.
//!
//! Paths resolve through [`Symbols::lookup_fn`] and
//! [`Symbols::lookup_type`]; locals through a per-function scope stack.
//! A literal takes the type its immediate context expects, passed down as
//! `expected`, with `i32` and `f64` as fallbacks; nothing is inferred
//! backwards. Diagnostics from every function are collected and returned
//! together. Within a function, a failed expression makes its enclosing
//! statement fail, and checking resumes at the next statement.

use std::collections::HashMap;

use varyk_syntax::{
    BinaryOp, Block, Expr, ExprKind, FixIt, ForHead, Function, Ident, Item, Path, SourceFile, Span,
    Stmt, TypeExpr, UnaryOp, UseDecl,
};

use crate::builtins::Owner;
use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{
    AssertKind, HirBlock, HirEnum, HirExpr, HirExprKind, HirForHead, HirFunction, HirModule,
    HirModuleKind, HirParam, HirProgram, HirStmt, HirStruct, HirUse, LocalId, LocalInfo, LocalKind,
    MethodRef, PlaceInfo, is_place,
};
use crate::interop::tidy_signature;
use crate::package::Kind;
use crate::resolve::{
    Callee, FnId, FnSig, ImportedSig, LookupError, ModuleId, ModuleKind, Resolved, StructDef,
    StructId, Symbols, Unusable, UserType, VariantFieldsDef, display_path, is_visible, no_parent,
    not_visible, path_text, rename_help, reserved_value_name, split_last,
};
use crate::types::derives::{self, Blocker, Judged, Medium, Reached};
use crate::types::{FloatKind, IntKind, ParamMode, Ty};

/// Type-checks every Varyk function and lowers the program to HIR, or
/// returns every diagnostic found. `sources` are the files `resolved` was
/// read from, indexed by `FileId.0`, for fix-its that quote the source.
pub fn typecheck(
    resolved: Resolved,
    sources: &[SourceFile],
) -> Result<HirProgram, Vec<Diagnostic>> {
    let symbols = &resolved.symbols;
    let mut diagnostics = Vec::new();
    let mut functions = Vec::new();
    let derives = derives::compute(&symbols.structs, &symbols.enums);
    let base = location_base(&resolved, sources);
    // Whether a `varyk-std` call is made; naming `Error` is found below.
    let mut uses_std = false;
    let mut logs = false;
    // The structs and enums the `json` calls reach (M5a spec 2.4).
    let mut reached = Reached::new(symbols.structs.len(), symbols.enums.len());
    // And those the `env` calls reach (M5a spec 2.5).
    let mut env_reached = Reached::new(symbols.structs.len(), symbols.enums.len());
    // Per function, the calls it makes to async functions (milestone 5b1
    // spec 2.2), for the cycle check.
    let mut async_calls = Vec::new();

    for (index, sig) in symbols.fns.iter().enumerate() {
        let id = FnId(index as u32);
        let decl = resolved.fn_decl(id);
        let mut checker = FnChecker {
            symbols,
            derives: &derives,
            sources,
            module: sig.module,
            name: &sig.name,
            ret: sig.ret.clone(),
            locals: Vec::new(),
            scopes: Vec::new(),
            loop_depth: 0,
            loop_heads: 0,
            closures: Vec::new(),
            range_vars: Vec::new(),
            option_try_operand: None,
            uses_std: false,
            logs: false,
            is_test: sig.is_test,
            is_async: sig.is_async,
            await_operand: None,
            await_whole: None,
            async_fix_it: asyncs::async_fix_it(decl, sources),
            async_callees: Vec::new(),
            task_places: Vec::new(),
            task_uses: Vec::new(),
            apps: HashMap::new(),
            base: &base,
            reached: &mut reached,
            env_reached: &mut env_reached,
            diagnostics: &mut diagnostics,
        };
        let function = checker.function(id, sig, decl);
        uses_std |= checker.uses_std;
        logs |= checker.logs;
        async_calls.push(std::mem::take(&mut checker.async_callees));
        // An async `main` or test runs on `varyk-std`'s runtime (milestone
        // 5b1 spec 4).
        let entry_main = sig.name == "main" && sig.owner.is_none() && sig.module == resolved.entry;
        uses_std |= sig.is_async && (sig.is_test || entry_main);
        if let Some(function) = function {
            functions.push(function);
        }
    }
    diagnostics.extend(asyncs::async_cycles(symbols, &async_calls));
    diagnostics.extend(derives::reached_checks(
        &symbols.structs,
        &symbols.enums,
        &reached,
        Medium::Json,
        str::to_string,
    ));
    // A variable is named in upper case (M5a spec 2.5).
    diagnostics.extend(derives::reached_checks(
        &symbols.structs,
        &symbols.enums,
        &env_reached,
        Medium::Env,
        str::to_uppercase,
    ));
    if !diagnostics.is_empty() {
        rename_help(symbols, &resolved.modules, sources, &mut diagnostics);
        return Err(diagnostics);
    }
    // Ledger: every `FnSig` produced a function, in the same order, so
    // `functions[i]` is always the lowering of `symbols.fns[i]`.
    for (index, function) in functions.iter().enumerate() {
        debug_assert!(function.id == FnId(index as u32));
    }

    let structs = symbols
        .structs
        .iter()
        .enumerate()
        .map(|(id, def)| HirStruct {
            name: def.name.clone(),
            module: def.module,
            is_pub: def.is_pub,
            imported: def.imported,
            fields: def.fields.clone(),
            derives: derives.of_struct(id),
            serde: reached.structs[id].or(env_reached.structs[id]),
            package: def.package.clone(),
            span: def.span,
        })
        .collect();
    let enums = symbols
        .enums
        .iter()
        .enumerate()
        .map(|(id, def)| HirEnum {
            name: def.name.clone(),
            module: def.module,
            is_pub: def.is_pub,
            imported: def.imported,
            variants: def.variants.clone(),
            drops: def.drops.clone(),
            derives: derives.of_enum(id),
            serde: reached.enums[id].or(env_reached.enums[id]),
            opaque: def.opaque.clone(),
            package: def.package.clone(),
            span: def.span,
        })
        .collect();
    let imported = symbols.imported.clone();
    let paths: Vec<String> = (0..resolved.modules.len())
        .map(|id| tree_path(&resolved, ModuleId(id as u32), sources))
        .collect();
    let package_modules = symbols.package_module_paths(resolved.modules.len());
    let modules = resolved
        .modules
        .into_iter()
        .zip(paths)
        .map(|(module, path)| {
            let uses = match &module.kind {
                ModuleKind::Varyk(program) => {
                    // `order` counts `Item::Use` entries as `register`
                    // did, so each declaration reads back its own
                    // resolved target even when two of them share a
                    // local name across namespaces (spec 2.1) and a
                    // lookup by name alone could not tell them apart.
                    let mut order = 0;
                    program
                        .items
                        .iter()
                        .filter_map(|item| {
                            let Item::Use(decl) = item else {
                                return None;
                            };
                            let hir = hir_use(symbols, module.id, order, decl);
                            order += 1;
                            Some(hir)
                        })
                        .collect()
                }
                ModuleKind::Rust(_) => Vec::new(),
            };
            HirModule {
                id: module.id,
                decl: symbols.mod_decl(module.id),
                name: module.name,
                parent: module.parent,
                is_pub: module.is_pub,
                path,
                uses,
                kind: match module.kind {
                    ModuleKind::Varyk(_) => HirModuleKind::Varyk,
                    ModuleKind::Rust(source) => HirModuleKind::Rust {
                        path: sources[module.file.0 as usize].path.clone(),
                        source,
                    },
                },
            }
        })
        .collect();

    Ok(HirProgram {
        uses_std: uses_std || names_error(symbols, &functions) || own_rust_names_std(symbols),
        logs,
        modules,
        functions,
        structs,
        enums,
        imported,
        entry: resolved.entry,
        package_modules,
    })
}

/// Whether a signature of one of the program's own `.rs` modules names
/// `varyk-std`, called or not: rustc compiles the module whole
/// (milestone 5b3 spec 2.4). One reached through a dependency package
/// counts only where it is called.
fn own_rust_names_std(symbols: &Symbols) -> bool {
    symbols
        .imported
        .iter()
        .any(|sig| sig.package.is_none() && sig.names_std)
}

/// Whether the program names `Error` (M5a spec 1): in a signature, a
/// field, a variant's payload, or a local's type, which is where every
/// written type argument ends up. A struct or enum of another Varyk
/// package is not counted (M5b2 spec 7.4).
fn names_error(symbols: &Symbols, functions: &[HirFunction]) -> bool {
    // Another package's types are that package's to build.
    let fields = symbols
        .structs
        .iter()
        .filter(|def| def.package.is_none())
        .flat_map(|def| def.fields.iter().map(|field| &field.ty));
    let payloads = symbols
        .enums
        .iter()
        .filter(|def| def.package.is_none())
        .flat_map(|def| def.variants.iter())
        .flat_map(|variant| match &variant.fields {
            VariantFieldsDef::Tuple(types) => types.iter().collect::<Vec<_>>(),
            VariantFieldsDef::Named(fields) => fields.iter().map(|(_, ty)| ty).collect(),
        });
    let locals = functions.iter().flat_map(|function| {
        let ret = std::iter::once(&function.ret);
        ret.chain(function.locals.iter().map(|local| &local.ty))
    });
    fields.chain(payloads).chain(locals).any(Ty::has_error)
}

/// `decl`, the `order`-th `use` declared in `module` (source order):
/// lowered to its emitted form, the canonical `crate::`-rooted path of
/// whatever it names (spec 3.3), from
/// [`Symbols::use_target_path_at`] rather than a lookup by name, since two
/// declarations of one module can introduce the same local name in
/// different namespaces (spec 2.1).
fn hir_use(symbols: &Symbols, module: ModuleId, order: usize, decl: &UseDecl) -> HirUse {
    let path = symbols
        .use_target_path_at(module, order)
        .expect("a `use` that resolved during `resolve` always has a recorded target");
    HirUse {
        path,
        alias: decl.alias.as_ref().map(|alias| alias.name.clone()),
        is_pub: decl.is_pub,
        span: decl.span,
    }
}

/// The directory an `assert`'s location is relative to (M5a spec 2.7),
/// so the generated Rust is the same wherever `varyk` runs: a package's
/// root, the directory of its `Cargo.toml` (`src/store.vr`), or a single
/// file's own directory (`main.vr`).
fn location_base(resolved: &Resolved, sources: &[SourceFile]) -> std::path::PathBuf {
    let manifest = sources.iter().find(|source| {
        source
            .path
            .file_name()
            .is_some_and(|name| name == "Cargo.toml")
    });
    let entry = resolved
        .modules
        .iter()
        .find(|module| module.parent.is_none())
        .and_then(|module| sources.get(module.file.0 as usize));
    manifest
        .or(entry)
        .and_then(|source| source.path.parent())
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default()
}

/// `path` relative to `base`, its parts joined with `/` on every
/// platform; the file name alone if it is not under `base`.
fn location_path(path: &std::path::Path, base: &std::path::Path) -> String {
    let relative = match path.strip_prefix(base) {
        Ok(relative) => relative,
        Err(_) => path.file_name().map_or(path, std::path::Path::new),
    };
    relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Module `id`'s generated file under `src/` (spec 2.3): `main.rs` for
/// a binary's entry, `lib.rs` for a library's; else its ancestors' names
/// as directories and its own as the file, `shop/cart.rs`, or
/// `shop/mod.rs` for a module read from `shop/mod.vr`.
fn tree_path(resolved: &Resolved, id: ModuleId, sources: &[SourceFile]) -> String {
    let module = &resolved.modules[id.0 as usize];
    let Some(parent) = module.parent else {
        return match resolved.kind {
            Kind::Binary => "main.rs",
            Kind::Library => "lib.rs",
        }
        .to_string();
    };
    let mut dirs = Vec::new();
    let mut current = parent;
    while let Some(up) = resolved.modules[current.0 as usize].parent {
        dirs.push(resolved.modules[current.0 as usize].name.as_str());
        current = up;
    }
    dirs.reverse();
    let from_mod_vr = sources[module.file.0 as usize]
        .path
        .file_name()
        .is_some_and(|name| name == "mod.vr");
    if from_mod_vr {
        dirs.push(&module.name);
        dirs.push("mod.rs");
        return dirs.join("/");
    }
    let file = format!("{}.rs", module.name);
    dirs.push(&file);
    dirs.join("/")
}

/// A scope maps a name to its local, or to `None` for a binding whose
/// initializer failed to check: uses of it fail silently, so one error
/// does not cascade into "unknown name" at every later use.
type Scope = HashMap<String, Option<LocalId>>;

/// What a callee's parameters take beyond their types (milestone 5b3
/// spec 2.2, 2.3; milestone 5b4 spec 2.7): the default for a callee with
/// none of them, which every Varyk function and built-in is.
#[derive(Debug, Default)]
struct ArgRules {
    /// Per parameter, whether it takes only text written in the program.
    literal: Vec<bool>,
    /// Any number of values follow the fixed arguments.
    variadic: bool,
    /// The parameter whose type is its argument's.
    serialize: Option<usize>,
}

impl ArgRules {
    /// Those of the imported function `sig`.
    fn of(sig: &ImportedSig) -> ArgRules {
        ArgRules {
            literal: sig.literal.clone(),
            variadic: sig.variadic,
            serialize: sig.serialize_param,
        }
    }
}

/// Checks one function. Every method returning `Option` has already
/// reported a diagnostic when it returns `None`.
struct FnChecker<'a> {
    symbols: &'a Symbols,
    /// Which types can be cloned and compared (M4 spec 2.10).
    derives: &'a Judged<'a>,
    sources: &'a [SourceFile],
    module: ModuleId,
    /// The function's name, for the message of a misused `?`.
    name: &'a str,
    ret: Ty,
    locals: Vec<LocalInfo>,
    scopes: Vec<Scope>,
    /// How many `while` and `for` loops enclose the current statement,
    /// within the innermost closure.
    loop_depth: u32,
    /// How many `while` conditions and `while let` values enclose the
    /// current expression: each runs every time around, as a loop's body
    /// does, though `break` and `continue` there leave no loop (milestone
    /// 5b4 spec 2.1, V0221).
    loop_heads: u32,
    /// The closures whose bodies enclose the current expression,
    /// outermost first.
    closures: Vec<closures::Frame>,
    /// The variables of `for` loops over a range, for the note of a V0200
    /// on one (its type is its range's).
    range_vars: Vec<LocalId>,
    /// The span of the operand of a `?` being checked in a function
    /// returning `Option`, so that `parse()?` there is V0206 rather than a
    /// mismatch (M4 spec 2.6, M5a spec 2.8).
    option_try_operand: Option<Span>,
    /// Whether the function makes a `varyk-std` call (M5a spec 1).
    uses_std: bool,
    /// Whether a `log` call was checked (M5a spec 2.6).
    logs: bool,
    /// Whether the function is a `#[test]`, the one place `assert` and
    /// `assert_eq` may be called (M5a spec 2.7).
    is_test: bool,
    /// Whether the function is async, the one place a call to an async
    /// function may be made (milestone 5b1 spec 2.2).
    is_async: bool,
    /// The span of the operand of the `.await` being checked: a call of an
    /// async function there is awaited (milestone 5b1 spec 2.3).
    await_operand: Option<Span>,
    /// The span of that whole `.await` expression, operand included: a
    /// `?` added to the operand goes after it.
    await_whole: Option<Span>,
    /// The fix-it making this function async, for V0211.
    async_fix_it: FixIt,
    /// The calls this function makes to async Varyk functions, for the
    /// cycle check (V0214).
    async_callees: Vec<(FnId, Span)>,
    /// The expressions being checked that stand where a task may be made
    /// (milestone 5b1 spec 2.3), by span, innermost last.
    task_places: Vec<(Span, asyncs::TaskPlace)>,
    /// The locals holding a task, or a `Vec` of tasks, that something
    /// uses (milestone 5b1 spec 2.4).
    task_uses: Vec<LocalId>,
    /// The locals bound to `App::new(..)` by a `let` of this function
    /// (milestone 5b4 spec 2.1): the apps whose routes may be added here.
    apps: HashMap<LocalId, routes::AppLocal>,
    /// The directory an `assert`'s location is written from (see
    /// [`location_base`]).
    base: &'a std::path::Path,
    /// What the `json` calls of every function reach so far.
    reached: &'a mut Reached,
    /// What the `env` calls reach.
    env_reached: &'a mut Reached,
    diagnostics: &'a mut Vec<Diagnostic>,
}

impl FnChecker<'_> {
    fn function(&mut self, id: FnId, sig: &FnSig, decl: &Function) -> Option<HirFunction> {
        self.scopes.push(Scope::new());
        let mut params = Vec::new();
        // A method's receiver is its first local, a parameter like any
        // other to borrow analysis (spec 2.5).
        if let (Some(mode), Some(owner), Some(span)) = (sig.self_mode, sig.owner, decl.self_span) {
            let name = Ident {
                name: "self".to_string(),
                span,
            };
            let ty = owner.ty();
            let local = self.bind(&name, ty.clone(), mode == ParamMode::MutableBorrow);
            params.push(HirParam {
                local,
                name: name.name,
                ty,
                mode,
                span,
            });
        }
        for (param, (name, ty, mode)) in decl.params.iter().zip(&sig.params) {
            let local = self.bind(&param.name, ty.clone(), param.mutable);
            params.push(HirParam {
                local,
                name: name.clone(),
                ty: ty.clone(),
                mode: *mode,
                span: param.span,
            });
        }
        let body = self.block(&decl.body, Some(sig.ret.clone()));
        self.scopes.pop();
        let body = body?;
        let before = self.diagnostics.len();
        unfinished_chains_in_block(&body, self.diagnostics);
        self.unused_tasks();
        if self.diagnostics.len() > before {
            return None;
        }

        if body.ty != sig.ret {
            match &body.tail {
                Some(tail) => {
                    self.mismatch(tail.span, &sig.ret, &tail.ty);
                }
                None => {
                    let end = decl.body.span.end;
                    let brace = Span::new(decl.body.span.file, end.saturating_sub(1), end);
                    let diagnostic = Diagnostic::new(
                        codes::V0200,
                        brace,
                        format!(
                            "mismatched types: expected `{}`, found `()`",
                            self.ty_name(&sig.ret)
                        ),
                    )
                    .with_note(
                        "the function body has no tail expression and does not return on every path",
                    );
                    self.diagnostics.push(diagnostic);
                }
            }
            return None;
        }

        Some(HirFunction {
            id,
            module: sig.module,
            owner: sig.owner,
            name: sig.name.clone(),
            is_pub: sig.is_pub,
            is_test: sig.is_test,
            is_async: sig.is_async,
            params,
            ret: sig.ret.clone(),
            body,
            locals: std::mem::take(&mut self.locals),
            span: decl.span,
            ret_root: None,
        })
    }

    // --- Locals ---------------------------------------------------------

    /// Adds a local and makes `name` refer to it in the innermost scope;
    /// V0103 for a reserved name (spec 2.1), bound all the same.
    fn bind(&mut self, name: &Ident, ty: Ty, mutable: bool) -> LocalId {
        self.diagnostics
            .extend(reserved_value_name(&name.name, name.span));
        // Rust reads such a name as the variant, not a new binding (E0170).
        if let Ty::Enum(enum_id) = &ty {
            let def = &self.symbols.enums[enum_id.0 as usize];
            if def
                .variants
                .iter()
                .any(|variant| variant.name == name.name && variant.is_unit())
            {
                let (n, e) = (&name.name, &def.name);
                self.diagnostics.push(
                    Diagnostic::new(
                        codes::V0103,
                        name.span,
                        format!("the name `{n}` is already taken by a variant of `{e}`"),
                    )
                    .with_note(format!(
                        "pick another name; to match the variant, write `{e}::{n}`"
                    )),
                );
            }
        }
        let id = LocalId(self.locals.len() as u32);
        self.locals.push(LocalInfo {
            name: name.name.clone(),
            ty,
            mutable,
            span: name.span,
            place: PlaceInfo::default(),
            repr: None,
            kind: LocalKind::Plain,
        });
        // `_` binds nothing: a later `_` used as a value is V0100.
        if name.name != "_" {
            self.scope().insert(name.name.clone(), Some(id));
        }
        id
    }

    /// Makes `name` refer to a binding whose initializer failed.
    fn bind_poisoned(&mut self, name: &Ident) {
        self.scope().insert(name.name.clone(), None);
    }

    fn scope(&mut self) -> &mut Scope {
        self.scopes
            .last_mut()
            .expect("a function always has a scope")
    }

    // --- Blocks and statements ------------------------------------------

    fn block(&mut self, block: &Block, expected: Option<Ty>) -> Option<HirBlock> {
        self.scopes.push(Scope::new());
        let mut failed = false;
        let mut stmts = Vec::new();
        for stmt in &block.stmts {
            match self.stmt(stmt) {
                Some(stmt) => stmts.push(stmt),
                None => failed = true,
            }
        }
        let tail = block
            .tail
            .as_deref()
            .map(|tail| self.expr_or_route(tail, expected.clone()));
        self.scopes.pop();

        // A route or hook call as the tail is a statement all the same.
        let tail = match tail {
            Some(Some(routes::Lowered::Expr(tail))) => Some(Box::new(tail)),
            Some(Some(routes::Lowered::Stmt(stmt))) => {
                stmts.push(*stmt);
                None
            }
            Some(None) => return None,
            None => None,
        };
        if failed {
            return None;
        }
        let ty = match &tail {
            Some(tail) => tail.ty.clone(),
            // A block that always leaves early fits wherever it is used,
            // like Rust's `!`.
            None if stmts.iter().any(stmt_diverges) => expected.unwrap_or(Ty::Unit),
            None => Ty::Unit,
        };
        Some(HirBlock {
            stmts,
            tail,
            ty,
            span: block.span,
        })
    }

    fn stmt(&mut self, stmt: &Stmt) -> Option<HirStmt> {
        match stmt {
            Stmt::Let {
                name,
                mutable,
                ty,
                value,
                span,
            } => {
                let annotated = match ty {
                    None => None,
                    Some(ty) => match self.symbols.resolve_binding_type(ty, self.module) {
                        Ok(ty) => Some(ty),
                        Err(diagnostic) => {
                            self.diagnostics.push(diagnostic);
                            self.bind_poisoned(name);
                            return None;
                        }
                    },
                };
                // A task is kept by a `let` with a name (milestone 5b1
                // spec 2.3); `let _` drops it at once.
                let value = if name.name == "_" {
                    self.expr(value, annotated.clone())
                } else {
                    self.expr_in(value, annotated.clone(), asyncs::TaskPlace::Let)
                };
                let value = match (value, annotated) {
                    (Some(value), Some(ty)) => self.expect(value, &ty),
                    (value, _) => value,
                };
                let Some(value) = value else {
                    self.bind_poisoned(name);
                    return None;
                };
                let local = self.bind(name, value.ty.clone(), *mutable);
                self.bind_app(local, &value);
                Some(HirStmt::Let {
                    local,
                    value,
                    span: *span,
                })
            }
            Stmt::Assign {
                target,
                value,
                span,
            } => {
                let target = self.expr(target, None)?;
                if !is_place(&target) {
                    self.diagnostics.push(Diagnostic::new(
                        codes::V0002,
                        target.span,
                        "invalid assignment target: expected a variable, or a field or element of one",
                    ));
                    return None;
                }
                self.app_assigned(&target)?;
                let value = self.expr(value, Some(target.ty.clone()))?;
                let value = self.expect(value, &target.ty)?;
                Some(HirStmt::Assign {
                    target,
                    value,
                    span: *span,
                })
            }
            Stmt::Expr {
                expr,
                span,
                has_semi,
            } => {
                let expr = match self.expr_or_route(expr, None)? {
                    routes::Lowered::Stmt(stmt) => return Some(*stmt),
                    routes::Lowered::Expr(expr) => expr,
                };
                if !has_semi {
                    // A block-like expression mid-block without `;` must be `()`.
                    return self
                        .expect(expr, &Ty::Unit)
                        .map(|expr| HirStmt::Expr { expr, span: *span });
                }
                Some(HirStmt::Expr { expr, span: *span })
            }
            Stmt::Return { value, span } => {
                let keyword = Span::new(span.file, span.start, span.start + "return".len() as u32);
                self.leaves_closure("`return`", keyword)?;
                let value = match value {
                    Some(value) => {
                        let ret = self.ret.clone();
                        let value = self.expr(value, Some(ret.clone()))?;
                        Some(self.expect(value, &ret)?)
                    }
                    None if self.ret != Ty::Unit => {
                        let message = format!(
                            "`return` needs a value of type `{}` here",
                            self.ty_name(&self.ret)
                        );
                        self.diagnostics
                            .push(Diagnostic::new(codes::V0200, *span, message));
                        return None;
                    }
                    None => None,
                };
                Some(HirStmt::Return { value, span: *span })
            }
            Stmt::While { cond, body, span } => {
                // The condition runs each time around, as the body does.
                self.loop_heads += 1;
                let cond = self.condition(cond);
                self.loop_heads -= 1;
                self.loop_depth += 1;
                let body = self.block(body, Some(Ty::Unit));
                self.loop_depth -= 1;
                let (cond, body) = (cond?, body?);
                if body.ty != Ty::Unit {
                    let tail = body.tail.as_deref().expect("a non-unit block has a tail");
                    self.mismatch(tail.span, &Ty::Unit, &tail.ty);
                    return None;
                }
                Some(HirStmt::While {
                    cond,
                    body,
                    span: *span,
                })
            }
            Stmt::WhileLet {
                pattern,
                value,
                body,
                span,
            } => self.while_let(pattern, value, body, *span),
            Stmt::For {
                var,
                head,
                body,
                span,
            } => {
                let head = self.for_head(head);
                // The variable is a fresh local of the body's scope, bound
                // even when the head failed so the body is still checked.
                self.scopes.push(Scope::new());
                let local = match &head {
                    Some((head, item)) => {
                        let local = self.bind(var, item.clone(), false);
                        if matches!(head, HirForHead::Range { .. }) {
                            self.range_vars.push(local);
                        }
                        Some(local)
                    }
                    None => {
                        self.bind_poisoned(var);
                        None
                    }
                };
                self.loop_depth += 1;
                let body = self.block(body, Some(Ty::Unit));
                self.loop_depth -= 1;
                self.scopes.pop();
                let ((head, _), local, body) = (head?, local?, body?);
                if body.ty != Ty::Unit {
                    let tail = body.tail.as_deref().expect("a non-unit block has a tail");
                    self.mismatch(tail.span, &Ty::Unit, &tail.ty);
                    return None;
                }
                Some(HirStmt::For {
                    local,
                    head,
                    body,
                    span: *span,
                })
            }
            Stmt::Break { span } => {
                self.require_loop("break", *span)?;
                Some(HirStmt::Break { span: *span })
            }
            Stmt::Continue { span } => {
                self.require_loop("continue", *span)?;
                Some(HirStmt::Continue { span: *span })
            }
        }
    }

    /// What a `for` goes over (spec 2.4), with the type of its variable: a
    /// range, a chain, whose variable is its item (M4 spec 2.3), or a `Vec`
    /// that is a place or a temporary (V0001 otherwise, as for `match`); a
    /// `HashMap` is V0001 naming `keys()` and `values()` (M4 spec 2.7),
    /// anything else V0200.
    fn for_head(&mut self, head: &ForHead) -> Option<(HirForHead, Ty)> {
        let head = match head {
            ForHead::Range {
                start,
                end,
                inclusive,
            } => return self.range(start, end, *inclusive),
            ForHead::Expr(head) => self.expr(head, None)?,
        };
        if let Ty::HashMap(..) = head.ty {
            let message = format!(
                "`for` cannot go over a whole `{}`, since each round would need a key and a \
                 value together; use `keys()` or `values()`",
                self.ty_name(&head.ty)
            );
            self.diagnostics
                .push(Diagnostic::new(codes::V0001, head.span, message));
            return None;
        }
        if let Ty::Chain(item) = &head.ty {
            // Anything but a call of a chain is left to the V0208 walk.
            let item = (**item).clone();
            return Some((HirForHead::Chain(head), item));
        }
        self.check_head(&head, "the `Vec` looped over", "loop over")?;
        let Ty::Vec(item) = &head.ty else {
            let message = format!(
                "`for` goes over a `Vec` or a range such as `0..n`, and this is `{}`",
                self.ty_name(&head.ty)
            );
            self.diagnostics
                .push(Diagnostic::new(codes::V0200, head.span, message));
            return None;
        };
        let item = (**item).clone();
        Some((HirForHead::Vec(head), item))
    }

    /// `start..end` or `start..=end` in a `for` head: two integers of one type, a literal
    /// end taking the other end's type, as the operands of `+` do.
    fn range(&mut self, start: &Expr, end: &Expr, inclusive: bool) -> Option<(HirForHead, Ty)> {
        // The end that is not a literal is checked first and types the other.
        let start_first = !(is_literal(start) && !is_literal(end));
        let (first, second) = if start_first {
            (start, end)
        } else {
            (end, start)
        };
        let first = self.expr(first, None)?;
        let second = self.expr(second, Some(first.ty.clone()))?;
        if second.ty != first.ty {
            self.mismatched_operands(&first, &second, "end");
            return None;
        }
        let (start, end) = if start_first {
            (first, second)
        } else {
            (second, first)
        };
        if !matches!(start.ty, Ty::Int(_)) {
            let span = Span::new(start.span.file, start.span.start, end.span.end);
            let message = format!(
                "the ends of a range must be integers, and these are `{}`",
                self.ty_name(&start.ty)
            );
            self.diagnostics
                .push(Diagnostic::new(codes::V0200, span, message));
            return None;
        }
        let ty = start.ty.clone();
        let (start, end) = (Box::new(start), Box::new(end));
        Some((
            HirForHead::Range {
                start,
                end,
                inclusive,
            },
            ty,
        ))
    }

    /// V0002 unless a `while` or `for` loop encloses the `keyword`
    /// statement; the error points at the keyword alone, not the trailing
    /// `;`.
    fn require_loop(&mut self, keyword: &str, stmt: Span) -> Option<()> {
        if self.loop_depth > 0 {
            return Some(());
        }
        let span = Span::new(stmt.file, stmt.start, stmt.start + keyword.len() as u32);
        self.leaves_closure(&format!("`{keyword}`"), span)?;
        let message = format!("`{keyword}` is only allowed inside a `while` or `for` loop");
        self.diagnostics
            .push(Diagnostic::new(codes::V0002, span, message));
        None
    }

    // --- Expressions ----------------------------------------------------

    /// Checks `expr`; a value whose type is declared in a package this
    /// one does not list is V0115 (M5b2 spec 2.4), and fails.
    fn expr(&mut self, expr: &Expr, expected: Option<Ty>) -> Option<HirExpr> {
        let checked = self.expr_of_any_package(expr, expected)?;
        if let Some(diagnostic) = self.symbols.unlisted_package(&checked.ty, checked.span) {
            self.diagnostics.push(diagnostic);
            return None;
        }
        Some(checked)
    }

    fn expr_of_any_package(&mut self, expr: &Expr, expected: Option<Ty>) -> Option<HirExpr> {
        let span = expr.span;
        let (kind, ty) = match &expr.kind {
            ExprKind::Integer(text) => return self.integer(text, expected, false, span),
            ExprKind::Float(text) => return self.float(text, expected, span),
            ExprKind::Bool(value) => (HirExprKind::Bool(*value), Ty::Bool),
            ExprKind::String(text) => (HirExprKind::String(text.clone()), Ty::String),
            ExprKind::Path { path, name } => {
                return self.path_value(path.as_ref(), name, expected, span);
            }
            ExprKind::Unary { op, operand } => {
                return self.unary(*op, operand, expected, span, true);
            }
            ExprKind::Binary { op, lhs, rhs } => {
                return self.binary(*op, lhs, rhs, expected, span);
            }
            ExprKind::Call { callee, args } => return self.call(callee, args, expected, span),
            ExprKind::Field { base, name } => {
                let base = self.expr(base, None)?;
                // A `Shared`'s fields are its struct's (milestone 5b1
                // spec 2.6).
                let field = match *base.ty.reached() {
                    Ty::Struct(id) => self.symbols.structs[id.0 as usize]
                        .fields
                        .iter()
                        .position(|field| field.name == name.name),
                    _ => None,
                };
                let Some(field_index) = field else {
                    let message = format!(
                        "no field `{}` on type `{}`",
                        name.name,
                        self.ty_name(&base.ty)
                    );
                    self.diagnostics
                        .push(Diagnostic::new(codes::V0102, name.span, message));
                    return None;
                };
                let Ty::Struct(id) = *base.ty.reached() else {
                    unreachable!("only a struct has fields")
                };
                if !field_visible(self.symbols, self.module, id, field_index) {
                    let diagnostic = self.symbols.private_field(id, field_index, name.span);
                    self.diagnostics.push(diagnostic);
                    return None;
                }
                let def = &self.symbols.structs[id.0 as usize];
                if let Some(unusable) =
                    def.fields[field_index]
                        .unusable
                        .as_ref()
                        .filter(|unusable| {
                            unusable.within.is_none()
                                || !self.symbols.usable_from(unusable.within, self.module)
                        })
                {
                    let diagnostic = unusable_field(def, field_index, unusable, name.span);
                    self.diagnostics.push(diagnostic);
                    return None;
                }
                let ty = def.fields[field_index].ty.clone();
                let kind = HirExprKind::Field {
                    base: Box::new(base),
                    name: name.name.clone(),
                };
                (kind, ty)
            }
            ExprKind::StructLit { path, name, fields } => {
                return self.struct_lit(path.as_ref(), name, fields, span);
            }
            ExprKind::Block(block) => {
                let block = self.block(block, expected)?;
                let ty = block.ty.clone();
                (HirExprKind::Block(block), ty)
            }
            ExprKind::If { cond, then, else_ } => {
                return self.if_expr(cond, then, else_.as_ref(), expected, span);
            }
            ExprKind::Intrinsic { name, format, args } => {
                let args = self.format_args(&format.0, format.1, args, span)?;
                let format = format.0.clone();
                if name.name == "format" {
                    (HirExprKind::Format { format, args }, Ty::String)
                } else {
                    (HirExprKind::Println { format, args }, Ty::Unit)
                }
            }
            ExprKind::MethodCall {
                receiver,
                method,
                args,
            } => return self.method_call(receiver, method, args, expected, span),
            ExprKind::Index { base, index } => return self.index(base, index, span),
            ExprKind::Try { operand } => return self.try_(operand, expected, span),
            ExprKind::Await(operand) => return self.await_expr(operand, expected, span),
            ExprKind::Match { scrutinee, arms } => {
                return self.match_expr(scrutinee, arms, expected, span);
            }
            ExprKind::VecLit(elements) => return self.vec_lit(elements, expected, span),
            ExprKind::IfLet {
                pattern,
                value,
                then,
                else_,
                span: _,
            } => {
                return self.if_let(pattern, value, then, else_.as_ref(), expected, span);
            }
            ExprKind::Closure { .. } => {
                self.diagnostics.push(closures::misplaced(span));
                return None;
            }
            ExprKind::Cast { expr, ty, .. } => return self.cast(expr, ty, span),
        };
        Some(HirExpr { kind, ty, span })
    }

    /// `expr as T` (M4 spec 2.9): both the operand and the named type are
    /// number types (V0200 otherwise). The operand is typed on its own, so
    /// a literal is an `i32` or an `f64`.
    fn cast(&mut self, operand: &Expr, target: &TypeExpr, span: Span) -> Option<HirExpr> {
        let operand = self.expr(operand, None);
        let target = match self.symbols.resolve_type(target, self.module) {
            Ok(ty) => ty,
            Err(diagnostic) => {
                self.diagnostics.push(diagnostic);
                return None;
            }
        };
        let operand = operand?;
        let number = |ty: &Ty| matches!(ty, Ty::Int(_) | Ty::Float(_));
        if !number(&operand.ty) || !number(&target) {
            let message = format!(
                "`as` converts between number types, and this is `{}` as `{}`",
                self.ty_name(&operand.ty),
                self.ty_name(&target)
            );
            self.diagnostics.push(
                Diagnostic::new(codes::V0200, span, message)
                    .with_note("both sides of `as` must be integers, `usize`, `f32`, or `f64`"),
            );
            return None;
        }
        Some(HirExpr {
            kind: HirExprKind::Cast {
                expr: Box::new(operand),
                ty: target.clone(),
            },
            ty: target,
            span,
        })
    }

    /// An integer literal takes the expected integer type, `i32` without
    /// one, and V0200 unless its value fits that type. `negated` marks a
    /// literal directly under unary `-`, which may reach the type's minimum
    /// (`-128` fits `i8`).
    fn integer(
        &mut self,
        text: &str,
        expected: Option<Ty>,
        negated: bool,
        span: Span,
    ) -> Option<HirExpr> {
        let kind = match expected {
            Some(Ty::Int(kind)) => kind,
            _ => IntKind::I32,
        };
        let value = text.replace('_', "").parse::<u128>().ok();
        let Some(value) = value.filter(|v| *v <= u128::from(u64::MAX)) else {
            let message = "integer literal is too large";
            self.diagnostics
                .push(Diagnostic::new(codes::V0200, span, message));
            return None;
        };
        let (min, max) = kind.range();
        let limit = if negated && min < 0 {
            min.unsigned_abs()
        } else {
            max as u128
        };
        if value > limit {
            let sign = if negated { "-" } else { "" };
            let message = format!(
                "`{sign}{text}` does not fit in `{}` ({min} to {max})",
                kind.name()
            );
            self.diagnostics
                .push(Diagnostic::new(codes::V0200, span, message));
            return None;
        }
        Some(HirExpr {
            kind: HirExprKind::Int(text.to_string()),
            ty: Ty::Int(kind),
            span,
        })
    }

    /// A float literal takes the expected float type, `f64` without one,
    /// and V0200 unless its value fits that type.
    fn float(&mut self, text: &str, expected: Option<Ty>, span: Span) -> Option<HirExpr> {
        let kind = match expected {
            Some(Ty::Float(kind)) => kind,
            _ => FloatKind::F64,
        };
        let value = text
            .replace('_', "")
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite());
        let Some(value) = value else {
            let message = "float literal is too large";
            self.diagnostics
                .push(Diagnostic::new(codes::V0200, span, message));
            return None;
        };
        if kind == FloatKind::F32 && value.abs() > f64::from(f32::MAX) {
            let message = format!("`{text}` does not fit in `f32`");
            self.diagnostics
                .push(Diagnostic::new(codes::V0200, span, message));
            return None;
        }
        Some(HirExpr {
            kind: HirExprKind::Float(text.to_string()),
            ty: Ty::Float(kind),
            span,
        })
    }

    fn lookup_local(&mut self, name: &Ident) -> Option<LocalId> {
        let found = self
            .scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(&name.name));
        match found {
            Some(local) => {
                let local = *local;
                if let Some(local) = local {
                    self.capture(local);
                }
                local
            }
            None => {
                let message = if name.name == "self" {
                    "`self` is only available inside a method".to_string()
                } else {
                    format!("nothing named `{}` exists here", name.name)
                };
                self.diagnostics
                    .push(Diagnostic::new(codes::V0100, name.span, message));
                None
            }
        }
    }

    /// A `bool` condition of `if` or `while`.
    fn condition(&mut self, cond: &Expr) -> Option<HirExpr> {
        let cond = self.expr(cond, Some(Ty::Bool))?;
        self.expect(cond, &Ty::Bool)
    }

    /// `allow_min` permits a literal directly under this unary `-` to
    /// reach the type's minimum (`-128` fits `i8`); it is false when this
    /// `-` is itself the operand of another unary `-`, so `--128` does
    /// not (rustc rejects the suffixed literal there).
    fn unary(
        &mut self,
        op: UnaryOp,
        operand: &Expr,
        expected: Option<Ty>,
        span: Span,
        allow_min: bool,
    ) -> Option<HirExpr> {
        let operand = match op {
            UnaryOp::Not => {
                let operand = self.expr(operand, Some(Ty::Bool))?;
                self.expect(operand, &Ty::Bool)?
            }
            UnaryOp::Neg => {
                let operand = match &operand.kind {
                    ExprKind::Integer(text) => {
                        self.integer(text, expected, allow_min, operand.span)?
                    }
                    ExprKind::Unary {
                        op: UnaryOp::Neg,
                        operand: inner,
                    } => self.unary(UnaryOp::Neg, inner, expected, operand.span, false)?,
                    _ => self.expr(operand, expected)?,
                };
                if !is_signed(&operand.ty) {
                    let message = format!(
                        "unary `-` cannot be applied to `{}`",
                        self.ty_name(&operand.ty)
                    );
                    self.diagnostics
                        .push(Diagnostic::new(codes::V0200, span, message));
                    return None;
                }
                operand
            }
        };
        Some(HirExpr {
            ty: operand.ty.clone(),
            kind: HirExprKind::Unary {
                op,
                operand: Box::new(operand),
            },
            span,
        })
    }

    fn binary(
        &mut self,
        op: BinaryOp,
        lhs: &Expr,
        rhs: &Expr,
        expected: Option<Ty>,
        span: Span,
    ) -> Option<HirExpr> {
        if matches!(op, BinaryOp::And | BinaryOp::Or) {
            let lhs = self
                .expr(lhs, Some(Ty::Bool))
                .and_then(|lhs| self.expect(lhs, &Ty::Bool));
            let rhs = self
                .expr(rhs, Some(Ty::Bool))
                .and_then(|rhs| self.expect(rhs, &Ty::Bool));
            return Some(binary_expr(op, lhs?, rhs?, Ty::Bool, span));
        }

        // An arithmetic result has its operands' type, so the context's
        // expectation passes down to them; a comparison's does not.
        let arithmetic = is_arithmetic(op);
        let outer = if arithmetic { expected } else { None };
        // The non-literal operand is checked first and types the other.
        let lhs_first = !(is_literal(lhs) && !is_literal(rhs));
        let (first, second) = if lhs_first { (lhs, rhs) } else { (rhs, lhs) };
        let first = self.expr(first, outer)?;
        let second = self.expr(second, Some(first.ty.clone()))?;
        if second.ty != first.ty {
            let range_var =
                is_range_var(&first, &self.range_vars) || is_range_var(&second, &self.range_vars);
            if range_var && usize_note(&first.ty, &second.ty).is_some() {
                let message = format!(
                    "mismatched types: expected `{}`, found `{}`",
                    self.ty_name(&first.ty),
                    self.ty_name(&second.ty)
                );
                self.diagnostics.push(
                    Diagnostic::new(codes::V0200, second.span, message).with_note(RANGE_USIZE_NOTE),
                );
                return None;
            }
            self.mismatched_operands(&first, &second, "operand");
            return None;
        }
        let ty = first.ty.clone();
        let (lhs, rhs) = if lhs_first {
            (first, second)
        } else {
            (second, first)
        };

        let symbol = op.as_str();
        if matches!(op, BinaryOp::Eq | BinaryOp::Ne) {
            if ty.has_shared() {
                let message = format!("`{}` cannot be compared with `{symbol}`", self.ty_name(&ty));
                self.diagnostics
                    .push(Diagnostic::new(codes::V0203, span, message).with_note(
                        "a `Shared` is a handle to a struct, not a value of its own; compare \
                         the fields you read through it instead",
                    ));
                return None;
            }
            if let Err(blocker) = self.derives.can_compare(&ty) {
                let headline =
                    format!("`{}` cannot be compared with `{symbol}`", self.ty_name(&ty));
                let rust = "in Rust terms, `==` and `!=` need the type to implement \
                            `PartialEq`, which Varyk derives for a struct or enum whose every \
                            field and payload has it, and reads from `#[derive(..)]` on a Rust \
                            type";
                self.blocked(span, headline, blocker, rust);
                return None;
            }
            if ty == Ty::Unit {
                self.diagnostics.push(Diagnostic::new(
                    codes::V0200,
                    span,
                    format!("`{symbol}` cannot be applied to `()`"),
                ));
                return None;
            }
            return Some(binary_expr(op, lhs, rhs, Ty::Bool, span));
        }

        if ty == Ty::String && op == BinaryOp::Add {
            self.join_strings(&lhs, &rhs, span);
            return None;
        }
        if !matches!(ty, Ty::Int(_) | Ty::Float(_)) {
            let diagnostic = Diagnostic::new(
                codes::V0200,
                span,
                format!("`{symbol}` cannot be applied to `{}`", self.ty_name(&ty)),
            )
            .with_note(format!("`{symbol}` is defined for numeric types only"));
            self.diagnostics.push(diagnostic);
            return None;
        }
        let result = if arithmetic { ty } else { Ty::Bool };
        Some(binary_expr(op, lhs, rhs, result, span))
    }

    /// V0200 at `second`, which should have the type of `first`, the other
    /// `part` (operand, or end of a range) that typed it.
    fn mismatched_operands(&mut self, first: &HirExpr, second: &HirExpr, part: &str) {
        let mut diagnostic = Diagnostic::new(
            codes::V0200,
            second.span,
            format!(
                "mismatched types: expected `{}`, found `{}`",
                self.ty_name(&first.ty),
                self.ty_name(&second.ty)
            ),
        )
        .with_label(
            first.span,
            format!("this {part} is `{}`", self.ty_name(&first.ty)),
        );
        if let Some(note) = usize_note(&first.ty, &second.ty) {
            diagnostic = diagnostic.with_note(note);
        }
        self.diagnostics.push(diagnostic);
    }

    /// V0200 for `lhs + rhs` on strings (spec 2.9), with a fix-it to
    /// `format!`, the one way to join strings.
    fn join_strings(&mut self, lhs: &HirExpr, rhs: &HirExpr, span: Span) {
        let text = |at: Span| {
            let source = &self.sources[at.file.0 as usize].text;
            source[at.start as usize..at.end as usize].to_string()
        };
        let replacement = format!(
            "format!(\"{{}}{{}}\", {}, {})",
            text(lhs.span),
            text(rhs.span)
        );
        let diagnostic = Diagnostic::new(
            codes::V0200,
            span,
            "`+` cannot join strings; join them with `format!` instead",
        )
        .with_note("`+` works on numbers only; `format!` builds new text from its parts")
        .with_fix_it(FixIt { span, replacement });
        self.diagnostics.push(diagnostic);
    }

    fn call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        expected: Option<Ty>,
        span: Span,
    ) -> Option<HirExpr> {
        let ExprKind::Path { path, name } = &callee.kind else {
            // A method call parses as `ExprKind::MethodCall`, never as a
            // `Call` with a `Field` or `MethodCall` callee; anything else
            // here (calling the result of a grouped expression, a method
            // call, an index, and so on) is a shape the parser already
            // flagged with `V0002`, or a defensive fallback if it did not.
            self.diagnostics.push(Diagnostic::new(
                codes::V0001,
                callee.span,
                "only a function's path can be called",
            ));
            return None;
        };
        if let Some(owner) = self.path_owner(path.as_ref(), callee.span) {
            let (owner, owner_path) = owner?;
            return self.type_member(
                owner,
                &owner_path,
                name,
                Some(args),
                expected,
                callee.span,
                span,
            );
        }
        if path.is_none() && is_builtin_variant(&name.name) {
            return self.builtin_variant(&name.name, Some(args), expected, span);
        }
        if path.is_none()
            && self
                .scopes
                .iter()
                .any(|scope| scope.contains_key(&name.name))
        {
            let message = format!("`{}` is a variable here, not a function", name.name);
            self.diagnostics
                .push(Diagnostic::new(codes::V0100, callee.span, message));
            return None;
        }
        if path.is_none() && matches!(name.name.as_str(), "assert" | "assert_eq") {
            return self.assert_call(&name.name, args, callee.span, span);
        }
        // `path_owner` returned `None` (not `Some(None)`, already handled
        // above): the path, if any, is a module path (spec 3.1).
        let module_path = path.as_ref();
        let path = match module_path {
            Some(module) => display_path(module, &name.name),
            None => name.name.clone(),
        };
        let found = match self.symbols.lookup_fn(self.module, module_path, &name.name) {
            Ok(found) => found,
            Err(error) => {
                // `m::M(1)` for a `.rs` tuple struct Varyk did not import
                // (M3 spec 4.1): V0101 saying why.
                let skipped = self.symbols.skipped_item(
                    self.module,
                    module_path,
                    &name.name,
                    &path,
                    callee.span,
                );
                if let (LookupError::Unknown, Some(diagnostic)) = (error, skipped) {
                    self.diagnostics.push(diagnostic);
                    return None;
                }
                // The last module segment may be a type written without
                // its module (`Item::new()`): look for it elsewhere.
                let hint = module_path
                    .and_then(|p| p.segments.last())
                    .map_or(name.name.as_str(), |s| s.name.as_str());
                self.lookup_error(error, "function", &path, callee.span, Some(hint));
                return None;
            }
        };
        let mut started = false;
        // A test is run by `varyk test` alone (M5a spec 2.7).
        if let Callee::Varyk(id) = found {
            let sig = &self.symbols.fns[id.0 as usize];
            // An async `main` is run by the generated `main`, an ordinary
            // Rust function (milestone 5b1 spec 2.2), and so is a `main`
            // that returns a `Result`, whose Rust `main` gives an exit code.
            let entry = sig.module == ModuleId(0);
            let wrapped = sig.is_async || sig.ret != Ty::Unit;
            if wrapped && sig.name == "main" && sig.owner.is_none() && entry {
                self.diagnostics.push(
                    Diagnostic::new(
                        codes::V0106,
                        callee.span,
                        "`main` is where the program starts and cannot be called",
                    )
                    .with_note(
                        "put what `main` does and another function needs in a function of its \
                         own, and call that from both",
                    ),
                );
                return None;
            }
            if sig.is_test {
                self.diagnostics.push(
                    Diagnostic::new(
                        codes::V0114,
                        callee.span,
                        format!("`{path}` is a test and cannot be called"),
                    )
                    .with_note(
                        "`varyk test` runs each test on its own; put what the tests share in \
                         a function without `#[test]` and call that",
                    ),
                );
                return None;
            }
            if sig.is_async {
                started = self.async_call(Some(id), &path, span)?;
            }
        }

        let (params, rules, ret): (Vec<Ty>, ArgRules, Ty) = match found {
            Callee::Varyk(id) => {
                let sig = &self.symbols.fns[id.0 as usize];
                (
                    sig.params.iter().map(|p| p.1.clone()).collect(),
                    ArgRules::default(),
                    sig.ret.clone(),
                )
            }
            Callee::Imported(id) => {
                let sig = &self.symbols.imported[id.0 as usize];
                if !sig.callable || !self.symbols.usable_from(sig.within, self.module) {
                    self.diagnostics
                        .push(unsupported_rust_signature(&path, sig, span));
                    return None;
                }
                // The call names `varyk-std` (milestone 5b3 spec 2.4).
                self.uses_std |= sig.names_std;
                let found = (
                    sig.params.iter().map(|p| p.0.clone()).collect(),
                    ArgRules::of(sig),
                    sig.ret.clone(),
                );
                if sig.is_async {
                    started = self.async_call(None, &path, span)?;
                }
                found
            }
            Callee::Builtin(_) => unreachable!("a plain name never finds a built-in"),
        };

        // A type hole is filled from where the result goes (milestone 5b3
        // spec 2.1).
        let imported = match found {
            Callee::Imported(id) => Some(id),
            _ => None,
        };
        let (ret, type_arg) =
            match self.filled(imported, callee.span, started, expected.as_ref(), span)? {
                Some((ret, t)) => (ret, Some(t)),
                None => (ret, None),
            };
        let (args, trailing) = self.arguments(&path, &params, &rules, args, span)?;
        Some(HirExpr {
            kind: HirExprKind::Call {
                callee: found,
                args,
                trailing,
                type_arg,
                rooted: None,
                started,
            },
            ty: asyncs::started_ty(started, ret),
            span,
        })
    }

    /// `assert(cond)` or `assert_eq(a, b)` (M5a spec 2.7), `name` called
    /// at `name_span`: only in a test (V0114), with the call's Varyk file
    /// and line for the failure message. `assert_eq`'s operands are
    /// checked as `a == b` is, so differing types are V0200 and a type
    /// without `==` is V0203.
    fn assert_call(
        &mut self,
        name: &str,
        args: &[Expr],
        name_span: Span,
        span: Span,
    ) -> Option<HirExpr> {
        if !self.is_test {
            self.diagnostics.push(
                Diagnostic::new(
                    codes::V0114,
                    name_span,
                    format!("`{name}` can only be used inside a `#[test]` function"),
                )
                .with_note(
                    "a failed check stops the program, which has no place in service code; \
                     return an `Err` or handle the case instead",
                ),
            );
            return None;
        }
        let (cond, kind) = if name == "assert" {
            let [cond] = args else {
                self.arguments(name, &[Ty::Bool], &ArgRules::default(), args, span);
                return None;
            };
            let cond = self
                .expr(cond, Some(Ty::Bool))
                .and_then(|cond| self.expect(cond, &Ty::Bool))?;
            (cond, AssertKind::Plain)
        } else {
            let [a, b] = args else {
                let message = format!(
                    "`assert_eq` takes 2 arguments but {} {} given",
                    args.len(),
                    if args.len() == 1 { "was" } else { "were" },
                );
                self.diagnostics
                    .push(Diagnostic::new(codes::V0201, span, message));
                return None;
            };
            let cond = self.binary(BinaryOp::Eq, a, b, None, span)?;
            let HirExprKind::Binary { lhs, .. } = &cond.kind else {
                return None;
            };
            let show = matches!(
                lhs.ty,
                Ty::Bool | Ty::Int(_) | Ty::Float(_) | Ty::String | Ty::Error
            );
            (cond, AssertKind::Eq { show })
        };
        let file = &self.sources[span.file.0 as usize];
        let (line, _) = file.line_col(span.start);
        Some(HirExpr {
            kind: HirExprKind::Assert {
                cond: Box::new(cond),
                location: format!("{}:{line}", location_path(&file.path, self.base)),
                kind,
            },
            ty: Ty::Unit,
            span,
        })
    }

    /// The arguments of a call to `path` (at `span`), each typed against
    /// its parameter type in `params`; V0201 when the count differs.
    /// `rules.literal` says, per parameter, whether it takes only text
    /// written in the program (milestone 5b3 spec 2.3): any other argument
    /// there is V0217. The argument of `rules.serialize` is typed from
    /// itself and checked as a `json::stringify` argument is (milestone
    /// 5b4 spec 2.7).
    ///
    /// When `rules.variadic` (milestone 5b3 spec 2.2), the arguments after
    /// `params` are values, each typed with nothing expected and of a
    /// type [`is_value_type`] admits (V0218), returned apart from the
    /// fixed ones; there may be none, and V0201 says "at least".
    fn arguments(
        &mut self,
        path: &str,
        params: &[Ty],
        rules: &ArgRules,
        args: &[Expr],
        span: Span,
    ) -> Option<(Vec<HirExpr>, Vec<HirExpr>)> {
        let variadic = rules.variadic;
        if args.len() < params.len() || (!variadic && args.len() > params.len()) {
            let plural = |n: usize| if n == 1 { "" } else { "s" };
            let message = format!(
                "`{path}` takes {}{} argument{} but {} {} given",
                if variadic { "at least " } else { "" },
                params.len(),
                plural(params.len()),
                args.len(),
                if args.len() == 1 { "was" } else { "were" },
            );
            self.diagnostics
                .push(Diagnostic::new(codes::V0201, span, message));
            return None;
        }
        let (fixed, values) = args.split_at(params.len());
        let mut checked = Vec::new();
        for (at, (arg, ty)) in fixed.iter().zip(params).enumerate() {
            if rules.literal.get(at) == Some(&true) && !matches!(arg.kind, ExprKind::String(_)) {
                self.diagnostics.push(not_literal_text(path, arg));
                checked.push(None);
                continue;
            }
            if rules.serialize == Some(at) {
                checked.push(self.serialized(arg));
                continue;
            }
            checked.push(
                self.expr(arg, Some(ty.clone()))
                    .and_then(|arg| self.expect(arg, ty)),
            );
        }
        let mut trailing = Vec::new();
        for arg in values {
            // `Some(x)` is passed as `x`, which `Value::from` turns into
            // the same value: `x` is read in place, not put into a new
            // `Option`, which would move or copy it.
            let inner = some_operand(arg);
            let value = self.expr(inner.unwrap_or(arg), None);
            trailing.push(match value {
                Some(value) if inner.is_some() => {
                    let ty = Ty::Option(Box::new(value.ty.clone()));
                    if is_value_type(&ty) {
                        Some(value)
                    } else {
                        let diagnostic = self.not_a_value(path, arg, &ty);
                        self.diagnostics.push(diagnostic);
                        None
                    }
                }
                Some(value) if !is_value_type(&value.ty) => {
                    let diagnostic = self.not_a_value(path, arg, &value.ty);
                    self.diagnostics.push(diagnostic);
                    None
                }
                value => value,
            });
        }
        let fixed: Option<Vec<HirExpr>> = checked.into_iter().collect();
        let trailing: Option<Vec<HirExpr>> = trailing.into_iter().collect();
        Some((fixed?, trailing?))
    }

    /// V0218 at `arg`, a value of type `ty` passed after the other
    /// arguments of `path` (milestone 5b3 spec 2.2), listing the types
    /// that can be; for a `u64` or `usize`, the cast that makes it one.
    fn not_a_value(&self, path: &str, arg: &Expr, ty: &Ty) -> Diagnostic {
        let diagnostic = Diagnostic::new(
            codes::V0218,
            arg.span,
            format!(
                "a value of type `{}` cannot be passed here",
                self.ty_name(ty)
            ),
        )
        .with_note(format!(
            "after its other arguments, `{path}` takes values of type `bool`, `string`, `f32`, \
             `f64`, `i8`, `i16`, `i32`, `i64`, `u8`, `u16`, `u32`, or an `Option` of one of \
             those"
        ));
        if !matches!(ty, Ty::Int(IntKind::U64 | IntKind::Usize)) {
            return diagnostic;
        }
        let diagnostic = diagnostic.with_note(format!(
            "a `{}` can be larger than an `i64` holds; write `as i64` to pass it as an `i64`",
            self.ty_name(ty)
        ));
        match arg.kind {
            // `as` binds tighter than an operator, and a second cast
            // would read `x as u64 as i64`.
            ExprKind::Binary { .. } | ExprKind::Unary { .. } | ExprKind::Cast { .. } => diagnostic,
            _ => diagnostic.with_fix_it(FixIt {
                span: Span::new(arg.span.file, arg.span.end, arg.span.end),
                replacement: " as i64".to_string(),
            }),
        }
    }

    /// `Name { .. }`, or `path::Name { .. }` for a `pub` struct of the
    /// module `path` names (spec 2.10, 3.1).
    fn struct_lit(
        &mut self,
        module: Option<&Path>,
        name: &Ident,
        fields: &[(Ident, Expr)],
        span: Span,
    ) -> Option<HirExpr> {
        let (path, path_span) = match module {
            Some(module) => (
                display_path(module, &name.name),
                Span::new(name.span.file, module.span.start, name.span.end),
            ),
            None => (name.name.clone(), name.span),
        };
        let found = self.symbols.lookup_type(self.module, module, &name.name);
        let id = match found {
            Ok(UserType::Struct(id)) => id,
            Ok(UserType::Enum(_)) => {
                let message = format!("`{path}` is an enum, not a struct");
                self.diagnostics
                    .push(Diagnostic::new(codes::V0101, path_span, message));
                return None;
            }
            Err(LookupError::NoParent { span }) => {
                self.diagnostics.push(no_parent(span));
                return None;
            }
            Err(LookupError::Dependency { span, dep }) => {
                let diagnostic = self.symbols.dependency_error(span, dep);
                self.diagnostics.push(diagnostic);
                return None;
            }
            Err(LookupError::Unknown) => {
                // `Shape::Circle { .. }`: a variant with named fields.
                if let Some((prefix, last)) = module.and_then(split_last) {
                    if let Ok(UserType::Enum(enum_id)) =
                        self.symbols
                            .lookup_type(self.module, prefix.as_ref(), &last.name)
                    {
                        let def = &self.symbols.enums[enum_id.0 as usize];
                        if def.variant(&name.name).is_some() {
                            let owner = module.map(path_text).unwrap_or_default();
                            return self.named_variant(enum_id, &owner, name, fields, span);
                        }
                    }
                }
                let diagnostic = self
                    .symbols
                    .skipped_item(self.module, module, &name.name, &path, path_span)
                    .unwrap_or_else(|| {
                        let message = format!("unknown struct `{path}`");
                        let diagnostic = Diagnostic::new(codes::V0101, path_span, message);
                        match self.symbols.not_added(self.module, &path) {
                            Some(note) => diagnostic.with_note(note),
                            None => diagnostic,
                        }
                    });
                self.diagnostics.push(diagnostic);
                return None;
            }
            Err(LookupError::NotVisible { decl, keyword }) => {
                self.diagnostics
                    .push(not_visible("struct", &path, path_span, decl, keyword));
                return None;
            }
            Err(LookupError::PrivateModule { module }) => {
                let diagnostic = self
                    .symbols
                    .private_module(module, "struct", &path, path_span);
                self.diagnostics.push(diagnostic);
                return None;
            }
        };
        let def = &self.symbols.structs[id.0 as usize];
        // An imported struct's literal needs every field visible and
        // usable (M3 spec 4.1); the Rust module's constructor is the way.
        if def.imported {
            let blocked: Vec<String> = (0..def.fields.len())
                .filter(|&index| {
                    !field_visible(self.symbols, self.module, id, index)
                        || def.fields[index].unusable.as_ref().is_some_and(|unusable| {
                            unusable.within.is_none()
                                || !self.symbols.usable_from(unusable.within, self.module)
                        })
                })
                .map(|index| format!("`{}`", def.fields[index].name))
                .collect();
            if !blocked.is_empty() {
                let message = format!(
                    "`{path} {{ .. }}` cannot be written in Varyk, because the Rust struct has \
                     {} Varyk cannot set: {}",
                    if blocked.len() == 1 {
                        "a field"
                    } else {
                        "fields"
                    },
                    blocked.join(", ")
                );
                self.diagnostics
                    .push(
                        Diagnostic::new(codes::V0105, path_span, message).with_note(format!(
                            "make one with a function of the Rust module instead, such as a \
                         `pub fn new` in an `impl {} {{ .. }}` block there",
                            def.name
                        )),
                    );
                return None;
            }
        }

        let mut failed = false;
        let mut seen: HashMap<usize, Span> = HashMap::new();
        let mut lowered = Vec::new();
        for (field, value) in fields {
            let Some(index) = def.fields.iter().position(|f| f.name == field.name) else {
                let message = format!("struct `{}` has no field `{}`", def.name, field.name);
                self.diagnostics
                    .push(Diagnostic::new(codes::V0102, field.span, message));
                failed = true;
                continue;
            };
            if let Some(&first) = seen.get(&index) {
                let diagnostic = Diagnostic::new(
                    codes::V0103,
                    field.span,
                    format!("field `{}` is specified more than once", field.name),
                )
                .with_label(first, format!("`{}` first specified here", field.name));
                self.diagnostics.push(diagnostic);
                failed = true;
                continue;
            }
            // Recorded as seen even when private, so the missing-field
            // check below does not also complain about it.
            seen.insert(index, field.span);
            if !field_visible(self.symbols, self.module, id, index) {
                let diagnostic = self.symbols.private_field_in_literal(id, index, field.span);
                self.diagnostics.push(diagnostic);
                failed = true;
                continue;
            }
            let ty = def.fields[index].ty.clone();
            match self
                .expr(value, Some(ty.clone()))
                .and_then(|v| self.expect(v, &ty))
            {
                Some(value) => lowered.push((index, value)),
                None => failed = true,
            }
        }

        let missing: Vec<String> = def
            .fields
            .iter()
            .enumerate()
            .filter(|(index, _)| !seen.contains_key(index))
            .map(|(_, field)| format!("`{}`", field.name))
            .collect();
        if !missing.is_empty() {
            let message = format!(
                "missing field{} {} in a `{}` literal",
                if missing.len() == 1 { "" } else { "s" },
                missing.join(", "),
                def.name
            );
            self.diagnostics
                .push(Diagnostic::new(codes::V0200, span, message));
            failed = true;
        }
        if failed {
            return None;
        }
        Some(HirExpr {
            kind: HirExprKind::StructLit {
                id,
                fields: lowered,
            },
            ty: Ty::Struct(id),
            span,
        })
    }

    fn if_expr(
        &mut self,
        cond: &Expr,
        then: &Block,
        else_: Option<&Block>,
        expected: Option<Ty>,
        span: Span,
    ) -> Option<HirExpr> {
        let cond = self.condition(cond);
        let branches = self.branches(
            |checker, expected| checker.block(then, expected),
            else_,
            expected,
        );
        let (cond, (then, else_, ty)) = (cond?, branches?);
        Some(HirExpr {
            kind: HirExprKind::If {
                cond: Box::new(cond),
                then,
                else_,
            },
            ty,
            span,
        })
    }

    /// The arguments of `println!` or `format!` (at `span`), checked
    /// against the placeholders of `format`, the raw text of the string
    /// literal at `format_span`.
    fn format_args(
        &mut self,
        format: &str,
        format_span: Span,
        args: &[Expr],
        span: Span,
    ) -> Option<Vec<HirExpr>> {
        // The raw text starts one byte after the opening quote.
        let text_start = format_span.start + 1;
        let at = |from: usize, to: usize| {
            Span::new(
                format_span.file,
                text_start + from as u32,
                text_start + to as u32,
            )
        };
        let bytes = format.as_bytes();
        let mut placeholders = 0;
        let mut i = 0;
        while i < bytes.len() {
            match (bytes[i], bytes.get(i + 1)) {
                (b'{', Some(b'{')) | (b'}', Some(b'}')) => i += 2,
                (b'{', Some(b'}')) => {
                    placeholders += 1;
                    i += 2;
                }
                (b'{', _) => {
                    let end = format[i..]
                        .find('}')
                        .map_or(bytes.len(), |offset| i + offset + 1);
                    let message = format!(
                        "unsupported placeholder `{}`: only `{{}}` is supported",
                        &format[i..end]
                    );
                    self.diagnostics
                        .push(Diagnostic::new(codes::V0202, at(i, end), message));
                    return None;
                }
                (b'}', _) => {
                    let diagnostic = Diagnostic::new(
                        codes::V0202,
                        at(i, i + 1),
                        "unmatched `}` in format string",
                    )
                    .with_note("write `}}` for a literal `}`");
                    self.diagnostics.push(diagnostic);
                    return None;
                }
                _ => i += 1,
            }
        }

        let mut failed = false;
        if placeholders != args.len() {
            let message = format!(
                "the format string has {placeholders} placeholder{} but {} argument{} {} given",
                if placeholders == 1 { "" } else { "s" },
                args.len(),
                if args.len() == 1 { "" } else { "s" },
                if args.len() == 1 { "was" } else { "were" },
            );
            self.diagnostics
                .push(Diagnostic::new(codes::V0202, span, message));
            failed = true;
        }

        let mut lowered = Vec::new();
        for arg in args {
            let Some(arg) = self.expr(arg, None) else {
                failed = true;
                continue;
            };
            match arg.ty {
                Ty::Struct(_) => {
                    let message = format!(
                        "a whole `{}` cannot be printed with `{{}}`; print its fields one by one instead",
                        self.ty_name(&arg.ty)
                    );
                    self.diagnostics.push(
                        Diagnostic::new(codes::V0203, arg.span, message)
                            .with_note("structs have no printed form yet"),
                    );
                    failed = true;
                }
                Ty::Enum(_) | Ty::Option(_) | Ty::Result(..) | Ty::Vec(_) | Ty::HashMap(..) => {
                    let message = format!(
                        "a whole `{}` cannot be printed with `{{}}`",
                        self.ty_name(&arg.ty)
                    );
                    self.diagnostics.push(
                        Diagnostic::new(codes::V0203, arg.span, message).with_note(
                            "only numbers, `bool`, `string`, and `Error` have a printed form",
                        ),
                    );
                    failed = true;
                }
                Ty::Shared(_) => {
                    let message = format!(
                        "a `{}` cannot be printed with `{{}}`; print the fields you read through \
                         it instead",
                        self.ty_name(&arg.ty)
                    );
                    self.diagnostics.push(
                        Diagnostic::new(codes::V0203, arg.span, message).with_note(
                            "only numbers, `bool`, `string`, and `Error` have a printed form",
                        ),
                    );
                    failed = true;
                }
                Ty::Unit => {
                    self.diagnostics.push(Diagnostic::new(
                        codes::V0200,
                        arg.span,
                        "`()` cannot be formatted with `{}`",
                    ));
                    failed = true;
                }
                _ => lowered.push(arg),
            }
        }
        if failed {
            return None;
        }
        Some(lowered)
    }

    // --- Diagnostics helpers --------------------------------------------

    /// `expr` if it has type `ty`, else V0200 at `expr`: the usual note
    /// names `let mut i: usize = 0;`, which is impossible for a `for`
    /// variable, so a `range_vars` local gets [`RANGE_USIZE_NOTE`] instead.
    fn expect(&mut self, expr: HirExpr, ty: &Ty) -> Option<HirExpr> {
        if expr.ty == *ty {
            Some(expr)
        } else if is_range_var(&expr, &self.range_vars) && usize_note(ty, &expr.ty).is_some() {
            let message = format!(
                "mismatched types: expected `{}`, found `{}`",
                self.ty_name(ty),
                self.ty_name(&expr.ty)
            );
            self.diagnostics.push(
                Diagnostic::new(codes::V0200, expr.span, message).with_note(RANGE_USIZE_NOTE),
            );
            None
        } else {
            self.mismatch(expr.span, ty, &expr.ty);
            None
        }
    }

    fn mismatch(&mut self, span: Span, expected: &Ty, found: &Ty) {
        if found.has_chain() {
            self.diagnostics.push(unfinished_chain(span));
            return;
        }
        let message = format!(
            "mismatched types: expected `{}`, found `{}`",
            self.ty_name(expected),
            self.ty_name(found)
        );
        let mut diagnostic = Diagnostic::new(codes::V0200, span, message);
        if let Some(note) = usize_note(expected, found) {
            diagnostic = diagnostic.with_note(note);
        }
        // A `Result` where its value is wanted: suggest `?`.
        if let Ty::Result(ok, error) = found {
            if **error == Ty::Error {
                diagnostic = self.question_help(diagnostic, span, expected, Some(ok));
            }
        }
        // A `Shared` is not the struct it holds (milestone 5b1 spec 2.6).
        if let Ty::Shared(inner) = found {
            if **inner == *expected {
                diagnostic = diagnostic.with_note(
                    "a `Shared` is a handle to the struct, not the struct itself; read the \
                     fields it needs through the handle and pass those instead",
                );
            }
        }
        self.diagnostics.push(diagnostic);
    }

    /// The help of a V0200 at `span` for a `Result` with `Error` where
    /// `expected`, its `Ok` value, is wanted (`ok` is the value's type, or
    /// `None` when it would be taken from `expected`): in a function that
    /// returns a `Result` with `Error`, a fix-it inserting `?` after the
    /// value (after its `.await` when it is awaited); in any other, a note
    /// to take the value out with `match` or `if let`. `diagnostic`
    /// unchanged when `expected` is a `Result` or is not the `Ok` type.
    pub(super) fn question_help(
        &self,
        diagnostic: Diagnostic,
        span: Span,
        expected: &Ty,
        ok: Option<&Ty>,
    ) -> Diagnostic {
        if matches!(expected, Ty::Result(..)) || ok.is_some_and(|ok| ok != expected) {
            return diagnostic;
        }
        let end = match self.await_whole {
            Some(whole) if self.await_operand == Some(span) => whole.end,
            _ => span.end,
        };
        match &self.ret {
            Ty::Result(_, error) if **error == Ty::Error && self.closures.is_empty() => diagnostic
                .with_note(
                    "this gives a `Result`, which holds the value or an error; `?` takes the \
                     value out, and on an error returns it from the function",
                )
                .with_fix_it(FixIt {
                    span: Span::new(span.file, end, end),
                    replacement: "?".to_string(),
                }),
            _ => diagnostic.with_note(
                "this gives a `Result`, which holds the value or an error; take the value out \
                 with `match` or `if let`",
            ),
        }
    }

    /// V0100 for an unknown item; V0105 with a `pub ` fix-it for one that
    /// is not visible from here. `hint`, when the item was unknown, is the
    /// bare name to look for in other modules: `Some("Item")` for a call
    /// `Item::new()` that took `Item` for an unknown module, so a note can
    /// say "did you mean `task::Item`?" when another module declares it.
    fn lookup_error(
        &mut self,
        error: LookupError,
        what: &str,
        path: &str,
        span: Span,
        hint: Option<&str>,
    ) {
        let mut diagnostic = match error {
            LookupError::NoParent { span } => no_parent(span),
            LookupError::Dependency { span, dep } => self.symbols.dependency_error(span, dep),
            LookupError::Unknown => {
                Diagnostic::new(codes::V0100, span, format!("cannot find {what} `{path}`"))
            }
            LookupError::NotVisible { decl, keyword } => {
                not_visible(what, path, span, decl, keyword)
            }
            LookupError::PrivateModule { module } => {
                self.symbols.private_module(module, what, path, span)
            }
        };
        if matches!(error, LookupError::Unknown) {
            if let Some(name) = hint {
                if let Some(note) = self.symbols.did_you_mean(self.module, name) {
                    diagnostic = diagnostic.with_note(note);
                }
            }
            if let Some(note) = self.symbols.not_added(self.module, path) {
                diagnostic = diagnostic.with_note(note);
            }
        }
        self.diagnostics.push(diagnostic);
    }

    /// V0203 at `span` for `.clone()` or `==` on a type that cannot have
    /// it (M4 spec 2.10): `headline`, naming the field in the way when the
    /// value's own struct or enum has one, why as a note, and `rust`, the
    /// Rust terms, as another.
    fn blocked(&mut self, span: Span, headline: String, blocker: Blocker, rust: &str) {
        let message = match &blocker.field {
            Some(field) => format!("{headline}, because of {field}"),
            None => headline,
        };
        let mut diagnostic = Diagnostic::new(codes::V0203, span, message);
        if !blocker.reason.is_empty() {
            diagnostic = diagnostic.with_note(blocker.reason);
        }
        self.diagnostics.push(diagnostic.with_note(rust));
    }

    fn ty_name(&self, ty: &Ty) -> String {
        derives::ty_name(&self.symbols.structs, &self.symbols.enums, ty)
    }
}

/// V0108 at `span` for touching field `field` of the imported struct
/// `def`, whose Rust type does not map (M3 spec 4.1): it names the type
/// and what to change.
fn unusable_field(def: &StructDef, field: usize, unusable: &Unusable, span: Span) -> Diagnostic {
    let decl = &def.fields[field];
    let rust_ty = tidy_signature(&unusable.rust_ty);
    let diagnostic = Diagnostic::new(
        codes::V0108,
        span,
        if unusable.within.is_some() {
            format!(
                "field `{}` of `{}` has the Rust type `{rust_ty}`, which cannot be used here",
                decl.name, def.name
            )
        } else {
            format!(
                "field `{}` of `{}` has the Rust type `{rust_ty}`, which Varyk cannot use",
                decl.name, def.name
            )
        },
    )
    .with_label(decl.span, "the Rust field is declared here");
    match &unusable.note {
        Some(note) => diagnostic.with_note(note.clone()),
        None => diagnostic.with_note(
            "make the field private in the Rust module and add `pub fn` methods that use it",
        ),
    }
}

/// Whether field `field` (its index into struct `id`'s fields) is
/// visible from module `from` (spec 3.4): a field's declaring module is
/// its struct's own, so this is [`is_visible`] applied there, without
/// re-deriving the rule.
fn field_visible(symbols: &Symbols, from: ModuleId, id: StructId, field: usize) -> bool {
    let def = &symbols.structs[id.0 as usize];
    is_visible(symbols, from, def.module, def.fields[field].is_pub)
}

/// V0208 at `span`, an unfinished chain where a value is needed (M4 spec
/// 2.3).
fn unfinished_chain(span: Span) -> Diagnostic {
    Diagnostic::new(
        codes::V0208,
        span,
        "this chain is not finished, and an unfinished chain cannot be kept or passed on; \
         finish the chain here with `collect()`, `count()`, `sum()`, `any`, `all`, or `find`",
    )
    .with_note(
        "in Rust terms, an unfinished chain is an iterator, whose type Varyk has no way to write",
    )
}

/// V0208 for every unfinished chain in `block` anywhere but as the
/// receiver of the next call of its chain or as a `for` head: where
/// nothing expected a type, as in an unannotated `let`, an expression
/// statement, or `Some(..)` and `vec![..]` with no expectation (M4 spec
/// 2.3). The outermost such expression is reported, and nothing inside it.
fn unfinished_chains_in_block(block: &HirBlock, out: &mut Vec<Diagnostic>) {
    for stmt in &block.stmts {
        match stmt {
            HirStmt::Let { value: expr, .. } | HirStmt::Expr { expr, .. } => {
                unfinished_chains(expr, false, out);
            }
            HirStmt::Assign { target, value, .. } => {
                unfinished_chains(target, false, out);
                unfinished_chains(value, false, out);
            }
            HirStmt::Return { value, .. } => {
                if let Some(value) = value {
                    unfinished_chains(value, false, out);
                }
            }
            HirStmt::While { cond, body, .. }
            | HirStmt::WhileLet {
                value: cond, body, ..
            } => {
                unfinished_chains(cond, false, out);
                unfinished_chains_in_block(body, out);
            }
            HirStmt::For { head, body, .. } => {
                match head {
                    HirForHead::Range { start, end, .. } => {
                        unfinished_chains(start, false, out);
                        unfinished_chains(end, false, out);
                    }
                    HirForHead::Vec(vec) => unfinished_chains(vec, false, out),
                    // A chain call is where a chain may end.
                    HirForHead::Chain(chain) => unfinished_chains(chain, true, out),
                }
                unfinished_chains_in_block(body, out);
            }
            HirStmt::Break { .. }
            | HirStmt::Continue { .. }
            | HirStmt::Route(_)
            | HirStmt::Hook(_) => {}
        }
    }
    if let Some(tail) = &block.tail {
        unfinished_chains(tail, false, out);
    }
}

/// [`unfinished_chains_in_block`] for `expr`; `next` when it is the
/// receiver of a call of a chain, where a chain call may be.
fn unfinished_chains(expr: &HirExpr, next: bool, out: &mut Vec<Diagnostic>) {
    let chain_call = matches!(expr.kind, HirExprKind::MethodCall { .. });
    // A closure's type is what its body gives, which the body answers for.
    let closure = matches!(expr.kind, HirExprKind::Closure { .. });
    if matches!(expr.ty, Ty::Chain(_)) && !(next && chain_call) && !closure {
        out.push(unfinished_chain(expr.span));
        return;
    }
    let each = |exprs: &[HirExpr], out: &mut Vec<Diagnostic>| {
        for expr in exprs {
            unfinished_chains(expr, false, out);
        }
    };
    match &expr.kind {
        HirExprKind::Int(_)
        | HirExprKind::Float(_)
        | HirExprKind::Bool(_)
        | HirExprKind::String(_)
        | HirExprKind::Local(_) => {}
        HirExprKind::Call { args, trailing, .. } => {
            each(args, out);
            each(trailing, out);
        }
        HirExprKind::Println { args, .. }
        | HirExprKind::Log { args, .. }
        | HirExprKind::Format { args, .. }
        | HirExprKind::EnumLit { args, .. }
        | HirExprKind::VecLit(args) => each(args, out),
        HirExprKind::MethodCall {
            receiver,
            method,
            args,
            trailing,
            ..
        } => {
            let on_chain =
                matches!(method, MethodRef::Builtin(id) if id.get().owner == Owner::Chain);
            unfinished_chains(receiver, on_chain, out);
            each(args, out);
            each(trailing, out);
        }
        HirExprKind::Field { base, .. } => unfinished_chains(base, false, out),
        HirExprKind::Index { base, index } => {
            unfinished_chains(base, false, out);
            unfinished_chains(index, false, out);
        }
        HirExprKind::StructLit { fields, .. } => {
            for (_, value) in fields {
                unfinished_chains(value, false, out);
            }
        }
        HirExprKind::Unary { operand, .. }
        | HirExprKind::Cast { expr: operand, .. }
        | HirExprKind::Try { operand, .. }
        | HirExprKind::Assert { cond: operand, .. }
        | HirExprKind::Await(operand) => unfinished_chains(operand, false, out),
        HirExprKind::Binary { lhs, rhs, .. } => {
            unfinished_chains(lhs, false, out);
            unfinished_chains(rhs, false, out);
        }
        HirExprKind::Block(block) | HirExprKind::Closure { body: block, .. } => {
            unfinished_chains_in_block(block, out);
        }
        HirExprKind::If { cond, then, else_ } => {
            unfinished_chains(cond, false, out);
            unfinished_chains_in_block(then, out);
            if let Some(else_) = else_ {
                unfinished_chains_in_block(else_, out);
            }
        }
        HirExprKind::IfLet {
            value, then, else_, ..
        } => {
            unfinished_chains(value, false, out);
            unfinished_chains_in_block(then, out);
            if let Some(else_) = else_ {
                unfinished_chains_in_block(else_, out);
            }
        }
        HirExprKind::Match {
            scrutinee, arms, ..
        } => {
            unfinished_chains(scrutinee, false, out);
            for arm in arms {
                unfinished_chains(&arm.body, false, out);
            }
        }
    }
}

fn binary_expr(op: BinaryOp, lhs: HirExpr, rhs: HirExpr, ty: Ty, span: Span) -> HirExpr {
    HirExpr {
        kind: HirExprKind::Binary {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        },
        ty,
        span,
    }
}

/// Whether `name` is one of the built-in constructors, resolved before
/// any user name (spec 2.1).
fn is_builtin_variant(name: &str) -> bool {
    matches!(name, "Some" | "None" | "Ok" | "Err")
}

/// The note of a V0200 where a `for` variable over a range of another
/// integer type is used as a `usize`: its type is its range's.
pub(super) const RANGE_USIZE_NOTE: &str = "number types never convert into each other; a `for` \
     variable has the type of its range, so make a range end `usize`, as in `0..v.len()` or \
     `let n: usize = 3;`";

/// Whether `expr` is a `for` variable bound to a range: such a variable
/// cannot be declared with a type, so a usize mismatch on it needs
/// [`RANGE_USIZE_NOTE`] instead of the `let mut i: usize = 0;` note.
fn is_range_var(expr: &HirExpr, range_vars: &[LocalId]) -> bool {
    matches!(expr.kind, HirExprKind::Local(id) if range_vars.contains(&id))
}

/// The note of a V0200 where another integer type meets `usize` (spec
/// 2.7): there are no casts, so the count is declared as `usize`.
pub(super) fn usize_note(a: &Ty, b: &Ty) -> Option<&'static str> {
    let usize = Ty::Int(IntKind::Usize);
    let other_int = |ty: &Ty| matches!(ty, Ty::Int(kind) if *kind != IntKind::Usize);
    ((*a == usize && other_int(b)) || (*b == usize && other_int(a))).then_some(
        "number types never convert into each other; declare the count as `usize`, as in \
         `let mut i: usize = 0;`",
    )
}

fn is_arithmetic(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem
    )
}

/// A numeric literal, possibly negated: its type comes from context.
fn is_literal(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Integer(_) | ExprKind::Float(_) => true,
        ExprKind::Unary {
            op: UnaryOp::Neg,
            operand,
        } => is_literal(operand),
        _ => false,
    }
}

fn is_signed(ty: &Ty) -> bool {
    matches!(
        ty,
        Ty::Int(IntKind::I8 | IntKind::I16 | IntKind::I32 | IntKind::I64) | Ty::Float(_)
    )
}

/// Whether control never reaches the end of `stmt`: a `return`, `break`,
/// or `continue`, or an expression that always does one.
fn stmt_diverges(stmt: &HirStmt) -> bool {
    match stmt {
        HirStmt::Return { .. } | HirStmt::Break { .. } | HirStmt::Continue { .. } => true,
        HirStmt::Expr { expr, .. } => expr_diverges(expr),
        _ => false,
    }
}

fn expr_diverges(expr: &HirExpr) -> bool {
    match &expr.kind {
        HirExprKind::Block(block) => block_diverges(block),
        HirExprKind::If {
            then,
            else_: Some(else_),
            ..
        } => block_diverges(then) && block_diverges(else_),
        HirExprKind::IfLet {
            then,
            else_: Some(else_),
            ..
        } => block_diverges(then) && block_diverges(else_),
        HirExprKind::Match { arms, .. } => {
            !arms.is_empty() && arms.iter().all(|arm| expr_diverges(&arm.body))
        }
        _ => false,
    }
}

fn block_diverges(block: &HirBlock) -> bool {
    block.stmts.iter().any(stmt_diverges) || block.tail.as_deref().is_some_and(expr_diverges)
}

/// `x` when `arg` is `Some(x)`, the built-in variant.
fn some_operand(arg: &Expr) -> Option<&Expr> {
    let ExprKind::Call { callee, args } = &arg.kind else {
        return None;
    };
    match (&callee.kind, args.as_slice()) {
        (ExprKind::Path { path: None, name }, [inner]) if name.name == "Some" => Some(inner),
        _ => None,
    }
}

/// Whether a value of type `ty` can be passed after the other arguments
/// to a `Vec<varyk_std::Value>` parameter (milestone 5b3 spec 2.2): a
/// type `varyk_std::Value::from` takes, which no conversion can fail.
fn is_value_type(ty: &Ty) -> bool {
    let scalar = |ty: &Ty| match ty {
        Ty::Bool | Ty::String | Ty::Float(_) => true,
        Ty::Int(kind) => !matches!(kind, IntKind::U64 | IntKind::Usize),
        _ => false,
    };
    match ty {
        Ty::Option(inner) => scalar(inner),
        other => scalar(other),
    }
}

/// V0217 at `arg`, an argument to a parameter of `path` that takes only
/// text written in the program (milestone 5b3 spec 2.3); for a
/// `format!`, how to pass its values instead.
fn not_literal_text(path: &str, arg: &Expr) -> Diagnostic {
    let diagnostic = Diagnostic::new(
        codes::V0217,
        arg.span,
        "this argument must be text written in the program",
    )
    .with_note(format!(
        "`{path}` takes only literal text, so no input can reach it"
    ));
    match &arg.kind {
        ExprKind::Intrinsic { name, .. } if name.name == "format" => {
            diagnostic.with_note("pass the values after the text instead")
        }
        _ => diagnostic,
    }
}

/// V0108 at `span` for a call to the imported `path` whose Rust signature
/// Varyk cannot call, or cannot call from here (one naming a type only
/// some modules can see): the signature, and what to change.
fn unsupported_rust_signature(path: &str, sig: &ImportedSig, span: Span) -> Diagnostic {
    // The signature is fine; the file could redefine what it names.
    if let Some(file) = &sig.redefined_in {
        let message = format!(
            "`{path}` cannot be called from Varyk, because `{file}` could redefine the types \
             its signature names"
        );
        let diagnostic = Diagnostic::new(codes::V0108, span, message);
        return match &sig.note {
            Some(note) => diagnostic.with_note(note.clone()),
            None => diagnostic,
        };
    }
    let message = if sig.callable {
        format!(
            "`{path}` cannot be called here, because its Rust signature names a type this \
             module cannot see"
        )
    } else {
        format!("unsupported Rust signature: `{path}` cannot be called from Varyk")
    };
    let mut diagnostic = Diagnostic::new(codes::V0108, span, message).with_note(format!(
        "the Rust signature is `{}`",
        tidy_signature(&sig.signature)
    ));
    if let Some(note) = &sig.note {
        diagnostic = diagnostic.with_note(note.clone());
    }
    // Only a `&String` parameter gets the hint, not a `-> &String` return.
    let params = sig.signature.split("->").next().unwrap_or_default();
    if params.contains("& String") {
        diagnostic = diagnostic.with_note(
            "take `&str` instead of `&String` in the Rust function; `&str` maps to a shared borrow of `string`",
        );
    }
    diagnostic
}

/// V0100 at `span` for naming a variant (`full`, its full path) of the
/// opaque imported enum `name` (M3 spec 4.3): `reason` says why the enum
/// is opaque, as what it has ("has a variant, `A`, ...").
fn opaque_variant(full: &str, name: &str, reason: &str, span: Span) -> Diagnostic {
    Diagnostic::new(
        codes::V0100,
        span,
        format!("Varyk cannot construct or match `{full}`"),
    )
    .with_note(format!(
        "`{name}` {reason}, so Varyk can pass {} `{name}` around but cannot build one or \
         look inside it; make one with a function in the `.rs` file",
        crate::borrow::article(name)
    ))
}

mod asyncs;
mod closures;
mod exhaustive;
mod methods;
mod patterns;
mod routes;
mod values;

#[cfg(test)]
mod tests;
