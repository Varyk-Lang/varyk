//! Borrow analysis (spec section 4.2), part 1: places and mutability
//! contracts.
//!
//! [`analyze`] fills in every local's [`PlaceInfo`] (is it a borrowed
//! place, is it a mutable place, and what does it borrow from), then
//! enforces the mutability contracts (V0300-V0303) and rejects borrowed
//! places flowing into owned slots (V0304): struct-literal fields,
//! assignments through a borrowed place or to a whole struct local, return
//! values, and imported parameters taken by value. It also rejects (V0304)
//! an `if` or block read in place whose values mix new ones with places,
//! and values that would be referred to after their block ends (see
//! `Leaf`). Two more passes follow per function:
//! `strings` infers each `string` local's representation (spec 4.3) and
//! rejects borrowed places flowing into a `let` that must be owned, and
//! `moves` rejects uses after a move (V0305).
//!
//! Places are locals and field paths rooted at one; only locals store
//! their [`PlaceInfo`]; a field path's info derives from its root local
//! plus the field's type (see [`place_info`]). Every other expression
//! (call results, struct literals, literals, operators) is a temporary: a
//! mutable place that is not borrowed.

use std::collections::HashMap;

use varyk_syntax::{FixIt, Span};

use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{
    HirBlock, HirExpr, HirExprKind, HirFunction, HirModuleKind, HirProgram, HirStmt, HirStruct,
    LocalId, LocalInfo, Origin, PlaceInfo, declared_inside, field_root, is_block_like, leaves,
};
use crate::resolve::{Callee, FnId, ImportedSig, StructId};
use crate::types::{ParamMode, Ty};

/// The note every V0304 carries: the plan, then the Rust-facing reason.
const MILESTONE_2_NOTE: &str = "milestone 2 will allow this; in Rust terms, a borrowed value \
                                cannot move into a place that owns it";

/// The note of a V0304 for a value gone before it is used.
const GONE_NOTE: &str = "in Rust terms, a reference cannot outlive the value it borrows";

/// A temporary: nobody else can observe it, so it is a mutable place.
const TEMPORARY: PlaceInfo = PlaceInfo {
    borrowed: false,
    mutable: true,
    origin: None,
};

/// Annotates every function's locals with their [`PlaceInfo`] and, for
/// `string` locals, their [`crate::hir::StringRepr`]; checks the
/// mutability contracts, owned slots, and moves, returning every
/// diagnostic across the program.
pub fn analyze(mut hir: HirProgram) -> Result<HirProgram, Vec<Diagnostic>> {
    let signatures: HashMap<FnId, (String, Vec<ParamMode>)> = hir
        .functions()
        .map(|f| {
            (
                f.id,
                (f.name.clone(), f.params.iter().map(|p| p.mode).collect()),
            )
        })
        .collect();
    let cx = Context {
        signatures: &signatures,
        imported: &hir.imported,
        structs: &hir.structs,
    };
    let mut diagnostics = Vec::new();
    for module in &mut hir.modules {
        if let HirModuleKind::Varyk { functions } = &mut module.kind {
            for function in functions {
                analyze_function(&cx, function, &mut diagnostics);
            }
        }
    }
    if diagnostics.is_empty() {
        Ok(hir)
    } else {
        Err(diagnostics)
    }
}

/// The [`PlaceInfo`] of a place expression (a `Local` or a `Field` chain),
/// read from `locals` as [`analyze`] annotated them; `None` for any other
/// expression, which is a temporary.
///
/// A field is a borrowed place exactly when its type is not Copy (whatever
/// its base: milestone 1 has no partial moves), borrowing from the struct
/// it belongs to, and is mutable exactly when its base is: for an `if` or
/// block base, when every value it may evaluate to is.
pub fn place_info(locals: &[LocalInfo], expr: &HirExpr) -> Option<PlaceInfo> {
    match &expr.kind {
        HirExprKind::Local(id) => Some(locals[id.0 as usize].place),
        HirExprKind::Field { base, .. } => {
            let mutable = if is_block_like(base) {
                leaves(base)
                    .into_iter()
                    .all(|leaf| place_info(locals, leaf).unwrap_or(TEMPORARY).mutable)
            } else {
                place_info(locals, base).unwrap_or(TEMPORARY).mutable
            };
            let borrowed = !expr.ty.is_copy();
            let origin = match base.ty {
                Ty::Struct(id) if borrowed => Some(Origin::Struct(id)),
                _ => None,
            };
            Some(PlaceInfo {
                borrowed,
                mutable,
                origin,
            })
        }
        _ => None,
    }
}

/// Program-wide facts a function's analysis reads.
struct Context<'a> {
    /// Every Varyk function's name and parameter modes.
    signatures: &'a HashMap<FnId, (String, Vec<ParamMode>)>,
    imported: &'a [ImportedSig],
    structs: &'a [HirStruct],
}

impl Context<'_> {
    /// The callee's name, its parameter modes, and whether it is imported.
    fn callee(&self, callee: Callee) -> (String, Vec<ParamMode>, bool) {
        match callee {
            Callee::Varyk(id) => {
                let (name, modes) = &self.signatures[&id];
                (name.clone(), modes.clone(), false)
            }
            Callee::Imported(id) => {
                let sig = &self.imported[id.0 as usize];
                let modes = sig.params.iter().map(|(_, mode)| *mode).collect();
                (sig.name.clone(), modes, true)
            }
        }
    }

    fn struct_name(&self, id: StructId) -> &str {
        &self.structs[id.0 as usize].name
    }
}

/// Says, in plain words, who a borrowed place belongs to, for V0304: the
/// first clause of its message.
fn owner_text(cx: &Context, locals: &[LocalInfo], origin: Option<Origin>) -> String {
    match origin {
        Some(Origin::Param(param)) => format!(
            "`{}` belongs to the caller of this function",
            locals[param.0 as usize].name
        ),
        Some(Origin::Struct(id)) => {
            format!("this value is kept inside a `{}`", cx.struct_name(id))
        }
        None => "this value belongs to someone else".to_string(),
    }
}

/// The local a place expression is rooted at; `None` for a temporary or a
/// field of one, and for a field of an `if` or block.
fn place_root(expr: &HirExpr) -> Option<LocalId> {
    match &expr.kind {
        HirExprKind::Local(id) => Some(*id),
        HirExprKind::Field { base, .. } => place_root(base),
        _ => None,
    }
}

/// Every local `expr` may borrow from: its root, or, through `if`s and
/// blocks (as a value or as the base of a field), the roots of every value
/// it may evaluate to.
fn roots(expr: &HirExpr) -> Vec<LocalId> {
    let base = field_root(expr);
    match &base.kind {
        HirExprKind::Local(id) => vec![*id],
        HirExprKind::If { .. } | HirExprKind::Block(_) => {
            leaves(base).into_iter().flat_map(roots).collect()
        }
        _ => Vec::new(),
    }
}

/// A V0304 message saying, in plain words, why `leaf` (a value inside an
/// `if` or block, or a field of a temporary) is gone once that `if` or
/// block ends, then `consequence`, then how to fix it when there is a
/// simple fix.
fn gone_message(cx: &Context, locals: &[LocalInfo], leaf: &HirExpr, consequence: &str) -> String {
    if let Some(root) = place_root(leaf) {
        return format!(
            "`{}` only exists inside this block, so {consequence}",
            locals[root.0 as usize].name
        );
    }
    if let HirExprKind::Field { base, .. } = &leaf.kind {
        if let (Ty::Struct(id), false) = (base.ty, is_block_like(field_root(base))) {
            let name = cx.struct_name(id);
            return format!(
                "this value is kept inside a `{name}` that is not stored anywhere, so \
                 {consequence}; store the `{name}` with `let` first"
            );
        }
    }
    format!("this value only exists inside this block, so {consequence}")
}

/// What each `let` may refer to, as (some value of its initializer is a
/// temporary, the locals it is rooted at); see [`dangling`].
type Refers = Vec<(bool, Vec<LocalId>)>;

/// Whether the binding `local`, a reference declared inside `whole`,
/// refers to something gone once `whole` ends: a temporary (whose life
/// Rust extends only to the end of the binding's block) or a local
/// declared inside `whole` that owns its value.
fn dangling(locals: &[LocalInfo], refers: &Refers, local: LocalId, within: Span) -> bool {
    let (temporary, roots) = &refers[local.0 as usize];
    *temporary
        || roots.iter().any(|&root| {
            let info = &locals[root.0 as usize];
            declared_inside(info, within)
                && (!info.place.borrowed || dangling(locals, refers, root, within))
        })
}

/// How a value an `if` or block may evaluate to (a leaf, see [`leaves`])
/// relates to that whole `if` or block when it is read in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Leaf {
    /// Made while the whole is evaluated and gone after it: a call
    /// result, a struct literal, a literal, a field of a temporary, or a
    /// local declared inside the whole that owns its value (or a field of
    /// one). It can only be borrowed after the whole is complete.
    New,
    /// Outlives the whole: a string literal, a place declared outside it,
    /// or a binding inside it of such a place.
    Lasting,
    /// A `string` `let` declared inside the whole: whether it is new text
    /// or a `&str` to lasting text is decided later, by the `strings` pass.
    Unsure,
    /// A binding inside the whole that refers to something gone once the
    /// whole ends.
    Dangling,
}

/// Where a value flows that must own it.
enum Slot {
    Return,
    StructField {
        id: StructId,
        field: usize,
    },
    ImportedParam(String),
    Assignment,
    /// A whole struct local that is not a borrowed place.
    Local(String),
}

fn analyze_function(cx: &Context, function: &mut HirFunction, diagnostics: &mut Vec<Diagnostic>) {
    let (reported, refers) = places(cx, function, diagnostics);
    strings::infer(cx, function, &reported, &refers, diagnostics);
    moves::check(cx, function, diagnostics);
}

/// The first pass: place info, mutability contracts, and owned slots.
/// Returns, per local, whether a V0304 was reported for a flow of it, and
/// what it may refer to.
fn places(
    cx: &Context,
    function: &mut HirFunction,
    diagnostics: &mut Vec<Diagnostic>,
) -> (Vec<bool>, Refers) {
    let HirFunction {
        name,
        params,
        ret,
        body,
        locals,
        ..
    } = function;
    let mut blame = vec![None; locals.len()];
    let reported = vec![false; locals.len()];
    let refers = vec![(false, Vec::new()); locals.len()];
    for param in params.iter() {
        let index = param.local.0 as usize;
        let local = &mut locals[index];
        let borrowed = !local.ty.is_copy();
        local.place = PlaceInfo {
            borrowed,
            mutable: local.mutable,
            origin: borrowed.then_some(Origin::Param(param.local)),
        };
        if !local.mutable {
            blame[index] = Some(param.local);
        }
    }
    let mut analyzer = FnAnalyzer {
        cx,
        name,
        param_count: params.len(),
        ret: *ret,
        locals,
        blame,
        reported,
        refers,
        diagnostics,
    };
    analyzer.block(body);
    if *ret != Ty::Unit {
        if let Some(tail) = &body.tail {
            analyzer.owned_slot(tail, &Slot::Return);
        }
    }
    (analyzer.reported, analyzer.refers)
}

struct FnAnalyzer<'a> {
    cx: &'a Context<'a>,
    /// The function's own name, for V0303's contract note.
    name: &'a str,
    param_count: usize,
    ret: Ty,
    locals: &'a mut Vec<LocalInfo>,
    /// Per local: the parameter or `let` whose missing `mut` makes it not
    /// a mutable place; `None` for a mutable place.
    blame: Vec<Option<LocalId>>,
    /// Per local: a V0304 was reported for a flow of it (so the owned-`let`
    /// check in [`strings`] stays quiet about it).
    reported: Vec<bool>,
    /// Per `let`: what its initializer may refer to, as (some value is a
    /// temporary, the locals it is rooted at). Read for bindings that are
    /// borrowed places, to tell whether they outlive their block.
    refers: Refers,
    diagnostics: &'a mut Vec<Diagnostic>,
}

impl FnAnalyzer<'_> {
    fn block(&mut self, block: &HirBlock) {
        for stmt in &block.stmts {
            self.stmt(stmt);
        }
        if let Some(tail) = &block.tail {
            self.expr(tail);
        }
    }

    fn stmt(&mut self, stmt: &HirStmt) {
        match stmt {
            HirStmt::Let { local, value, .. } => {
                self.expr(value);
                self.bind(*local, value);
            }
            HirStmt::Assign { target, value, .. } => {
                self.expr(value);
                self.assign(target, value);
            }
            HirStmt::Expr { expr, .. } => {
                self.expr(expr);
                self.mixed(expr, false);
            }
            HirStmt::Return { value, .. } => {
                if let Some(value) = value {
                    self.expr(value);
                    if self.ret != Ty::Unit {
                        self.owned_slot(value, &Slot::Return);
                    }
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
                for arg in args {
                    self.expr(arg);
                }
                self.call(*callee, args);
            }
            HirExprKind::Field { base, .. } => {
                self.expr(base);
                // A field access reads its base in place.
                self.mixed(base, false);
            }
            HirExprKind::StructLit { id, fields } => {
                for (field, value) in fields {
                    self.expr(value);
                    self.owned_slot(
                        value,
                        &Slot::StructField {
                            id: *id,
                            field: *field,
                        },
                    );
                }
            }
            HirExprKind::Unary { operand, .. } => self.expr(operand),
            HirExprKind::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
                let reported = self.mixed(lhs, false) | self.mixed(rhs, false);
                // String operands are both borrowed for the comparison.
                if !reported && lhs.ty == Ty::String {
                    self.used_while_lent(&[lhs, rhs], &[Some(false), Some(false)], false);
                }
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
                let mut reported = false;
                for arg in args {
                    self.expr(arg);
                    reported |= self.mixed(arg, false);
                }
                // Every argument is a shared read of its roots; V0306 when
                // a later one changes or gives away a root an earlier one
                // reads, same as a multi-argument call (spec 4.2).
                if !reported {
                    let values: Vec<&HirExpr> = args.iter().collect();
                    let lent = vec![Some(false); args.len()];
                    // The format machinery borrows every argument, Copy or
                    // not, for the whole call (spec 4.2): unlike an
                    // ordinary call, a Copy argument here is not passed by
                    // value.
                    self.used_while_lent(&values, &lent, true);
                }
            }
        }
    }

    // --- Places ---------------------------------------------------------------

    fn info(&self, expr: &HirExpr) -> PlaceInfo {
        place_info(self.locals, expr).unwrap_or(TEMPORARY)
    }

    /// Who is at fault that `expr` is not a mutable place.
    fn blame(&self, expr: &HirExpr) -> Option<LocalId> {
        place_root(expr).and_then(|root| self.blame[root.0 as usize])
    }

    fn is_param(&self, local: LocalId) -> bool {
        (local.0 as usize) < self.param_count
    }

    fn local(&self, local: LocalId) -> &LocalInfo {
        &self.locals[local.0 as usize]
    }

    /// Computes the place info of `let local = value`.
    fn bind(&mut self, local: LocalId, value: &HirExpr) {
        let declared_mut = self.local(local).mutable;
        let mut borrowed = None;
        let mut shared = None;
        self.refers[local.0 as usize] = (
            leaves(value).into_iter().any(|leaf| {
                !matches!(leaf.kind, HirExprKind::String(_)) && place_root(leaf).is_none()
            }),
            roots(value),
        );
        for leaf in leaves(value) {
            let info = self.info(leaf);
            if info.borrowed && borrowed.is_none() {
                borrowed = Some(info.origin);
            }
            if !info.mutable && shared.is_none() {
                shared = Some(self.blame(leaf));
            }
        }
        let (place, blame) = match borrowed {
            // A binding of a borrowed place inherits its kind: a shared
            // source makes it shared whatever `mut` the `let` has.
            Some(origin) => {
                let mutable = declared_mut && shared.is_none();
                let blame = match shared {
                    Some(source) => source.or(Some(local)),
                    None if mutable => None,
                    None => Some(local),
                };
                (
                    PlaceInfo {
                        borrowed: true,
                        mutable,
                        origin,
                    },
                    blame,
                )
            }
            None => (
                PlaceInfo {
                    borrowed: false,
                    mutable: declared_mut,
                    origin: None,
                },
                (!declared_mut).then_some(local),
            ),
        };
        self.locals[local.0 as usize].place = place;
        self.blame[local.0 as usize] = blame;
        if place.borrowed && is_block_like(value) {
            self.kept_by_reference(local, value);
        }
    }

    /// V0304 for `let local = value`, a binding that is a reference, when
    /// `value` is an `if` or block that may evaluate to something gone
    /// once it ends: a local declared inside it (or a field of one). Rust
    /// extends the life of a temporary borrowed there, so a call result, a
    /// struct literal, or a field of one lives on with the binding, except
    /// for a field of one in a `string` binding that some other branch
    /// makes a `&str`, which borrows it through `.as_str()` instead.
    fn kept_by_reference(&mut self, local: LocalId, value: &HirExpr) {
        let leaves = leaves(value);
        // A field or a `mut string` parameter is a reference to a `String`.
        let as_str = value.ty == Ty::String
            && leaves.iter().any(|leaf| match leaf.kind {
                HirExprKind::Field { .. } => false,
                HirExprKind::Local(id) => !(self.is_param(id) && self.local(id).mutable),
                _ => true,
            });
        for leaf in leaves {
            let gone = match self.leaf(leaf, value, false) {
                Leaf::Lasting => false,
                Leaf::New => match leaf.kind {
                    HirExprKind::Local(_) => true,
                    HirExprKind::Field { .. } => place_root(leaf).is_some() || as_str,
                    _ => false,
                },
                Leaf::Unsure | Leaf::Dangling => true,
            };
            if gone {
                let consequence = format!("it cannot be kept in `{}`", self.local(local).name);
                let message = gone_message(self.cx, self.locals, leaf, &consequence);
                self.diagnostics
                    .push(Diagnostic::new(codes::V0304, leaf.span, message).with_note(GONE_NOTE));
                self.reported[local.0 as usize] = true;
            }
        }
    }

    // --- Checks ---------------------------------------------------------------

    fn assign(&mut self, target: &HirExpr, value: &HirExpr) {
        let info = self.info(target);
        if !info.mutable {
            let root = place_root(target).expect("an assignment target is a place");
            let blamed = self.blame(target).unwrap_or(root);
            let diagnostic = if self.is_param(blamed) {
                let param = &self.local(blamed).name;
                Diagnostic::new(
                    codes::V0300,
                    target.span,
                    format!(
                        "this function only reads `{param}`; to change it, add `mut` to the parameter"
                    ),
                )
            } else {
                let binding = &self.local(blamed).name;
                Diagnostic::new(
                    codes::V0301,
                    target.span,
                    format!("`{binding}` cannot be changed because it was declared without `mut`"),
                )
            };
            let diagnostic = self.add_mut_fix_it(diagnostic, root, blamed);
            self.diagnostics.push(diagnostic);
        } else if info.borrowed {
            // Writing through a borrowed place stores into storage the
            // caller or a struct owns.
            self.owned_slot(value, &Slot::Assignment);
        } else if let HirExprKind::Local(id) = target.kind {
            // A whole struct local that is not a borrowed place owns its
            // value. (A `string` one may be `&str`; the `strings` pass
            // decides.)
            if matches!(target.ty, Ty::Struct(_)) {
                let name = self.local(id).name.clone();
                self.owned_slot(value, &Slot::Local(name));
            }
        }
    }

    fn call(&mut self, callee: Callee, args: &[HirExpr]) {
        let (name, modes, imported) = self.cx.callee(callee);
        let reported = self.diagnostics.len();
        for (arg, mode) in args.iter().zip(modes.iter().copied()) {
            match mode {
                ParamMode::MutableBorrow => {
                    if !self.mixed(arg, true) {
                        self.mut_argument(arg, &name);
                    }
                }
                ParamMode::SharedBorrow => {
                    self.mixed(arg, false);
                }
                // A Varyk-declared `Owned` parameter is always Copy.
                ParamMode::Owned if imported => {
                    self.owned_slot(arg, &Slot::ImportedParam(name.clone()))
                }
                ParamMode::Owned => {}
            }
        }
        if self.diagnostics.len() == reported {
            self.overlapping_borrows(args, &modes);
        }
        if self.diagnostics.len() == reported {
            let values: Vec<&HirExpr> = args.iter().collect();
            let lent: Vec<Option<bool>> = modes
                .iter()
                .map(|mode| match mode {
                    ParamMode::MutableBorrow => Some(true),
                    ParamMode::SharedBorrow => Some(false),
                    ParamMode::Owned => None,
                })
                .collect();
            self.used_while_lent(&values, &lent, false);
        }
    }

    /// V0306: a value (an argument, or an operand of a string comparison)
    /// that, while being evaluated, changes or gives away a local an
    /// earlier one still borrows: in `f(p, { change(p); q })` or
    /// `s == { let t = s; t }` the earlier borrow lasts until the whole
    /// call or comparison is made. `lent[i]` is `Some(mutable)` when
    /// `values[i]` is borrowed, `None` when it is copied or moved (a move
    /// followed by a use is V0305). Uses by the later value's own result
    /// are the overlap check's business; this one looks at what it does on
    /// the way (statements, nested calls, conditions). It is conservative:
    /// an assignment or a struct field counts as giving the value away
    /// even where Rust would only borrow it, and so does a `let` unless it
    /// only gives the value another name.
    ///
    /// `include_copy` skips the usual exemption for a Copy value: an
    /// ordinary call passes a Copy argument by value, so nothing of the
    /// caller's stays borrowed once the argument is evaluated (a Copy read
    /// beside a mutable borrow of the same local is instead caught by
    /// [`Self::overlapping_borrows`]). `println!` is different: its format
    /// machinery borrows every argument, Copy or not, for the whole call
    /// (spec 4.2), so a later argument that changes or gives away a Copy
    /// argument's root is still V0306.
    fn used_while_lent(&mut self, values: &[&HirExpr], lent: &[Option<bool>], include_copy: bool) {
        let mut held: Vec<(LocalId, bool, Span)> = Vec::new();
        for (value, lent) in values.iter().zip(lent) {
            let mut uses = Vec::new();
            self.uses_on_the_way(value, true, false, &mut uses);
            for (local, exclusive, span) in uses {
                let Some(&(_, _, first)) = held
                    .iter()
                    .find(|(other, mutable, _)| *other == local && (exclusive || *mutable))
                else {
                    continue;
                };
                let name = &self.local(local).name;
                let message = format!(
                    "`{name}` is used here while it is still lent out earlier in the same expression"
                );
                let diagnostic = Diagnostic::new(codes::V0306, span, message)
                    .with_label(first, format!("`{name}` is lent out here"))
                    .with_note(
                        "in Rust terms, the earlier borrow lasts until the whole call or comparison \
                         is made; store the value with `let` first",
                    );
                self.diagnostics.push(diagnostic);
                return;
            }
            if let Some(mutable) = *lent {
                if include_copy || !value.ty.is_copy() {
                    for root in roots(value) {
                        held.push((root, mutable, value.span));
                    }
                }
            }
        }
    }

    /// Collects into `out` every local `expr` uses other than as the value
    /// it evaluates to (when `at_leaf`), with whether that use changes or
    /// gives it away (`exclusive`: lent to a `mut` parameter, moved,
    /// assigned, or kept).
    fn uses_on_the_way(
        &self,
        expr: &HirExpr,
        at_leaf: bool,
        exclusive: bool,
        out: &mut Vec<(LocalId, bool, Span)>,
    ) {
        match &expr.kind {
            HirExprKind::Int(_)
            | HirExprKind::Float(_)
            | HirExprKind::Bool(_)
            | HirExprKind::String(_) => {}
            HirExprKind::Local(id) => {
                if !at_leaf {
                    out.push((*id, exclusive, expr.span));
                }
            }
            HirExprKind::Field { .. } => {
                let base = field_root(expr);
                match &base.kind {
                    HirExprKind::Local(id) => {
                        if !at_leaf {
                            out.push((*id, exclusive, expr.span));
                        }
                    }
                    _ => self.uses_on_the_way(base, at_leaf, exclusive, out),
                }
            }
            HirExprKind::Call { callee, args } => {
                let (_, modes, imported) = self.cx.callee(*callee);
                for (arg, mode) in args.iter().zip(modes) {
                    let exclusive = match mode {
                        ParamMode::MutableBorrow => true,
                        ParamMode::Owned => imported && !arg.ty.is_copy(),
                        ParamMode::SharedBorrow => false,
                    };
                    self.uses_on_the_way(arg, false, exclusive, out);
                }
            }
            HirExprKind::StructLit { fields, .. } => {
                for (_, value) in fields {
                    self.uses_on_the_way(value, false, !value.ty.is_copy(), out);
                }
            }
            HirExprKind::Unary { operand, .. } => self.uses_on_the_way(operand, false, false, out),
            HirExprKind::Binary { lhs, rhs, .. } => {
                self.uses_on_the_way(lhs, false, false, out);
                self.uses_on_the_way(rhs, false, false, out);
            }
            HirExprKind::Println { args, .. } => {
                for arg in args {
                    self.uses_on_the_way(arg, false, false, out);
                }
            }
            HirExprKind::Block(block) => self.block_uses(block, at_leaf, exclusive, out),
            HirExprKind::If { cond, then, else_ } => {
                self.uses_on_the_way(cond, false, false, out);
                self.block_uses(then, at_leaf, exclusive, out);
                if let Some(else_) = else_ {
                    self.block_uses(else_, at_leaf, exclusive, out);
                }
            }
        }
    }

    /// [`FnAnalyzer::uses_on_the_way`] for a block: every statement, then
    /// its tail.
    fn block_uses(
        &self,
        block: &HirBlock,
        at_leaf: bool,
        exclusive: bool,
        out: &mut Vec<(LocalId, bool, Span)>,
    ) {
        for stmt in &block.stmts {
            match stmt {
                // A `let` of a borrowed place is another name for it, not
                // a move: exclusive only when it is a `mut` reference.
                HirStmt::Let { local, value, .. } => {
                    let place = self.local(*local).place;
                    let exclusive = !value.ty.is_copy() && (!place.borrowed || place.mutable);
                    self.uses_on_the_way(value, false, exclusive, out)
                }
                HirStmt::Assign { target, value, .. } => {
                    self.uses_on_the_way(target, false, true, out);
                    self.uses_on_the_way(value, false, !value.ty.is_copy(), out);
                }
                HirStmt::Expr { expr, .. } => self.uses_on_the_way(expr, false, false, out),
                HirStmt::Return { value, .. } => {
                    if let Some(value) = value {
                        self.uses_on_the_way(value, false, !value.ty.is_copy(), out);
                    }
                }
                HirStmt::While { cond, body, .. } => {
                    self.uses_on_the_way(cond, false, false, out);
                    self.block_uses(body, false, false, out);
                }
                HirStmt::Break { .. } | HirStmt::Continue { .. } => {}
            }
        }
        if let Some(tail) = &block.tail {
            self.uses_on_the_way(tail, at_leaf, exclusive, out);
        }
    }

    /// V0306: one local (or a field of it) passed by reference twice to the
    /// same call, once to a `mut` parameter; Rust cannot hand out a `&mut`
    /// alongside another borrow of the same value. A block-like argument
    /// (an `if` or a block), or a field of one, has no single root, so
    /// every root among its leaves (see [`roots`]) is a candidate. An
    /// `Owned` non-`Copy` argument is exclusive too: the emitted call moves
    /// (or borrows then moves) it, so it cannot coexist with any other pass
    /// of the same value. A `Copy` argument is a plain read: it does not
    /// conflict with another `Copy` read or a shared borrow of the same
    /// root, but the emitted call still hands out a `&mut` alongside it
    /// when another argument borrows that root mutably, so it conflicts
    /// with (and is conflicted with, in either order) a `MutableBorrow` or
    /// non-`Copy` `Owned` argument of the same root.
    fn overlapping_borrows(&mut self, args: &[HirExpr], modes: &[ParamMode]) {
        let mut seen: Vec<(LocalId, bool, Span)> = Vec::new();
        for (arg, mode) in args.iter().zip(modes) {
            let mutable = match mode {
                ParamMode::MutableBorrow => true,
                ParamMode::SharedBorrow => false,
                ParamMode::Owned if arg.ty.is_copy() => false,
                ParamMode::Owned => true,
            };
            let roots = roots(arg);
            if roots.is_empty() {
                continue;
            }
            let earlier = roots.iter().find_map(|root| {
                seen.iter()
                    .find(|(other, other_mut, _)| other == root && (mutable || *other_mut))
            });
            if let Some(&(root, _, first)) = earlier {
                let name = &self.local(root).name;
                let message = format!(
                    "`{name}` is passed to this call more than once, and one of those may change it"
                );
                let diagnostic = Diagnostic::new(codes::V0306, arg.span, message)
                    .with_label(first, format!("`{name}` is first passed here"));
                self.diagnostics.push(diagnostic);
                return;
            }
            for root in roots {
                seen.push((root, mutable, arg.span));
            }
        }
    }

    /// How `leaf`, a value the `if` or block `whole` may evaluate to,
    /// relates to `whole` (see [`Leaf`]); `mutable` when `whole` is read
    /// through a mutable borrow, which turns a string literal into new
    /// text.
    fn leaf(&self, leaf: &HirExpr, whole: &HirExpr, mutable: bool) -> Leaf {
        let base = field_root(leaf);
        match &base.kind {
            HirExprKind::String(_) if mutable => Leaf::New,
            HirExprKind::String(_) => Leaf::Lasting,
            HirExprKind::Local(id) => {
                let info = self.local(*id);
                if !declared_inside(info, whole.span) {
                    Leaf::Lasting
                } else if info.place.borrowed {
                    if dangling(self.locals, &self.refers, *id, whole.span) {
                        Leaf::Dangling
                    } else {
                        Leaf::Lasting
                    }
                } else if info.ty == Ty::String && !mutable {
                    Leaf::Unsure
                } else {
                    Leaf::New
                }
            }
            // A field of an `if` or block: lasting when its base is, gone
            // with it when its base is new (the base is borrowed where it
            // is read, which is inside `whole`). A mixed base is reported
            // where the field is read.
            HirExprKind::If { .. } | HirExprKind::Block(_) => {
                let kinds: Vec<Leaf> = leaves(base)
                    .into_iter()
                    .map(|inner| self.leaf(inner, whole, mutable))
                    .collect();
                if kinds.contains(&Leaf::Dangling)
                    || !kinds.is_empty() && kinds.iter().all(|kind| *kind == Leaf::New)
                {
                    Leaf::Dangling
                } else {
                    Leaf::Lasting
                }
            }
            _ => Leaf::New,
        }
    }

    /// An `if` or block read in place (printed, compared, discarded, lent
    /// to a parameter, or the base of a field access) is borrowed as a
    /// whole when every value it may evaluate to is new (see [`Leaf`]),
    /// and branch by branch when every one lasts. When it mixes the two,
    /// it has no single Rust form: borrowing a new value inside its branch
    /// outlives it, and making the lasting one new would copy it, which
    /// spec 4.3 forbids, so it must go through a `let` first. A binding
    /// inside it that refers to something gone once it ends is rejected
    /// outright. This applies to `string` and struct values; a Copy value
    /// is simply copied, unless it is lent to a `mut` parameter
    /// (`mutable`), which also makes a string literal new text. A literal
    /// goes with either kind. Returns whether it reported.
    fn mixed(&mut self, expr: &HirExpr, mutable: bool) -> bool {
        let checked = match expr.ty {
            Ty::String | Ty::Struct(_) => true,
            Ty::Unit => false,
            _ => mutable,
        };
        if !checked || !is_block_like(expr) {
            return false;
        }
        let leaves = leaves(expr);
        let kinds: Vec<Leaf> = leaves
            .iter()
            .map(|leaf| self.leaf(leaf, expr, mutable))
            .collect();
        if let Some(i) = kinds.iter().position(|kind| *kind == Leaf::Dangling) {
            let leaf = leaves[i];
            let message = match place_root(leaf) {
                Some(root) => format!(
                    "`{}` refers to something that only exists inside this block, so the \
                     block cannot hand it out",
                    self.local(root).name
                ),
                None => gone_message(self.cx, self.locals, leaf, "it cannot be handed out here"),
            };
            self.diagnostics
                .push(Diagnostic::new(codes::V0304, leaf.span, message).with_note(GONE_NOTE));
            return true;
        }
        let literal = |leaf: &HirExpr| matches!(leaf.kind, HirExprKind::String(_));
        let new = kinds.contains(&Leaf::New);
        let unsure = kinds.iter().filter(|kind| **kind == Leaf::Unsure).count();
        let lasting = leaves
            .iter()
            .zip(&kinds)
            .any(|(leaf, kind)| *kind == Leaf::Lasting && !literal(leaf));
        if !(new && lasting || unsure > 0 && (unsure > 1 || new || lasting)) {
            return false;
        }
        let text = expr.ty == Ty::String;
        let new = if text { "new text" } else { "new" };
        let named = leaves
            .iter()
            .zip(&kinds)
            .filter(|(_, kind)| **kind != Leaf::New)
            .find_map(|(leaf, _)| place_root(leaf).map(|root| (leaf, root)));
        let message = match named {
            Some((leaf, root)) => {
                let name = &self.local(root).name;
                let owned_let = matches!(leaf.kind, HirExprKind::Local(_))
                    && !self.is_param(root)
                    && !self.local(root).place.borrowed;
                let source = if owned_let {
                    "the value of"
                } else {
                    "borrowed from"
                };
                let mut message =
                    format!("this value is sometimes {new} and sometimes {source} `{name}`");
                // Only a whole owned `let` can go through another `string`
                // `let`; a parameter, a field, or an alias of either is
                // rejected there too. A struct `let` accepts all of them, as
                // a reference.
                if owned_let || !text {
                    message.push_str("; store it with `let` first");
                }
                message
            }
            None => format!(
                "this value is sometimes {new} and sometimes kept elsewhere; store it with `let` first"
            ),
        };
        self.diagnostics
            .push(Diagnostic::new(codes::V0304, expr.span, message).with_note(MILESTONE_2_NOTE));
        true
    }

    /// An argument to a `mut` parameter must be a mutable place; so must
    /// every value an `if` or block base of a field argument may evaluate
    /// to.
    fn mut_argument(&mut self, arg: &HirExpr, callee: &str) {
        for leaf in leaves(arg) {
            if self.info(leaf).mutable {
                continue;
            }
            let base = field_root(leaf);
            if is_block_like(base) {
                self.mut_argument(base, callee);
                continue;
            }
            let Some(root) = place_root(leaf) else {
                continue;
            };
            let blamed = self.blame(leaf).unwrap_or(root);
            let diagnostic = if self.is_param(blamed) {
                let param = &self.local(blamed).name;
                Diagnostic::new(
                    codes::V0303,
                    leaf.span,
                    format!(
                        "`{callee}` may change `{param}`, but this function only reads `{param}`"
                    ),
                )
                .with_note(format!(
                    "adding `mut` to `{param}` means every caller of `{}` must also pass \
                     something it is allowed to change",
                    self.name
                ))
            } else {
                let binding = &self.local(blamed).name;
                Diagnostic::new(
                    codes::V0302,
                    leaf.span,
                    format!(
                        "`{callee}` may change `{binding}`, but `{binding}` was declared without `mut`"
                    ),
                )
            };
            let diagnostic = self.add_mut_fix_it(diagnostic, root, blamed);
            self.diagnostics.push(diagnostic);
        }
    }

    /// Adds the fix-it inserting `mut ` before `blamed`'s name, a label
    /// there, and, when the place is reached through another binding
    /// `root`, a note saying so.
    fn add_mut_fix_it(&self, diagnostic: Diagnostic, root: LocalId, blamed: LocalId) -> Diagnostic {
        let decl = self.local(blamed);
        let at = Span::new(decl.span.file, decl.span.start, decl.span.start);
        let kind = if self.is_param(blamed) {
            "parameter"
        } else {
            "variable"
        };
        let mut diagnostic = diagnostic
            .with_label(
                decl.span,
                format!("`{}` is declared here without `mut`", decl.name),
            )
            .with_fix_it(FixIt {
                span: at,
                replacement: "mut ".to_string(),
            });
        if root != blamed {
            diagnostic = diagnostic.with_note(format!(
                "`{}` comes from {kind} `{}`",
                self.local(root).name,
                decl.name
            ));
        }
        diagnostic
    }

    /// A borrowed place of non-Copy type cannot flow into an owned slot.
    fn owned_slot(&mut self, value: &HirExpr, slot: &Slot) {
        for leaf in leaves(value) {
            let info = self.info(leaf);
            if !info.borrowed || leaf.ty.is_copy() {
                continue;
            }
            if let Some(root) = place_root(leaf) {
                self.reported[root.0 as usize] = true;
            }
            let owner = owner_text(self.cx, self.locals, info.origin);
            let into = match slot {
                Slot::Return => "returned from here".to_string(),
                Slot::StructField { id, field } => format!(
                    "stored in field `{}` of `{}`",
                    self.cx.structs[id.0 as usize].fields[*field].0,
                    self.cx.struct_name(*id)
                ),
                Slot::ImportedParam(callee) => {
                    format!("given to `{callee}`, which keeps what it is given")
                }
                Slot::Assignment => "stored there".to_string(),
                Slot::Local(name) => format!("kept in `{name}`"),
            };
            self.diagnostics.push(
                Diagnostic::new(
                    codes::V0304,
                    leaf.span,
                    format!("{owner}, so it cannot be {into}"),
                )
                .with_note(MILESTONE_2_NOTE),
            );
        }
    }
}

mod moves;
mod strings;
#[cfg(test)]
mod tests;
