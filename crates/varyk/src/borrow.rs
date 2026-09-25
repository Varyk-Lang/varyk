//! Borrow analysis (spec section 4.2), part 1: places and mutability
//! contracts.
//!
//! [`analyze`] fills in every local's [`PlaceInfo`] (is it a borrowed
//! place, is it a mutable place, and what does it borrow from), then
//! enforces the mutability contracts (V0300-V0303) and rejects borrowed
//! places flowing into owned slots (V0304): struct-literal fields,
//! variant payloads, `Some`, `Ok`, and `Err` arguments, `vec!` elements,
//! assignments through a borrowed place or to a whole struct local, return
//! values, and imported parameters taken by value. It also rejects (V0304)
//! an `if` or block read in place whose values mix new ones with places,
//! and values that would be referred to after their block ends (see
//! `Leaf`). Three more passes follow per function:
//! `strings` infers each `string` local's representation (spec 4.3) and
//! rejects borrowed places flowing into a `let` that must be owned;
//! `moves` rejects uses after a move (V0305); and `aliases` rejects
//! changing a place while another name for it is still used (V0307).
//!
//! Places are locals and field paths rooted at one; only locals store
//! their [`PlaceInfo`]; a field path's info derives from its root local
//! plus the field's type (see [`place_info`]). Every other expression
//! (call results, struct literals, literals, operators) is a temporary: a
//! mutable place that is not borrowed.

use varyk_syntax::{FixIt, Span};

use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{
    HirBlock, HirEnum, HirExpr, HirExprKind, HirForHead, HirFunction, HirProgram, HirStmt,
    HirStruct, LocalId, LocalInfo, MethodRef, Origin, PlaceInfo, VariantRef, declared_inside,
    field_root, is_block_like, leaves,
};
use crate::resolve::{Callee, ImportedSig, StructId};
use crate::types::{ParamMode, Ty};
use patterns::Bound;

/// The Rust-facing note of a V0304 for a borrowed value kept somewhere
/// that owns it; [`what_to_do`] gives the plain-words one.
const BORROWED_NOTE: &str = "in Rust terms, a borrowed value cannot move into a place that owns it";

/// The fix for an `if`, block, or `match` that cannot be read in place or
/// stored with `let`.
const NEW_BRANCHES: &str = "make every branch give a new value instead (for text, with `.clone()`)";

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
pub fn analyze(hir: HirProgram) -> Result<HirProgram, Vec<Diagnostic>> {
    let (hir, diagnostics) = analyze_unchecked(hir);
    if diagnostics.is_empty() {
        Ok(hir)
    } else {
        Err(diagnostics)
    }
}

/// [`analyze`], returning the annotated program along with whatever it
/// reports: the soundness tests generate Rust for programs it rejects to
/// confirm rustc rejects them too.
#[doc(hidden)]
pub fn analyze_unchecked(mut hir: HirProgram) -> (HirProgram, Vec<Diagnostic>) {
    let signatures: Vec<(String, Vec<ParamMode>)> = hir
        .functions()
        .map(|f| (f.name.clone(), f.params.iter().map(|p| p.mode).collect()))
        .collect();
    let cx = Context {
        signatures: &signatures,
        imported: &hir.imported,
        structs: &hir.structs,
        enums: &hir.enums,
    };
    let mut diagnostics = Vec::new();
    for function in &mut hir.functions {
        analyze_function(&cx, function, &mut diagnostics);
    }
    (hir, diagnostics)
}

/// The [`PlaceInfo`] of a place expression (a `Local`, or a chain of
/// `Field`s and `Index`es on one), read from `locals` as [`analyze`]
/// annotated them; `None` for any other expression, which is a temporary.
///
/// A field or an element is a borrowed place exactly when its type is not
/// Copy (whatever its base: there are no partial moves), and is mutable
/// exactly when its base is: for an `if` or block base, when every value it
/// may evaluate to is. A field borrows from the struct it belongs to; an
/// element from what its `Vec` borrows from, or from the local owning that
/// `Vec` (spec 3.1).
pub fn place_info(locals: &[LocalInfo], expr: &HirExpr) -> Option<PlaceInfo> {
    match &expr.kind {
        HirExprKind::Local(id) => Some(locals[id.0 as usize].place),
        HirExprKind::Field { base, .. } | HirExprKind::Index { base, .. } => {
            let mutable = if is_block_like(base) {
                leaves(base)
                    .into_iter()
                    .all(|leaf| place_info(locals, leaf).unwrap_or(TEMPORARY).mutable)
            } else {
                place_info(locals, base).unwrap_or(TEMPORARY).mutable
            };
            let borrowed = !expr.ty.is_copy();
            let origin = match (&expr.kind, &base.ty) {
                (_, _) if !borrowed => None,
                (HirExprKind::Field { .. }, Ty::Struct(id)) => Some(Origin::Struct(*id)),
                (HirExprKind::Index { .. }, _) => match place_info(locals, base) {
                    Some(base) if base.borrowed => base.origin,
                    _ => place_root(base).map(Origin::Local),
                },
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
    /// Every Varyk function's name and parameter modes, indexed by `FnId`.
    signatures: &'a [(String, Vec<ParamMode>)],
    imported: &'a [ImportedSig],
    structs: &'a [HirStruct],
    enums: &'a [HirEnum],
}

impl Context<'_> {
    /// The callee's name, its parameter modes, and whether it is imported.
    fn callee(&self, callee: Callee) -> (String, Vec<ParamMode>, bool) {
        match callee {
            Callee::Varyk(id) => {
                let (name, modes) = &self.signatures[id.0 as usize];
                (name.clone(), modes.clone(), false)
            }
            Callee::Imported(id) => {
                let sig = &self.imported[id.0 as usize];
                let modes = sig.params.iter().map(|(_, mode)| *mode).collect();
                (sig.name.clone(), modes, true)
            }
            Callee::Builtin(id) => (id.path(), id.modes(), true),
        }
    }

    /// A method's name and parameter modes, its receiver's first, and
    /// whether an `Owned` parameter is an owned slot (as for an imported
    /// callee): true for the built-in table, whose `push` keeps its
    /// argument, false for a Varyk method, whose `Owned` parameters are
    /// Copy.
    fn method(&self, method: MethodRef) -> (String, Vec<ParamMode>, bool) {
        match method {
            MethodRef::Varyk(id) => self.callee(Callee::Varyk(id)),
            MethodRef::Builtin(id) => (id.path(), id.modes(), true),
        }
    }

    fn struct_name(&self, id: StructId) -> &str {
        &self.structs[id.0 as usize].name
    }

    /// A variant as written in a value: `Shape::Circle`, `Some`.
    fn variant_name(&self, variant: VariantRef) -> String {
        match variant {
            VariantRef::User(id, index) => {
                let e = &self.enums[id.0 as usize];
                format!("{}::{}", e.name, e.variants[index].0)
            }
            VariantRef::Some => "Some".to_string(),
            VariantRef::None => "None".to_string(),
            VariantRef::Ok => "Ok".to_string(),
            VariantRef::Err => "Err".to_string(),
        }
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
        Some(Origin::Local(local)) => {
            format!(
                "this value is kept inside `{}`",
                locals[local.0 as usize].name
            )
        }
        None => "this value belongs to someone else".to_string(),
    }
}

/// Says, in plain words, what to do instead of keeping a borrowed place
/// with `origin` somewhere that owns it: the plain-words note of V0304.
fn what_to_do(cx: &Context, locals: &[LocalInfo], origin: Option<Origin>) -> String {
    match origin {
        Some(Origin::Param(param)) => {
            let name = &locals[param.0 as usize].name;
            format!(
                "this function can read `{name}` but not keep it; make a new value here \
                 instead, or keep it where the caller made it"
            )
        }
        Some(Origin::Struct(id)) => {
            let name = cx.struct_name(id);
            format!(
                "a field stays inside its `{name}`; make a new value here instead of taking \
                 it out"
            )
        }
        Some(Origin::Local(local)) => {
            let name = &locals[local.0 as usize].name;
            format!(
                "this value stays inside `{name}`; use `{name}` itself, or make a new value \
                 here instead"
            )
        }
        None => "make a new value here instead".to_string(),
    }
}

/// The local a place expression is rooted at; `None` for a temporary or a
/// field or element of one, and for a field or element of an `if` or block.
fn place_root(expr: &HirExpr) -> Option<LocalId> {
    match &expr.kind {
        HirExprKind::Local(id) => Some(*id),
        HirExprKind::Field { base, .. } | HirExprKind::Index { base, .. } => place_root(base),
        _ => None,
    }
}

/// Whether `expr` is an element, or a field of one: its chain of fields
/// and elements passes through an index.
fn through_index(expr: &HirExpr) -> bool {
    match &expr.kind {
        HirExprKind::Index { .. } => true,
        HirExprKind::Field { base, .. } => through_index(base),
        _ => false,
    }
}

/// The source text of a place expression built from locals, fields, and
/// indexes, for [`alias_note`]. An index that is not a literal or a local
/// falls back to `0`, the shape of the place rather than its exact text.
fn place_shape_text(locals: &[LocalInfo], expr: &HirExpr) -> String {
    match &expr.kind {
        HirExprKind::Local(id) => locals[id.0 as usize].name.clone(),
        HirExprKind::Field { base, name } => {
            format!("{}.{}", place_shape_text(locals, base), name)
        }
        HirExprKind::Index { base, index } => {
            let index_text = match &index.kind {
                HirExprKind::Int(digits) => digits.clone(),
                HirExprKind::Local(id) => locals[id.0 as usize].name.clone(),
                _ => "0".to_string(),
            };
            format!("{}[{}]", place_shape_text(locals, base), index_text)
        }
        _ => "..".to_string(),
    }
}

/// The V0300 note for `local`, a `let mut` binding whose initializer
/// `value` is a whole other place, or an element or a field of one, that
/// turned out shared: `local` is only another name for that place (or part
/// of it), so writing through it needs a copy, not `mut`. `None` when
/// `value` is not simply a local, an element, or a field.
fn alias_note(locals: &[LocalInfo], local: LocalId, value: &HirExpr) -> Option<String> {
    let alias = &locals[local.0 as usize].name;
    if let HirExprKind::Local(root) = value.kind {
        let source = &locals[root.0 as usize].name;
        return Some(format!(
            "`{alias}` is another name for `{source}`; to keep your own copy, write \
             `let mut {alias} = {source}.clone();`"
        ));
    }
    let (noun, article) = match value.kind {
        HirExprKind::Index { .. } => ("element", "an"),
        HirExprKind::Field { .. } => ("field", "a"),
        _ => return None,
    };
    let root = place_root(value)?;
    let source = &locals[root.0 as usize].name;
    let text = place_shape_text(locals, value);
    Some(format!(
        "`{alias}` is another name for {article} {noun} of `{source}`; to keep your own copy, \
         write `let mut {alias} = {text}.clone();`"
    ))
}

/// Every local `expr` may borrow from: its root, or, through `if`s,
/// blocks, and `match`es (as a value or as the base of a field), the roots
/// of every value it may evaluate to.
fn roots(expr: &HirExpr) -> Vec<LocalId> {
    let base = field_root(expr);
    match &base.kind {
        HirExprKind::Local(id) => vec![*id],
        _ if is_block_like(base) => leaves(base).into_iter().flat_map(roots).collect(),
        _ => Vec::new(),
    }
}

/// A V0304 message saying, in plain words, why `leaf` (a value of
/// `whole`: inside an `if`, block, or `match`, or a field or element of a
/// temporary) is gone once `whole` ends, then `consequence`, then how to
/// fix it when there is a simple fix.
fn gone_message(
    cx: &Context,
    locals: &[LocalInfo],
    leaf: &HirExpr,
    whole: &HirExpr,
    consequence: &str,
) -> String {
    if let Some(root) = place_root(leaf) {
        let scope = if arm_binds(whole, root) {
            "arm"
        } else {
            "block"
        };
        return format!(
            "`{}` only exists inside this {scope}, so {consequence}",
            locals[root.0 as usize].name
        );
    }
    if through_index(leaf) {
        let when = if is_block_like(whole) {
            "only exists inside this block"
        } else {
            "is gone after this line"
        };
        return format!("the `Vec` this value is part of {when}, so {consequence}");
    }
    if let HirExprKind::Field { base, .. } = &leaf.kind {
        if let (Ty::Struct(id), false) = (&base.ty, is_block_like(field_root(base))) {
            let name = cx.struct_name(*id);
            return format!(
                "this value is kept inside a `{name}` that is not stored anywhere, so \
                 {consequence}; store the `{name}` with `let` first"
            );
        }
    }
    if !is_block_like(whole) {
        return format!("this value is gone after this line, so {consequence}");
    }
    format!("this value only exists inside this block, so {consequence}")
}

/// Whether `local` is bound by the pattern of a `match` arm that `expr`
/// may evaluate through (see [`leaves`]).
fn arm_binds(expr: &HirExpr, local: LocalId) -> bool {
    let tail = |block: &HirBlock| block.tail.as_deref().is_some_and(|t| arm_binds(t, local));
    match &expr.kind {
        HirExprKind::Block(block) => tail(block),
        HirExprKind::If { then, else_, .. } => tail(then) || else_.as_ref().is_some_and(tail),
        HirExprKind::Match { arms, .. } => arms.iter().any(|arm| {
            arm.pattern
                .bindings()
                .iter()
                .any(|(bound, _)| *bound == local)
                || arm_binds(&arm.body, local)
        }),
        _ => false,
    }
}

/// What each `let` may refer to, as (some value of its initializer is a
/// temporary, the locals it is rooted at); see [`dangling`].
type Refers = Vec<(bool, Vec<LocalId>)>;

/// The text flowing into `let`s that hold text and are not borrowed places,
/// per local. A borrowed place assigned to one makes it a reference to
/// that place, so it is an alias of that place's roots (spec 3.1); another
/// such `let` flowing into one copies that reference, so it is an alias of
/// whatever the other one is an alias of.
struct Assigned {
    /// The locals rooting the borrowed places assigned to it.
    roots: Vec<Vec<LocalId>>,
    /// The `let`s holding text that flow into it, by its `let` or an
    /// assignment.
    copies: Vec<Vec<LocalId>>,
}

/// Pushes `local` onto `list` unless it is already there.
fn push_unique(list: &mut Vec<LocalId>, local: LocalId) {
    if !list.contains(&local) {
        list.push(local);
    }
}

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
    /// A payload of the variant written this way (`Shape::Circle`, `Some`).
    Payload(String),
    VecElement,
    /// The operand of `?`.
    Question,
}

fn analyze_function(cx: &Context, function: &mut HirFunction, diagnostics: &mut Vec<Diagnostic>) {
    let (reported, refers, assigned, held) = places(cx, function, diagnostics);
    strings::infer(cx, function, &reported, &refers, &assigned, diagnostics);
    moves::check(cx, function, diagnostics);
    aliases::check(cx, function, &refers, &assigned, &held, diagnostics);
}

/// The first pass: place info, mutability contracts, and owned slots.
/// Returns, per local, whether a V0304 was reported for a flow of it, what
/// it may refer to, (see [`Assigned`]) the roots of the borrowed places
/// assigned to it, and whether it is the variable of a `for` that holds
/// the place it loops over.
fn places(
    cx: &Context,
    function: &mut HirFunction,
    diagnostics: &mut Vec<Diagnostic>,
) -> (Vec<bool>, Refers, Assigned, Vec<bool>) {
    let HirFunction {
        name,
        params,
        ret,
        body,
        locals,
        ..
    } = function;
    let mut blame = vec![None; locals.len()];
    let patterns = vec![None; locals.len()];
    let held = vec![false; locals.len()];
    let reported = vec![false; locals.len()];
    let alias_notes = vec![None; locals.len()];
    let refers = vec![(false, Vec::new()); locals.len()];
    let assigned = Assigned {
        roots: vec![Vec::new(); locals.len()],
        copies: vec![Vec::new(); locals.len()],
    };
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
        ret: ret.clone(),
        locals,
        blame,
        reported,
        alias_notes,
        refers,
        assigned,
        patterns,
        held,
        diagnostics,
    };
    analyzer.block(body);
    if *ret != Ty::Unit {
        if let Some(tail) = &body.tail {
            analyzer.owned_slot(tail, &Slot::Return);
        }
    }
    (
        analyzer.reported,
        analyzer.refers,
        analyzer.assigned,
        analyzer.held,
    )
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
    /// Per local: the extra note V0300 adds when the local is a `let mut`
    /// alias of an element or field reached through a place that turned out
    /// shared, so writing through it needs a copy instead of `mut`.
    alias_notes: Vec<Option<String>>,
    /// Per `let`: what its initializer may refer to, as (some value is a
    /// temporary, the locals it is rooted at). Read for bindings that are
    /// borrowed places, to tell whether they outlive their block.
    refers: Refers,
    assigned: Assigned,
    /// Per local: what it is bound to when a `match` pattern or a `for`
    /// binds it.
    patterns: Vec<Option<Bound>>,
    /// Per local: the variable of a `for` over a place, which the loop
    /// holds until it ends (spec 3.1).
    held: Vec<bool>,
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
            HirStmt::For {
                local, head, body, ..
            } => {
                match head {
                    HirForHead::Range { start, end } => {
                        self.expr(start);
                        self.expr(end);
                    }
                    HirForHead::Vec(vec) => self.expr(vec),
                }
                self.for_variable(*local, head, body);
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
                let (name, modes, keeps) = self.cx.callee(*callee);
                let args: Vec<&HirExpr> = args.iter().collect();
                self.call(&name, &modes, keeps, &args, None);
            }
            // The receiver is an argument passed as the method's `self`, or
            // the table's receiver mode, says (spec 2.5).
            HirExprKind::MethodCall {
                receiver,
                method,
                args,
            } => {
                self.expr(receiver);
                for arg in args {
                    self.expr(arg);
                }
                let (name, modes, keeps) = self.cx.method(*method);
                let values: Vec<&HirExpr> = std::iter::once(&**receiver).chain(args).collect();
                self.call(&name, &modes, keeps, &values, Some(*method));
            }
            HirExprKind::Field { base, .. } => {
                self.expr(base);
                // A field access reads its base in place.
                self.mixed(base, false);
            }
            HirExprKind::Index { base, index } => {
                self.expr(base);
                // So does indexing.
                self.mixed(base, false);
                self.expr(index);
                self.index_changes_root(expr, index);
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
                    self.used_while_lent(&[lhs, rhs], &[Some(false), Some(false)], false, None);
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
            HirExprKind::EnumLit { variant, args } => {
                let slot = Slot::Payload(self.cx.variant_name(*variant));
                for arg in args {
                    self.expr(arg);
                    self.owned_slot(arg, &slot);
                }
            }
            HirExprKind::Try(operand) => {
                self.expr(operand);
                self.owned_slot(operand, &Slot::Question);
            }
            HirExprKind::VecLit(elements) => {
                for element in elements {
                    self.expr(element);
                    self.owned_slot(element, &Slot::VecElement);
                }
            }
            HirExprKind::Match { scrutinee, arms } => {
                self.expr(scrutinee);
                for arm in arms {
                    self.pattern(scrutinee, arm);
                    self.expr(&arm.body);
                }
            }
            HirExprKind::Println { args, .. } | HirExprKind::Format { args, .. } => {
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
                    self.used_while_lent(&values, &lent, true, None);
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

    /// What `leaf`, a borrowed place a `let` is initialized from, derives
    /// from: for a field or element of a local that owns its value, that
    /// local.
    fn origin(&self, leaf: &HirExpr, info: PlaceInfo) -> Option<Origin> {
        match (&leaf.kind, place_root(leaf)) {
            (HirExprKind::Field { .. } | HirExprKind::Index { .. }, Some(root))
                if !self.local(root).place.borrowed =>
            {
                Some(Origin::Local(root))
            }
            _ => info.origin,
        }
    }

    /// Records, in [`Assigned`], what `value` brings into `local`, a `let`
    /// holding text that is not a borrowed place: the roots of its borrowed
    /// places and the other such `let`s it copies.
    fn text_flow(&mut self, local: LocalId, value: &HirExpr) {
        let index = local.0 as usize;
        for leaf in leaves(value) {
            if self.info(leaf).borrowed {
                for root in roots(leaf) {
                    push_unique(&mut self.assigned.roots[index], root);
                }
            } else if let HirExprKind::Local(id) = leaf.kind {
                if leaf.ty == Ty::String {
                    push_unique(&mut self.assigned.copies[index], id);
                }
            }
        }
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
                borrowed = Some(self.origin(leaf, info));
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
        if place.borrowed && declared_mut && !place.mutable && value.ty == Ty::String {
            self.alias_notes[local.0 as usize] = alias_note(self.locals, local, value);
        }
        if place.borrowed && place.mutable {
            // A mutable alias reaches its element mutably.
            for leaf in leaves(value) {
                self.index_uses_root(leaf);
            }
        }
        if !place.borrowed && value.ty == Ty::String {
            self.text_flow(local, value);
        }
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
        // A field, an element, or a `mut string` parameter is a reference
        // to a `String`.
        let as_str = value.ty == Ty::String
            && leaves.iter().any(|leaf| match leaf.kind {
                HirExprKind::Field { .. } | HirExprKind::Index { .. } => false,
                HirExprKind::Local(id) => !(self.is_param(id) && self.local(id).mutable),
                _ => true,
            });
        for leaf in leaves {
            let gone = match self.leaf_kind(leaf, value, false) {
                Leaf::Lasting => false,
                Leaf::New => match leaf.kind {
                    HirExprKind::Local(_) => true,
                    HirExprKind::Field { .. } | HirExprKind::Index { .. } => {
                        place_root(leaf).is_some() || as_str
                    }
                    _ => false,
                },
                Leaf::Unsure | Leaf::Dangling => true,
            };
            if gone {
                let consequence = format!("it cannot be kept in `{}`", self.local(local).name);
                let message = gone_message(self.cx, self.locals, leaf, value, &consequence);
                self.diagnostics
                    .push(Diagnostic::new(codes::V0304, leaf.span, message).with_note(GONE_NOTE));
                self.reported[local.0 as usize] = true;
            }
        }
    }

    // --- Checks ---------------------------------------------------------------

    fn assign(&mut self, target: &HirExpr, value: &HirExpr) {
        if self.index_uses_root(target) {
            return;
        }
        let info = self.info(target);
        if !info.mutable {
            let root = place_root(target).expect("an assignment target is a place");
            let blamed = self.blame(target).unwrap_or(root);
            let diagnostic = self.cannot_change(target.span, root, blamed);
            self.diagnostics.push(diagnostic);
        } else if info.borrowed {
            // Writing through a borrowed place stores into storage the
            // caller or a struct owns.
            self.owned_slot(value, &Slot::Assignment);
        } else if let HirExprKind::Local(id) = target.kind {
            // A whole struct local that is not a borrowed place owns its
            // value. (A `string` one may be `&str`; the `strings` pass
            // decides.)
            if target.ty.is_compound() {
                let name = self.local(id).name.clone();
                self.owned_slot(value, &Slot::Local(name));
            } else if target.ty == Ty::String {
                self.text_flow(id, value);
            }
        }
    }

    /// A call to `name`, whose parameters take `args` by `modes`; `keeps`
    /// says an `Owned` parameter is an owned slot (see [`Context::callee`]).
    /// For a method call, `args` starts with the receiver and `method`
    /// names the method: the receiver of a built-in that changes it is
    /// changed like an assignment target (V0300, V0301), a `mut self`
    /// receiver is passed like any argument to a `mut` parameter (V0302,
    /// V0303).
    fn call(
        &mut self,
        name: &str,
        modes: &[ParamMode],
        keeps: bool,
        args: &[&HirExpr],
        method: Option<MethodRef>,
    ) {
        let reported = self.diagnostics.len();
        for (index, (arg, mode)) in args.iter().zip(modes.iter().copied()).enumerate() {
            match mode {
                ParamMode::MutableBorrow => {
                    let indexed = leaves(arg)
                        .into_iter()
                        .any(|leaf| self.index_uses_root(leaf));
                    if !indexed && !self.mixed(arg, true) {
                        let built_in = matches!(method, Some(MethodRef::Builtin(_)));
                        self.mut_argument(arg, name, index == 0 && built_in);
                    }
                }
                ParamMode::SharedBorrow => {
                    self.mixed(arg, false);
                }
                // A Varyk-declared `Owned` parameter is always Copy.
                ParamMode::Owned if keeps => {
                    self.owned_slot(arg, &Slot::ImportedParam(name.to_string()))
                }
                ParamMode::Owned => {}
            }
        }
        if self.diagnostics.len() == reported {
            // `args[0]` is the receiver exactly for a method call, and only
            // a changing one is passed a `MutableBorrow` receiver.
            let changing_receiver = method
                .filter(|_| modes.first() == Some(&ParamMode::MutableBorrow))
                .map(|_| name);
            self.overlapping_borrows(args, modes, changing_receiver);
        }
        if self.diagnostics.len() == reported {
            let lent: Vec<Option<bool>> = modes
                .iter()
                .map(|mode| match mode {
                    ParamMode::MutableBorrow => Some(true),
                    ParamMode::SharedBorrow => Some(false),
                    ParamMode::Owned => None,
                })
                .collect();
            let receiver = method.map(|_| name);
            self.used_while_lent(args, &lent, false, receiver);
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
    ///
    /// `method` names the method when `values[0]` is its receiver, which
    /// the message then speaks of, as in `v.push(v.len())`.
    fn used_while_lent(
        &mut self,
        values: &[&HirExpr],
        lent: &[Option<bool>],
        include_copy: bool,
        method: Option<&str>,
    ) {
        let mut held: Vec<(LocalId, bool, Span, bool)> = Vec::new();
        for (index, (value, lent)) in values.iter().zip(lent).enumerate() {
            let mut uses = Vec::new();
            self.uses_on_the_way(value, true, false, &mut uses);
            for (local, exclusive, span) in uses {
                let Some(&(_, _, first, receiver)) = held
                    .iter()
                    .find(|(other, mutable, ..)| *other == local && (exclusive || *mutable))
                else {
                    continue;
                };
                let name = &self.local(local).name;
                let message = match method {
                    Some(method) if receiver => format!(
                        "`{name}` is used here while this call of `{method}` already has it; \
                         store this value with `let` first, then pass that"
                    ),
                    _ => format!(
                        "`{name}` is used here while it is still lent out earlier in the same \
                         expression"
                    ),
                };
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
                    let receiver = index == 0 && method.is_some();
                    for root in roots(value) {
                        held.push((root, mutable, value.span, receiver));
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
            HirExprKind::Field { .. } | HirExprKind::Index { .. } => {
                // The indices along the path are evaluated on the way.
                let mut path = expr;
                while let HirExprKind::Field { base, .. } | HirExprKind::Index { base, .. } =
                    &path.kind
                {
                    if let HirExprKind::Index { index, .. } = &path.kind {
                        self.uses_on_the_way(index, false, false, out);
                    }
                    path = base;
                }
                let base = path;
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
                let (_, modes, keeps) = self.cx.callee(*callee);
                self.call_uses(args.iter(), &modes, keeps, out);
            }
            // The receiver of a changing method is changed (spec 2.5).
            HirExprKind::MethodCall {
                receiver,
                method,
                args,
            } => {
                let (_, modes, keeps) = self.cx.method(*method);
                self.call_uses(std::iter::once(&**receiver).chain(args), &modes, keeps, out);
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
            HirExprKind::Println { args, .. } | HirExprKind::Format { args, .. } => {
                for arg in args {
                    self.uses_on_the_way(arg, false, false, out);
                }
            }
            HirExprKind::Try(operand) => self.uses_on_the_way(operand, false, true, out),
            HirExprKind::EnumLit { args, .. } | HirExprKind::VecLit(args) => {
                for arg in args {
                    self.uses_on_the_way(arg, false, !arg.ty.is_copy(), out);
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
            HirExprKind::Match { scrutinee, arms } => {
                self.uses_on_the_way(scrutinee, false, false, out);
                for arm in arms {
                    self.uses_on_the_way(&arm.body, at_leaf, exclusive, out);
                }
            }
        }
    }

    /// [`FnAnalyzer::uses_on_the_way`] for the arguments of a call, each
    /// exclusive when lent to a `mut` parameter or kept by an `Owned` one.
    fn call_uses<'e>(
        &self,
        args: impl Iterator<Item = &'e HirExpr>,
        modes: &[ParamMode],
        keeps: bool,
        out: &mut Vec<(LocalId, bool, Span)>,
    ) {
        for (arg, mode) in args.zip(modes) {
            let exclusive = match mode {
                ParamMode::MutableBorrow => true,
                ParamMode::Owned => keeps && !arg.ty.is_copy(),
                ParamMode::SharedBorrow => false,
            };
            self.uses_on_the_way(arg, false, exclusive, out);
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
                HirStmt::For { head, body, .. } => {
                    match head {
                        HirForHead::Range { start, end } => {
                            self.uses_on_the_way(start, false, false, out);
                            self.uses_on_the_way(end, false, false, out);
                        }
                        HirForHead::Vec(vec) => self.uses_on_the_way(vec, false, false, out),
                    }
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
    ///
    /// `changing_receiver`, the method's name exactly when `args[0]` is the
    /// receiver of a changing method, sharpens the note when the
    /// conflicting argument is an element or field of that receiver's
    /// root, as in `v.push(v[0])`: a `string` element or field is fixed by
    /// `.clone()`; a `Copy` one by storing it with a plain `let`; anything
    /// else (a struct, say) needs a stored copy, and, when the value came
    /// through an index, looping by index and changing the element in
    /// place instead.
    fn overlapping_borrows(
        &mut self,
        args: &[&HirExpr],
        modes: &[ParamMode],
        changing_receiver: Option<&str>,
    ) {
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
                let mut diagnostic = Diagnostic::new(codes::V0306, arg.span, message)
                    .with_label(first, format!("`{name}` is first passed here"));
                let of_receiver = changing_receiver.is_some()
                    && place_root(args[0]) == Some(root)
                    && matches!(
                        arg.kind,
                        HirExprKind::Field { .. } | HirExprKind::Index { .. }
                    );
                if of_receiver {
                    let note = if arg.ty == Ty::String {
                        format!(
                            "store this value with `let` first, as in `let x = {}.clone();`",
                            place_shape_text(self.locals, arg)
                        )
                    } else if arg.ty.is_copy() {
                        format!(
                            "store it with `let` first, as in `let x = {};`",
                            place_shape_text(self.locals, arg)
                        )
                    } else if through_index(arg) {
                        "store a copy first, or loop by index and change the element in place"
                            .to_string()
                    } else {
                        "store a copy first".to_string()
                    };
                    diagnostic = diagnostic.with_note(note);
                }
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
    /// A non-`Copy` element (or a field of one) of a `Vec` that is new is
    /// gone with it: read in place, it cannot be moved out of its `Vec`,
    /// and nothing keeps the `Vec` alive as `let` does (see
    /// [`Self::kept_by_reference`]).
    fn leaf(&self, leaf: &HirExpr, whole: &HirExpr, mutable: bool) -> Leaf {
        match self.leaf_kind(leaf, whole, mutable) {
            Leaf::New if !leaf.ty.is_copy() && through_index(leaf) => Leaf::Dangling,
            kind => kind,
        }
    }

    /// [`Self::leaf`], without the rule for elements of a new `Vec`.
    fn leaf_kind(&self, leaf: &HirExpr, whole: &HirExpr, mutable: bool) -> Leaf {
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
            _ if is_block_like(base) => {
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
        let checked = match &expr.ty {
            Ty::String => true,
            ty if ty.is_compound() => true,
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
            // An element of a `Vec` gone with `expr` (see [`Self::leaf`]).
            let element = self.leaf_kind(leaf, expr, mutable) == Leaf::New;
            let message = match place_root(leaf) {
                Some(root) if !element => format!(
                    "`{}` refers to something that only exists inside this block, so the \
                     block cannot hand it out",
                    self.local(root).name
                ),
                _ => gone_message(
                    self.cx,
                    self.locals,
                    leaf,
                    expr,
                    "it cannot be handed out here",
                ),
            };
            let mut diagnostic = Diagnostic::new(codes::V0304, leaf.span, message);
            if element {
                diagnostic = diagnostic.with_note(NEW_BRANCHES);
            }
            self.diagnostics.push(diagnostic.with_note(GONE_NOTE));
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
        let mut advice = None;
        let message = match named {
            Some((leaf, root)) => {
                // A binding is borrowed from what it was matched on or
                // looped over.
                let shown = self.patterns[root.0 as usize]
                    .and_then(|bound| bound.root)
                    .unwrap_or(root);
                let name = &self.local(shown).name;
                let owned_let = matches!(leaf.kind, HirExprKind::Local(_))
                    && !self.is_param(root)
                    && !self.local(root).place.borrowed;
                // A `let` of this value would keep a reference to a new
                // value declared inside it (see [`Self::kept_by_reference`]).
                let unstorable = !owned_let
                    && leaves
                        .iter()
                        .zip(&kinds)
                        .any(|(leaf, kind)| *kind == Leaf::New && place_root(leaf).is_some());
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
                if unstorable {
                    advice = Some(NEW_BRANCHES);
                } else if owned_let || !text {
                    message.push_str("; store it with `let` first");
                } else {
                    advice = Some(
                        "make every branch give new text, or every branch give text kept \
                         elsewhere",
                    );
                }
                message
            }
            None => format!(
                "this value is sometimes {new} and sometimes kept elsewhere; store it with `let` first"
            ),
        };
        let mut diagnostic = Diagnostic::new(codes::V0304, expr.span, message);
        if let Some(advice) = advice {
            diagnostic = diagnostic.with_note(advice);
        }
        self.diagnostics.push(diagnostic.with_note(BORROWED_NOTE));
        true
    }

    /// An argument to a `mut` parameter must be a mutable place; so must
    /// every value an `if` or block base of a field argument may evaluate
    /// to. `changed` marks the receiver of a built-in that changes it,
    /// reported like an assignment target.
    fn mut_argument(&mut self, arg: &HirExpr, callee: &str, changed: bool) {
        for leaf in leaves(arg) {
            if self.info(leaf).mutable {
                continue;
            }
            let base = field_root(leaf);
            if is_block_like(base) {
                self.mut_argument(base, callee, changed);
                continue;
            }
            let Some(root) = place_root(leaf) else {
                continue;
            };
            let blamed = self.blame(leaf).unwrap_or(root);
            if changed {
                let diagnostic = self.cannot_change(leaf.span, root, blamed);
                self.diagnostics.push(diagnostic);
                continue;
            }
            if self.patterns[blamed.0 as usize].is_some() {
                let diagnostic = self.binding_unchangeable(codes::V0303, leaf.span, blamed, callee);
                self.diagnostics.push(diagnostic);
                continue;
            }
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

    /// V0306 when an index along `place`, a place reached mutably (an
    /// assignment target, an argument to a `mut` parameter, the receiver
    /// of a changing method, or the source of a mutable alias), uses the
    /// place's root: `v[v.len() - 1] = x`. Rust borrows `v` mutably to
    /// reach the element before it works out the index, and indexing gets
    /// none of the leeway a method call's receiver gets. Returns whether
    /// it reported.
    fn index_uses_root(&mut self, place: &HirExpr) -> bool {
        let Some(root) = place_root(place) else {
            return false;
        };
        let mut uses = Vec::new();
        let mut path = place;
        loop {
            match &path.kind {
                HirExprKind::Field { base, .. } => path = base,
                HirExprKind::Index { base, index } => {
                    self.uses_on_the_way(index, false, false, &mut uses);
                    path = base;
                }
                _ => break,
            }
        }
        let Some(&(_, _, span)) = uses.iter().find(|(local, ..)| *local == root) else {
            return false;
        };
        self.index_uses_root_at(root, span, place.span, false);
        true
    }

    /// V0306 when `index`, the index of `place` read, changes the place's
    /// root (passes it to a `mut` parameter or calls a changing method on
    /// it): `v[g(v)]`. Rust borrows `v` to reach the element before it
    /// works out the index.
    fn index_changes_root(&mut self, place: &HirExpr, index: &HirExpr) {
        let Some(root) = place_root(place) else {
            return;
        };
        let mut uses = Vec::new();
        self.uses_on_the_way(index, false, false, &mut uses);
        if let Some(&(_, _, span)) = uses
            .iter()
            .find(|&&(local, exclusive, _)| local == root && exclusive)
        {
            self.index_uses_root_at(root, span, place.span, true);
        }
    }

    /// The V0306 of [`Self::index_uses_root`] (or, when `read`, of
    /// [`Self::index_changes_root`]), once per use of `root` at `span` in
    /// an index of the element at `element`.
    fn index_uses_root_at(&mut self, root: LocalId, span: Span, element: Span, read: bool) {
        if self
            .diagnostics
            .iter()
            .any(|d| d.code == codes::V0306 && d.span == span)
        {
            return;
        }
        let name = &self.local(root).name;
        let (used, element_verb, label, mutably) = if read {
            ("changed", "read", "read", "")
        } else {
            ("used", "changed", "changed", " mutably")
        };
        let message = format!(
            "`{name}` is {used} in this index while an element of `{name}` is being \
             {element_verb}; store the index with `let` first, as in `let i = ...;`, and index \
             with that"
        );
        let diagnostic = Diagnostic::new(codes::V0306, span, message)
            .with_label(element, format!("this element of `{name}` is {label}"))
            .with_note(format!(
                "in Rust terms, `{name}` is borrowed{mutably} to reach the element before the \
                 index is worked out"
            ));
        self.diagnostics.push(diagnostic);
    }

    /// V0300 or V0301 at `span`, a place rooted at `root` that is changed
    /// but is not a mutable place because `blamed` lacks `mut`.
    fn cannot_change(&self, span: Span, root: LocalId, blamed: LocalId) -> Diagnostic {
        if self.patterns[blamed.0 as usize].is_some() {
            let diagnostic = self.binding_unchangeable(codes::V0301, span, blamed, "");
            // `root` is a `let mut` alias of `blamed` (a pattern or `for`
            // binding), so add the same `.clone()` note V0300 gives: the
            // assignment blames `blamed`, but `root` is the name the code
            // actually changed.
            return match &self.alias_notes[root.0 as usize] {
                Some(note) if root != blamed => diagnostic.with_note(note.clone()),
                _ => diagnostic,
            };
        }
        let diagnostic = if self.is_param(blamed) {
            let param = &self.local(blamed).name;
            Diagnostic::new(
                codes::V0300,
                span,
                format!(
                    "this function only reads `{param}`; to change it, add `mut` to the parameter"
                ),
            )
        } else {
            let binding = &self.local(blamed).name;
            Diagnostic::new(
                codes::V0301,
                span,
                format!("`{binding}` cannot be changed because it was declared without `mut`"),
            )
        };
        self.add_mut_fix_it(diagnostic, root, blamed)
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
        if diagnostic.code == codes::V0300 {
            if let Some(note) = &self.alias_notes[root.0 as usize] {
                diagnostic = diagnostic.with_note(note.clone());
            }
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
            let owner = if info.origin.is_none() && through_index(leaf) {
                "the `Vec` this value is part of is gone after this line".to_string()
            } else {
                owner_text(self.cx, self.locals, info.origin)
            };
            let advice = self
                .match_on_the_call(leaf)
                .unwrap_or_else(|| what_to_do(self.cx, self.locals, info.origin));
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
                Slot::Payload(variant) => format!("put inside `{variant}`"),
                Slot::VecElement => "put in a `vec!`".to_string(),
                Slot::Question => "used up by `?`".to_string(),
            };
            let diagnostic = Diagnostic::new(
                codes::V0304,
                leaf.span,
                format!("{owner}, so it cannot be {into}"),
            )
            .with_note(advice)
            .with_note(BORROWED_NOTE);
            self.diagnostics
                .push(clone_fix_it(diagnostic, leaf.span, &leaf.ty));
        }
    }
}

/// Adds the fix-it `.clone()` after the value at `span` to a V0304 when
/// the value (of type `ty`) is a `string`: the one deliberate copy (spec
/// 3.3, 3.5). Nothing else has a `clone`.
fn clone_fix_it(diagnostic: Diagnostic, span: Span, ty: &Ty) -> Diagnostic {
    if *ty != Ty::String {
        return diagnostic;
    }
    let end = Span::new(span.file, span.end, span.end);
    diagnostic.with_fix_it(FixIt {
        span: end,
        replacement: ".clone()".to_string(),
    })
}

mod aliases;
mod moves;
mod patterns;
mod strings;
#[cfg(test)]
mod tests;
