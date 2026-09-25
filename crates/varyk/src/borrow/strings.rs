//! String representation (spec 4.3): whether each `string` local is a
//! `&str`, a `String`, a `&String`, or a `&mut String` in the generated
//! Rust, plus the `let`-as-owned-slot case of V0304.
//!
//! A `let` that is not a borrowed place (a *free* `let`) needs to be owned
//! if any value assigned to it is owned (a call or method call result, a
//! `clone` included, a `format!`, or an owned free `let`), if it is passed to
//! a `mut string` parameter, or if it flows into an owned slot: a struct
//! field, a variant payload, a `Some`, `Ok`, or `Err` argument, a `vec!`
//! element, `push`'s argument, storage written through a borrowed place (a
//! `Vec` element included), a return value, an imported parameter taken by
//! value, or another free `let` that needs to be owned. Value flows between free
//! `let`s therefore link them both ways, and [`infer`] computes the owned
//! set by fixed-point iteration over those links. A `let` that is a
//! changeable borrowed place is a `&mut String`, so every free `let` it may
//! refer to is owned too. A `match` pattern binding or a `for` variable is
//! a `&String` into a place matched on or looped over, or an owned `String`
//! moved out of a temporary (spec 3.5).
//!
//! Assigning a free `let` text that is gone after the assignment (a field
//! of a temporary, or of a local declared inside the assigned block) is
//! V0304 whatever the `let`'s representation. Scope is lexical, by
//! declaration span: a `match` binding's scope is its arm, and a `for`
//! variable's is its loop, so text from one over a temporary cannot be
//! kept in a `let` outside it.

use varyk_syntax::Span;

use super::{
    Assigned, BORROWED_NOTE, Context, GONE_NOTE, Refers, clone_fix_it, dangling, gone_message,
    owner_text, place_info, push_unique, what_to_do,
};
use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{
    HirBlock, HirExpr, HirExprKind, HirForHead, HirFunction, HirStmt, LocalId, LocalInfo, Origin,
    StringRepr, declared_inside, field_root, is_block_like, leaves,
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
    assigned: &Assigned,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut flows = Flows {
        cx,
        refers,
        assigned,
        locals: &function.locals,
        param_count: function.params.len(),
        owned: vec![false; function.locals.len()],
        links: Vec::new(),
        borrowed_into: Vec::new(),
        gone: Vec::new(),
        gone_copies: Vec::new(),
        mixed: Vec::new(),
        str_leaves: vec![None; function.locals.len()],
        pattern_refs: vec![false; function.locals.len()],
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
        gone_copies,
        mixed,
        str_leaves,
        pattern_refs,
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
    // In `LocalId` order: the locals a binding's initializer may evaluate
    // to are declared before it, so their representation is known.
    let mut reprs: Vec<Option<StringRepr>> = vec![None; locals.len()];
    for (index, local) in locals.iter().enumerate() {
        if local.ty != Ty::String {
            continue;
        }
        reprs[index] = Some(if index < param_count {
            if local.mutable {
                StringRepr::MutOwned
            } else {
                StringRepr::Str
            }
        } else if !local.place.borrowed {
            if owned[index] {
                StringRepr::Owned
            } else {
                StringRepr::Str
            }
        } else if local.place.mutable {
            // It may change what it refers to, which a `&str` cannot.
            StringRepr::MutOwned
        } else if pattern_refs[index] {
            // A `String` inside the value a `match` or a `for` looks at
            // (spec 3.5).
            StringRepr::RefOwned
        } else {
            // A reference, except that one to a non-`mut` parameter's
            // `&str`, or one whose initializer may evaluate to a `&str`,
            // is a `&str` itself, so every branch has one type. Without an
            // origin it is an element of a temporary `Vec`, a `String`
            // borrowed as `&v()[i]` so that the temporary lives on.
            let to_str = match local.place.origin {
                Some(Origin::Param(param)) => !locals[param.0 as usize].mutable,
                Some(Origin::Struct(_) | Origin::Local(_)) | None => false,
            };
            let str_leaf = str_leaves[index].as_ref().is_some_and(|(literal, ids)| {
                *literal
                    || ids
                        .iter()
                        .any(|id| reprs[id.0 as usize] == Some(StringRepr::Str))
            });
            if to_str || str_leaf {
                StringRepr::Str
            } else {
                StringRepr::RefOwned
            }
        });
    }

    for (local, span, message) in gone {
        if !reported[local.0 as usize] {
            diagnostics.push(Diagnostic::new(codes::V0304, span, message).with_note(GONE_NOTE));
        }
    }
    // A `String` copied in is moved, not borrowed; a `String` assigned a
    // borrowed place is reported where that happens.
    for (source, local, span, message) in gone_copies {
        if !owned[source.0 as usize] && !reported[local.0 as usize] {
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
        let diagnostic = Diagnostic::new(codes::V0304, span, message)
            .with_note(why)
            .with_note(what_to_do(cx, locals, origin))
            .with_note(BORROWED_NOTE);
        diagnostics.push(clone_fix_it(diagnostic, span, &Ty::String));
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
    /// Some leaf is new text: a `string` call result or a `format!`.
    owned_call: bool,
    /// Free `let`s among the leaves: owned ones make the binding need to be
    /// owned too.
    free: Vec<LocalId>,
}

/// The string value flows of one function.
struct Flows<'a> {
    cx: &'a Context<'a>,
    refers: &'a Refers,
    assigned: &'a Assigned,
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
    /// A free `let` (the first) copied into another free `let` (the
    /// second) that outlives what its text may refer to, with its span and
    /// message: an error unless the first is an owned `String`.
    gone_copies: Vec<(LocalId, LocalId, Span, String)>,
    mixed: Vec<Mixed>,
    /// Per borrowed-place `string` `let`: whether its initializer may
    /// evaluate to a literal, and the locals it may evaluate to, which
    /// decide whether it is a `&str`.
    str_leaves: Vec<Option<(bool, Vec<LocalId>)>>,
    /// Per local: a `string` a `match` pattern or a `for` binds on a
    /// place, a reference to the `String` inside what is matched on or
    /// looped over.
    pattern_refs: Vec<bool>,
    /// The scopes enclosing the statement being visited, outermost first:
    /// blocks, and the `match` arms and `for` statements whose bindings
    /// live only inside them.
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
            HirStmt::For {
                local,
                head,
                body,
                span,
            } => {
                match head {
                    HirForHead::Range { start, end } => {
                        self.expr(start);
                        self.expr(end);
                    }
                    HirForHead::Vec(vec) => self.expr(vec),
                }
                self.blocks.push(*span);
                self.binding(*local);
                self.block(body);
                self.blocks.pop();
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
                let (_, modes, keeps) = self.cx.callee(*callee);
                self.call(args.iter(), &modes, keeps);
            }
            HirExprKind::MethodCall {
                receiver,
                method,
                args,
            } => {
                let (_, modes, keeps) = self.cx.method(*method);
                self.call(std::iter::once(&**receiver).chain(args), &modes, keeps);
            }
            HirExprKind::Field { base, .. } => self.expr(base),
            HirExprKind::Index { base, index } => {
                self.expr(base);
                self.expr(index);
            }
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
            HirExprKind::Println { args, .. } | HirExprKind::Format { args, .. } => {
                for arg in args {
                    self.expr(arg);
                }
            }
            HirExprKind::Try(operand) => {
                self.expr(operand);
                self.owned_slot(operand);
            }
            HirExprKind::EnumLit { args, .. } | HirExprKind::VecLit(args) => {
                for arg in args {
                    self.expr(arg);
                    self.owned_slot(arg);
                }
            }
            HirExprKind::Match { scrutinee, arms } => {
                self.expr(scrutinee);
                for arm in arms {
                    self.blocks.push(arm.span);
                    for (local, _) in arm.pattern.bindings() {
                        self.binding(local);
                    }
                    self.expr(&arm.body);
                    self.blocks.pop();
                }
            }
        }
    }

    /// `local`, bound by a `match` pattern or a `for`: a `string` one
    /// refers into a place matched on or looped over, and owns its text
    /// moved out of a temporary (spec 3.5).
    fn binding(&mut self, local: LocalId) {
        let index = local.0 as usize;
        if self.locals[index].ty != Ty::String {
            return;
        }
        if self.locals[index].place.borrowed {
            self.pattern_refs[index] = true;
        } else {
            self.owned[index] = true;
        }
    }

    /// The arguments of a call: one lent to a `mut` parameter, or kept by
    /// an `Owned` one when `keeps`, is an owned slot.
    fn call<'e>(
        &mut self,
        args: impl Iterator<Item = &'e HirExpr>,
        modes: &[ParamMode],
        keeps: bool,
    ) {
        for (arg, mode) in args.zip(modes) {
            self.expr(arg);
            match mode {
                ParamMode::MutableBorrow => self.owned_slot(arg),
                ParamMode::Owned if keeps => self.owned_slot(arg),
                ParamMode::Owned | ParamMode::SharedBorrow => {}
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
        let leaves_of_value = leaves(value);
        let literal = leaves_of_value
            .iter()
            .any(|leaf| matches!(leaf.kind, HirExprKind::String(_)));
        let ids = leaves_of_value
            .iter()
            .filter_map(|leaf| match leaf.kind {
                HirExprKind::Local(id) => Some(id),
                _ => None,
            })
            .collect();
        self.str_leaves[local.0 as usize] = Some((literal, ids));
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
            } else if is_new_text(leaf) {
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
                if let Some(root) = self.gone_copy(id, local) {
                    let message = format!(
                        "`{}` holds text from `{}`, which only exists inside this block, so \
                         it cannot be kept in `{}`",
                        self.locals[id.0 as usize].name,
                        self.locals[root.0 as usize].name,
                        self.locals[local.0 as usize].name,
                    );
                    self.gone_copies.push((id, local, leaf.span, message));
                }
            } else if is_new_text(leaf) {
                // A call result or a `format!` is a new `String` (spec 3.5).
                self.owned[local.0 as usize] = true;
            } else if let Some(place) = place_info(self.locals, leaf) {
                if place.borrowed && (self.gone(leaf, value) || self.gone_at_block_end(leaf, local))
                {
                    let name = &self.locals[local.0 as usize].name;
                    let consequence = format!("it cannot be kept in `{name}`");
                    let message = gone_message(self.cx, self.locals, leaf, value, &consequence);
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
        self.root_gone(id, local)
    }

    /// Whether `id` is declared in an enclosing scope that `local` is not
    /// declared in, and owns its value there (or refers to something gone
    /// there).
    fn root_gone(&self, id: LocalId, local: LocalId) -> bool {
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

    /// A local rooting the borrowed places `source`, a free `let`, may hold
    /// (directly or through the free `let`s copied into it) that is gone
    /// while `local`, which `source` is copied into, lives on.
    fn gone_copy(&self, source: LocalId, local: LocalId) -> Option<LocalId> {
        let mut seen = vec![source];
        let mut index = 0;
        while let Some(&at) = seen.get(index) {
            index += 1;
            let at = at.0 as usize;
            if let Some(&root) = self.assigned.roots[at]
                .iter()
                .find(|&&root| self.root_gone(root, local))
            {
                return Some(root);
            }
            for &copied in &self.assigned.copies[at] {
                push_unique(&mut seen, copied);
            }
        }
        None
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
            _ if is_block_like(base) => leaves(base)
                .into_iter()
                .any(|inner| self.gone(inner, whole)),
            _ => true,
        }
    }
}

/// Whether `leaf`, a `string` value, is new text: a call or method call
/// result (a `clone` included), a `format!` (spec 3.5), or the value of a
/// `?`, taken out of an owned `Result`.
fn is_new_text(leaf: &HirExpr) -> bool {
    matches!(
        leaf.kind,
        HirExprKind::Call { .. }
            | HirExprKind::MethodCall { .. }
            | HirExprKind::Format { .. }
            | HirExprKind::Try(_)
    )
}
