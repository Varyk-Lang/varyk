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

use std::collections::{HashMap, HashSet};

use varyk_syntax::{FixIt, SourceFile, Span};

use crate::builtins::{BuiltinId, Owner, Receiver};
use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{
    HirBlock, HirEnum, HirExpr, HirExprKind, HirForHead, HirFunction, HirPattern, HirProgram,
    HirStmt, HirStruct, LocalId, LocalInfo, MethodRef, Origin, PlaceInfo, StringRepr, VariantRef,
    declared_inside, field_root, is_block_like, is_place, leaves, rooted_argument,
};
use crate::resolve::{Callee, ImportedSig, StructId};
use crate::types::{ParamMode, Ty};
use chains::{ItemKind, Items, chain_row};
use patterns::{Body, Bound};
use returns::{Classification, FnClassified, ReturnLeaf, Returns};
use slots::{BORROWED_NOTE, Slot, clone_fix_it};

/// The fix for an `if`, block, or `match` that cannot be read in place or
/// stored with `let`.
const NEW_BRANCHES: &str =
    "make every branch give a new value instead (for text or `Bytes`, with `.clone()`)";

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
///
/// `sources` are the program's files, indexed by `FileId.0`, for a fix-it
/// that rewrites a piece of them.
pub fn analyze(hir: HirProgram, sources: &[SourceFile]) -> Result<HirProgram, Vec<Diagnostic>> {
    let (hir, diagnostics) = analyze_unchecked(hir, sources);
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
pub fn analyze_unchecked(
    mut hir: HirProgram,
    sources: &[SourceFile],
) -> (HirProgram, Vec<Diagnostic>) {
    let signatures = signatures(&hir);
    let cx = Context {
        signatures: &signatures,
        imported: &hir.imported,
        structs: &hir.structs,
        enums: &hir.enums,
        sources,
    };
    let classified = returns::classify(&cx, &mut hir.functions);
    let mut diagnostics = Vec::new();
    for (function, classified) in hir.functions.iter_mut().zip(classified) {
        analyze_function(&cx, function, classified, &mut diagnostics);
    }
    (hir, diagnostics)
}

/// Every Varyk function's name and parameter modes, indexed by `FnId`.
fn signatures(hir: &HirProgram) -> Vec<(String, Vec<ParamMode>)> {
    hir.functions()
        .map(|f| (f.name.clone(), f.params.iter().map(|p| p.mode).collect()))
        .collect()
}

/// The [`PlaceInfo`] of a place expression (a `Local`, or a chain of
/// `Field`s and `Index`es on one), read from `locals` as [`analyze`]
/// annotated them, or of a call with `rooted`, a place of the argument it
/// borrows from; `None` for any other expression, which is a temporary.
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
            // Everything reached through a `Shared` is read-only
            // (milestone 5b1 spec 2.6).
            let mutable = mutable && !matches!(base.ty, Ty::Shared(_));
            let borrowed = !expr.ty.is_copy();
            let origin = match (&expr.kind, base.ty.reached()) {
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
        // A call with `rooted` is part of that argument, as an element is
        // of its `Vec` (M4 spec 2.8), and never changeable through: `let
        // mut n = user.name_ref()` is not a mutable alias.
        _ => rooted_argument(expr).map(|argument| {
            let borrowed = !expr.ty.is_copy();
            let origin = match place_info(locals, argument) {
                _ if !borrowed => None,
                Some(info) if info.borrowed => info.origin,
                _ => place_root(argument).map(Origin::Local),
            };
            PlaceInfo {
                borrowed,
                mutable: false,
                origin,
            }
        }),
    }
}

/// Whether `local` is a capture where the analysis is: declared outside
/// the innermost of `closures`, the spans of the closures enclosing it,
/// outermost first (M4 spec 3.2). Every pass decides captures with this.
fn captured_in(locals: &[LocalInfo], closures: &[Span], local: LocalId) -> bool {
    closures
        .last()
        .is_some_and(|&closure| !declared_inside(&locals[local.0 as usize], closure))
}

/// [`place_info`] of `expr` where the analysis is, inside `closures` (see
/// [`captured_in`]): a place rooted at a name the closure around it
/// captures is read-only and, unless Copy, borrowed from the function
/// around the closure, whatever it is outside (M4 spec 3.2); `None` for a
/// temporary.
fn place_in(locals: &[LocalInfo], closures: &[Span], expr: &HirExpr) -> Option<PlaceInfo> {
    let info = place_info(locals, expr)?;
    match place_root(expr) {
        Some(root) if captured_in(locals, closures, root) => {
            let borrowed = !expr.ty.is_copy();
            Some(PlaceInfo {
                borrowed,
                mutable: false,
                origin: borrowed.then_some(Origin::Captured(root)),
            })
        }
        _ => Some(info),
    }
}

/// Program-wide facts a function's analysis reads.
struct Context<'a> {
    /// Every Varyk function's name and parameter modes, indexed by `FnId`.
    signatures: &'a [(String, Vec<ParamMode>)],
    imported: &'a [ImportedSig],
    structs: &'a [HirStruct],
    enums: &'a [HirEnum],
    /// The program's files, indexed by `FileId.0`.
    sources: &'a [SourceFile],
}

/// The modes a call passes its arguments by, with `keeps` as
/// [`Context::callee`] gives it: for a started call (milestone 5b1 spec
/// 3), every argument, a method's receiver included, is an owned slot the
/// task keeps.
fn started_modes(started: bool, modes: Vec<ParamMode>, keeps: bool) -> (Vec<ParamMode>, bool) {
    if started {
        (vec![ParamMode::Owned; modes.len()], true)
    } else {
        (modes, keeps)
    }
}

/// `modes` followed by a shared borrow for each of `trailing`: a value
/// passed after the other arguments is read, never moved, whether the
/// call is started or not (milestone 5b3 spec 2.2, 3).
fn with_trailing(mut modes: Vec<ParamMode>, trailing: &[HirExpr]) -> Vec<ParamMode> {
    modes.extend(trailing.iter().map(|_| ParamMode::SharedBorrow));
    modes
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
                (sig.name.clone(), sig.modes(), true)
            }
            Callee::Builtin(id) => (id.path(), id.modes(), true),
        }
    }

    /// A method's name and parameter modes, its receiver's first, and
    /// whether an `Owned` parameter is an owned slot (as for an imported
    /// callee): true for the built-in table, whose `push` keeps its
    /// argument, and for an imported method, false for a Varyk method,
    /// whose `Owned` parameters are Copy.
    fn method(&self, method: MethodRef) -> (String, Vec<ParamMode>, bool) {
        match method {
            MethodRef::Varyk(id) => self.callee(Callee::Varyk(id)),
            MethodRef::Imported(id) => self.callee(Callee::Imported(id)),
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
                format!("{}::{}", e.name, e.variants[index].name)
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
            let name = cx.struct_name(id);
            format!("this value is kept inside {} `{name}`", article(name))
        }
        Some(Origin::Local(local)) => {
            format!(
                "this value is kept inside `{}`",
                locals[local.0 as usize].name
            )
        }
        Some(Origin::Captured(local)) => format!(
            "`{}` belongs to the function around this closure",
            locals[local.0 as usize].name
        ),
        None => "this value belongs to someone else".to_string(),
    }
}

/// `an` before a name that starts with `A`, `E`, `I`, or `O` (`an Acc`),
/// else `a`: a type name starting with `U` mostly sounds like "you" (`a
/// User`, `a Unit`).
pub(crate) fn article(name: &str) -> &'static str {
    if name.starts_with(['A', 'E', 'I', 'O', 'a', 'e', 'i', 'o']) {
        "an"
    } else {
        "a"
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
        Some(Origin::Captured(local)) => {
            let name = &locals[local.0 as usize].name;
            format!(
                "inside a closure a name from outside can only be read; make a new value here \
                 instead of keeping `{name}`"
            )
        }
        None => "make a new value here instead".to_string(),
    }
}

/// The locals the argument of a call with `rooted`, or the receiver of a
/// looked-into row, is rooted at (M4 spec 3.1, 2.8): it must be a place,
/// the result of another call with `rooted`, or a string literal, which
/// lives for the whole program and has no root. Anything else (a call
/// whose result is owned, a struct or enum value, an `if`, a block) is
/// V0001: Varyk only lets the result point into a stored value (Rust
/// drops a temporary at the end of its statement, too soon for a `let`).
#[expect(
    clippy::result_large_err,
    reason = "a single diagnostic on a cold path, as `resolve::resolve_path`"
)]
fn borrowed_receiver(cx: &Context, expr: &HirExpr) -> Result<Vec<LocalId>, Diagnostic> {
    match &expr.kind {
        HirExprKind::String(_) => Ok(Vec::new()),
        HirExprKind::Local(_) | HirExprKind::Field { .. } | HirExprKind::Index { .. }
            if is_place(expr) =>
        {
            Ok(place_root(expr).into_iter().collect())
        }
        _ => match rooted_argument(expr) {
            Some(argument) => borrowed_receiver(cx, argument),
            None => {
                let (what, kind) = match &expr.ty {
                    Ty::Struct(id) => {
                        let name = cx.struct_name(*id);
                        (
                            format!("this `{name}`"),
                            format!("{} `{name}`", article(name)),
                        )
                    }
                    Ty::String => ("this text".to_string(), "text".to_string()),
                    _ => ("this value".to_string(), "a value".to_string()),
                };
                Err(Diagnostic::new(
                    codes::V0001,
                    expr.span,
                    format!(
                        "{what} is made right here, but this call needs {kind} stored in a name, \
                         because its result is part of it; store it with `let` first, then use \
                         that"
                    ),
                )
                .with_note(
                    "in Rust terms, the result is a reference into the value the call is made \
                     on, and Varyk only lets it point into a value stored in a `let`, not into \
                     a temporary one",
                ))
            }
        },
    }
}

/// The local a place expression (or a call with `rooted`) is rooted at;
/// `None` for a temporary or a field or element of one, and for a field or
/// element of an `if` or block.
fn place_root(expr: &HirExpr) -> Option<LocalId> {
    match &expr.kind {
        HirExprKind::Local(id) => Some(*id),
        HirExprKind::Field { base, .. } | HirExprKind::Index { base, .. } => place_root(base),
        _ => rooted_argument(expr).and_then(place_root),
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
                "this value is kept inside {} `{name}` that is not stored anywhere, so \
                 {consequence}; store the `{name}` with `let` first",
                article(name)
            );
        }
    }
    if !is_block_like(whole) {
        return format!("this value is gone after this line, so {consequence}");
    }
    format!("this value only exists inside this block, so {consequence}")
}

/// Whether `local` is bound by the pattern of a `match` arm or an `if
/// let` that `expr` may evaluate through (see [`leaves`]).
fn arm_binds(expr: &HirExpr, local: LocalId) -> bool {
    let tail = |block: &HirBlock| block.tail.as_deref().is_some_and(|t| arm_binds(t, local));
    match &expr.kind {
        HirExprKind::Block(block) => tail(block),
        HirExprKind::If { then, else_, .. } => tail(then) || else_.as_ref().is_some_and(tail),
        HirExprKind::IfLet {
            pattern,
            then,
            else_,
            ..
        } => {
            pattern.bindings().iter().any(|(bound, _)| *bound == local)
                || tail(then)
                || else_.as_ref().is_some_and(tail)
        }
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

/// Runs the passes after the first on `function`, whose first pass and
/// return classification `classified` holds, appending every diagnostic
/// of the function to `diagnostics` in order.
fn analyze_function(
    cx: &Context,
    function: &mut HirFunction,
    classified: FnClassified,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let start = diagnostics.len();
    let FnClassified {
        class,
        places,
        buffered,
        marks,
        diagnostics: first,
    } = classified;
    let PlacesOutput {
        mut reported,
        refers,
        assigned,
        held,
        head_reads,
        dropped,
        never_given_away,
        items,
    } = places;
    diagnostics.extend(first);
    diagnostics.extend(buffered);
    for local in marks {
        reported[local.0 as usize] = true;
    }
    strings::infer(
        cx,
        function,
        &class,
        &reported,
        &refers,
        &assigned,
        &dropped,
        &items,
        diagnostics,
    );
    moves::check(cx, function, &never_given_away, diagnostics);
    let loops = aliases::Loops {
        held: &held,
        head_reads: &head_reads,
    };
    aliases::check(cx, function, &refers, &assigned, loops, diagnostics);
    patterns::merge_arm_fix_its(cx.sources, &mut diagnostics[start..]);
}

/// What the first pass ([`places`]) finds, per local.
pub(super) struct PlacesOutput {
    /// A V0304 was reported for a flow of the local.
    reported: Vec<bool>,
    /// What it may refer to.
    refers: Refers,
    /// The roots of the borrowed places assigned to it (see [`Assigned`]).
    assigned: Assigned,
    /// It is the variable of a `for` that holds the place it loops over.
    held: Vec<bool>,
    /// For the variable of a `for` over a chain, every other local the
    /// loop's head reads until the loop ends: the source's receiver and
    /// argument, and the captures of its closures (M4 spec 3.3); empty for
    /// every other local.
    pub(super) head_reads: Vec<Vec<LocalId>>,
    /// For a name bound inside a temporary of an enum with a destructor,
    /// why it cannot be kept.
    dropped: Vec<Option<String>>,
    /// The parameters of `any` and `all` closures over owned items: owned
    /// locals of the closure that only look at the item, so never given
    /// away (M4 spec 3.3).
    pub(super) never_given_away: HashSet<LocalId>,
    /// The items of each chain call that makes a chain, by its span, with
    /// their representation when they are text (M4 spec 3.3, 3.5).
    pub(super) items: HashMap<Span, (ItemKind, Option<StringRepr>)>,
}

/// The first pass: place info, mutability contracts, and owned slots.
/// Returns what it found about the function's locals and chains (see
/// [`PlacesOutput`]) and what the function returns (see [`Returns`]).
fn places(
    cx: &Context,
    function: &mut HirFunction,
    diagnostics: &mut Vec<Diagnostic>,
) -> (PlacesOutput, Returns) {
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
    let call_results = vec![false; locals.len()];
    let through_shared = vec![false; locals.len()];
    let made_by = vec![None; locals.len()];
    let made_from = vec![None; locals.len()];
    let head_reads = vec![Vec::new(); locals.len()];
    let reported = vec![false; locals.len()];
    let gone = vec![false; locals.len()];
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
        gone,
        alias_notes,
        refers,
        assigned,
        patterns,
        held,
        call_results,
        through_shared,
        made_by,
        made_from,
        head_reads,
        closures: Vec::new(),
        returns: Returns {
            leaves: Vec::new(),
            buffered: Vec::new(),
            marks: Vec::new(),
        },
        items: HashMap::new(),
        finds: HashSet::new(),
        parts: HashSet::new(),
        never_given_away: HashSet::new(),
        diagnostics,
    };
    analyzer.block(body);
    if *ret != Ty::Unit {
        if let Some(tail) = &body.tail {
            analyzer.returned(tail);
        }
    }
    let dropped = (0..analyzer.locals.len())
        .map(|id| analyzer.dropped_advice(LocalId(id as u32)))
        .collect();
    // What the walk decided about chains, recorded in the HIR for the
    // passes after it and the backend.
    let (finds, parts) = (&analyzer.finds, &analyzer.parts);
    returns::walk_block(body, &mut |expr| match &mut expr.kind {
        HirExprKind::MethodCall { looked_into, .. } if finds.contains(&expr.span) => {
            *looked_into = true;
        }
        HirExprKind::Closure { returns_part, .. } if parts.contains(&expr.span) => {
            *returns_part = true;
        }
        _ => {}
    });
    let places = PlacesOutput {
        reported: analyzer.reported,
        refers: analyzer.refers,
        assigned: analyzer.assigned,
        held: analyzer.held,
        head_reads: analyzer.head_reads,
        dropped,
        never_given_away: analyzer.never_given_away,
        items: analyzer
            .items
            .into_iter()
            .map(|(span, items)| (span, (items.kind, items.repr)))
            .collect(),
    };
    (places, analyzer.returns)
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
    /// Per `let`: a V0304 said its value is gone before the `let` is used,
    /// so a later use of it says nothing more.
    gone: Vec<bool>,
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
    /// Per `let`: bound to the result of a call returning part of a value
    /// (M4 spec 3.1), which no `mut` can make changeable.
    call_results: Vec<bool>,
    /// Per `let`: another name for something reached through a `Shared`
    /// (milestone 5b1 spec 2.6), which no `mut` can make changeable.
    through_shared: Vec<bool>,
    /// Per `let`: what made its value when it is a `match`, `if`, or
    /// block, as V0304's note names it: an article and a noun.
    made_by: Vec<Option<(&'static str, &'static str)>>,
    /// Per `let mut`: the element or field it is another name for, as
    /// written (`names[0]`), and whether it is an element, for V0304's
    /// wording when a borrowed value is assigned to it.
    made_from: Vec<Option<(String, bool)>>,
    /// See [`PlacesOutput::head_reads`].
    head_reads: Vec<Vec<LocalId>>,
    /// The closures whose bodies enclose the expression being analysed,
    /// outermost first, each by the span of the whole closure: a local
    /// declared outside the innermost one is a capture there (M4 spec
    /// 3.2).
    closures: Vec<Span>,
    /// The function's returns, for its classification (M4 spec 3.1).
    returns: Returns,
    /// The items of each chain call that makes a chain, by its span (M4
    /// spec 3.3).
    items: HashMap<Span, Items>,
    /// The `find`s on borrowed items, by span: looked into (M4 spec 2.8).
    finds: HashSet<Span>,
    /// The closures of `map`s whose value is part of something, by span.
    parts: HashSet<Span>,
    /// See [`PlacesOutput::never_given_away`].
    never_given_away: HashSet<LocalId>,
    diagnostics: &'a mut Vec<Diagnostic>,
}

impl FnAnalyzer<'_> {
    /// `value`, the tail or a `return` operand of a function returning a
    /// value: records its leaves, and runs the return owned-slot check
    /// into [`Returns::buffered`], which only a function whose returns are
    /// all new reports (M4 spec 3.1).
    fn returned(&mut self, value: &HirExpr) {
        for leaf in leaves(value) {
            let leaf = self.return_leaf(leaf);
            self.returns.leaves.push(leaf);
        }
        let start = self.diagnostics.len();
        let before = self.reported.clone();
        self.owned_slot(value, &Slot::Return);
        let reported: Vec<Diagnostic> = self.diagnostics.drain(start..).collect();
        self.returns.buffered.extend(reported);
        for (index, was) in before.into_iter().enumerate() {
            if self.reported[index] && !was {
                self.reported[index] = false;
                push_unique(&mut self.returns.marks, LocalId(index as u32));
            }
        }
    }

    /// `leaf`, a value a function or a closure returns, as its
    /// classification sees it (M4 spec 3.1).
    fn return_leaf(&self, leaf: &HirExpr) -> ReturnLeaf {
        let root = place_root(leaf);
        let borrowed = self.info(leaf).borrowed
            && !leaf.ty.is_copy()
            && !self.looked_into(leaf)
            // Already said: what it holds is gone before this.
            && !root.is_some_and(|root| self.gone[root.0 as usize]);
        let roots = match leaf.kind {
            _ if borrowed => roots(leaf),
            // A `let` holding text may have been assigned parts.
            HirExprKind::Local(id) if leaf.ty == Ty::String => vec![id],
            _ => Vec::new(),
        };
        ReturnLeaf {
            span: leaf.span,
            roots,
            new: !borrowed,
            literal: matches!(leaf.kind, HirExprKind::String(_)),
        }
    }

    /// `closure`, an argument of the built-in `row` (`None` for another
    /// call), analysed as any expression unless it is a closure. A
    /// closure's parameter is bound (handed `items` when `row` is a row of
    /// a chain), its body analysed with every name from outside it
    /// read-only (M4 spec 3.2), and what it returns classified by row: a
    /// closure of `Option::map` or `Result::map_err` must return something
    /// new (M4 spec 3.1); one of a chain's `map` may also return part of
    /// its parameter, when the items are borrowed, or of a captured name;
    /// those of `filter`, `any`, `all`, and `find` give a `bool`. Returns
    /// the classification of a chain's `map` closure when it is not in
    /// error.
    fn argument(
        &mut self,
        closure: &HirExpr,
        row: Option<BuiltinId>,
        items: Option<&Items>,
    ) -> Option<Classification> {
        let HirExprKind::Closure {
            param,
            captures,
            body,
            ..
        } = &closure.kind
        else {
            self.expr(closure);
            return None;
        };
        self.closures.push(closure.span);
        match (row, items) {
            (Some(row), Some(items)) => self.chain_param(*param, row.get(), items, body),
            _ => self.closure_param(*param, body, None),
        }
        self.block(body);
        let tail = body.tail.as_deref();
        let leaves: Vec<ReturnLeaf> = tail.map_or_else(Vec::new, |tail| {
            leaves(tail)
                .into_iter()
                .map(|leaf| self.return_leaf(leaf))
                .collect()
        });
        self.closures.pop();
        let (Some(tail), Some(row)) = (tail, row) else {
            return None;
        };
        let entry = row.get();
        let chain_map = entry.owner == Owner::Chain && entry.name == "map";
        // The other closures of a chain give a `bool`.
        if entry.owner == Owner::Chain && !chain_map {
            return None;
        }
        // Over items that are not borrowed, the parameter is a local of the
        // closure like any other.
        let borrowed_items = items.is_some_and(|items| matches!(items.kind, ItemKind::Borrowed(_)));
        let mut allowed = Vec::new();
        if !chain_map || borrowed_items {
            allowed.push(*param);
        }
        allowed.extend(captures);
        let alias_roots = aliases::alias_roots(
            self.locals,
            self.param_count,
            &self.refers,
            &self.assigned,
            &[],
            &allowed,
        );
        let class = returns::classify_body(
            self.cx,
            &leaves,
            &alias_roots,
            self.locals,
            &allowed,
            &[],
            false,
        );
        let closure_roots = returns::ClosureRoots {
            leaves: &leaves,
            alias_roots: &alias_roots,
            locals: self.locals,
            allowed: &allowed,
            param: *param,
            span: closure.span,
            chain: chain_map,
        };
        let diagnostic = match class {
            Classification::Part(_) if chain_map => None,
            _ => closure_roots.diagnostic(class, &tail.ty),
        };
        let Some(diagnostic) = diagnostic else {
            return chain_map.then_some(class);
        };
        self.diagnostics.push(diagnostic);
        for leaf in &leaves {
            for root in &leaf.roots {
                self.reported[root.0 as usize] = true;
            }
        }
        None
    }

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
                        self.returned(value);
                    }
                }
            }
            HirStmt::While { cond, body, .. } => {
                self.expr(cond);
                self.block(body);
            }
            HirStmt::WhileLet {
                pattern,
                value,
                body,
                head_is_str,
                ..
            } => {
                self.head(value, &[pattern]);
                self.pattern(value, pattern, Body::Block(body), *head_is_str);
                self.block(body);
            }
            HirStmt::For {
                local, head, body, ..
            } => {
                match head {
                    HirForHead::Range { start, end, .. } => {
                        self.expr(start);
                        self.expr(end);
                    }
                    HirForHead::Vec(head) | HirForHead::Chain(head) => self.expr(head),
                }
                self.for_variable(*local, head, body);
                self.block(body);
            }
            HirStmt::Break { .. } | HirStmt::Continue { .. } => {}
            HirStmt::Route(route) => self.added(route.app, route.method.call(), route.span),
            HirStmt::Hook(hook) => self.added(hook.app, hook.call(), hook.span),
        }
    }

    /// A route or hook call, `call` on the app local `app` at `span`,
    /// changes its app as a `mut self` method does (milestone 5b4 spec
    /// 7.4); its handler or hook is not a value, and borrows nothing.
    fn added(&mut self, app: LocalId, call: &str, span: Span) {
        let app = HirExpr {
            kind: HirExprKind::Local(app),
            ty: self.local(app).ty.clone(),
            span,
        };
        let modes = [ParamMode::MutableBorrow];
        self.call(call, &modes, false, &[&app], None);
    }

    fn expr(&mut self, expr: &HirExpr) {
        match &expr.kind {
            HirExprKind::Int(_)
            | HirExprKind::Float(_)
            | HirExprKind::Bool(_)
            | HirExprKind::String(_)
            | HirExprKind::Local(_) => {}
            HirExprKind::Call {
                callee,
                args,
                trailing,
                started,
                ..
            } => {
                for arg in args.iter().chain(trailing) {
                    self.expr(arg);
                }
                let (name, modes, keeps) = self.cx.callee(*callee);
                if *started {
                    let args: Vec<&HirExpr> = args.iter().collect();
                    self.started_call(&name, &modes, &args);
                    return self.trailing_values(&name, trailing);
                }
                let args: Vec<&HirExpr> = args.iter().chain(trailing).collect();
                self.call(&name, &with_trailing(modes, trailing), keeps, &args, None);
                self.rooted_argument_stored(expr);
            }
            HirExprKind::MethodCall { .. } => self.method_value(expr, None),
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
            // The awaited call's arguments are passed as any call's are.
            HirExprKind::Unary { operand, .. }
            | HirExprKind::Cast { expr: operand, .. }
            | HirExprKind::Assert { cond: operand, .. }
            | HirExprKind::Await(operand) => {
                self.expr(operand);
            }
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
            HirExprKind::EnumLit { variant, args, .. } => {
                let slot = Slot::Payload(self.cx.variant_name(*variant));
                for arg in args {
                    self.expr(arg);
                    self.owned_slot(arg, &slot);
                }
            }
            HirExprKind::Try { operand, .. } => {
                self.expr(operand);
                self.owned_slot(operand, &Slot::Question);
                // A `?` returns early with a new `None` or `Err` (M4 spec
                // 3.1); closures cannot hold one.
                if self.closures.is_empty() {
                    self.returns.leaves.push(ReturnLeaf {
                        span: expr.span,
                        roots: Vec::new(),
                        new: true,
                        literal: false,
                    });
                }
            }
            HirExprKind::VecLit(elements) => {
                for element in elements {
                    self.expr(element);
                    self.owned_slot(element, &Slot::VecElement);
                }
            }
            HirExprKind::IfLet {
                pattern,
                value,
                then,
                else_,
                head_is_str,
            } => {
                self.head(value, &[&**pattern]);
                self.pattern(value, pattern, Body::Block(then), *head_is_str);
                self.block(then);
                if let Some(else_) = else_ {
                    self.block(else_);
                }
            }
            HirExprKind::Match {
                scrutinee,
                arms,
                head_is_str,
            } => {
                let patterns: Vec<&HirPattern> = arms.iter().map(|arm| &arm.pattern).collect();
                self.head(scrutinee, &patterns);
                for arm in arms {
                    self.pattern(scrutinee, &arm.pattern, Body::Arm(&arm.body), *head_is_str);
                    self.expr(&arm.body);
                }
            }
            HirExprKind::Closure { .. } => {
                self.argument(expr, None, None);
            }
            HirExprKind::Println { args, .. }
            | HirExprKind::Log { args, .. }
            | HirExprKind::Format { args, .. } => {
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

    /// `call`, `receiver.method(args)`: the receiver is an argument passed
    /// as the method's `self`, or the table's receiver mode, says (spec
    /// 2.5). A call of a chain records its items (M4 spec 3.3), which a
    /// row of a chain hands its closure.
    fn method_call(&mut self, call: &HirExpr) {
        let HirExprKind::MethodCall {
            receiver,
            method,
            args,
            trailing,
            ..
        } = &call.kind
        else {
            return self.expr(call);
        };
        let (name, modes, keeps) = self.cx.method(*method);
        if let HirExprKind::MethodCall { .. } = receiver.kind {
            self.method_value(receiver, Some(&name));
        } else {
            self.expr(receiver);
        }
        let row = match method {
            MethodRef::Builtin(id) => Some(*id),
            _ => None,
        };
        let chain = chain_row(*method);
        let incoming = chain
            .filter(|entry| entry.owner == Owner::Chain)
            .map(|_| self.items_of(receiver));
        let mut mapped = None;
        for arg in args {
            if let Some(class) = self.argument(arg, row, incoming.as_ref()) {
                mapped = Some(class);
            }
        }
        for value in trailing {
            self.expr(value);
        }
        let values: Vec<&HirExpr> = std::iter::once(&**receiver).chain(args).collect();
        // A `mut self` method of the struct a `Shared` holds would change
        // it (milestone 5b1 spec 2.6).
        if matches!(receiver.ty, Ty::Shared(_)) && modes.first() == Some(&ParamMode::MutableBorrow)
        {
            let diagnostic = self.shared_unchangeable(receiver, &name);
            self.diagnostics.push(diagnostic);
            return;
        }
        if let HirExprKind::MethodCall { started: true, .. } = call.kind {
            self.started_call(&name, &modes, &values);
            return self.trailing_values(&name, trailing);
        }
        let values: Vec<&HirExpr> = values.into_iter().chain(trailing).collect();
        let modes = with_trailing(modes, trailing);
        self.call(&name, &modes, keeps, &values, Some(*method));
        if let Some(entry) = chain {
            self.chain_call(call, receiver, entry, incoming, mapped);
        }
    }

    /// `call`, a method call used as a value, or as the receiver of the
    /// method `called` (for V0208's wording).
    fn method_value(&mut self, call: &HirExpr, called: Option<&str>) {
        self.method_call(call);
        // Only a head may hold a looked-into result (see [`Self::head`]).
        if self.looked_into(call) {
            self.looked_into_here(call, None, called);
        } else {
            self.rooted_argument_stored(call);
        }
    }

    /// The head of a `match`, `if let`, or `while let`, matched by
    /// `patterns`: the one place a looked-into result may be (M4 spec
    /// 2.8). Its receiver must be a place, so that the names bound inside
    /// it have a root (V0001), and no pattern may name it whole (V0208).
    fn head(&mut self, value: &HirExpr, patterns: &[&HirPattern]) {
        let HirExprKind::MethodCall { receiver, .. } = &value.kind else {
            return self.expr(value);
        };
        self.method_call(value);
        if !self.looked_into(value) {
            return self.rooted_argument_stored(value);
        }
        // A `find`'s items have their roots from the chain's source.
        let found = self.found_items(value).is_some();
        if !found && borrowed_receiver(self.cx, receiver).is_err() {
            let diagnostic = Diagnostic::new(
                codes::V0001,
                receiver.span,
                "this value is made right here, but looking inside the result of this call \
                 needs a value stored in a name, because the names the pattern makes are parts \
                 of it; store it with `let` first, then look inside that",
            )
            .with_note(
                "the names a pattern binds inside this `Option` are other names for parts of \
                 the value it is called on, and Varyk only lets them name parts of a value \
                 stored in a `let`",
            );
            self.diagnostics.push(diagnostic);
            return;
        }
        let whole = patterns
            .iter()
            .flat_map(|pattern| pattern.bindings())
            .find(|(_, whole)| *whole);
        if let Some((local, _)) = whole {
            self.looked_into_here(value, Some(local), None);
        }
    }

    /// V0001 when `call` has `rooted` and the argument it borrows from is
    /// not stored anywhere (see [`borrowed_receiver`]).
    fn rooted_argument_stored(&mut self, call: &HirExpr) {
        if let Some(argument) = rooted_argument(call) {
            if let Err(diagnostic) = borrowed_receiver(self.cx, argument) {
                self.diagnostics.push(diagnostic);
            }
        }
    }

    /// V0208 for `value`, a looked-into result anywhere but a head, or
    /// named whole there by `binding`, or with the method `called` called
    /// on it (M4 spec 2.8).
    fn looked_into_here(
        &mut self,
        value: &HirExpr,
        binding: Option<LocalId>,
        called: Option<&str>,
    ) {
        let found = self
            .found_items(value)
            .and_then(|items| items.roots().first().copied());
        let part = match rooted_argument(value).and_then(place_root).or(found) {
            Some(root) => format!("part of `{}`", self.local(root).name),
            None => "part of the value it is called on".to_string(),
        };
        let (span, message) = match binding {
            Some(local) => {
                let info = self.local(local);
                (
                    info.span,
                    format!(
                        "`{}` would be this `Option` itself, which holds {part} and must be \
                         used where it is made; look inside it with a pattern such as \
                         `Some(x)`, in `match` or `if let`",
                        info.name
                    ),
                )
            }
            None => match called {
                Some(method) => (
                    value.span,
                    format!(
                        "this `Option` holds {part}, so no method can be called on it, \
                         `{method}` included; look inside it here instead, with \
                         `if let Some(x) = ... {{ .. }}` or `match`"
                    ),
                ),
                None => (
                    value.span,
                    format!(
                        "this `Option` holds {part} and must be used where it is made; look \
                         inside it here with `match` or `if let`"
                    ),
                ),
            },
        };
        let diagnostic = Diagnostic::new(codes::V0208, span, message).with_note(
            "in Rust terms, this is an `Option<&T>`, a reference inside an `Option`, which \
             Varyk has no type for",
        );
        self.diagnostics.push(diagnostic);
    }

    // --- Places ---------------------------------------------------------------

    /// The place info of `expr`: a place rooted at a name captured by the
    /// closure around it is read-only and, unless Copy, borrowed from the
    /// function around the closure, whatever it is outside (M4 spec 3.2).
    fn info(&self, expr: &HirExpr) -> PlaceInfo {
        place_in(self.locals, &self.closures, expr).unwrap_or(TEMPORARY)
    }

    /// Whether `local` is a capture where the analysis is: declared
    /// outside the innermost closure around it (M4 spec 3.2).
    fn captured(&self, local: LocalId) -> bool {
        captured_in(self.locals, &self.closures, local)
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
                if !self.local(root).place.borrowed && !self.captured(root) =>
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
        self.made_by[local.0 as usize] = match value.kind {
            HirExprKind::Match { .. } => Some(("a", "`match`")),
            HirExprKind::If { .. } => Some(("an", "`if`")),
            HirExprKind::IfLet { .. } => Some(("an", "`if let`")),
            HirExprKind::Block(_) => Some(("a", "block")),
            _ => None,
        };
        let mut borrowed = None;
        let mut shared = None;
        self.refers[local.0 as usize] = (
            leaves(value).into_iter().any(|leaf| {
                !matches!(field_root(leaf).kind, HirExprKind::String(_))
                    && place_root(leaf).is_none()
            }),
            roots(value),
        );
        for leaf in leaves(value) {
            let info = self.info(leaf);
            if info.borrowed && borrowed.is_none() {
                borrowed = Some(self.origin(leaf, info));
            }
            if !info.mutable && shared.is_none() {
                // A call's result, or a part of it, is read-only whatever
                // the argument is: the `let` itself is at fault, not the
                // argument's `mut`.
                let mut part = leaf;
                while let HirExprKind::Field { base, .. } | HirExprKind::Index { base, .. } =
                    &part.kind
                {
                    part = base;
                }
                if rooted_argument(part).is_some() {
                    self.call_results[local.0 as usize] = true;
                    shared = Some(Some(local));
                } else {
                    shared = Some(self.blame(leaf));
                }
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
        self.through_shared[local.0 as usize] = place.borrowed
            && leaves(value)
                .into_iter()
                .any(|leaf| self.reaches_shared(leaf));
        if place.borrowed && declared_mut && !place.mutable && value.ty == Ty::String {
            self.alias_notes[local.0 as usize] = alias_note(self.locals, local, value);
        }
        if place.borrowed && place.mutable {
            if let HirExprKind::Index { .. } | HirExprKind::Field { .. } = value.kind {
                self.made_from[local.0 as usize] = Some((
                    place_shape_text(self.locals, value),
                    matches!(value.kind, HirExprKind::Index { .. }),
                ));
            }
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
                    // Part of a local declared inside.
                    _ if rooted_argument(leaf).is_some() => true,
                    HirExprKind::Field { .. } | HirExprKind::Index { .. } => {
                        place_root(leaf).is_some() || as_str
                    }
                    _ => false,
                },
                Leaf::Unsure | Leaf::Dangling => true,
            };
            if gone {
                let name = &self.local(local).name;
                // The same name inside and outside: say which one.
                let outer = if place_root(leaf).is_some_and(|root| self.local(root).name == *name) {
                    "the outer "
                } else {
                    ""
                };
                let consequence = format!("it cannot be kept in {outer}`{name}`");
                let message = gone_message(self.cx, self.locals, leaf, value, &consequence);
                let mut diagnostic = Diagnostic::new(codes::V0304, leaf.span, message);
                // A name bound inside a temporary with a destructor: say
                // why, and copy a `string` out with `.clone()`.
                if let Some(advice) = place_root(leaf).and_then(|root| self.dropped_advice(root)) {
                    diagnostic = clone_fix_it(diagnostic.with_note(advice), leaf.span, &leaf.ty);
                }
                self.diagnostics.push(diagnostic.with_note(GONE_NOTE));
                self.reported[local.0 as usize] = true;
                self.gone[local.0 as usize] = true;
            }
        }
    }

    // --- Checks ---------------------------------------------------------------

    fn assign(&mut self, target: &HirExpr, value: &HirExpr) {
        if self.index_uses_root(target) {
            return;
        }
        let info = self.info(target);
        if !info.mutable && self.reaches_shared(target) {
            let diagnostic = self.shared_unchangeable(target, "");
            self.diagnostics.push(diagnostic);
        } else if !info.mutable {
            let root = place_root(target).expect("an assignment target is a place");
            let blamed = self.blame(target).unwrap_or(root);
            let diagnostic = self.cannot_change(target.span, root, blamed);
            self.diagnostics.push(diagnostic);
        } else if info.borrowed {
            // Writing through a borrowed place stores into storage the
            // caller or a struct owns.
            let slot = match target.kind {
                HirExprKind::Local(id) => match &self.made_from[id.0 as usize] {
                    Some((from, element)) => Slot::Alias {
                        name: self.local(id).name.clone(),
                        from: from.clone(),
                        element: *element,
                    },
                    None => Slot::Assignment,
                },
                _ => Slot::Assignment,
            };
            self.owned_slot(value, &slot);
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
        let takes = matches!(method, Some(MethodRef::Builtin(id))
            if id.get().receiver == Receiver::Takes);
        for (index, (arg, mode)) in args.iter().zip(modes.iter().copied()).enumerate() {
            match mode {
                // A taking row's receiver (M4 spec 3.4): a stored one is
                // copied out when it is Copy in Rust, and otherwise it
                // cannot be taken.
                ParamMode::Owned if index == 0 && takes => {
                    if !arg.ty.copy_in_rust() {
                        self.owned_slot(arg, &Slot::Taken(name.to_string()));
                    }
                }
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

    /// The values passed after the other arguments of a started call of
    /// `name`: read before the task starts, as the arguments of a call
    /// that is not started are (milestone 5b3 spec 2.2).
    fn trailing_values(&mut self, name: &str, trailing: &[HirExpr]) {
        let values: Vec<&HirExpr> = trailing.iter().collect();
        let modes = with_trailing(Vec::new(), trailing);
        self.call(name, &modes, false, &values, None);
    }

    /// A started call of `name`, whose parameters take `args` by `modes`
    /// (milestone 5b1 spec 3): the task keeps every argument, a method's
    /// receiver first, so each is an owned slot (V0304), and none may go
    /// to a `mut` parameter, since the task would change only its own
    /// copy (V0309).
    fn started_call(&mut self, name: &str, modes: &[ParamMode], args: &[&HirExpr]) {
        for (arg, mode) in args.iter().zip(modes) {
            if *mode != ParamMode::MutableBorrow {
                self.owned_slot(arg, &Slot::Started(name.to_string()));
                continue;
            }
            self.diagnostics.push(
                Diagnostic::new(
                    codes::V0309,
                    arg.span,
                    format!(
                        "`{name}` changes what it is given here, but a started task changes \
                         only its own copy, which nobody would ever see"
                    ),
                )
                .with_note(format!(
                    "wait for the call with `.await` so that `{name}` changes this value, or \
                     have it return what it makes"
                )),
            );
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
            HirExprKind::Call {
                callee,
                args,
                trailing,
                started,
                ..
            } => {
                let (_, modes, keeps) = self.cx.callee(*callee);
                let (modes, keeps) = started_modes(*started, modes, keeps);
                let modes = with_trailing(modes, trailing);
                self.call_uses(args.iter().chain(trailing), &modes, keeps, out);
            }
            // The receiver of a changing method is changed (spec 2.5).
            HirExprKind::MethodCall {
                receiver,
                method,
                args,
                trailing,
                started,
                ..
            } => {
                let (_, modes, keeps) = self.cx.method(*method);
                let (modes, keeps) = started_modes(*started, modes, keeps);
                let modes = with_trailing(modes, trailing);
                let args = std::iter::once(&**receiver).chain(args).chain(trailing);
                self.call_uses(args, &modes, keeps, out);
            }
            HirExprKind::StructLit { fields, .. } => {
                for (_, value) in fields {
                    self.uses_on_the_way(value, false, !value.ty.is_copy(), out);
                }
            }
            HirExprKind::Unary { operand, .. }
            | HirExprKind::Cast { expr: operand, .. }
            | HirExprKind::Assert { cond: operand, .. } => {
                self.uses_on_the_way(operand, false, false, out);
            }
            HirExprKind::Binary { lhs, rhs, .. } => {
                self.uses_on_the_way(lhs, false, false, out);
                self.uses_on_the_way(rhs, false, false, out);
            }
            HirExprKind::Println { args, .. }
            | HirExprKind::Log { args, .. }
            | HirExprKind::Format { args, .. } => {
                for arg in args {
                    self.uses_on_the_way(arg, false, false, out);
                }
            }
            HirExprKind::Try { operand, .. } => self.uses_on_the_way(operand, false, true, out),
            HirExprKind::Await(operand) => self.uses_on_the_way(operand, at_leaf, exclusive, out),
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
            HirExprKind::IfLet {
                value, then, else_, ..
            } => {
                self.uses_on_the_way(value, false, false, out);
                self.block_uses(then, at_leaf, exclusive, out);
                if let Some(else_) = else_ {
                    self.block_uses(else_, at_leaf, exclusive, out);
                }
            }
            HirExprKind::Match {
                scrutinee, arms, ..
            } => {
                self.uses_on_the_way(scrutinee, false, false, out);
                for arm in arms {
                    self.uses_on_the_way(&arm.body, at_leaf, exclusive, out);
                }
            }
            // What a closure uses, it uses while the call runs; it can
            // only read a name from outside (M4 spec 3.2).
            HirExprKind::Closure { body, .. } => self.block_uses(body, false, false, out),
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
                HirStmt::While { cond, body, .. }
                | HirStmt::WhileLet {
                    value: cond, body, ..
                } => {
                    self.uses_on_the_way(cond, false, false, out);
                    self.block_uses(body, false, false, out);
                }
                HirStmt::For { head, body, .. } => {
                    match head {
                        HirForHead::Range { start, end, .. } => {
                            self.uses_on_the_way(start, false, false, out);
                            self.uses_on_the_way(end, false, false, out);
                        }
                        HirForHead::Vec(head) | HirForHead::Chain(head) => {
                            self.uses_on_the_way(head, false, false, out);
                        }
                    }
                    self.block_uses(body, false, false, out);
                }
                HirStmt::Break { .. } | HirStmt::Continue { .. } => {}
                HirStmt::Route(route) => out.push((route.app, true, route.span)),
                HirStmt::Hook(hook) => out.push((hook.app, true, hook.span)),
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
            // So is the result of a call with `rooted` on a local
            // declared inside `whole`: it is part of that local.
            Leaf::New
                if !leaf.ty.is_copy()
                    && (through_index(leaf) || rooted_argument(leaf).is_some()) =>
            {
                Leaf::Dangling
            }
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
            if self.reaches_shared(leaf) {
                let callee = if changed { "" } else { callee };
                let diagnostic = self.shared_unchangeable(leaf, callee);
                self.diagnostics.push(diagnostic);
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
            if self.captured(root) {
                let diagnostic = self.capture_unchangeable(codes::V0303, leaf.span, root, callee);
                self.diagnostics.push(diagnostic);
                continue;
            }
            if self.patterns[blamed.0 as usize].is_some() {
                let diagnostic =
                    self.binding_unchangeable(codes::V0303, leaf.span, blamed, root, callee);
                self.diagnostics.push(diagnostic);
                continue;
            }
            if let Some(diagnostic) = self.read_only_result(codes::V0302, leaf.span, blamed, callee)
            {
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
                "in Rust terms, `{name}` is borrowed{mutably} to get at the element before the \
                 index is worked out"
            ));
        self.diagnostics.push(diagnostic);
    }

    /// V0300 or V0301 at `span`, a place rooted at `root` that is changed
    /// but is not a mutable place because `blamed` lacks `mut`.
    fn cannot_change(&self, span: Span, root: LocalId, blamed: LocalId) -> Diagnostic {
        if self.captured(root) {
            return self.capture_unchangeable(codes::V0301, span, root, "");
        }
        if self.patterns[blamed.0 as usize].is_some() {
            let diagnostic = self.binding_unchangeable(codes::V0301, span, blamed, root, "");
            // `root` is a `let mut` alias of `blamed` (a pattern or `for`
            // binding), so add the same `.clone()` note V0300 gives: the
            // assignment blames `blamed`, but `root` is the name the code
            // actually changed.
            return match &self.alias_notes[root.0 as usize] {
                Some(note) if root != blamed => diagnostic.with_note(note.clone()),
                _ => diagnostic,
            };
        }
        if let Some(diagnostic) = self.read_only_result(codes::V0301, span, blamed, "") {
            return diagnostic;
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

    /// V0301 (an assignment or a changing method, `callee` empty) or V0303
    /// (an argument to a `mut` parameter of `callee`) at `span`, changing
    /// `root`, a name captured by the closure around `span`, which can
    /// only read it (M4 spec 2.2, 3.2).
    fn capture_unchangeable(
        &self,
        code: &'static str,
        span: Span,
        root: LocalId,
        callee: &str,
    ) -> Diagnostic {
        let info = self.local(root);
        let name = &info.name;
        let message = if callee.is_empty() {
            format!(
                "inside a closure a name from outside can only be read, so `{name}` cannot be \
                 changed here"
            )
        } else {
            format!(
                "`{callee}` may change `{name}`, but inside a closure a name from outside can \
                 only be read"
            )
        };
        Diagnostic::new(code, span, message)
            .with_label(info.span, format!("`{name}` is declared outside the closure"))
            .with_note("change it after the call the closure is given to, or make a new value inside the closure instead")
            .with_note(format!(
                "in Rust terms, the closure borrows `{name}` as a shared reference"
            ))
    }

    /// Whether `place` is reached through a `Shared` (milestone 5b1 spec
    /// 2.6): a field or element of one, or a `let` that is another name
    /// for such a place.
    fn reaches_shared(&self, place: &HirExpr) -> bool {
        match &place.kind {
            HirExprKind::Local(id) => self.through_shared[id.0 as usize],
            HirExprKind::Field { base, .. } | HirExprKind::Index { base, .. } => {
                matches!(base.ty, Ty::Shared(_)) || self.reaches_shared(base)
            }
            _ => rooted_argument(place).is_some_and(|argument| self.reaches_shared(argument)),
        }
    }

    /// V0310 at `place`, something reached through a `Shared` that an
    /// assignment or a changing method (`callee` empty) or a `mut`
    /// parameter or `mut self` of `callee` would change.
    fn shared_unchangeable(&self, place: &HirExpr, callee: &str) -> Diagnostic {
        // The handle itself, as the receiver of a `mut self` method of
        // the struct it holds.
        let whole = matches!(place.ty, Ty::Shared(_));
        let message = if whole {
            format!("`{callee}` may change the struct this `Shared` holds, which can only be read")
        } else if callee.is_empty() {
            "this is reached through a `Shared`, so it can only be read".to_string()
        } else {
            format!(
                "`{callee}` may change this, but it is reached through a `Shared`, so it can \
                 only be read"
            )
        };
        let mut diagnostic = Diagnostic::new(codes::V0310, place.span, message);
        if let Some(root) = place_root(place) {
            let info = self.local(root);
            let label = if matches!(info.ty, Ty::Shared(_)) {
                Some(format!("`{}` is a `Shared`", info.name))
            } else if self.through_shared[root.0 as usize] {
                Some(format!(
                    "`{}` is another name for part of a `Shared`",
                    info.name
                ))
            } else {
                None
            };
            if let Some(label) = label {
                diagnostic = diagnostic.with_label(info.span, label);
            }
        }
        let copy = if whole {
            ""
        } else if place.ty.is_copy() {
            "; to change a copy, put it in a `let mut` and change that"
        } else {
            "; to change a copy, write `.clone()` after this and change that"
        };
        diagnostic
            .with_note(format!(
                "a `Shared` lets many tasks read one value at once, so nothing reached through \
                 it may change{copy}"
            ))
            .with_note("in Rust terms, an `Arc<T>` gives only shared references to its `T`")
    }

    /// V0301 (an assignment, `callee` empty) or V0302 (an argument to a
    /// `mut` parameter of `callee`) at `span`, changing `blamed`, a `let`
    /// that is read-only whatever its `mut`: it holds the result of a call
    /// returning part of a value (M4 spec 3.1), which no `mut` can make
    /// changeable. `None` for any other local.
    fn read_only_result(
        &self,
        code: &'static str,
        span: Span,
        blamed: LocalId,
        callee: &str,
    ) -> Option<Diagnostic> {
        if !self.call_results[blamed.0 as usize] {
            return None;
        }
        let info = self.local(blamed);
        let name = &info.name;
        let part = match self.refers[blamed.0 as usize].1.first() {
            Some(root) => format!("part of `{}`", self.local(*root).name),
            None => "part of what a call was made on".to_string(),
        };
        let lead = if callee.is_empty() {
            String::new()
        } else {
            format!("`{callee}` may change `{name}`, but ")
        };
        let diagnostic = Diagnostic::new(
            code,
            span,
            format!(
                "{lead}`{name}` is another name for {part}, given by a call, and cannot be changed"
            ),
        )
        .with_label(info.span, format!("`{name}` is bound here"))
        .with_note("to change a copy of it instead, write `.clone()` after that call")
        .with_note(format!(
            "in Rust terms, `{name}` is a shared reference, whatever `mut` says about the name"
        ));
        Some(diagnostic)
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
}

mod aliases;
mod chains;
mod moves;
mod patterns;
pub(crate) mod returns;
mod slots;
mod strings;
#[cfg(test)]
mod tests;
