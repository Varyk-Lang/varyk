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
    BinaryOp, Block, Expr, ExprKind, FixIt, ForHead, Function, Ident, SourceFile, Span, Stmt,
    UnaryOp,
};

use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{
    HirBlock, HirEnum, HirExpr, HirExprKind, HirForHead, HirFunction, HirModule, HirModuleKind,
    HirParam, HirProgram, HirStmt, HirStruct, LocalId, LocalInfo, PlaceInfo, is_place,
};
use crate::resolve::{
    Callee, FnId, FnSig, LookupError, ModuleId, ModuleKind, Resolved, Symbols, UserType,
    not_visible, reserved_value_name,
};
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

    for (index, sig) in symbols.fns.iter().enumerate() {
        let id = FnId(index as u32);
        let decl = resolved.fn_decl(id);
        let mut checker = FnChecker {
            symbols,
            sources,
            module: sig.module,
            name: &sig.name,
            ret: sig.ret.clone(),
            locals: Vec::new(),
            scopes: Vec::new(),
            loop_depth: 0,
            range_vars: Vec::new(),
            diagnostics: &mut diagnostics,
        };
        if let Some(function) = checker.function(id, sig, decl) {
            functions.push(function);
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    let structs = symbols
        .structs
        .iter()
        .map(|def| HirStruct {
            name: def.name.clone(),
            module: def.module,
            is_pub: def.is_pub,
            fields: def.fields.clone(),
            span: def.span,
        })
        .collect();
    let enums = symbols
        .enums
        .iter()
        .map(|def| HirEnum {
            name: def.name.clone(),
            module: def.module,
            is_pub: def.is_pub,
            variants: def.variants.clone(),
            span: def.span,
        })
        .collect();
    let imported = symbols.imported.clone();
    let modules = resolved
        .modules
        .into_iter()
        .map(|module| HirModule {
            id: module.id,
            name: module.name,
            kind: match module.kind {
                ModuleKind::Varyk(_) => HirModuleKind::Varyk,
                ModuleKind::Rust(source) => HirModuleKind::Rust { source },
            },
        })
        .collect();

    Ok(HirProgram {
        modules,
        functions,
        structs,
        enums,
        imported,
        entry: resolved.entry,
    })
}

/// A scope maps a name to its local, or to `None` for a binding whose
/// initializer failed to check: uses of it fail silently, so one error
/// does not cascade into "unknown name" at every later use.
type Scope = HashMap<String, Option<LocalId>>;

/// Checks one function. Every method returning `Option` has already
/// reported a diagnostic when it returns `None`.
struct FnChecker<'a> {
    symbols: &'a Symbols,
    sources: &'a [SourceFile],
    module: ModuleId,
    /// The function's name, for the message of a misused `?`.
    name: &'a str,
    ret: Ty,
    locals: Vec<LocalInfo>,
    scopes: Vec<Scope>,
    /// How many `while` and `for` loops enclose the current statement.
    loop_depth: u32,
    /// The variables of `for` loops over a range, for the note of a V0200
    /// on one (its type is its range's).
    range_vars: Vec<LocalId>,
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
            params,
            ret: sig.ret.clone(),
            body,
            locals: std::mem::take(&mut self.locals),
            span: decl.span,
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
                .any(|(variant, payload)| *variant == name.name && payload.is_empty())
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
            .map(|tail| self.expr(tail, expected.clone()));
        self.scopes.pop();

        let tail = match tail {
            Some(Some(tail)) => Some(Box::new(tail)),
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
                    Some(ty) => match self.symbols.resolve_type(ty, self.module) {
                        Ok(ty) => Some(ty),
                        Err(diagnostic) => {
                            self.diagnostics.push(diagnostic);
                            self.bind_poisoned(name);
                            return None;
                        }
                    },
                };
                let value = self.expr(value, annotated.clone());
                let value = match (value, annotated) {
                    (Some(value), Some(ty)) => self.expect(value, &ty),
                    (value, _) => value,
                };
                let Some(value) = value else {
                    self.bind_poisoned(name);
                    return None;
                };
                let local = self.bind(name, value.ty.clone(), *mutable);
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
                let expr = self.expr(expr, None)?;
                if !has_semi {
                    // A block-like expression mid-block without `;` must be `()`.
                    return self
                        .expect(expr, &Ty::Unit)
                        .map(|expr| HirStmt::Expr { expr, span: *span });
                }
                Some(HirStmt::Expr { expr, span: *span })
            }
            Stmt::Return { value, span } => {
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
                let cond = self.condition(cond);
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
    /// range, or a `Vec` that is a place or a temporary (V0001 otherwise,
    /// as for `match`); anything else is V0200.
    fn for_head(&mut self, head: &ForHead) -> Option<(HirForHead, Ty)> {
        let head = match head {
            ForHead::Range { start, end } => return self.range(start, end),
            ForHead::Expr(head) => self.expr(head, None)?,
        };
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

    /// `start..end` in a `for` head: two integers of one type, a literal
    /// end taking the other end's type, as the operands of `+` do.
    fn range(&mut self, start: &Expr, end: &Expr) -> Option<(HirForHead, Ty)> {
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
        Some((HirForHead::Range { start, end }, ty))
    }

    /// V0002 unless a `while` or `for` loop encloses the `keyword`
    /// statement; the error points at the keyword alone, not the trailing
    /// `;`.
    fn require_loop(&mut self, keyword: &str, stmt: Span) -> Option<()> {
        if self.loop_depth > 0 {
            return Some(());
        }
        let span = Span::new(stmt.file, stmt.start, stmt.start + keyword.len() as u32);
        let message = format!("`{keyword}` is only allowed inside a `while` or `for` loop");
        self.diagnostics
            .push(Diagnostic::new(codes::V0002, span, message));
        None
    }

    // --- Expressions ----------------------------------------------------

    fn expr(&mut self, expr: &Expr, expected: Option<Ty>) -> Option<HirExpr> {
        let span = expr.span;
        let (kind, ty) = match &expr.kind {
            ExprKind::Integer(text) => return self.integer(text, expected, false, span),
            ExprKind::Float(text) => return self.float(text, expected, span),
            ExprKind::Bool(value) => (HirExprKind::Bool(*value), Ty::Bool),
            ExprKind::String(text) => (HirExprKind::String(text.clone()), Ty::String),
            ExprKind::Path {
                module,
                type_,
                name,
            } => {
                return self.path_value(module.as_ref(), type_.as_ref(), name, expected, span);
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
                let field = match base.ty {
                    Ty::Struct(id) => self.symbols.structs[id.0 as usize]
                        .fields
                        .iter()
                        .position(|(field, _)| *field == name.name),
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
                let Ty::Struct(id) = base.ty else {
                    unreachable!("only a struct has fields")
                };
                let def = &self.symbols.structs[id.0 as usize];
                let ty = def.fields[field_index].1.clone();
                let kind = HirExprKind::Field {
                    base: Box::new(base),
                    name: name.name.clone(),
                };
                (kind, ty)
            }
            ExprKind::StructLit {
                module,
                name,
                fields,
            } => {
                return self.struct_lit(module.as_ref(), name, fields, span);
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
            } => return self.method_call(receiver, method, args, span),
            ExprKind::Index { base, index } => return self.index(base, index, span),
            ExprKind::Try { operand } => return self.try_(operand, span),
            ExprKind::Match { scrutinee, arms } => {
                return self.match_expr(scrutinee, arms, expected, span);
            }
            ExprKind::VecLit(elements) => return self.vec_lit(elements, expected, span),
        };
        Some(HirExpr { kind, ty, span })
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
        let (min, max) = int_range(kind);
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
            Some(local) => *local,
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
            if let Ty::Struct(_) = ty {
                let message = format!(
                    "`{symbol}` cannot be applied to struct `{}`",
                    self.ty_name(&ty)
                );
                let diagnostic = Diagnostic::new(codes::V0203, span, message)
                    .with_note("`==` and `!=` work only on numbers, `bool`, and `string`");
                self.diagnostics.push(diagnostic);
                return None;
            }
            if matches!(
                ty,
                Ty::Enum(_) | Ty::Option(_) | Ty::Result(..) | Ty::Vec(_)
            ) {
                let message = format!("`{symbol}` cannot be applied to `{}`", self.ty_name(&ty));
                let diagnostic = Diagnostic::new(codes::V0203, span, message)
                    .with_note("`==` and `!=` work only on numbers, `bool`, and `string`");
                self.diagnostics.push(diagnostic);
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
        let ExprKind::Path {
            module,
            type_,
            name,
        } = &callee.kind
        else {
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
        if let Some(owner) = self.path_owner(module.as_ref(), type_.as_ref(), callee.span) {
            let (owner, path) = owner?;
            return self.type_member(owner, &path, name, Some(args), expected, callee.span, span);
        }
        if module.is_none() && is_builtin_variant(&name.name) {
            return self.builtin_variant(&name.name, Some(args), expected, span);
        }
        if module.is_none()
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
        let module_name = module.as_ref().map(|m| m.name.as_str());
        let path = match module_name {
            Some(module) => format!("{module}::{}", name.name),
            None => name.name.clone(),
        };
        let found = match self.symbols.lookup_fn(self.module, module_name, &name.name) {
            Ok(found) => found,
            Err(error) => {
                let hint = Some(module_name.unwrap_or(name.name.as_str()));
                self.lookup_error(error, "function", &path, callee.span, hint);
                return None;
            }
        };

        let (params, ret): (Vec<Ty>, Ty) = match found {
            Callee::Varyk(id) => {
                let sig = &self.symbols.fns[id.0 as usize];
                (
                    sig.params.iter().map(|p| p.1.clone()).collect(),
                    sig.ret.clone(),
                )
            }
            Callee::Imported(id) => {
                let sig = &self.symbols.imported[id.0 as usize];
                if !sig.callable {
                    let mut diagnostic = Diagnostic::new(
                        codes::V0108,
                        span,
                        format!("unsupported Rust signature: `{path}` cannot be called from Varyk"),
                    )
                    .with_note(format!(
                        "the Rust signature is `{}`",
                        tidy_signature(&sig.signature)
                    ));
                    // Only a `&String` parameter gets the hint, not a
                    // `-> &String` return.
                    let params = sig.signature.split("->").next().unwrap_or_default();
                    if params.contains("& String") {
                        diagnostic = diagnostic.with_note(
                            "take `&str` instead of `&String` in the Rust function; `&str` maps to a shared borrow of `string`",
                        );
                    }
                    self.diagnostics.push(diagnostic);
                    return None;
                }
                (
                    sig.params.iter().map(|p| p.0.clone()).collect(),
                    sig.ret.clone(),
                )
            }
            Callee::Builtin(_) => unreachable!("a plain name never finds a built-in"),
        };

        let args = self.arguments(&path, &params, args, span)?;
        Some(HirExpr {
            kind: HirExprKind::Call {
                callee: found,
                args,
            },
            ty: ret,
            span,
        })
    }

    /// The arguments of a call to `path` (at `span`), each typed against
    /// its parameter type in `params`; V0201 when the count differs.
    fn arguments(
        &mut self,
        path: &str,
        params: &[Ty],
        args: &[Expr],
        span: Span,
    ) -> Option<Vec<HirExpr>> {
        if args.len() != params.len() {
            let plural = |n: usize| if n == 1 { "" } else { "s" };
            let message = format!(
                "`{path}` takes {} argument{} but {} {} given",
                params.len(),
                plural(params.len()),
                args.len(),
                if args.len() == 1 { "was" } else { "were" },
            );
            self.diagnostics
                .push(Diagnostic::new(codes::V0201, span, message));
            return None;
        }
        let mut checked = Vec::new();
        for (arg, ty) in args.iter().zip(params) {
            checked.push(
                self.expr(arg, Some(ty.clone()))
                    .and_then(|arg| self.expect(arg, ty)),
            );
        }
        checked.into_iter().collect()
    }

    /// `Name { .. }`, or `m::Name { .. }` for a `pub` struct of module
    /// `m` (spec 2.10).
    fn struct_lit(
        &mut self,
        module: Option<&Ident>,
        name: &Ident,
        fields: &[(Ident, Expr)],
        span: Span,
    ) -> Option<HirExpr> {
        let (path, path_span) = match module {
            Some(module) => (
                format!("{}::{}", module.name, name.name),
                Span::new(name.span.file, module.span.start, name.span.end),
            ),
            None => (name.name.clone(), name.span),
        };
        let found =
            self.symbols
                .lookup_type(self.module, module.map(|m| m.name.as_str()), &name.name);
        let id = match found {
            Ok(UserType::Struct(id)) => id,
            Ok(UserType::Enum(_)) => {
                let message = format!("`{path}` is an enum, not a struct");
                self.diagnostics
                    .push(Diagnostic::new(codes::V0101, path_span, message));
                return None;
            }
            Err(LookupError::Unknown) => {
                if let Some(module) = module {
                    if let Ok(UserType::Enum(enum_id)) =
                        self.symbols.lookup_type(self.module, None, &module.name)
                    {
                        let def = &self.symbols.enums[enum_id.0 as usize];
                        if def.variant(&name.name).is_some() {
                            self.diagnostics.push(Diagnostic::new(
                                codes::V0001,
                                span,
                                "enum variants with named fields are not supported until \
                                 milestone 4; use a tuple variant instead",
                            ));
                            return None;
                        }
                    }
                }
                let message = format!("unknown struct `{path}`");
                self.diagnostics
                    .push(Diagnostic::new(codes::V0101, path_span, message));
                return None;
            }
            Err(LookupError::NotVisible { decl, keyword }) => {
                self.diagnostics
                    .push(not_visible("struct", &path, path_span, decl, keyword));
                return None;
            }
        };
        let def = &self.symbols.structs[id.0 as usize];

        let mut failed = false;
        let mut seen: HashMap<usize, Span> = HashMap::new();
        let mut lowered = Vec::new();
        for (field, value) in fields {
            let Some(index) = def.fields.iter().position(|(f, _)| *f == field.name) else {
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
            seen.insert(index, field.span);
            let ty = def.fields[index].1.clone();
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
            .map(|(_, (field, _))| format!("`{field}`"))
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
        let Some(else_) = else_ else {
            let then = self.block(then, Some(Ty::Unit));
            let (cond, then) = (cond?, then?);
            if then.ty != Ty::Unit {
                let tail = then.tail.as_deref().expect("a non-unit block has a tail");
                let message = format!(
                    "`if` without `else` must have type `()`, found `{}`",
                    self.ty_name(&tail.ty)
                );
                self.diagnostics
                    .push(Diagnostic::new(codes::V0200, tail.span, message));
                return None;
            }
            return Some(HirExpr {
                kind: HirExprKind::If {
                    cond: Box::new(cond),
                    then,
                    else_: None,
                },
                ty: Ty::Unit,
                span,
            });
        };

        let then = self.block(then, expected.clone());
        // The then-branch types the else-branch's literals, unless it
        // always leaves early and so says nothing about the value.
        let then_diverges = then.as_ref().is_some_and(block_diverges);
        let else_expected = match &then {
            Some(then) if !then_diverges => expected.clone().or(Some(then.ty.clone())),
            _ => expected,
        };
        let else_block = self.block(else_, else_expected);
        let (cond, then, else_block) = (cond?, then?, else_block?);

        let ty = if then_diverges {
            else_block.ty.clone()
        } else if block_diverges(&else_block) || else_block.ty == then.ty {
            then.ty.clone()
        } else {
            let message = format!(
                "`if` and `else` have incompatible types: expected `{}`, found `{}`",
                self.ty_name(&then.ty),
                self.ty_name(&else_block.ty)
            );
            self.diagnostics
                .push(Diagnostic::new(codes::V0200, else_block.span, message));
            return None;
        };
        Some(HirExpr {
            kind: HirExprKind::If {
                cond: Box::new(cond),
                then,
                else_: Some(else_block),
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
                Ty::Enum(_) | Ty::Option(_) | Ty::Result(..) | Ty::Vec(_) => {
                    let message = format!(
                        "a whole `{}` cannot be printed with `{{}}`",
                        self.ty_name(&arg.ty)
                    );
                    self.diagnostics.push(
                        Diagnostic::new(codes::V0203, arg.span, message)
                            .with_note("only numbers, `bool`, and `string` have a printed form"),
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
        let message = format!(
            "mismatched types: expected `{}`, found `{}`",
            self.ty_name(expected),
            self.ty_name(found)
        );
        let mut diagnostic = Diagnostic::new(codes::V0200, span, message);
        if let Some(note) = usize_note(expected, found) {
            diagnostic = diagnostic.with_note(note);
        }
        self.diagnostics.push(diagnostic);
    }

    /// V0100 for an unknown item; V0105 with a `pub ` fix-it for one that
    /// is not visible from here. `hint`, when the item was unknown, is the
    /// bare name to look for in other modules: `Some("Task")` for a call
    /// `Task::new()` that took `Task` for an unknown module, so a note can
    /// say "did you mean `task::Task`?" when another module declares it.
    fn lookup_error(
        &mut self,
        error: LookupError,
        what: &str,
        path: &str,
        span: Span,
        hint: Option<&str>,
    ) {
        let mut diagnostic = match error {
            LookupError::Unknown => {
                Diagnostic::new(codes::V0100, span, format!("cannot find {what} `{path}`"))
            }
            LookupError::NotVisible { decl, keyword } => {
                not_visible(what, path, span, decl, keyword)
            }
        };
        if matches!(error, LookupError::Unknown) {
            if let Some(name) = hint {
                if let Some(note) = self.symbols.did_you_mean(self.module, name) {
                    diagnostic = diagnostic.with_note(note);
                }
            }
        }
        self.diagnostics.push(diagnostic);
    }

    fn ty_name(&self, ty: &Ty) -> String {
        match ty {
            Ty::Bool => "bool".to_string(),
            Ty::Int(kind) => kind.name().to_string(),
            Ty::Float(kind) => kind.name().to_string(),
            Ty::String => "string".to_string(),
            Ty::Struct(id) => self.symbols.structs[id.0 as usize].name.clone(),
            Ty::Enum(id) => self.symbols.enums[id.0 as usize].name.clone(),
            Ty::Option(inner) => format!("Option<{}>", self.ty_name(inner)),
            Ty::Result(ok, err) => {
                format!("Result<{}, {}>", self.ty_name(ok), self.ty_name(err))
            }
            Ty::Vec(inner) => format!("Vec<{}>", self.ty_name(inner)),
            Ty::Unit => "()".to_string(),
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
        HirExprKind::Match { arms, .. } => {
            !arms.is_empty() && arms.iter().all(|arm| expr_diverges(&arm.body))
        }
        _ => false,
    }
}

fn block_diverges(block: &HirBlock) -> bool {
    block.stmts.iter().any(stmt_diverges) || block.tail.as_deref().is_some_and(expr_diverges)
}

/// Respaces a signature's token-stream text (`fn f < T > (x : & T)`) the
/// way a person writes it (`fn f<T>(x: &T)`), for V0108's note.
fn tidy_signature(tokens: &str) -> String {
    let mut out = String::new();
    let mut prev = "";
    for token in tokens.split_whitespace() {
        // A group prints as one piece (`(value`), so compare the edges.
        let glue_to_prev = token.starts_with([',', ':', ';', ')', '>', '(', '<'])
            || prev.ends_with(['(', '<', '&'])
            || prev.ends_with("::");
        if !out.is_empty() && !glue_to_prev {
            out.push(' ');
        }
        out.push_str(token);
        prev = token;
    }
    out
}

mod methods;
mod patterns;
mod values;

#[cfg(test)]
mod tests;

/// The smallest and largest value of an integer type.
fn int_range(kind: IntKind) -> (i128, i128) {
    match kind {
        IntKind::I8 => (i8::MIN.into(), i8::MAX.into()),
        IntKind::I16 => (i16::MIN.into(), i16::MAX.into()),
        IntKind::I32 => (i32::MIN.into(), i32::MAX.into()),
        IntKind::I64 => (i64::MIN.into(), i64::MAX.into()),
        IntKind::U8 => (0, u8::MAX.into()),
        IntKind::U16 => (0, u16::MAX.into()),
        IntKind::U32 => (0, u32::MAX.into()),
        // Taken as 64 bits wide, as on every 64-bit target.
        IntKind::U64 | IntKind::Usize => (0, u64::MAX.into()),
    }
}
