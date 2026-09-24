//! The move checker (spec 4.2): a use of a local after its value moved is
//! V0305.
//!
//! Only locals that are not borrowed places and have struct or `string`
//! type move, whatever the string's representation (`string` is never Copy
//! in Varyk). A move is the local evaluated as a value that flows into a
//! `let`, an assignment, or an owned slot (a struct-literal field, a return
//! value, an imported parameter taken by value, storage written through a
//! borrowed place). Passing to a borrowing parameter, reading a field,
//! comparing, and printing are uses, not moves.
//!
//! The walk follows statement order with a "maybe moved" set: both `if`
//! branches may run, so the set after an `if` is the union of both;
//! `return`, `break`, and `continue` make the rest of their block
//! unreachable; a `while` body is walked twice, so a move in one iteration
//! is seen by the next. Assigning to a moved local (or re-running its
//! `let`) makes it usable again.

use std::collections::{HashMap, HashSet};

use varyk_syntax::Span;

use super::Context;
use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{HirBlock, HirExpr, HirExprKind, HirFunction, HirStmt, LocalId, LocalInfo};
use crate::types::{ParamMode, Ty};

/// The locals that may have moved, each with the span of a move; `None`
/// for unreachable code.
type State = Option<HashMap<LocalId, Span>>;

fn union(a: State, b: State) -> State {
    match (a, b) {
        (None, state) | (state, None) => state,
        (Some(mut a), Some(b)) => {
            for (local, span) in b {
                a.entry(local).or_insert(span);
            }
            Some(a)
        }
    }
}

/// Reports every use after move in `function`.
pub(super) fn check(cx: &Context, function: &HirFunction, diagnostics: &mut Vec<Diagnostic>) {
    let mut checker = Checker {
        cx,
        locals: &function.locals,
        ret: function.ret,
        state: Some(HashMap::new()),
        loops: Vec::new(),
        repeating: 0,
        reported: HashSet::new(),
        diagnostics,
    };
    checker.block(&function.body, function.ret != Ty::Unit);
}

/// The states flowing out of a loop other than through its condition.
#[derive(Default)]
struct Loop {
    breaks: State,
    continues: State,
}

struct Checker<'a> {
    cx: &'a Context<'a>,
    locals: &'a [LocalInfo],
    ret: Ty,
    state: State,
    loops: Vec<Loop>,
    /// How many enclosing loops are in their second walk.
    repeating: u32,
    /// Start and end of the use spans already reported (one function is
    /// one file): a loop body is walked twice.
    reported: HashSet<(u32, u32)>,
    diagnostics: &'a mut Vec<Diagnostic>,
}

impl Checker<'_> {
    /// Walks `block`; its tail is moved when `consume`.
    fn block(&mut self, block: &HirBlock, consume: bool) {
        for stmt in &block.stmts {
            self.stmt(stmt);
        }
        if let Some(tail) = &block.tail {
            self.expr(tail, consume);
        }
    }

    fn stmt(&mut self, stmt: &HirStmt) {
        match stmt {
            HirStmt::Let { local, value, .. } => {
                self.expr(value, true);
                self.revive(*local);
            }
            HirStmt::Assign { target, value, .. } => {
                self.expr(value, true);
                match target.kind {
                    HirExprKind::Local(id) => self.revive(id),
                    // Writing a field of a moved struct is a use of it.
                    _ => self.expr(target, false),
                }
            }
            HirStmt::Expr { expr, .. } => self.expr(expr, false),
            HirStmt::Return { value, .. } => {
                if let Some(value) = value {
                    self.expr(value, self.ret != Ty::Unit);
                }
                self.state = None;
            }
            HirStmt::While { cond, body, .. } => self.while_(cond, body),
            HirStmt::Break { .. } => {
                let state = self.state.take();
                let frame = self.loops.last_mut().expect("`break` is inside a loop");
                frame.breaks = union(frame.breaks.take(), state);
            }
            HirStmt::Continue { .. } => {
                let state = self.state.take();
                let frame = self.loops.last_mut().expect("`continue` is inside a loop");
                frame.continues = union(frame.continues.take(), state);
            }
        }
    }

    /// Walks the loop twice: the second walk starts from every state that
    /// can reach the loop head, so it sees the first iteration's moves.
    fn while_(&mut self, cond: &HirExpr, body: &HirBlock) {
        let entry = self.state.clone();
        self.loops.push(Loop::default());
        self.expr(cond, false);
        let mut exit = self.state.clone();
        self.block(body, false);
        let continues = self
            .loops
            .last_mut()
            .expect("pushed above")
            .continues
            .take();
        self.state = union(union(entry, self.state.take()), continues);

        self.repeating += 1;
        self.expr(cond, false);
        exit = union(exit, self.state.clone());
        self.block(body, false);
        self.repeating -= 1;

        let frame = self.loops.pop().expect("pushed above");
        self.state = union(exit, frame.breaks);
    }

    /// Walks `expr`; its value is moved when `consume`.
    fn expr(&mut self, expr: &HirExpr, consume: bool) {
        match &expr.kind {
            HirExprKind::Int(_)
            | HirExprKind::Float(_)
            | HirExprKind::Bool(_)
            | HirExprKind::String(_) => {}
            HirExprKind::Local(id) => self.use_(*id, expr.span, consume),
            HirExprKind::Call { callee, args } => {
                let (_, modes, imported) = self.cx.callee(*callee);
                for (arg, mode) in args.iter().zip(modes) {
                    self.expr(arg, imported && mode == ParamMode::Owned);
                }
            }
            HirExprKind::Field { base, .. } => self.expr(base, false),
            HirExprKind::StructLit { fields, .. } => {
                for (_, value) in fields {
                    self.expr(value, true);
                }
            }
            HirExprKind::Unary { operand, .. } => self.expr(operand, false),
            HirExprKind::Binary { lhs, rhs, .. } => {
                self.expr(lhs, false);
                self.expr(rhs, false);
            }
            HirExprKind::Block(block) => self.block(block, consume),
            HirExprKind::If { cond, then, else_ } => {
                self.expr(cond, false);
                let before = self.state.clone();
                self.block(then, consume);
                let after_then = std::mem::replace(&mut self.state, before);
                if let Some(else_) = else_ {
                    self.block(else_, consume);
                }
                self.state = union(after_then, self.state.take());
            }
            HirExprKind::Println { args, .. } => {
                for arg in args {
                    self.expr(arg, false);
                }
            }
        }
    }

    /// A use of `local` at `span`, which moves it when `consume`.
    fn use_(&mut self, local: LocalId, span: Span, consume: bool) {
        let Some(state) = &mut self.state else {
            return;
        };
        if let Some(&moved) = state.get(&local) {
            if self.reported.insert((span.start, span.end)) {
                let name = &self.locals[local.0 as usize].name;
                let mut diagnostic = Diagnostic::new(
                    codes::V0305,
                    span,
                    format!("`{name}` was given away earlier and cannot be used again"),
                )
                .with_label(moved, format!("`{name}` is given away here"));
                if self.repeating > 0 {
                    diagnostic = diagnostic
                        .with_note("it was given away the previous time through the loop");
                }
                self.diagnostics.push(diagnostic);
            }
        }
        let info = &self.locals[local.0 as usize];
        let movable = !info.place.borrowed && matches!(info.ty, Ty::String | Ty::Struct(_));
        if consume && movable {
            // Keep the first move's span: a further consuming use after the
            // one already reported must still point back at that original
            // move, not at itself.
            state.entry(local).or_insert(span);
        }
    }

    /// `local` holds a fresh value.
    fn revive(&mut self, local: LocalId) {
        if let Some(state) = &mut self.state {
            state.remove(&local);
        }
    }
}
