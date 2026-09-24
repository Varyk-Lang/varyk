//! The type checker (spec sections 4.1, 4.2, 4.5): one pass that lowers
//! each function's AST to typed HIR while checking it.
//!
//! Paths resolve through [`Symbols::lookup_fn`] and
//! [`Symbols::lookup_struct`]; locals through a per-function scope stack.
//! A literal takes the type its immediate context expects, passed down as
//! `expected`, with `i32` and `f64` as fallbacks; nothing is inferred
//! backwards. Diagnostics from every function are collected and returned
//! together. Within a function, a failed expression makes its enclosing
//! statement fail, and checking resumes at the next statement.

use std::collections::HashMap;

use varyk_syntax::{BinaryOp, Block, Expr, ExprKind, Function, Ident, Span, Stmt, UnaryOp};

use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{
    HirBlock, HirExpr, HirExprKind, HirFunction, HirModule, HirModuleKind, HirParam, HirProgram,
    HirStmt, HirStruct, LocalId, LocalInfo, PlaceInfo,
};
use crate::resolve::{
    Callee, FnId, FnSig, LookupError, ModuleId, ModuleKind, Resolved, Symbols, declared_without_pub,
};
use crate::types::{FloatKind, IntKind, Ty};

/// Type-checks every Varyk function and lowers the program to HIR, or
/// returns every diagnostic found.
pub fn typecheck(resolved: Resolved) -> Result<HirProgram, Vec<Diagnostic>> {
    let symbols = &resolved.symbols;
    let mut diagnostics = Vec::new();
    let mut functions: Vec<Vec<HirFunction>> = vec![Vec::new(); resolved.modules.len()];

    for (index, sig) in symbols.fns.iter().enumerate() {
        let id = FnId(index as u32);
        let decl = resolved.fn_decl(id);
        let mut checker = FnChecker {
            symbols,
            module: sig.module,
            ret: sig.ret,
            locals: Vec::new(),
            scopes: Vec::new(),
            loop_depth: 0,
            diagnostics: &mut diagnostics,
        };
        if let Some(function) = checker.function(id, sig, decl) {
            functions[sig.module.0 as usize].push(function);
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
    let imported = symbols.imported.clone();
    let modules = resolved
        .modules
        .into_iter()
        .zip(functions)
        .map(|(module, functions)| HirModule {
            id: module.id,
            name: module.name,
            kind: match module.kind {
                ModuleKind::Varyk(_) => HirModuleKind::Varyk { functions },
                ModuleKind::Rust(source) => HirModuleKind::Rust { source },
            },
        })
        .collect();

    Ok(HirProgram {
        modules,
        structs,
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
    module: ModuleId,
    ret: Ty,
    locals: Vec<LocalInfo>,
    scopes: Vec<Scope>,
    /// How many `while` loops enclose the current statement.
    loop_depth: u32,
    diagnostics: &'a mut Vec<Diagnostic>,
}

impl FnChecker<'_> {
    fn function(&mut self, id: FnId, sig: &FnSig, decl: &Function) -> Option<HirFunction> {
        self.scopes.push(Scope::new());
        let mut params = Vec::new();
        for (param, (name, ty, mode)) in decl.params.iter().zip(&sig.params) {
            let local = self.bind(&param.name, *ty, param.mutable);
            params.push(HirParam {
                local,
                name: name.clone(),
                ty: *ty,
                mode: *mode,
                span: param.span,
            });
        }
        let body = self.block(&decl.body, Some(sig.ret));
        self.scopes.pop();
        let body = body?;

        if body.ty != sig.ret {
            match &body.tail {
                Some(tail) => {
                    self.mismatch(tail.span, sig.ret, tail.ty);
                }
                None => {
                    let end = decl.body.span.end;
                    let brace = Span::new(decl.body.span.file, end.saturating_sub(1), end);
                    let diagnostic = Diagnostic::new(
                        codes::V0200,
                        brace,
                        format!(
                            "mismatched types: expected `{}`, found `()`",
                            self.ty_name(sig.ret)
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
            name: sig.name.clone(),
            is_pub: sig.is_pub,
            params,
            ret: sig.ret,
            body,
            locals: std::mem::take(&mut self.locals),
            span: decl.span,
        })
    }

    // --- Locals ---------------------------------------------------------

    /// Adds a local and makes `name` refer to it in the innermost scope.
    fn bind(&mut self, name: &Ident, ty: Ty, mutable: bool) -> LocalId {
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
        let tail = block.tail.as_deref().map(|tail| self.expr(tail, expected));
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
            Some(tail) => tail.ty,
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
                let value = self.expr(value, annotated);
                let value = match (value, annotated) {
                    (Some(value), Some(ty)) => self.expect(value, ty),
                    (value, _) => value,
                };
                let Some(value) = value else {
                    self.bind_poisoned(name);
                    return None;
                };
                let local = self.bind(name, value.ty, *mutable);
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
                        "invalid assignment target: expected a variable or a field of one",
                    ));
                    return None;
                }
                let value = self.expr(value, Some(target.ty))?;
                let value = self.expect(value, target.ty)?;
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
                        .expect(expr, Ty::Unit)
                        .map(|expr| HirStmt::Expr { expr, span: *span });
                }
                Some(HirStmt::Expr { expr, span: *span })
            }
            Stmt::Return { value, span } => {
                let value = match value {
                    Some(value) => {
                        let value = self.expr(value, Some(self.ret))?;
                        Some(self.expect(value, self.ret)?)
                    }
                    None if self.ret != Ty::Unit => {
                        let message = format!(
                            "`return` needs a value of type `{}` here",
                            self.ty_name(self.ret)
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
                    self.mismatch(tail.span, Ty::Unit, tail.ty);
                    return None;
                }
                Some(HirStmt::While {
                    cond,
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

    /// V0002 unless a `while` loop encloses the `keyword` statement; the
    /// error points at the keyword alone, not the trailing `;`.
    fn require_loop(&mut self, keyword: &str, stmt: Span) -> Option<()> {
        if self.loop_depth > 0 {
            return Some(());
        }
        let span = Span::new(stmt.file, stmt.start, stmt.start + keyword.len() as u32);
        let message = format!("`{keyword}` is only allowed inside a `while` loop");
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
            ExprKind::Path { module, name } => {
                if let Some(module) = module {
                    let message = format!("cannot find value `{}::{}`", module.name, name.name);
                    self.diagnostics
                        .push(Diagnostic::new(codes::V0100, span, message));
                    return None;
                }
                let id = self.lookup_local(name)?;
                (HirExprKind::Local(id), self.locals[id.0 as usize].ty)
            }
            ExprKind::Unary { op, operand } => {
                return self.unary(*op, operand, expected, span, true);
            }
            ExprKind::Binary { op, lhs, rhs } => {
                return self.binary(*op, lhs, rhs, expected, span);
            }
            ExprKind::Call { callee, args } => return self.call(callee, args, span),
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
                        self.ty_name(base.ty)
                    );
                    self.diagnostics
                        .push(Diagnostic::new(codes::V0102, name.span, message));
                    return None;
                };
                let Ty::Struct(id) = base.ty else {
                    unreachable!("only a struct has fields")
                };
                let def = &self.symbols.structs[id.0 as usize];
                let ty = def.fields[field_index].1;
                let kind = HirExprKind::Field {
                    base: Box::new(base),
                    name: name.name.clone(),
                };
                (kind, ty)
            }
            ExprKind::StructLit { name, fields } => {
                return self.struct_lit(name, fields, span);
            }
            ExprKind::Block(block) => {
                let block = self.block(block, expected)?;
                let ty = block.ty;
                (HirExprKind::Block(block), ty)
            }
            ExprKind::If { cond, then, else_ } => {
                return self.if_expr(cond, then, else_.as_ref(), expected, span);
            }
            ExprKind::Intrinsic { format, args, .. } => {
                return self.println(&format.0, format.1, args, span);
            }
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
                let message = format!("nothing named `{}` exists here", name.name);
                self.diagnostics
                    .push(Diagnostic::new(codes::V0100, name.span, message));
                None
            }
        }
    }

    /// A `bool` condition of `if` or `while`.
    fn condition(&mut self, cond: &Expr) -> Option<HirExpr> {
        let cond = self.expr(cond, Some(Ty::Bool))?;
        self.expect(cond, Ty::Bool)
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
                self.expect(operand, Ty::Bool)?
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
                if !is_signed(operand.ty) {
                    let message = format!(
                        "unary `-` cannot be applied to `{}`",
                        self.ty_name(operand.ty)
                    );
                    self.diagnostics
                        .push(Diagnostic::new(codes::V0200, span, message));
                    return None;
                }
                operand
            }
        };
        Some(HirExpr {
            ty: operand.ty,
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
                .and_then(|lhs| self.expect(lhs, Ty::Bool));
            let rhs = self
                .expr(rhs, Some(Ty::Bool))
                .and_then(|rhs| self.expect(rhs, Ty::Bool));
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
        let second = self.expr(second, Some(first.ty))?;
        if second.ty != first.ty {
            let diagnostic = Diagnostic::new(
                codes::V0200,
                second.span,
                format!(
                    "mismatched types: expected `{}`, found `{}`",
                    self.ty_name(first.ty),
                    self.ty_name(second.ty)
                ),
            )
            .with_label(
                first.span,
                format!("this operand is `{}`", self.ty_name(first.ty)),
            );
            self.diagnostics.push(diagnostic);
            return None;
        }
        let ty = first.ty;
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
                    self.ty_name(ty)
                );
                let diagnostic = Diagnostic::new(codes::V0203, span, message).with_note(
                    "milestone 1 defines `==` and `!=` only for primitives and `string`",
                );
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

        if !matches!(ty, Ty::Int(_) | Ty::Float(_)) {
            let mut diagnostic = Diagnostic::new(
                codes::V0200,
                span,
                format!("`{symbol}` cannot be applied to `{}`", self.ty_name(ty)),
            )
            .with_note(format!("`{symbol}` is defined for numeric types only"));
            if ty == Ty::String && op == BinaryOp::Add {
                diagnostic = diagnostic.with_note("`+` on `string` is not in milestone 1");
            }
            self.diagnostics.push(diagnostic);
            return None;
        }
        let result = if arithmetic { ty } else { Ty::Bool };
        Some(binary_expr(op, lhs, rhs, result, span))
    }

    fn call(&mut self, callee: &Expr, args: &[Expr], span: Span) -> Option<HirExpr> {
        let ExprKind::Path { module, name } = &callee.kind else {
            // The parser already rejected method-call syntax.
            self.diagnostics.push(Diagnostic::new(
                codes::V0001,
                callee.span,
                "only a function path can be called in milestone 1",
            ));
            return None;
        };
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
                self.lookup_error(error, "function", &path, callee.span);
                return None;
            }
        };

        let (params, ret): (Vec<Ty>, Ty) = match found {
            Callee::Varyk(id) => {
                let sig = &self.symbols.fns[id.0 as usize];
                (sig.params.iter().map(|p| p.1).collect(), sig.ret)
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
                (sig.params.iter().map(|p| p.0).collect(), sig.ret)
            }
        };

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
                self.expr(arg, Some(ty))
                    .and_then(|arg| self.expect(arg, ty)),
            );
        }
        let args = checked.into_iter().collect::<Option<Vec<_>>>()?;
        Some(HirExpr {
            kind: HirExprKind::Call {
                callee: found,
                args,
            },
            ty: ret,
            span,
        })
    }

    fn struct_lit(
        &mut self,
        name: &Ident,
        fields: &[(Ident, Expr)],
        span: Span,
    ) -> Option<HirExpr> {
        let Some(id) = self.symbols.lookup_struct(self.module, &name.name) else {
            let message = format!("unknown struct `{}`", name.name);
            self.diagnostics
                .push(Diagnostic::new(codes::V0101, name.span, message));
            return None;
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
            let ty = def.fields[index].1;
            match self.expr(value, Some(ty)).and_then(|v| self.expect(v, ty)) {
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
                    self.ty_name(tail.ty)
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

        let then = self.block(then, expected);
        // The then-branch types the else-branch's literals, unless it
        // always leaves early and so says nothing about the value.
        let then_diverges = then.as_ref().is_some_and(block_diverges);
        let else_expected = match &then {
            Some(then) if !then_diverges => expected.or(Some(then.ty)),
            _ => expected,
        };
        let else_block = self.block(else_, else_expected);
        let (cond, then, else_block) = (cond?, then?, else_block?);

        let ty = if then_diverges {
            else_block.ty
        } else if block_diverges(&else_block) || else_block.ty == then.ty {
            then.ty
        } else {
            let message = format!(
                "`if` and `else` have incompatible types: expected `{}`, found `{}`",
                self.ty_name(then.ty),
                self.ty_name(else_block.ty)
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

    fn println(
        &mut self,
        format: &str,
        format_span: Span,
        args: &[Expr],
        span: Span,
    ) -> Option<HirExpr> {
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
                        "unsupported placeholder `{}`: milestone 1 supports only `{{}}`",
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
                        self.ty_name(arg.ty)
                    );
                    self.diagnostics.push(
                        Diagnostic::new(codes::V0203, arg.span, message)
                            .with_note("structs have no printed form yet"),
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
        Some(HirExpr {
            kind: HirExprKind::Println {
                format: format.to_string(),
                args: lowered,
            },
            ty: Ty::Unit,
            span,
        })
    }

    // --- Diagnostics helpers --------------------------------------------

    /// `expr` if it has type `ty`, else V0200 at `expr`.
    fn expect(&mut self, expr: HirExpr, ty: Ty) -> Option<HirExpr> {
        if expr.ty == ty {
            Some(expr)
        } else {
            self.mismatch(expr.span, ty, expr.ty);
            None
        }
    }

    fn mismatch(&mut self, span: Span, expected: Ty, found: Ty) {
        let message = format!(
            "mismatched types: expected `{}`, found `{}`",
            self.ty_name(expected),
            self.ty_name(found)
        );
        self.diagnostics
            .push(Diagnostic::new(codes::V0200, span, message));
    }

    /// V0100 for an unknown item; V0105 with a `pub ` fix-it for one that
    /// is not visible from here.
    fn lookup_error(&mut self, error: LookupError, what: &str, path: &str, span: Span) {
        let diagnostic = match error {
            LookupError::Unknown => {
                Diagnostic::new(codes::V0100, span, format!("cannot find {what} `{path}`"))
            }
            LookupError::NotVisible(decl) => {
                let diagnostic = Diagnostic::new(
                    codes::V0105,
                    span,
                    format!(
                        "{what} `{path}` is not marked `pub`, so it can only be used inside its own module"
                    ),
                );
                declared_without_pub(diagnostic, decl, "fn")
            }
        };
        self.diagnostics.push(diagnostic);
    }

    fn ty_name(&self, ty: Ty) -> String {
        match ty {
            Ty::Bool => "bool".to_string(),
            Ty::Int(kind) => kind.name().to_string(),
            Ty::Float(kind) => kind.name().to_string(),
            Ty::String => "string".to_string(),
            Ty::Struct(id) => self.symbols.structs[id.0 as usize].name.clone(),
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

fn is_signed(ty: Ty) -> bool {
    matches!(
        ty,
        Ty::Int(IntKind::I8 | IntKind::I16 | IntKind::I32 | IntKind::I64) | Ty::Float(_)
    )
}

/// A local, or a field of a place.
fn is_place(expr: &HirExpr) -> bool {
    match &expr.kind {
        HirExprKind::Local(_) => true,
        HirExprKind::Field { base, .. } => is_place(base),
        _ => false,
    }
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
        IntKind::U64 => (0, u64::MAX.into()),
    }
}
