//! Borrowed returns (M4 spec 3.1): what each function returns, worked
//! out before the bodies are analysed, over the call graph.
//!
//! A function's returns are its tail and every `return` operand, each
//! read through `if`s, blocks, and `match`es to its leaves
//! ([`ReturnLeaf`]). A leaf is new (an owned or Copy value, a literal) or
//! part of the locals it is rooted at, found through `let` aliases by
//! [`alias_roots`]. When every leaf is new the function returns an owned
//! value; when the leaves that are parts are all part of one read-only
//! parameter, and none is new, it has a borrowed return rooted there
//! ([`Classification::Part`]); anything else is an error, reported once.
//!
//! [`classify`] builds the call graph of every Varyk function and walks
//! its strongly connected components callees first. Before a component's
//! bodies are analysed, every call they make to a function already
//! classified as a borrowed return is marked `rooted` at the argument in
//! the rooted position, so the first pass (`places`) sees the result as a
//! place of that argument. A call inside the component cannot be
//! classified first, so it counts as new, and a function of a recursive
//! group (one that calls itself, or a component of more than one
//! function) never has a borrowed return: a return of it that is part of
//! a parameter is V0304.

use varyk_syntax::Span;

use super::slots::clone_fix_it;
use super::{BORROWED_NOTE, Context, GONE_NOTE, PlacesOutput, places, push_unique};
use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{
    HirBlock, HirExpr, HirExprKind, HirForHead, HirFunction, HirStmt, LocalId, LocalInfo, MethodRef,
};
use crate::resolve::{Callee, ImportedSig};
use crate::types::Ty;

/// One value a function or a closure body may return: a leaf of its tail
/// or of a `return` operand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReturnLeaf {
    pub(super) span: Span,
    /// The locals the leaf is rooted at, before following aliases: those
    /// of a borrowed place, or, for a new leaf that is a `string` `let`,
    /// that `let`, which may have been assigned parts of other values.
    pub(super) roots: Vec<LocalId>,
    /// Not a borrowed place: an owned or Copy value, or a literal.
    pub(super) new: bool,
    /// A string literal.
    pub(super) literal: bool,
}

/// What a function returns (M4 spec 3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Classification {
    /// Every return is new: an owned value.
    New,
    /// A borrowed return, part of this parameter.
    Part(LocalId),
    /// A return that is part of a parameter (the first span) beside one
    /// that is new (the second): V0304.
    Mixed(Span, Span),
    /// Returns that are parts of two different parameters: V0308.
    TwoRoots(LocalId, LocalId),
    /// A return that is part of a local, gone when the function returns:
    /// V0304.
    OfLocal(LocalId),
    /// A return that is part of a `mut` parameter: V0304.
    OfMutParam(LocalId),
    /// A return that is a borrowed place with no root, such as the result
    /// of a borrowed-return call on a literal: V0304.
    Rootless(Span),
    /// A return that is part of a parameter, in a recursive group: V0304.
    Recursive(Span),
}

/// One function's classification, with what its first pass found.
pub(super) struct FnClassified {
    pub(super) class: Classification,
    pub(super) places: PlacesOutput,
    /// The diagnostics about its returns: the return owned-slot V0304s of
    /// a function classified `New`, or the one V0304 or V0308 of a
    /// function in error; nothing for a borrowed return.
    pub(super) buffered: Vec<Diagnostic>,
    /// The locals `buffered` reports a flow of (see
    /// `PlacesOutput::reported`).
    pub(super) marks: Vec<LocalId>,
    /// Everything else the first pass reported, in source order.
    pub(super) diagnostics: Vec<Diagnostic>,
}

/// Classifies every function of `functions` (indexed by `FnId`), filling
/// in [`HirFunction::ret_root`] and the `rooted` of every call to a
/// borrowed return, and runs the first pass on each body; returns each
/// function's result, indexed by `FnId`.
pub(super) fn classify(cx: &Context, functions: &mut [HirFunction]) -> Vec<FnClassified> {
    let graph: Vec<Vec<usize>> = functions
        .iter_mut()
        .map(|function| {
            let mut callees = Vec::new();
            walk_block(&mut function.body, &mut |expr| {
                if let Some(id) = varyk_callee(expr) {
                    if !callees.contains(&id) {
                        callees.push(id);
                    }
                }
            });
            callees
        })
        .collect();
    // Per function: the parameter position its borrowed return is rooted
    // at, for its callers.
    let mut ret_roots: Vec<Option<usize>> = vec![None; functions.len()];
    let mut results: Vec<Option<FnClassified>> = functions.iter().map(|_| None).collect();
    for component in components(&graph) {
        let in_group = component.len() > 1 || graph[component[0]].contains(&component[0]);
        for &id in &component {
            let function = &mut functions[id];
            walk_block(&mut function.body, &mut |expr| {
                mark_rooted(expr, &ret_roots, &component, cx.imported)
            });
        }
        for &id in &component {
            let function = &mut functions[id];
            let result = classify_function(cx, function, in_group);
            if let Classification::Part(root) = result.class {
                ret_roots[id] = Some(root.0 as usize);
            }
            results[id] = Some(result);
        }
    }
    results.into_iter().flatten().collect()
}

/// Runs the first pass on `function` and classifies its returns.
fn classify_function(cx: &Context, function: &mut HirFunction, in_group: bool) -> FnClassified {
    let mut diagnostics = Vec::new();
    let (places, returns) = places(cx, function, &mut diagnostics);
    let params = &function.params;
    let alias_roots = super::aliases::alias_roots(
        &function.locals,
        params.len(),
        &places.refers,
        &places.assigned,
        &[],
        &[],
    );
    // A parameter of a Copy type roots nothing: Rust passes it by value,
    // so a part of it ends when the function returns, like a local's.
    let allowed: Vec<LocalId> = params
        .iter()
        .filter(|p| !function.locals[p.local.0 as usize].ty.is_copy())
        .map(|p| p.local)
        .collect();
    let mutable: Vec<LocalId> = params
        .iter()
        .filter(|p| function.locals[p.local.0 as usize].mutable)
        .map(|p| p.local)
        .collect();
    let class = classify_body(
        cx,
        &returns.leaves,
        &alias_roots,
        &function.locals,
        &allowed,
        &mutable,
        in_group,
    );
    let roots = Roots {
        alias_roots: &alias_roots,
        locals: &function.locals,
        allowed: &allowed,
    };
    let leaf_root = |span: Span| {
        returns
            .leaves
            .iter()
            .find(|leaf| leaf.span == span)
            .and_then(|leaf| roots.terminal(leaf).first().copied())
    };
    function.ret_root = match class {
        Classification::Part(root)
        | Classification::OfMutParam(root)
        | Classification::TwoRoots(root, _) => Some(root),
        Classification::Mixed(part, _) | Classification::Recursive(part) => leaf_root(part),
        Classification::New | Classification::OfLocal(_) | Classification::Rootless(_) => None,
    };
    let (buffered, marks) = match class {
        Classification::Part(_) => (Vec::new(), Vec::new()),
        Classification::New => (returns.buffered, returns.marks),
        _ => {
            let mut marks = returns.marks;
            for leaf in &returns.leaves {
                for &root in &leaf.roots {
                    push_unique(&mut marks, root);
                }
            }
            let report = Report {
                function,
                leaves: &returns.leaves,
                roots: &roots,
                buffered: &returns.buffered,
            };
            (report.diagnostic(class).into_iter().collect(), marks)
        }
    };
    FnClassified {
        class,
        places,
        buffered,
        marks,
        diagnostics,
    }
}

/// Classifies a body from its return leaves (M4 spec 3.1): `alias_roots`
/// as [`alias_roots`] gives them for its locals, `allowed_roots` the
/// locals a borrowed return may be part of (a function's parameters),
/// `mut_params` those of them that may be changed, and `in_group` whether
/// the body belongs to a recursive group.
///
/// [`alias_roots`]: super::aliases::alias_roots
pub(super) fn classify_body(
    cx: &Context,
    leaves: &[ReturnLeaf],
    alias_roots: &[Vec<LocalId>],
    locals: &[LocalInfo],
    allowed_roots: &[LocalId],
    mut_params: &[LocalId],
    in_group: bool,
) -> Classification {
    let _ = cx;
    let roots = Roots {
        alias_roots,
        locals,
        allowed: allowed_roots,
    };
    let mut new = None;
    let mut parts: Vec<(Span, Vec<LocalId>)> = Vec::new();
    for leaf in leaves {
        let terminal = roots.terminal(leaf);
        if leaf.new && terminal.is_empty() {
            new.get_or_insert(leaf.span);
            continue;
        }
        if terminal.is_empty() {
            return Classification::Rootless(leaf.span);
        }
        for &root in &terminal {
            if mut_params.contains(&root) {
                return Classification::OfMutParam(root);
            }
            if !allowed_roots.contains(&root) {
                return Classification::OfLocal(root);
            }
        }
        parts.push((leaf.span, terminal));
    }
    let Some((part, first)) = parts
        .first()
        .and_then(|(span, roots)| Some((*span, *roots.first()?)))
    else {
        return Classification::New;
    };
    // In a recursive group the calls inside it count as new, so the group
    // is the reason for any part, before a mix or a second root.
    if in_group {
        return Classification::Recursive(part);
    }
    if let Some(new) = new {
        return Classification::Mixed(part, new);
    }
    let other = parts
        .iter()
        .flat_map(|(_, roots)| roots)
        .find(|&&root| root != first);
    if let Some(&other) = other {
        return Classification::TwoRoots(first, other);
    }
    Classification::Part(first)
}

/// Finds the locals a return leaf is part of, through aliases.
struct Roots<'a> {
    alias_roots: &'a [Vec<LocalId>],
    locals: &'a [LocalInfo],
    allowed: &'a [LocalId],
}

impl Roots<'_> {
    /// The terminal roots of `leaf`: the locals it is part of that are not
    /// themselves other names for something, intermediate aliases dropped.
    /// A binding with no root of its own (an alias of a temporary, or of a
    /// literal through a borrowed-return call) contributes nothing, and a
    /// new leaf contributes only what was assigned into it.
    fn terminal(&self, leaf: &ReturnLeaf) -> Vec<LocalId> {
        let mut out = Vec::new();
        for &root in &leaf.roots {
            let direct = &self.alias_roots[root.0 as usize];
            if direct.is_empty() {
                if !leaf.new && !self.rootless_alias(root) {
                    push_unique(&mut out, root);
                }
                continue;
            }
            for &inner in direct {
                if self.alias_roots[inner.0 as usize].is_empty() && !self.rootless_alias(inner) {
                    push_unique(&mut out, inner);
                }
            }
        }
        out
    }

    /// A borrowed local that is not an allowed root and has no roots: it
    /// refers to nothing a caller could keep.
    fn rootless_alias(&self, local: LocalId) -> bool {
        self.locals[local.0 as usize].place.borrowed && !self.allowed.contains(&local)
    }
}

/// A closure's return leaves with what finds their roots, for the
/// diagnostic of a closure of `Option::map` or `Result::map_err`, which
/// must return something new (M4 spec 3.2).
pub(super) struct ClosureRoots<'a> {
    pub(super) leaves: &'a [ReturnLeaf],
    pub(super) alias_roots: &'a [Vec<LocalId>],
    pub(super) locals: &'a [LocalInfo],
    /// The closure's parameter, then its captures.
    pub(super) allowed: &'a [LocalId],
    pub(super) param: LocalId,
    /// The whole closure.
    pub(super) span: Span,
    /// The closure of a chain's `map`, which may return a part, of one
    /// thing: its parameter or one captured name.
    pub(super) chain: bool,
}

impl ClosureRoots<'_> {
    fn roots(&self) -> Roots<'_> {
        Roots {
            alias_roots: self.alias_roots,
            locals: self.locals,
            allowed: self.allowed,
        }
    }

    fn name(&self, local: LocalId) -> &str {
        &self.locals[local.0 as usize].name
    }

    /// The first leaf that is part of `root`, or the whole closure.
    fn leaf_of(&self, root: LocalId) -> Span {
        let roots = self.roots();
        self.leaves
            .iter()
            .find(|leaf| roots.terminal(leaf).contains(&root))
            .map_or(self.span, |leaf| leaf.span)
    }

    /// The V0304 or V0308 of a closure whose returns are classified
    /// `class`, its body of type `ty`; `None` when it returns something
    /// new.
    pub(super) fn diagnostic(&self, class: Classification, ty: &Ty) -> Option<Diagnostic> {
        let copy = if *ty == Ty::String {
            "return a copy instead, with `.clone()`"
        } else {
            "return a new value instead"
        };
        // What a closure may return: something new, or, for a chain's
        // `map`, parts of one thing.
        let rule = if self.chain {
            "every value the closure returns must be new, or part of the same one thing"
        } else {
            "the closure must return something new"
        };
        let (span, diagnostic) = match class {
            Classification::New => return None,
            Classification::Part(root)
            | Classification::OfLocal(root)
            | Classification::OfMutParam(root) => {
                let span = self.leaf_of(root);
                (span, self.part(root, span, copy))
            }
            Classification::Mixed(part, new) => {
                let root = self
                    .leaves
                    .iter()
                    .find(|leaf| leaf.span == part)
                    .and_then(|leaf| self.roots().terminal(leaf).first().copied());
                let root = root.map_or("something else".to_string(), |r| {
                    format!("`{}`", self.name(r))
                });
                let diagnostic = Diagnostic::new(
                    codes::V0304,
                    part,
                    format!(
                        "this closure returns part of {root} here and something new elsewhere, \
                         so this cannot be returned as it is"
                    ),
                )
                .with_label(new, "this return is new")
                .with_note(format!("{rule}; {copy}"))
                .with_note(BORROWED_NOTE);
                (part, diagnostic)
            }
            Classification::TwoRoots(a, b) => {
                let (at_a, at_b) = (self.leaf_of(a), self.leaf_of(b));
                let (a, b) = (self.name(a), self.name(b));
                let mut diagnostic = Diagnostic::new(
                    codes::V0308,
                    at_b,
                    format!(
                        "this closure returns part of `{a}` in one place and part of `{b}` in \
                         another"
                    ),
                );
                if at_a != at_b {
                    diagnostic =
                        diagnostic.with_label(at_a, format!("part of `{a}` is returned here"));
                }
                if self.chain {
                    // A copy in one place leaves a part beside something
                    // new: no one fix-it mends it.
                    let copies = if *ty == Ty::String {
                        "return copies in both places, with `.clone()`"
                    } else {
                        "return new values in both places"
                    };
                    return Some(diagnostic.with_note(format!("{rule}; {copies}")).with_note(
                        "in Rust terms, the items would borrow from two values, which \
                                 one lifetime cannot name",
                    ));
                }
                let diagnostic = diagnostic
                    .with_note(format!("{rule}; {copy}"))
                    .with_note(BORROWED_NOTE);
                (at_b, diagnostic)
            }
            Classification::Rootless(span) | Classification::Recursive(span) => {
                let diagnostic = Diagnostic::new(
                    codes::V0304,
                    span,
                    "this value is part of something the closure does not keep, so it cannot be \
                     returned from it",
                )
                .with_note(copy)
                .with_note(BORROWED_NOTE);
                (span, diagnostic)
            }
        };
        Some(clone_fix_it(diagnostic, span, ty))
    }

    /// The V0304 of a closure returning, at `span`, part of `root`: its
    /// parameter, a name it captures, or one of its own locals.
    fn part(&self, root: LocalId, span: Span, copy: &str) -> Diagnostic {
        let name = self.name(root);
        if root == self.param {
            return Diagnostic::new(
                codes::V0304,
                span,
                format!(
                    "this is part of `{name}`, the closure's parameter, which ends when the \
                     closure returns, so it cannot be returned"
                ),
            )
            .with_note(copy.to_string())
            .with_note(GONE_NOTE);
        }
        if self.allowed.contains(&root) {
            return Diagnostic::new(
                codes::V0304,
                span,
                format!(
                    "`{name}` belongs to the function around this closure, so the closure cannot \
                     return it"
                ),
            )
            .with_note(format!(
                "the `Option` or `Result` this call makes must own what it holds; {copy}"
            ))
            .with_note(BORROWED_NOTE);
        }
        Diagnostic::new(
            codes::V0304,
            span,
            format!(
                "this is part of `{name}`, which ends when the closure returns, so it cannot be \
                 returned"
            ),
        )
        .with_note(copy.to_string())
        .with_note(GONE_NOTE)
    }
}

/// What the first pass records about a body's returns.
pub(super) struct Returns {
    pub(super) leaves: Vec<ReturnLeaf>,
    /// The V0304s the owned-slot check of the returns reported.
    pub(super) buffered: Vec<Diagnostic>,
    /// The locals those V0304s report a flow of.
    pub(super) marks: Vec<LocalId>,
}

/// The one diagnostic of a function whose returns are in error.
struct Report<'a> {
    function: &'a HirFunction,
    leaves: &'a [ReturnLeaf],
    roots: &'a Roots<'a>,
    buffered: &'a [Diagnostic],
}

impl Report<'_> {
    fn name(&self, local: LocalId) -> &str {
        &self.function.locals[local.0 as usize].name
    }

    /// The first leaf that is part of `root`.
    fn leaf_of(&self, root: LocalId) -> Span {
        self.leaves
            .iter()
            .find(|leaf| self.roots.terminal(leaf).contains(&root))
            .map_or(self.function.span, |leaf| leaf.span)
    }

    /// The parameter the leaf at `span` is part of.
    fn root_at(&self, span: Span) -> Option<LocalId> {
        self.leaves
            .iter()
            .find(|leaf| leaf.span == span)
            .and_then(|leaf| self.roots.terminal(leaf).first().copied())
    }

    /// The diagnostic of `class`; `None` for a class that is no error.
    fn diagnostic(&self, class: Classification) -> Option<Diagnostic> {
        let ret = &self.function.ret;
        // For text, `.clone()` makes the copy.
        let copy = if *ret == Ty::String {
            "return a copy instead, with `.clone()`"
        } else {
            "return a new value instead"
        };
        let diagnostic = match class {
            Classification::Mixed(part, new) => {
                let root = self
                    .root_at(part)
                    .map_or("a parameter".to_string(), |r| format!("`{}`", self.name(r)));
                let literal = self
                    .leaves
                    .iter()
                    .any(|leaf| leaf.span == new && leaf.literal);
                let mut diagnostic = Diagnostic::new(
                    codes::V0304,
                    part,
                    format!(
                        "this function returns part of {root} here and something new elsewhere, \
                         so this cannot be returned as it is"
                    ),
                )
                .with_label(new, "this return is new");
                if literal {
                    diagnostic = diagnostic
                        .with_note("a string literal a function returns is new text, as if copied");
                }
                let diagnostic = diagnostic
                    .with_note(format!(
                        "every return must be new, or every return part of the same parameter; \
                         {copy}"
                    ))
                    .with_note(
                        "in Rust terms, a function returns either an owned value or a reference, \
                         not one or the other",
                    );
                clone_fix_it(diagnostic, part, ret)
            }
            Classification::TwoRoots(a, b) => {
                let (at_a, at_b) = (self.leaf_of(a), self.leaf_of(b));
                let (a, b) = (self.name(a), self.name(b));
                let mut diagnostic = Diagnostic::new(
                    codes::V0308,
                    at_b,
                    format!(
                        "this function returns part of `{a}` in one place and part of `{b}` in \
                         another"
                    ),
                );
                if at_a != at_b {
                    diagnostic =
                        diagnostic.with_label(at_a, format!("part of `{a}` is returned here"));
                }
                diagnostic
                    .with_note(format!(
                        "a function may return part of only one of its parameters; {one}, or make \
                         two functions",
                        one = if *ret == Ty::String {
                            "return a copy in one of the places, with `.clone()`"
                        } else {
                            "return a new value in one of the places"
                        }
                    ))
                    .with_note(
                        "in Rust terms, the returned reference must borrow from one parameter, \
                         named by its lifetime",
                    )
            }
            Classification::OfLocal(local) => {
                let span = self.leaf_of(local);
                let name = self.name(local);
                let diagnostic = Diagnostic::new(
                    codes::V0304,
                    span,
                    format!(
                        "this is part of `{name}`, which ends when this function returns, so it \
                         cannot be returned"
                    ),
                )
                .with_note(copy)
                .with_note(GONE_NOTE);
                clone_fix_it(diagnostic, span, ret)
            }
            Classification::OfMutParam(param) => {
                let span = self.leaf_of(param);
                let name = self.name(param);
                let diagnostic = Diagnostic::new(
                    codes::V0304,
                    span,
                    format!(
                        "this is part of `{name}`, which this function may change, so it cannot \
                         be returned; make `{name}` read-only, or {copy}"
                    ),
                )
                .with_note(format!(
                    "the caller could not even read what it passed as `{name}` while the result \
                     is used"
                ))
                .with_note(format!(
                    "in Rust terms, the result would keep the mutable borrow of `{name}` alive \
                     for as long as it is used"
                ));
                clone_fix_it(diagnostic, span, ret)
            }
            Classification::Rootless(span) => {
                if let Some(found) = self.buffered.iter().find(|d| d.span == span) {
                    return Some(found.clone());
                }
                let diagnostic = Diagnostic::new(
                    codes::V0304,
                    span,
                    "this value is part of something this function does not keep, so it cannot \
                     be returned from here",
                )
                .with_note(copy)
                .with_note(BORROWED_NOTE);
                clone_fix_it(diagnostic, span, ret)
            }
            Classification::Recursive(span) => {
                let root = self
                    .root_at(span)
                    .map_or("a parameter".to_string(), |r| format!("`{}`", self.name(r)));
                let diagnostic = Diagnostic::new(
                    codes::V0304,
                    span,
                    format!(
                        "`{}` calls itself, directly or through other functions, so it cannot \
                         return part of {root}; {copy}",
                        self.function.name
                    ),
                )
                .with_note(
                    "Varyk works out what a function returns before the functions that call it, \
                     which it cannot do for functions that call each other",
                );
                clone_fix_it(diagnostic, span, ret)
            }
            Classification::New | Classification::Part(_) => return None,
        };
        Some(diagnostic)
    }
}

/// The Varyk function `expr` calls, if it is such a call.
fn varyk_callee(expr: &HirExpr) -> Option<usize> {
    match &expr.kind {
        HirExprKind::Call {
            callee: Callee::Varyk(id),
            ..
        }
        | HirExprKind::MethodCall {
            method: MethodRef::Varyk(id),
            ..
        } => Some(id.0 as usize),
        _ => None,
    }
}

/// Marks `expr`, a call of a function with a borrowed return outside
/// `component`, `rooted` at the argument in the rooted position (the
/// receiver, position 0, for a method: its `self`).
fn mark_rooted(
    expr: &mut HirExpr,
    ret_roots: &[Option<usize>],
    component: &[usize],
    imported: &[ImportedSig],
) {
    // A call of an imported function is rooted where its signature says.
    if let HirExprKind::Call {
        callee: Callee::Imported(id),
        rooted,
        ..
    }
    | HirExprKind::MethodCall {
        method: MethodRef::Imported(id),
        rooted,
        ..
    } = &mut expr.kind
    {
        *rooted = imported[id.0 as usize].ret_root;
        return;
    }
    let Some(id) = varyk_callee(expr) else {
        return;
    };
    if component.contains(&id) {
        return;
    }
    match &mut expr.kind {
        HirExprKind::Call { rooted, .. } | HirExprKind::MethodCall { rooted, .. } => {
            *rooted = ret_roots[id];
        }
        _ => {}
    }
}

/// The strongly connected components of `graph` (edges from a function
/// to its callees), callees first: every component comes after the
/// components it calls into (Tarjan's algorithm).
fn components(graph: &[Vec<usize>]) -> Vec<Vec<usize>> {
    struct State<'a> {
        graph: &'a [Vec<usize>],
        index: Vec<Option<usize>>,
        low: Vec<usize>,
        on_stack: Vec<bool>,
        stack: Vec<usize>,
        next: usize,
        out: Vec<Vec<usize>>,
    }
    fn visit(state: &mut State, node: usize) {
        state.index[node] = Some(state.next);
        state.low[node] = state.next;
        state.next += 1;
        state.stack.push(node);
        state.on_stack[node] = true;
        for &callee in &state.graph[node] {
            match state.index[callee] {
                None => {
                    visit(state, callee);
                    state.low[node] = state.low[node].min(state.low[callee]);
                }
                Some(index) if state.on_stack[callee] => {
                    state.low[node] = state.low[node].min(index);
                }
                Some(_) => {}
            }
        }
        if state.index[node] == Some(state.low[node]) {
            let mut component = Vec::new();
            while let Some(member) = state.stack.pop() {
                state.on_stack[member] = false;
                component.push(member);
                if member == node {
                    break;
                }
            }
            component.sort_unstable();
            state.out.push(component);
        }
    }
    let count = graph.len();
    let mut state = State {
        graph,
        index: vec![None; count],
        low: vec![0; count],
        on_stack: vec![false; count],
        stack: Vec::new(),
        next: 0,
        out: Vec::new(),
    };
    for node in 0..count {
        if state.index[node].is_none() {
            visit(&mut state, node);
        }
    }
    state.out
}

/// Calls `visit` on every expression of `block`, inner ones first.
pub(super) fn walk_block(block: &mut HirBlock, visit: &mut impl FnMut(&mut HirExpr)) {
    for stmt in &mut block.stmts {
        walk_stmt(stmt, visit);
    }
    if let Some(tail) = &mut block.tail {
        walk_expr(tail, visit);
    }
}

fn walk_stmt(stmt: &mut HirStmt, visit: &mut impl FnMut(&mut HirExpr)) {
    match stmt {
        HirStmt::Let { value, .. } => walk_expr(value, visit),
        HirStmt::Assign { target, value, .. } => {
            walk_expr(target, visit);
            walk_expr(value, visit);
        }
        HirStmt::Expr { expr, .. } => walk_expr(expr, visit),
        HirStmt::Return { value, .. } => {
            if let Some(value) = value {
                walk_expr(value, visit);
            }
        }
        HirStmt::While { cond, body, .. } => {
            walk_expr(cond, visit);
            walk_block(body, visit);
        }
        HirStmt::WhileLet { value, body, .. } => {
            walk_expr(value, visit);
            walk_block(body, visit);
        }
        HirStmt::For { head, body, .. } => {
            match head {
                HirForHead::Range { start, end, .. } => {
                    walk_expr(start, visit);
                    walk_expr(end, visit);
                }
                HirForHead::Vec(head) | HirForHead::Chain(head) => walk_expr(head, visit),
            }
            walk_block(body, visit);
        }
        HirStmt::Break { .. } | HirStmt::Continue { .. } => {}
    }
}

fn walk_expr(expr: &mut HirExpr, visit: &mut impl FnMut(&mut HirExpr)) {
    match &mut expr.kind {
        HirExprKind::Int(_)
        | HirExprKind::Float(_)
        | HirExprKind::Bool(_)
        | HirExprKind::String(_)
        | HirExprKind::Local(_) => {}
        HirExprKind::Call { args, .. }
        | HirExprKind::VecLit(args)
        | HirExprKind::EnumLit { args, .. }
        | HirExprKind::Println { args, .. }
        | HirExprKind::Format { args, .. } => {
            for arg in args {
                walk_expr(arg, visit);
            }
        }
        HirExprKind::MethodCall { receiver, args, .. } => {
            walk_expr(receiver, visit);
            for arg in args {
                walk_expr(arg, visit);
            }
        }
        HirExprKind::Field { base, .. } => walk_expr(base, visit),
        HirExprKind::Index { base, index } => {
            walk_expr(base, visit);
            walk_expr(index, visit);
        }
        HirExprKind::StructLit { fields, .. } => {
            for (_, value) in fields {
                walk_expr(value, visit);
            }
        }
        HirExprKind::Unary { operand, .. }
        | HirExprKind::Cast { expr: operand, .. }
        | HirExprKind::Try { operand, .. } => walk_expr(operand, visit),
        HirExprKind::Binary { lhs, rhs, .. } => {
            walk_expr(lhs, visit);
            walk_expr(rhs, visit);
        }
        HirExprKind::Block(block) => walk_block(block, visit),
        HirExprKind::If { cond, then, else_ } => {
            walk_expr(cond, visit);
            walk_block(then, visit);
            if let Some(else_) = else_ {
                walk_block(else_, visit);
            }
        }
        HirExprKind::IfLet {
            value, then, else_, ..
        } => {
            walk_expr(value, visit);
            walk_block(then, visit);
            if let Some(else_) = else_ {
                walk_block(else_, visit);
            }
        }
        HirExprKind::Match {
            scrutinee, arms, ..
        } => {
            walk_expr(scrutinee, visit);
            for arm in arms {
                walk_expr(&mut arm.body, visit);
            }
        }
        HirExprKind::Closure { body, .. } => walk_block(body, visit),
    }
    visit(expr);
}

#[cfg(test)]
mod tests;
