//! The owned-slot check: a borrowed value of non-Copy type may not flow
//! into a place that owns it (V0304).

use varyk_syntax::{FixIt, Span};

use super::{FnAnalyzer, owner_text, place_root, through_index, what_to_do};
use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{HirExpr, leaves};
use crate::resolve::StructId;
use crate::types::Ty;

/// The Rust-facing note of a V0304 for a borrowed value kept somewhere
/// that owns it; [`what_to_do`] gives the plain-words one.
pub(super) const BORROWED_NOTE: &str =
    "in Rust terms, a borrowed value cannot move into a place that owns it";

/// The plain-words note of a V0304 for a borrowed value given to a
/// started call (milestone 5b1 spec 3).
const STARTED_NOTE: &str = "a started task may outlive this function, so it must own what it \
                            is given: give it a copy with `.clone()`, or share one value \
                            between tasks with `Shared`";

/// Where a value flows that must own it.
pub(super) enum Slot {
    Return,
    StructField {
        id: StructId,
        field: usize,
    },
    ImportedParam(String),
    Assignment,
    /// An assignment to `name`, a `let mut` made `from` an element (or a
    /// field): another name for it, so the assignment overwrites it.
    Alias {
        name: String,
        from: String,
        element: bool,
    },
    /// A whole struct local that is not a borrowed place.
    Local(String),
    /// A payload of the variant written this way (`Shape::Circle`, `Some`).
    Payload(String),
    VecElement,
    /// The operand of `?`.
    Question,
    /// The receiver of the taking row of this name (M4 spec 3.4).
    Taken(String),
    /// An argument of a started call of this name, which the task keeps
    /// (milestone 5b1 spec 3).
    Started(String),
}

impl FnAnalyzer<'_> {
    /// A borrowed place of non-Copy type cannot flow into an owned slot.
    pub(super) fn owned_slot(&mut self, value: &HirExpr, slot: &Slot) {
        for leaf in leaves(value) {
            let info = self.info(leaf);
            // A looked-into result is V0208 wherever it is not a head.
            if !info.borrowed || leaf.ty.is_copy() || self.looked_into(leaf) {
                continue;
            }
            if let Some(root) = place_root(leaf) {
                // Already said: what it holds is gone before this.
                if self.gone[root.0 as usize] {
                    continue;
                }
                self.reported[root.0 as usize] = true;
            }
            let owner = if info.origin.is_none() && through_index(leaf) {
                "the `Vec` this value is part of is gone after this line".to_string()
            } else {
                owner_text(self.cx, self.locals, info.origin)
            };
            let advice = match slot {
                Slot::Taken(method) => format!(
                    "`{method}` uses up the value it is called on, and what is inside this one \
                     cannot be copied out; match on it instead"
                ),
                Slot::Alias {
                    name,
                    from,
                    element: true,
                } => format!(
                    "to keep a separate value in `{name}`, keep an index instead, or copy the \
                     element when making `{name}` and every value assigned to it later: \
                     `let mut {name} = {from}.clone();`"
                ),
                Slot::Alias { name, from, .. } => format!(
                    "to keep a separate value in `{name}`, copy the field when making it and \
                     every value assigned to it later: `let mut {name} = {from}.clone();`"
                ),
                Slot::Started(_) => STARTED_NOTE.to_string(),
                _ => self
                    .match_on_the_call(leaf)
                    .unwrap_or_else(|| what_to_do(self.cx, self.locals, info.origin)),
            };
            let into = match slot {
                Slot::Return => "returned from here".to_string(),
                Slot::StructField { id, field } => format!(
                    "stored in field `{}` of `{}`",
                    self.cx.structs[id.0 as usize].fields[*field].name,
                    self.cx.struct_name(*id)
                ),
                Slot::ImportedParam(callee) => {
                    format!("given to `{callee}`, which keeps what it is given")
                }
                Slot::Assignment => "stored there".to_string(),
                Slot::Alias {
                    name,
                    from,
                    element,
                } => {
                    let noun = if *element { "element" } else { "field" };
                    format!(
                        "stored in `{name}`: `{name}` was made from `{from}`, so it is that \
                         {noun} under another name, and assigning to it would overwrite that \
                         {noun}"
                    )
                }
                Slot::Local(name) => format!("kept in `{name}`"),
                Slot::Payload(variant) => format!("put inside `{variant}`"),
                Slot::VecElement => "put in a `vec!`".to_string(),
                Slot::Question => "used up by `?`".to_string(),
                Slot::Taken(method) => format!("used up by `{method}`"),
                Slot::Started(callee) => {
                    format!("given to the task that runs `{callee}`, which keeps it")
                }
            };
            let diagnostic = Diagnostic::new(
                codes::V0304,
                leaf.span,
                format!("{owner}, so it cannot be {into}"),
            )
            .with_note(advice)
            .with_note(BORROWED_NOTE);
            // A copy stored in an alias still overwrites what it is another
            // name for: the note says to copy when making it instead.
            let diagnostic = match slot {
                Slot::Alias { .. } => diagnostic,
                _ => clone_fix_it(diagnostic, leaf.span, &leaf.ty),
            };
            self.diagnostics.push(diagnostic);
        }
    }
}

/// Whether a V0304 on a value of type `ty` offers `.clone()`: a `string`,
/// the one deliberate copy (spec 3.3, 3.5), or a `Bytes`, whose clone
/// copies the handle and not the bytes (milestone 5c spec 2.3, 7.4).
pub(super) fn clones(ty: &Ty) -> bool {
    matches!(ty, Ty::String | Ty::Bytes)
}

/// Adds the fix-it `.clone()` after the value at `span` to a V0304 when
/// the value (of type `ty`) is one [`clones`] takes.
pub(super) fn clone_fix_it(diagnostic: Diagnostic, span: Span, ty: &Ty) -> Diagnostic {
    if !clones(ty) {
        return diagnostic;
    }
    let end = Span::new(span.file, span.end, span.end);
    diagnostic.with_fix_it(FixIt {
        span: end,
        replacement: ".clone()".to_string(),
    })
}
