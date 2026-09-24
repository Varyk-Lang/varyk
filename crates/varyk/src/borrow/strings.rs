//! String representation (spec 4.3): which `string` locals are `&str` and
//! which are `String` in the generated Rust, plus the `let`-as-owned-slot
//! case of V0304.
//!
//! A `let` that is not a borrowed place (a *free* `let`) needs to be owned
//! if any value assigned to it is owned (a call result or an owned free
//! `let`), if it is passed to a `mut string` parameter, or if it flows into
//! an owned slot: a struct field, storage written through a borrowed place,
//! a return value, an imported parameter taken by value, or another free
//! `let` that needs to be owned. Value flows between free `let`s therefore
//! link them both ways, and [`infer`] computes the owned set by fixed-point
//! iteration over those links. A `let` that is a changeable borrowed place
//! is a `&mut String`, so every free `let` it may refer to is owned too.
//!
//! Assigning a free `let` text that is gone after the assignment (a field
//! of a temporary, or of a local declared inside the assigned block) is
//! V0304 whatever the `let`'s representation.

use varyk_syntax::Span;

use super::{
    Context, GONE_NOTE, MILESTONE_2_NOTE, Refers, dangling, gone_message, owner_text, place_info,
};
use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{
    HirBlock, HirExpr, HirExprKind, HirFunction, HirStmt, LocalId, LocalInfo, Origin, StringRepr,
    declared_inside, field_root, leaves,
};
use crate::types::{ParamMode, Ty};

/// Fills in [`LocalInfo::repr`] for every `string` local of `function` and
/// reports borrowed places flowing into a free `let` that must be owned,
/// except for locals the places pass already reported (`reported`), so one mistake
/// gets one code.
pub(super) fn infer(
    cx: &Context,
    function: &mut HirFunction,
    reported: &[bool],
    refers: &Refers,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut flows = Flows {
        cx,
        refers,
        locals: &function.locals,
        param_count: function.params.len(),
        owned: vec![false; function.locals.len()],
        links: Vec::new(),
        borrowed_into: Vec::new(),
        gone: Vec::new(),
        mixed: Vec::new(),
        blocks: Vec::new(),
    };
    flows.block(&function.body);
    if function.ret != Ty::Unit {
        if let Some(tail) = &function.body.tail {
            flows.owned_slot(tail);
        }
    }
    let Flows {
        mut owned,
        links,
        borrowed_into,
        gone,
        mixed,
        ..
    } = flows;

    let mut changed = true;
    while changed {
        changed = false;
        for &(a, b) in &links {
            let (a, b) = (a.0 as usize, b.0 as usize);
            if owned[a] != owned[b] {
                owned[a] = true;
                owned[b] = true;
                changed = true;
            }
        }
    }

    let param_count = function.params.len();
    let locals = &function.locals;
    let mut reprs = vec![None; locals.len()];
    for (index, local) in locals.iter().enumerate() {
        if local.ty != Ty::String {
            continue;
        }
        let owned_repr = if index < param_count {
            local.mutable
        } else if local.place.borrowed {
            match local.place.origin {
                Some(Origin::Param(param)) => locals[param.0 as usize].mutable,
                Some(Origin::Struct(_)) => true,
                None => false,
            }
        } else {
            owned[index]
        };
        reprs[index] = Some(if owned_repr {
            StringRepr::Owned
        } else {
            StringRepr::Borrowed
        });
    }

    for (local, span, message) in gone {
        if !reported[local.0 as usize] {
            diagnostics.push(Diagnostic::new(codes::V0304, span, message).with_note(GONE_NOTE));
        }
    }
    let mut report = |local: LocalId, span: Span, origin: Option<Origin>| {
        if reported[local.0 as usize] {
            return;
        }
        let owner = owner_text(cx, locals, origin);
        let name = &locals[local.0 as usize].name;
        let message = format!("{owner}, so it cannot be kept in `{name}`");
        let why = format!(
            "`{name}` needs its own copy of its text, because elsewhere it is given new text, \
             passed to a `mut string` parameter, or stored or returned"
        );
        diagnostics.push(
            Diagnostic::new(codes::V0304, span, message)
                .with_note(why)
                .with_note(MILESTONE_2_NOTE),
        );
    };
    for (local, span, origin) in borrowed_into {
        if owned[local.0 as usize] {
            report(local, span, origin);
        }
    }
    for mix in mixed {
        let needs_owned = mix.owned_call || mix.free.iter().any(|l| owned[l.0 as usize]);
        if needs_owned {
            for (span, origin) in mix.borrowed {
                report(mix.local, span, origin);
            }
        }
    }

    for (local, repr) in function.locals.iter_mut().zip(reprs) {
        local.repr = repr;
    }
}

/// A `let` that is a borrowed place and whose initializer also has
/// branches that might be owned.
struct Mixed {
    local: LocalId,
    /// The borrowed leaves: where V0304 points.
    borrowed: Vec<(Span, Option<Origin>)>,
    /// Some leaf is a `string` call result.
    owned_call: bool,
    /// Free `let`s among the leaves: owned ones make the binding need to be
    /// owned too.
    free: Vec<LocalId>,
}

/// The string value flows of one function.
struct Flows<'a> {
    cx: &'a Context<'a>,
    refers: &'a Refers,
    locals: &'a [LocalInfo],
    param_count: usize,
    /// Per local: directly known to need an owned `String`.
    owned: Vec<bool>,
    /// Value flows between free `let`s, which share ownedness.
    links: Vec<(LocalId, LocalId)>,
    /// A borrowed place assigned to a free `let`: an error if it is owned.
    borrowed_into: Vec<(LocalId, Span, Option<Origin>)>,
    /// A borrowed place assigned to a free `let` that is gone after the
    /// assignment (a field of a temporary, or of a local declared inside
    /// the assigned value), with its message: always an error.
    gone: Vec<(LocalId, Span, String)>,
    mixed: Vec<Mixed>,
    /// The blocks enclosing the statement being visited, outermost first.
    blocks: Vec<Span>,
}

impl Flows<'_> {
    /// A `string` `let` that is not a borrowed place.
    fn free(&self, local: LocalId) -> bool {
        let index = local.0 as usize;
        let info = &self.locals[index];
        index >= self.param_count && info.ty == Ty::String && !info.place.borrowed
    }

    fn free_leaf(&self, leaf: &HirExpr) -> Option<LocalId> {
        match leaf.kind {
            HirExprKind::Local(id) if self.free(id) => Some(id),
            _ => None,
        }
    }

    fn block(&mut self, block: &HirBlock) {
        self.blocks.push(block.span);
        for stmt in &block.stmts {
            self.stmt(stmt);
        }
        if let Some(tail) = &block.tail {
            self.expr(tail);
        }
        self.blocks.pop();
    }

    fn stmt(&mut self, stmt: &HirStmt) {
        match stmt {
            HirStmt::Let { local, value, .. } => {
                self.expr(value);
                self.bind(*local, value);
            }
            HirStmt::Assign { target, value, .. } => {
                self.expr(value);
                match target.kind {
                    HirExprKind::Local(id) if self.free(id) => self.assign_free(id, value),
                    _ => {
                        let borrowed = place_info(self.locals, target).is_some_and(|i| i.borrowed);
                        if borrowed {
                            self.owned_slot(value);
                        }
                    }
                }
            }
            HirStmt::Expr { expr, .. } => self.expr(expr),
            HirStmt::Return { value, .. } => {
                if let Some(value) = value {
                    self.expr(value);
                    self.owned_slot(value);
                }
            }
            HirStmt::While { cond, body, .. } => {
                self.expr(cond);
                self.block(body);
            }
            HirStmt::Break { .. } | HirStmt::Continue { .. } => {}
        }
    }

    fn expr(&mut self, expr: &HirExpr) {
        match &expr.kind {
            HirExprKind::Int(_)
            | HirExprKind::Float(_)
            | HirExprKind::Bool(_)
            | HirExprKind::String(_)
            | HirExprKind::Local(_) => {}
            HirExprKind::Call { callee, args } => {
                let (_, modes, imported) = self.cx.callee(*callee);
                for (arg, mode) in args.iter().zip(modes) {
                    self.expr(arg);
                    match mode {
                        ParamMode::MutableBorrow => self.owned_slot(arg),
                        ParamMode::Owned if imported => self.owned_slot(arg),
                        ParamMode::Owned | ParamMode::SharedBorrow => {}
                    }
                }
            }
            HirExprKind::Field { base, .. } => self.expr(base),
            HirExprKind::StructLit { fields, .. } => {
                for (_, value) in fields {
                    self.expr(value);
                    self.owned_slot(value);
                }
            }
            HirExprKind::Unary { operand, .. } => self.expr(operand),
            HirExprKind::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            HirExprKind::Block(block) => self.block(block),
            HirExprKind::If { cond, then, else_ } => {
                self.expr(cond);
                self.block(then);
                if let Some(else_) = else_ {
                    self.block(else_);
                }
            }
            HirExprKind::Println { args, .. } => {
                for arg in args {
                    self.expr(arg);
                }
            }
        }
    }

    /// `value` flows somewhere that needs an owned `String` (or, for a
    /// `mut string` parameter, a `&mut String`): every free `let` it
    /// evaluates to must be owned.
    fn owned_slot(&mut self, value: &HirExpr) {
        for leaf in leaves(value) {
            if let Some(id) = self.free_leaf(leaf) {
                self.owned[id.0 as usize] = true;
            }
        }
    }

    fn bind(&mut self, local: LocalId, value: &HirExpr) {
        if self.free(local) {
            self.assign_free(local, value);
            return;
        }
        let info = &self.locals[local.0 as usize];
        if info.ty != Ty::String || (local.0 as usize) < self.param_count {
            return;
        }
        // A binding that may change what it refers to is a `&mut String`
        // (never a `&str`, which cannot change its text), so every `let`
        // it may refer to must be a `String`.
        if info.place.mutable {
            self.owned_slot(value);
        }
        // A borrowed-place binding: it is a reference, which an owned
        // branch of its initializer cannot be.
        let mut mix = Mixed {
            local,
            borrowed: Vec::new(),
            owned_call: false,
            free: Vec::new(),
        };
        for leaf in leaves(value) {
            if let Some(id) = self.free_leaf(leaf) {
                mix.free.push(id);
            } else if matches!(leaf.kind, HirExprKind::Call { .. }) {
                mix.owned_call = true;
            } else if let Some(place) = place_info(self.locals, leaf) {
                if place.borrowed {
                    mix.borrowed.push((leaf.span, place.origin));
                }
            }
        }
        if mix.owned_call || !mix.free.is_empty() {
            self.mixed.push(mix);
        }
    }

    /// `value` is assigned to (or initializes) the free `let` `local`.
    fn assign_free(&mut self, local: LocalId, value: &HirExpr) {
        for leaf in leaves(value) {
            if let Some(id) = self.free_leaf(leaf) {
                self.links.push((id, local));
            } else if matches!(leaf.kind, HirExprKind::Call { .. }) {
                self.owned[local.0 as usize] = true;
            } else if let Some(place) = place_info(self.locals, leaf) {
                if place.borrowed && (self.gone(leaf, value) || self.gone_at_block_end(leaf, local))
                {
                    let name = &self.locals[local.0 as usize].name;
                    let consequence = format!("it cannot be kept in `{name}`");
                    let message = gone_message(self.cx, self.locals, leaf, &consequence);
                    self.gone.push((local, leaf.span, message));
                } else if place.borrowed {
                    self.borrowed_into.push((local, leaf.span, place.origin));
                }
            }
        }
    }

    /// Whether the place `leaf`, assigned to `local`, is rooted at a local
    /// declared in an enclosing block that `local` is not declared in (or
    /// a binding there that refers to one): it is dropped when that block
    /// ends, while `local` lives on.
    fn gone_at_block_end(&self, leaf: &HirExpr, local: LocalId) -> bool {
        let HirExprKind::Local(id) = field_root(leaf).kind else {
            return false;
        };
        let info = &self.locals[id.0 as usize];
        let target = &self.locals[local.0 as usize];
        self.blocks
            .iter()
            .filter(|&&block| !declared_inside(target, block))
            .any(|&block| {
                declared_inside(info, block)
                    && (!info.place.borrowed || dangling(self.locals, self.refers, id, block))
            })
    }

    /// Whether the place `leaf`, which `whole` evaluates to, is gone once
    /// `whole` has been evaluated: a temporary or a field of one, a local
    /// declared inside `whole` that owns its value (or a field of one), a
    /// binding declared there that refers to one of those, or a field of
    /// an `if` or block that may evaluate to any of those.
    fn gone(&self, leaf: &HirExpr, whole: &HirExpr) -> bool {
        let base = field_root(leaf);
        match &base.kind {
            HirExprKind::Local(id) => {
                let info = &self.locals[id.0 as usize];
                declared_inside(info, whole.span)
                    && (!info.place.borrowed || dangling(self.locals, self.refers, *id, whole.span))
            }
            HirExprKind::If { .. } | HirExprKind::Block(_) => leaves(base)
                .into_iter()
                .any(|inner| self.gone(inner, whole)),
            _ => true,
        }
    }
}
