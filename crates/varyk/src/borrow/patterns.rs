//! Borrow analysis of the names `match` patterns and `for` loops bind
//! (spec 3.1, 3.2): their place info, and the wording of the diagnostics
//! about changing or keeping them.

use varyk_syntax::{FixIt, SourceFile, Span};

use super::{FnAnalyzer, place_root, roots};
use crate::diagnostics::Diagnostic;
use crate::hir::{
    HirArm, HirBlock, HirExpr, HirExprKind, HirForHead, LocalId, Origin, PlaceInfo, is_place,
    matched_in_place,
};
use crate::resolve::EnumId;
use crate::types::Ty;

/// What a `match` pattern binding or a `for` variable is bound to, for the
/// wording of the diagnostics about it (spec 3.1, 3.4).
#[derive(Debug, Clone, Copy)]
pub(super) struct Bound {
    /// Bound by a `for`, not a `match`.
    looped: bool,
    /// The local the scrutinee or the `Vec` looped over is rooted at;
    /// `None` for a temporary or a range.
    pub(super) root: Option<LocalId>,
    /// The binding names the whole scrutinee, which is that local itself.
    whole: bool,
    /// The scrutinee is a `let` that owns its value, as a whole.
    owned_local: bool,
    /// Just inside the arm's or loop's body when it is a block: where a
    /// `let mut n = n;` can go.
    body_start: Option<Span>,
    /// The arm's body when it is not a block: what a block holding a
    /// `let mut n = n;` and then it can replace.
    arm_body: Option<Span>,
    /// The name of the enum a `match` on a temporary looks inside because
    /// it runs code when it is thrown away (spec 3.4): the binding is part
    /// of it, and gone with it.
    dropped: Option<EnumId>,
}

impl FnAnalyzer<'_> {
    /// Computes the place info of the bindings of `arm`'s pattern (spec
    /// 3.2). On a place, a binding of a non-Copy value is a read-only alias
    /// rooted where the scrutinee is, and one of a Copy value is a copy; on
    /// a temporary, every binding owns its value. None can be changed.
    pub(super) fn pattern(&mut self, scrutinee: &HirExpr, arm: &HirArm) {
        let place = matched_in_place(self.cx.enums, scrutinee);
        let root = place_root(scrutinee);
        let origin = self.head_origin(scrutinee);
        let owned_local = matches!(scrutinee.kind, HirExprKind::Local(id)
            if !self.is_param(id) && !self.local(id).place.borrowed);
        let dropped = match scrutinee.ty {
            Ty::Enum(id) if place && !is_place(scrutinee) => Some(id),
            _ => None,
        };
        let (body_start, arm_body) = match &arm.body.kind {
            HirExprKind::Block(block) => {
                let at = block.span.start + 1;
                (Some(Span::new(block.span.file, at, at)), None)
            }
            _ => (None, Some(arm.body.span)),
        };
        for (local, whole) in arm.pattern.bindings() {
            let index = local.0 as usize;
            let alias = place && !self.local(local).ty.is_copy();
            self.locals[index].place = PlaceInfo {
                borrowed: alias,
                mutable: false,
                origin: if alias { origin } else { None },
            };
            self.blame[index] = Some(local);
            if alias {
                // An alias into a temporary (of an enum with a
                // destructor) is gone once the `match` ends.
                self.refers[index] = (!is_place(scrutinee), roots(scrutinee));
            }
            self.patterns[index] = Some(Bound {
                looped: false,
                root,
                whole: whole && matches!(scrutinee.kind, HirExprKind::Local(_)),
                owned_local,
                body_start,
                arm_body,
                dropped,
            });
        }
    }

    /// Computes the place info of `local`, the variable of a `for` over
    /// `head` with body `body` (spec 3.1, 3.2). Over a place, it is a
    /// read-only alias of one element rooted where the `Vec` is, or a copy
    /// of a Copy one, and the loop holds the `Vec` until it ends (`held`);
    /// over a temporary, it owns each element; over a range, it is a copy.
    /// It cannot be changed.
    pub(super) fn for_variable(&mut self, local: LocalId, head: &HirForHead, body: &HirBlock) {
        let index = local.0 as usize;
        let (root, alias, origin) = match head {
            HirForHead::Vec(vec) if is_place(vec) => {
                self.refers[index] = (false, roots(vec));
                self.held[index] = true;
                let alias = !self.local(local).ty.is_copy();
                (place_root(vec), alias, self.head_origin(vec))
            }
            HirForHead::Vec(_) | HirForHead::Range { .. } => (None, false, None),
        };
        self.locals[index].place = PlaceInfo {
            borrowed: alias,
            mutable: false,
            origin: if alias { origin } else { None },
        };
        self.blame[index] = Some(local);
        let at = body.span.start + 1;
        self.patterns[index] = Some(Bound {
            looped: true,
            root,
            whole: false,
            owned_local: false,
            body_start: Some(Span::new(body.span.file, at, at)),
            arm_body: None,
            dropped: None,
        });
    }

    /// What an alias of a part of `head`, a value matched on or looped
    /// over, derives from; `None` for a temporary.
    fn head_origin(&self, head: &HirExpr) -> Option<Origin> {
        if !is_place(head) {
            return None;
        }
        let info = self.info(head);
        self.origin(head, info)
            .or(place_root(head).map(Origin::Local))
    }

    /// V0301 (an assignment, `callee` empty) or V0303 (an argument to a
    /// `mut` parameter of `callee`) at `span`, changing `binding`, a name a
    /// `match` pattern or a `for` bound, which is read-only (spec 3.1): an
    /// alias says to change the original; a copy or an owned value gets
    /// the fix-it `let mut n = n;` at the start of its arm's or loop's
    /// block. `written` is the name changed at `span`: `binding` itself,
    /// or a `let` that took `binding`'s value.
    pub(super) fn binding_unchangeable(
        &self,
        code: &'static str,
        span: Span,
        binding: LocalId,
        written: LocalId,
        callee: &str,
    ) -> Diagnostic {
        let bound = self.patterns[binding.0 as usize].expect("a pattern binding");
        let info = self.local(binding);
        let name = &info.name;
        if let (Some(id), true) = (bound.dropped, info.place.borrowed) {
            return self.dropped_unchangeable(code, span, binding, written, callee, bound, id);
        }
        let lead = if callee.is_empty() {
            String::new()
        } else {
            format!("`{callee}` may change `{name}`, but ")
        };
        let (by, what, looked_at) = if bound.looped {
            ("for", "a `for` variable", "the `Vec` looped over")
        } else {
            (
                "match",
                "a name bound by a `match` pattern",
                "the value matched on",
            )
        };
        let diagnostic = match bound.root {
            Some(root_id) if info.place.borrowed => {
                let root = &self.local(root_id).name;
                let part = if bound.whole { "" } else { "part of " };
                // The parameter or `let` whose missing `mut` keeps `root`
                // from being changed, when there is one.
                let blamed = self.blame[root_id.0 as usize]
                    .filter(|blamed| self.patterns[blamed.0 as usize].is_none());
                let first = match blamed {
                    Some(blamed) => format!("mark `{}` `mut`, then ", self.local(blamed).name),
                    None => String::new(),
                };
                // A `for` variable, changed directly or passed to a `mut`
                // parameter or a changing method: also point at the usual
                // way around it, looping by index instead.
                let index_hint = if bound.looped {
                    format!(
                        ", for example by looping with `for i in 0..{root}.len()` and using \
                         `{root}[i]`"
                    )
                } else {
                    String::new()
                };
                let diagnostic = Diagnostic::new(
                    code,
                    span,
                    format!(
                        "{lead}`{name}` is another name for {part}`{root}`, given by `{by}`, \
                         and cannot be changed; {first}change `{root}` itself instead{index_hint}"
                    ),
                )
                .with_note(format!(
                    "{what} is read-only; in Rust terms, it is a shared reference into \
                     {looked_at}"
                ));
                match blamed {
                    Some(blamed) => self.add_mut_fix_it(diagnostic, root_id, blamed),
                    None => diagnostic,
                }
            }
            _ => {
                let mut diagnostic = Diagnostic::new(
                    code,
                    span,
                    format!(
                        "{lead}`{name}` is {what}, so it cannot be changed; to change it, \
                         first make a changeable copy with `let mut {name} = {name};`"
                    ),
                );
                if let Some(fix_it) = self.first_in_body(bound, format!("let mut {name} = {name};"))
                {
                    diagnostic = diagnostic.with_fix_it(fix_it);
                }
                diagnostic
            }
        };
        diagnostic.with_label(info.span, format!("`{name}` is bound here"))
    }

    /// [`Self::binding_unchangeable`] for `binding`, which stays inside a
    /// temporary of enum `id`, which runs code when it is thrown away: a
    /// `string` can be copied with `.clone()` and the copy changed; nothing
    /// else can be changed here at all.
    #[allow(clippy::too_many_arguments)]
    fn dropped_unchangeable(
        &self,
        code: &'static str,
        span: Span,
        binding: LocalId,
        written: LocalId,
        callee: &str,
        bound: Bound,
        id: EnumId,
    ) -> Diagnostic {
        let info = self.local(binding);
        let name = &info.name;
        let shown = &self.local(written).name;
        let lead = if callee.is_empty() {
            String::new()
        } else {
            format!("`{callee}` may change `{shown}`, but ")
        };
        let def = &self.cx.enums[id.0 as usize];
        let owner = &def.name;
        let runs = def.drops.as_ref().map_or("runs", |cause| cause.runs());
        let stays = format!(
            "{lead}`{shown}` stays inside `{owner}`, which {runs} code when it is thrown away, so \
             it cannot be changed here"
        );
        let mut diagnostic = if info.ty == Ty::String {
            let copy = format!("let mut {shown} = {name}.clone();");
            let mut diagnostic = Diagnostic::new(
                code,
                span,
                format!("{stays}; to change it, first make a changeable copy with `{copy}`"),
            );
            if let (Some(fix_it), true) = (self.first_in_body(bound, copy), written == binding) {
                diagnostic = diagnostic.with_fix_it(fix_it);
            }
            diagnostic
        } else {
            Diagnostic::new(code, span, stays)
        };
        if let Some(cause) = &def.drops {
            diagnostic = diagnostic.with_note(cause.note(owner));
        }
        diagnostic.with_label(info.span, format!("`{name}` is bound here"))
    }

    /// The fix-it putting `stmt` first in the body of `bound`'s arm or
    /// loop: inserted inside a block, or, for an arm whose body is not a
    /// block, a block made of `stmt` and that body; `None` when neither is
    /// known.
    fn first_in_body(&self, bound: Bound, stmt: String) -> Option<FixIt> {
        if let Some(at) = bound.body_start {
            return Some(FixIt {
                span: at,
                replacement: format!(" {stmt}"),
            });
        }
        let body = bound.arm_body?;
        let text = self
            .cx
            .sources
            .get(body.file.0 as usize)?
            .text
            .get(body.start as usize..body.end as usize)?;
        Some(FixIt {
            span: body,
            replacement: format!("{{ {stmt} {text} }}"),
        })
    }

    /// For `leaf`, a name a `match` on an owned `let` bound: what to do to
    /// take its value out, which is to `match` on the call the `let` was
    /// made from, a temporary the `match` owns (spec 3.4).
    pub(super) fn match_on_the_call(&self, leaf: &HirExpr) -> Option<String> {
        let HirExprKind::Local(id) = leaf.kind else {
            return None;
        };
        self.binding_advice(id)
    }

    /// For `id`, a name a `match` on a temporary of an enum with a
    /// destructor bound (an alias gone once the `match` ends): why it
    /// cannot be kept, and, for a `string`, to copy it with `.clone()`.
    pub(super) fn dropped_advice(&self, id: LocalId) -> Option<String> {
        let bound = self.patterns[id.0 as usize]?;
        if bound.dropped.is_none() || !self.local(id).place.borrowed {
            return None;
        }
        self.binding_advice(id)
    }

    /// [`Self::match_on_the_call`] for the binding `id`.
    fn binding_advice(&self, id: LocalId) -> Option<String> {
        let bound = self.patterns[id.0 as usize]?;
        let info = self.local(id);
        let name = &info.name;
        // Only a `string` has a `clone`.
        let keep = if info.ty == Ty::String {
            "copy it with `.clone()` to keep it".to_string()
        } else {
            format!("`{name}` can be read here, but not kept")
        };
        if let Some(dropped) = bound.dropped {
            let def = &self.cx.enums[dropped.0 as usize];
            let owner = &def.name;
            let runs = def.drops.as_ref().map_or("runs", |cause| cause.runs());
            let aside = def
                .drops
                .as_ref()
                .map_or_else(String::new, |cause| format!(" ({})", cause.aside(owner)));
            return Some(format!(
                "`{name}` stays inside the `{owner}` matched on, which {runs} code when it is \
                 thrown away, so nothing can take `{name}` out of it; {keep}{aside}"
            ));
        }
        let root_info = self.local(bound.root?);
        let root = &root_info.name;
        let drops = match root_info.ty {
            Ty::Enum(e) => {
                let def = &self.cx.enums[e.0 as usize];
                def.drops
                    .as_ref()
                    .map(|cause| (cause.runs(), cause.aside(&def.name)))
            }
            _ => None,
        };
        bound.owned_local.then(|| {
            if let Some((runs, aside)) = drops {
                return format!(
                    "`match` on a stored value only looks inside it, so `{name}` stays inside \
                     `{root}`, and the type of `{root}` {runs} code when it is thrown away, so \
                     nothing can take `{name}` out of it; {keep} ({aside})"
                );
            }
            format!(
                "`match` on a stored value only looks inside it, so `{name}` stays inside \
                 `{root}`; to take it out, `match` on the call that made `{root}` instead of \
                 storing its result with `let` first"
            )
        })
    }
}

/// Makes the fix-its of [`FnAnalyzer::first_in_body`] that each wrap the
/// same arm body in a block one fix-it: the first diagnostic gets a block
/// holding every copy and then the body, and the others lose theirs, so
/// no two fix-its replace the same text.
pub(super) fn merge_arm_fix_its(sources: &[SourceFile], diagnostics: &mut [Diagnostic]) {
    for first in 0..diagnostics.len() {
        let Some(fix_it) = &diagnostics[first].fix_it else {
            continue;
        };
        let body = fix_it.span;
        let Some(text) = sources
            .get(body.file.0 as usize)
            .and_then(|file| file.text.get(body.start as usize..body.end as usize))
        else {
            continue;
        };
        let end = format!(" {text} }}");
        let stmt = |fix_it: &FixIt| {
            (fix_it.span == body && body.start < body.end)
                .then(|| fix_it.replacement.strip_prefix("{ ")?.strip_suffix(&end))
                .flatten()
                .map(str::to_string)
        };
        let Some(mut stmts) = stmt(fix_it) else {
            continue;
        };
        for later in &mut diagnostics[first + 1..] {
            if let Some(more) = later.fix_it.as_ref().and_then(stmt) {
                stmts.push(' ');
                stmts.push_str(&more);
                later.fix_it = None;
            }
        }
        if let Some(fix_it) = &mut diagnostics[first].fix_it {
            fix_it.replacement = format!("{{ {stmts}{end}");
        }
    }
}
