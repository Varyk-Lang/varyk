//! The move checker (spec 4.2): a use of a local after its value moved is
//! V0305. The flow walk it runs on, [`walk`], is shared with the alias
//! rule (`aliases`).
//!
//! Only locals that are not borrowed places and have struct or `string`
//! type move, whatever the string's representation (`string` is never Copy
//! in Varyk). A move is the local evaluated as a value that flows into a
//! `let`, an assignment, or an owned slot (a struct-literal field, a variant
//! payload, a `Some`, `Ok`, or `Err` argument, a `vec!` element, `push`'s
//! argument, the operand of `?`, a return value, an imported parameter
//! taken by value, storage written through a borrowed place). Passing to a
//! borrowing parameter or as a receiver, reading a field or an element,
//! comparing, and printing are uses, not moves.
//!
//! The walk follows statement order with a set of marked locals (for this
//! checker, "maybe moved"): both `if` branches may run, so the set after an
//! `if` is the union of both, and likewise for the arms of a `match`;
//! `return`, `break`, and `continue` make the rest of their block
//! unreachable; a `while` or `for` body is walked twice, so a mark made in
//! one iteration is seen by the next: a loop body counts as following
//! itself. Assigning to a moved local (or re-running its `let`, or its
//! `for` binding it again) makes it usable again.

use std::collections::{HashMap, HashSet};

use varyk_syntax::Span;

use super::{Context, roots};
use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{
    HirBlock, HirExpr, HirExprKind, HirForHead, HirFunction, HirStmt, LocalId, LocalInfo,
    field_root, is_block_like, is_place, leaves,
};
use crate::types::{ParamMode, Ty};

/// The marked locals, each with what its rule records about the mark.
pub(super) type Marks<M> = HashMap<LocalId, M>;

/// How a local is accessed where it is mentioned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Access {
    /// Read, compared, printed, or lent to a parameter that only reads it.
    Read,
    /// Evaluated as a value that flows into a `let`, an assignment, or an
    /// owned slot: a move when the local owns a struct or `string`.
    Move,
    /// The local, or a part of it, is assigned to or lent to a `mut`
    /// parameter.
    Change,
    /// The whole local is assigned a new value.
    Replace,
}

/// A flow-sensitive rule: what happens to the marks at each event of
/// [`walk`]. `repeating` is whether the event is seen in the second walk
/// of an enclosing loop body.
pub(super) trait Rule {
    type Mark: Clone;

    /// `local` is accessed at `span` (evaluation order).
    fn access(
        &mut self,
        marks: &mut Marks<Self::Mark>,
        local: LocalId,
        span: Span,
        access: Access,
        repeating: bool,
    );

    /// `let local = value` binds `local`; `value` was walked just before.
    fn bind(&mut self, marks: &mut Marks<Self::Mark>, local: LocalId, value: &HirExpr);

    /// A `match` pattern binds `local` to `scrutinee` (walked just
    /// before) as a whole (`whole`, a catch-all name) or to a part of it.
    fn bind_pattern(
        &mut self,
        marks: &mut Marks<Self::Mark>,
        local: LocalId,
        scrutinee: &HirExpr,
        whole: bool,
    ) {
        let _ = whole;
        self.bind(marks, local, scrutinee);
    }

    /// `local`, the variable of a `for` over a place, is used at the end
    /// of a round, whether or not the body mentions it: the loop holds that
    /// place until it ends (spec 3.1).
    fn held(&mut self, marks: &mut Marks<Self::Mark>, local: LocalId, head: Head) {
        let _ = (marks, local, head);
    }

    /// `local`, mentioned at `span`, is lent to a call or a comparison
    /// that is made only after every argument or operand is evaluated, so
    /// it is used again there.
    fn lent(&mut self, marks: &mut Marks<Self::Mark>, local: LocalId, span: Span, repeating: bool) {
        let _ = (marks, local, span, repeating);
    }
}

/// Walks `function`'s body in evaluation order, telling `rule` about every
/// access and binding along each path (see the module docs for the flow).
pub(super) fn walk<R: Rule>(cx: &Context, function: &HirFunction, rule: &mut R) {
    let mut walker = Walker {
        cx,
        ret: function.ret.clone(),
        rule,
        marks: Some(HashMap::new()),
        loops: Vec::new(),
        repeating: 0,
    };
    let access = if function.ret == Ty::Unit {
        Access::Read
    } else {
        Access::Move
    };
    walker.block(&function.body, access);
}

/// Reports every use after move in `function`.
pub(super) fn check(cx: &Context, function: &HirFunction, diagnostics: &mut Vec<Diagnostic>) {
    let mut moves = Moves {
        locals: &function.locals,
        reported: HashSet::new(),
        diagnostics,
    };
    walk(cx, function, &mut moves);
}

/// The move checker's [`Rule`]: a mark is a local that may have moved,
/// with the span of a move.
struct Moves<'a> {
    locals: &'a [LocalInfo],
    /// Start and end of the use spans already reported (one function is
    /// one file): a loop body is walked twice.
    reported: HashSet<(u32, u32)>,
    diagnostics: &'a mut Vec<Diagnostic>,
}

impl Rule for Moves<'_> {
    type Mark = Span;

    fn access(
        &mut self,
        marks: &mut Marks<Span>,
        local: LocalId,
        span: Span,
        access: Access,
        repeating: bool,
    ) {
        if access == Access::Replace {
            // The local holds a fresh value.
            marks.remove(&local);
            return;
        }
        if let Some(&moved) = marks.get(&local) {
            if self.reported.insert((span.start, span.end)) {
                let name = &self.locals[local.0 as usize].name;
                let mut diagnostic = Diagnostic::new(
                    codes::V0305,
                    span,
                    format!("`{name}` was given away earlier and cannot be used again"),
                )
                .with_label(moved, format!("`{name}` is given away here"));
                if repeating {
                    diagnostic = diagnostic
                        .with_note("it was given away the previous time through the loop");
                }
                self.diagnostics.push(diagnostic);
            }
        }
        if access == Access::Move && movable(&self.locals[local.0 as usize]) {
            // Keep the first move's span: a further consuming use after the
            // one already reported must still point back at that original
            // move, not at itself.
            marks.entry(local).or_insert(span);
        }
    }

    fn bind(&mut self, marks: &mut Marks<Span>, local: LocalId, _value: &HirExpr) {
        marks.remove(&local);
    }
}

/// Whether evaluating `local` as a value moves it: it owns a struct (or
/// another compound value) or a `string`.
pub(super) fn movable(local: &LocalInfo) -> bool {
    !local.place.borrowed && (local.ty == Ty::String || local.ty.is_compound())
}

/// The place a `for` goes over, as [`Rule::held`] sees it.
#[derive(Debug, Clone, Copy)]
pub(super) struct Head {
    pub(super) span: Span,
    /// The local, when the head is one as a whole (not a part of one).
    pub(super) local: Option<LocalId>,
}

/// The marks flowing out of a loop other than through its condition;
/// `None` for none (or only from unreachable code).
struct Loop<M> {
    breaks: Option<Marks<M>>,
    continues: Option<Marks<M>>,
    /// For a `for` over a place: its variable and its head, held at the
    /// end of every round (see [`Rule::held`]).
    hold: Option<(LocalId, Head)>,
}

/// The marks of both ways into a join; `None` is unreachable code.
fn union<M>(a: Option<Marks<M>>, b: Option<Marks<M>>) -> Option<Marks<M>> {
    match (a, b) {
        (None, marks) | (marks, None) => marks,
        (Some(mut a), Some(b)) => {
            for (local, mark) in b {
                a.entry(local).or_insert(mark);
            }
            Some(a)
        }
    }
}

struct Walker<'a, R: Rule> {
    cx: &'a Context<'a>,
    ret: Ty,
    rule: &'a mut R,
    /// `None` for unreachable code.
    marks: Option<Marks<R::Mark>>,
    loops: Vec<Loop<R::Mark>>,
    /// How many enclosing loops are in their second walk.
    repeating: u32,
}

impl<R: Rule> Walker<'_, R> {
    /// Walks `block`; its tail is accessed as `access`.
    fn block(&mut self, block: &HirBlock, access: Access) {
        for stmt in &block.stmts {
            self.stmt(stmt);
        }
        if let Some(tail) = &block.tail {
            self.expr(tail, access);
        }
    }

    fn stmt(&mut self, stmt: &HirStmt) {
        match stmt {
            HirStmt::Let { local, value, .. } => {
                self.expr(value, Access::Move);
                if let Some(marks) = &mut self.marks {
                    self.rule.bind(marks, *local, value);
                }
            }
            HirStmt::Assign { target, value, .. } => {
                self.expr(value, Access::Move);
                match target.kind {
                    HirExprKind::Local(id) => self.access(id, target.span, Access::Replace),
                    _ => self.expr(target, Access::Change),
                }
            }
            HirStmt::Expr { expr, .. } => self.expr(expr, Access::Read),
            HirStmt::Return { value, .. } => {
                if let Some(value) = value {
                    let access = if self.ret == Ty::Unit {
                        Access::Read
                    } else {
                        Access::Move
                    };
                    self.expr(value, access);
                }
                self.marks = None;
            }
            HirStmt::While { cond, body, .. } => self.while_(cond, body),
            HirStmt::For {
                local, head, body, ..
            } => self.for_(*local, head, body),
            HirStmt::Break { .. } => {
                self.hold();
                let marks = self.marks.take();
                let frame = self.loops.last_mut().expect("`break` is inside a loop");
                frame.breaks = union(frame.breaks.take(), marks);
            }
            HirStmt::Continue { .. } => {
                self.hold();
                let marks = self.marks.take();
                let frame = self.loops.last_mut().expect("`continue` is inside a loop");
                frame.continues = union(frame.continues.take(), marks);
            }
        }
    }

    /// Walks the loop twice: the second walk starts from every state that
    /// can reach the loop head, so it sees the first iteration's marks.
    fn while_(&mut self, cond: &HirExpr, body: &HirBlock) {
        let entry = self.marks.clone();
        self.loops.push(Loop {
            breaks: None,
            continues: None,
            hold: None,
        });
        self.expr(cond, Access::Read);
        let mut exit = self.marks.clone();
        self.block(body, Access::Read);
        let continues = self
            .loops
            .last_mut()
            .expect("pushed above")
            .continues
            .take();
        self.marks = union(union(entry, self.marks.take()), continues);

        self.repeating += 1;
        self.expr(cond, Access::Read);
        exit = union(exit, self.marks.clone());
        self.block(body, Access::Read);
        self.repeating -= 1;

        let frame = self.loops.pop().expect("pushed above");
        self.marks = union(exit, frame.breaks);
    }

    /// Walks the head, then the body twice, as [`Walker::while_`] does:
    /// the loop ends where the next round would start, so every state
    /// that can reach that point can leave the loop. A place looped over
    /// is only looked at; a temporary is owned by the loop (spec 3.2).
    fn for_(&mut self, local: LocalId, head: &HirForHead, body: &HirBlock) {
        let hold = match head {
            HirForHead::Range { start, end } => {
                self.expr(start, Access::Read);
                self.expr(end, Access::Read);
                None
            }
            HirForHead::Vec(vec) if is_place(vec) => {
                self.expr(vec, Access::Read);
                let whole = match vec.kind {
                    HirExprKind::Local(id) => Some(id),
                    _ => None,
                };
                Some((
                    local,
                    Head {
                        span: vec.span,
                        local: whole,
                    },
                ))
            }
            HirForHead::Vec(vec) => {
                self.expr(vec, Access::Move);
                None
            }
        };
        let entry = self.marks.clone();
        self.loops.push(Loop {
            breaks: None,
            continues: None,
            hold,
        });
        let mut exit = self.marks.clone();
        self.round(local, body);
        let continues = self
            .loops
            .last_mut()
            .expect("pushed above")
            .continues
            .take();
        self.marks = union(union(entry, self.marks.take()), continues);

        self.repeating += 1;
        exit = union(exit, self.marks.clone());
        self.round(local, body);
        self.repeating -= 1;

        let frame = self.loops.pop().expect("pushed above");
        let rounds = union(self.marks.take(), frame.continues);
        self.marks = union(union(exit, rounds), frame.breaks);
    }

    /// One round of a `for`: its variable is bound afresh, so whatever was
    /// marked on it is gone; then the body runs, and the loop's hold is
    /// used at its end.
    fn round(&mut self, local: LocalId, body: &HirBlock) {
        if let Some(marks) = &mut self.marks {
            marks.remove(&local);
        }
        self.block(body, Access::Read);
        self.hold();
    }

    /// Tells the rule that the innermost loop's hold, if any, is used here:
    /// at the end of a round, or at a `break` or `continue` leaving it.
    fn hold(&mut self) {
        let Some((local, head)) = self.loops.last().and_then(|frame| frame.hold) else {
            return;
        };
        if let Some(marks) = &mut self.marks {
            self.rule.held(marks, local, head);
        }
    }

    /// Walks `expr`; its value is accessed as `access`.
    fn expr(&mut self, expr: &HirExpr, access: Access) {
        match &expr.kind {
            HirExprKind::Int(_)
            | HirExprKind::Float(_)
            | HirExprKind::Bool(_)
            | HirExprKind::String(_) => {}
            HirExprKind::Local(id) => self.access(*id, expr.span, access),
            HirExprKind::Call { callee, args } => {
                let (_, modes, keeps) = self.cx.callee(*callee);
                self.call(args.iter(), &modes, keeps);
            }
            // The receiver is an argument: a changing method changes it.
            HirExprKind::MethodCall {
                receiver,
                method,
                args,
            } => {
                let (_, modes, keeps) = self.cx.method(*method);
                self.call(std::iter::once(&**receiver).chain(args), &modes, keeps);
            }
            // Reading or moving a field or an element reads its base;
            // changing one changes its base.
            HirExprKind::Field { base, .. } | HirExprKind::Index { base, .. } => {
                let base_access = match access {
                    Access::Change | Access::Replace => Access::Change,
                    Access::Read | Access::Move => Access::Read,
                };
                self.expr(base, base_access);
                if let HirExprKind::Index { index, .. } = &expr.kind {
                    self.expr(index, Access::Read);
                }
            }
            HirExprKind::StructLit { fields, .. } => {
                for (_, value) in fields {
                    self.expr(value, Access::Move);
                }
            }
            HirExprKind::Unary { operand, .. } => self.expr(operand, Access::Read),
            HirExprKind::Binary { lhs, rhs, .. } => {
                self.expr(lhs, Access::Read);
                self.expr(rhs, Access::Read);
                // String operands are both borrowed for the comparison.
                if lhs.ty == Ty::String {
                    self.lent(&[lhs, rhs]);
                }
            }
            HirExprKind::Block(block) => self.block(block, access),
            HirExprKind::If { cond, then, else_ } => {
                self.expr(cond, Access::Read);
                let before = self.marks.clone();
                self.block(then, access);
                let after_then = std::mem::replace(&mut self.marks, before);
                if let Some(else_) = else_ {
                    self.block(else_, access);
                }
                self.marks = union(after_then, self.marks.take());
            }
            // A place is only looked at; a temporary is owned by the
            // `match` (spec 3.2). Each arm starts where the scrutinee
            // leaves off, and any may be the one that runs.
            HirExprKind::Match { scrutinee, arms } => {
                let head = if is_place(scrutinee) {
                    Access::Read
                } else {
                    Access::Move
                };
                self.expr(scrutinee, head);
                let before = self.marks.clone();
                let mut after = None;
                for arm in arms {
                    self.marks = before.clone();
                    if let Some(marks) = &mut self.marks {
                        for (local, whole) in arm.pattern.bindings() {
                            self.rule.bind_pattern(marks, local, scrutinee, whole);
                        }
                    }
                    self.expr(&arm.body, access);
                    after = union(after, self.marks.take());
                }
                self.marks = after;
            }
            HirExprKind::Try(operand) => self.expr(operand, Access::Move),
            HirExprKind::EnumLit { args, .. } | HirExprKind::VecLit(args) => {
                for arg in args {
                    self.expr(arg, Access::Move);
                }
            }
            HirExprKind::Println { args, .. } | HirExprKind::Format { args, .. } => {
                for arg in args {
                    self.expr(arg, Access::Read);
                }
                // The format machinery borrows every argument for the whole
                // call (spec 4.2).
                let args: Vec<&HirExpr> = args.iter().collect();
                self.lent(&args);
            }
        }
    }

    /// The arguments of a call, in order, each accessed as its parameter's
    /// mode says; `keeps` makes an `Owned` parameter an owned slot.
    fn call<'e>(
        &mut self,
        args: impl Iterator<Item = &'e HirExpr>,
        modes: &[ParamMode],
        keeps: bool,
    ) {
        let mut lent = Vec::new();
        for (arg, &mode) in args.zip(modes) {
            let access = match mode {
                ParamMode::MutableBorrow => Access::Change,
                ParamMode::Owned if keeps => Access::Move,
                ParamMode::Owned => Access::Read,
                ParamMode::SharedBorrow => Access::Read,
            };
            self.expr(arg, access);
            // A Copy argument to a Varyk `Owned` parameter is copied, and
            // one to an imported or built-in `Owned` parameter is moved;
            // every other argument is a reference.
            if mode != ParamMode::Owned {
                lent.push(arg);
            }
        }
        self.lent(&lent);
    }

    fn access(&mut self, local: LocalId, span: Span, access: Access) {
        if let Some(marks) = &mut self.marks {
            self.rule
                .access(marks, local, span, access, self.repeating > 0);
        }
    }

    /// Tells the rule about every local `values` (already walked) are
    /// rooted at: they stay lent until the call or comparison is made.
    fn lent(&mut self, values: &[&HirExpr]) {
        let Some(marks) = &mut self.marks else {
            return;
        };
        for value in values {
            for leaf in leaves(value) {
                let base = field_root(leaf);
                match base.kind {
                    HirExprKind::Local(id) => {
                        self.rule.lent(marks, id, base.span, self.repeating > 0)
                    }
                    // A field of an `if`, block, or `match`: every local it
                    // may be rooted at, pointing at the whole value.
                    _ if is_block_like(base) => {
                        for id in roots(base) {
                            self.rule.lent(marks, id, leaf.span, self.repeating > 0);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
