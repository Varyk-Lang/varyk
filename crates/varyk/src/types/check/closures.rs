//! Typing of closures (M4 spec 2.2): a closure is only the argument of a
//! table row whose parameter is one, and is typed at that call, its
//! parameter from the row and its body in a nested scope.

use varyk_syntax::{Expr, ExprKind, Ident, Span};

use super::methods::{listing, owner_words};
use super::{FnChecker, Scope};
use crate::builtins::{self, BuiltinId, Owner};
use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{HirBlock, HirExpr, HirExprKind, LocalId, LocalKind};
use crate::types::Ty;

/// A closure whose body is being checked.
pub(super) struct Frame {
    /// The first local the closure declares, its parameter: every local
    /// below it belongs to the function around the closure.
    first: LocalId,
    /// The outer locals the body uses, in order of first use.
    captures: Vec<LocalId>,
}

impl FnChecker<'_> {
    /// Records `local`, just looked up, as a capture of every enclosing
    /// closure it is declared outside of.
    pub(super) fn capture(&mut self, local: LocalId) {
        for frame in &mut self.closures {
            if local.0 < frame.first.0 && !frame.captures.contains(&local) {
                frame.captures.push(local);
            }
        }
    }

    /// V0001 at `span` for `what` (`break`, `return`, `?`, ...) inside a
    /// closure, when one encloses it: the closure's body is a value of its
    /// own, so it would leave the closure, not the function.
    pub(super) fn leaves_closure(&mut self, what: &str, span: Span) -> Option<()> {
        if self.closures.is_empty() {
            return Some(());
        }
        let message = format!(
            "{what} cannot be used inside a closure, which is a value of its own: it would \
             leave the closure, not the function around it"
        );
        self.diagnostics
            .push(Diagnostic::new(codes::V0001, span, message).with_note(
                "give the closure's value as its last expression instead, and use `match` on \
                 an `Option` or `Result` inside it",
            ));
        None
    }
}

/// V0001 at `span` for a closure anywhere but the argument of a call that
/// takes one, naming those calls (M4 spec 2.2).
pub(super) fn misplaced(span: Span) -> Diagnostic {
    // The rows by owner, in table order: "`map`, `filter`, ... on a chain".
    let mut owners: Vec<(Owner, Vec<&str>)> = Vec::new();
    for entry in builtins::closure_rows() {
        match owners.iter_mut().find(|(owner, _)| *owner == entry.owner) {
            Some((_, names)) => names.push(entry.name),
            None => owners.push((entry.owner, vec![entry.name])),
        }
    }
    let calls: Vec<String> = owners
        .iter()
        .map(|(owner, names)| format!("{} on {}", listing(names), owner_words(*owner)))
        .collect();
    let calls = match calls.as_slice() {
        [first, last] => format!("{first} and {last}"),
        [init @ .., last] if !init.is_empty() => format!("{}, and {last}", init.join(", ")),
        _ => calls.concat(),
    };
    Diagnostic::new(
        codes::V0001,
        span,
        format!("a closure can only be the argument of a call that takes one: {calls}"),
    )
    .with_note("in Varyk a closure is never a value of its own: it cannot be stored, passed to a function, or returned")
}

/// Checks `closure`, the argument of row `row` for a closure parameter
/// handed `param_ty`, its body expected to be `expected` when the call's
/// own expected type says what the closure must return. Returns the
/// closure and what its body gives: exactly one untyped parameter (V0201
/// otherwise), bound in a scope of its own, and a body that never leaves
/// the closure early.
pub(super) fn check(
    ctx: &mut FnChecker,
    row: BuiltinId,
    closure: &Expr,
    param_ty: Ty,
    expected: Option<Ty>,
) -> Option<(HirExpr, Ty)> {
    let name = row.get().name;
    let ExprKind::Closure { params, body, .. } = &closure.kind else {
        let message = format!("`{name}` takes a closure here, such as `|x| ..`");
        ctx.diagnostics
            .push(Diagnostic::new(codes::V0200, closure.span, message));
        return None;
    };
    let [param] = params.as_slice() else {
        let count = match params.len() {
            0 => "none".to_string(),
            n => n.to_string(),
        };
        let message = format!(
            "`{name}` hands its closure one value, so the closure takes exactly one parameter, \
             as in `|x| ..`; this one has {count}"
        );
        ctx.diagnostics
            .push(Diagnostic::new(codes::V0201, closure.span, message));
        return None;
    };
    let first = LocalId(ctx.locals.len() as u32);
    ctx.closures.push(Frame {
        first,
        captures: Vec::new(),
    });
    let loop_depth = std::mem::replace(&mut ctx.loop_depth, 0);
    ctx.scopes.push(Scope::new());
    let local = bind_param(ctx, param, param_ty);
    let before = ctx.diagnostics.len();
    let body = match &body.kind {
        ExprKind::Block(block) => ctx.block(block, expected),
        _ => ctx.expr(body, expected).map(|tail| HirBlock {
            stmts: Vec::new(),
            ty: tail.ty.clone(),
            span: tail.span,
            tail: Some(Box::new(tail)),
        }),
    };
    ctx.scopes.pop();
    ctx.loop_depth = loop_depth;
    let frame = ctx.closures.pop().expect("pushed above");
    if body.is_none() {
        // A value typed from where it goes: here, from where the call's
        // result goes.
        if let Some(hole) = ctx.diagnostics[before..]
            .iter_mut()
            .find(|d| d.code == codes::V0207)
        {
            hole.notes.push(format!(
                "a closure's value takes its type from where the result of `{name}` goes, \
                 such as a `let` with a written type"
            ));
        }
    }
    let body = body?;
    let ty = body.ty.clone();
    let expr = HirExpr {
        kind: HirExprKind::Closure {
            param: local,
            captures: frame.captures,
            body,
            returns_part: false,
        },
        ty: ty.clone(),
        span: closure.span,
    };
    Some((expr, ty))
}

/// Binds the closure's parameter, a read-only local of the closure.
fn bind_param(ctx: &mut FnChecker, param: &Ident, ty: Ty) -> LocalId {
    let local = ctx.bind(param, ty, false);
    ctx.locals[local.0 as usize].kind = LocalKind::ClosureParam { deref: false };
    local
}
