//! Borrow analysis of the names `match` patterns and `for` loops bind
//! (spec 3.1, 3.2): their place info, and the wording of the diagnostics
//! about changing or keeping them.

use varyk_syntax::{FixIt, Span};

use super::{FnAnalyzer, place_root, roots};
use crate::diagnostics::Diagnostic;
use crate::hir::{
    HirArm, HirBlock, HirExpr, HirExprKind, HirForHead, LocalId, Origin, PlaceInfo, is_place,
};

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
}

impl FnAnalyzer<'_> {
    /// Computes the place info of the bindings of `arm`'s pattern (spec
    /// 3.2). On a place, a binding of a non-Copy value is a read-only alias
    /// rooted where the scrutinee is, and one of a Copy value is a copy; on
    /// a temporary, every binding owns its value. None can be changed.
    pub(super) fn pattern(&mut self, scrutinee: &HirExpr, arm: &HirArm) {
        let place = is_place(scrutinee);
        let root = place_root(scrutinee);
        let origin = self.head_origin(scrutinee);
        let owned_local = matches!(scrutinee.kind, HirExprKind::Local(id)
            if !self.is_param(id) && !self.local(id).place.borrowed);
        let body_start = match &arm.body.kind {
            HirExprKind::Block(block) => {
                let at = block.span.start + 1;
                Some(Span::new(block.span.file, at, at))
            }
            _ => None,
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
                self.refers[index] = (false, roots(scrutinee));
            }
            self.patterns[index] = Some(Bound {
                looped: false,
                root,
                whole: whole && matches!(scrutinee.kind, HirExprKind::Local(_)),
                owned_local,
                body_start,
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
    /// block.
    pub(super) fn binding_unchangeable(
        &self,
        code: &'static str,
        span: Span,
        binding: LocalId,
        callee: &str,
    ) -> Diagnostic {
        let bound = self.patterns[binding.0 as usize].expect("a pattern binding");
        let info = self.local(binding);
        let name = &info.name;
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
                if let Some(at) = bound.body_start {
                    diagnostic = diagnostic.with_fix_it(FixIt {
                        span: at,
                        replacement: format!(" let mut {name} = {name};"),
                    });
                }
                diagnostic
            }
        };
        diagnostic.with_label(info.span, format!("`{name}` is bound here"))
    }

    /// For `leaf`, a name a `match` on an owned `let` bound: what to do to
    /// take its value out, which is to `match` on the call the `let` was
    /// made from, a temporary the `match` owns (spec 3.4).
    pub(super) fn match_on_the_call(&self, leaf: &HirExpr) -> Option<String> {
        let HirExprKind::Local(id) = leaf.kind else {
            return None;
        };
        let bound = self.patterns[id.0 as usize]?;
        let root = &self.local(bound.root?).name;
        let name = &self.local(id).name;
        bound.owned_local.then(|| {
            format!(
                "`match` on a stored value only looks inside it, so `{name}` stays inside \
                 `{root}`; to take it out, `match` on the call that made `{root}` instead of \
                 storing its result with `let` first"
            )
        })
    }
}
