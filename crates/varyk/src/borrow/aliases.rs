//! The alias rule (spec 3.1): while an alias may still be used, its root
//! may not be changed or given away; while a mutable alias may still be
//! used, its root may not be used at all. Breaking it is V0307.
//!
//! An alias is a local that is another name for a place: today a `let`
//! that is a borrowed place (initialized from a parameter, a field, an
//! element, or another alias), or a `let mut` holding text that is later
//! assigned one, or a `let` holding text that copies such a reference (by
//! its `let` or an assignment).
//! Its roots are the locals that place lives under, found by following
//! field and index paths down (`hir::field_root`) and other aliases
//! through: `let t = p.s; let u = t;` makes `t` and `p` roots of `u`, and
//! `let first = tasks[1];` makes `tasks` the root of `first`.
//! A name a `match` pattern binds to a non-Copy value of a place is an
//! alias too (spec 3.2), its roots those of the scrutinee, and so is the
//! variable of a `for` over a place, its roots those of the `Vec`: borrow
//! analysis records them as it records a borrowed `let`'s, and
//! [`alias_roots`] reads them from there. The roots of an alias made by
//! assignment count from its `let` on, whatever it holds at the time:
//! over-strict before the assignment, never unsound. Scope is lexical, by
//! declaration span: a `match` binding's scope is its arm, and a `for`
//! variable's is its loop.
//!
//! A `for` over a place holds it for the whole loop (spec 3.1): its
//! variable, even a copy of a Copy element, counts as used at the end of
//! every round and at a `break` or `continue`, so any change to the place
//! inside the loop is V0307, worded for the loop. A copy is not itself an
//! alias: its uses in the body report nothing.
//!
//! The check runs on the move checker's flow walk ([`moves::walk`]), so "may
//! still be used" means what it means there: used later along some path,
//! a loop body following itself. A mark on an alias says one of its roots
//! was changed, given away, or (for a mutable alias) used; a later use of
//! the alias reports it, and its `let` running again (or, for an alias
//! made by assignment, a new value assigned to it) clears it. Changing a
//! root means assigning to it or a part of it, passing it or a part to a
//! `mut` parameter or as the receiver of a changing method
//! (`tasks[0].complete()` changes `tasks`, spec 3.1), or making a mutable
//! alias of it; giving it away means moving it. An argument or operand lent to a call or comparison counts
//! as used again when the call or comparison is made.

use std::collections::HashSet;

use varyk_syntax::Span;

use super::moves::{self, Access, Head, Marks, Rule};
use super::{Assigned, Context, Refers, push_unique};
use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{HirExpr, HirExprKind, HirFunction, LocalId, LocalInfo, leaves};

/// Reports every alias used after its root was changed, given away, or
/// (for a mutable alias) used, in `function`.
pub(super) fn check(
    cx: &Context,
    function: &HirFunction,
    refers: &Refers,
    assigned: &Assigned,
    held: &[bool],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let roots = alias_roots(function, refers, assigned, held);
    let mut aliases_of = vec![Vec::new(); function.locals.len()];
    for (alias, roots) in roots.iter().enumerate() {
        for root in roots {
            aliases_of[root.0 as usize].push(LocalId(alias as u32));
        }
    }
    let mut rule = Aliases {
        locals: &function.locals,
        roots,
        aliases_of,
        whole: vec![Vec::new(); function.locals.len()],
        held,
        reported: HashSet::new(),
        diagnostics,
    };
    moves::walk(cx, function, &mut rule);
}

/// Per local: its roots when it is an alias or the variable of a `for`
/// over a place (`held`), else nothing.
fn alias_roots(
    function: &HirFunction,
    refers: &Refers,
    assigned: &Assigned,
    held: &[bool],
) -> Vec<Vec<LocalId>> {
    let count = function.locals.len();
    // The locals each alias names directly: a borrowed `let`'s initializer
    // roots (or a `for` variable's `Vec`'s), plus the roots of borrowed
    // places assigned to it.
    let mut roots: Vec<Vec<LocalId>> = (0..count)
        .map(|index| {
            let mut direct = assigned.roots[index].clone();
            let borrowed = function.locals[index].place.borrowed;
            if index >= function.params.len() && (borrowed || held[index]) {
                for &root in &refers[index].1 {
                    push_unique(&mut direct, root);
                }
            }
            direct
        })
        .collect();
    // Follow roots that are aliases through to their own roots, and take
    // on the roots of every `let` whose reference is copied in (not that
    // `let` itself: the copy refers to what it refers to), until nothing
    // changes. Flows may run backwards through a loop, hence the fixed
    // point.
    let mut changed = true;
    while changed {
        changed = false;
        for index in 0..count {
            let mut add: Vec<LocalId> = Vec::new();
            for root in &roots[index] {
                add.extend(&roots[root.0 as usize]);
            }
            for copied in &assigned.copies[index] {
                add.extend(&roots[copied.0 as usize]);
            }
            for root in add {
                if root.0 as usize != index && !roots[index].contains(&root) {
                    roots[index].push(root);
                    changed = true;
                }
            }
        }
    }
    roots
}

/// What happened to an alias's root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum How {
    Changed,
    GivenAway,
    /// Used at all, which a mutable alias forbids.
    Used,
}

/// A mark on an alias: its root `root` was changed, given away, or used
/// at `at`.
#[derive(Debug, Clone, Copy)]
struct Broken {
    root: LocalId,
    at: Span,
    how: How,
}

struct Aliases<'a> {
    locals: &'a [LocalInfo],
    /// Per local: its roots when it is an alias.
    roots: Vec<Vec<LocalId>>,
    /// Per local: the aliases it is a root of.
    aliases_of: Vec<Vec<LocalId>>,
    /// Per alias: the locals its `let` names whole (`p` in `let q = p`),
    /// for the wording.
    whole: Vec<Vec<LocalId>>,
    /// Per local: the variable of a `for` over a place, which the loop
    /// holds until it ends.
    held: &'a [bool],
    /// Each (alias, change) already reported: a loop body is walked twice.
    reported: HashSet<(LocalId, u32, u32)>,
    diagnostics: &'a mut Vec<Diagnostic>,
}

impl Aliases<'_> {
    /// A `let mut` alias of a mutable place, which can change its roots
    /// (an alias made by assignment is a reference that only reads).
    fn is_mutable_alias(&self, local: LocalId) -> bool {
        let place = self.locals[local.0 as usize].place;
        place.borrowed && place.mutable
    }

    /// A copy made by a `for` over a place: not an alias itself, it only
    /// stands for its loop's hold, so its own uses report nothing.
    fn only_holds(&self, local: LocalId) -> bool {
        self.held[local.0 as usize] && !self.locals[local.0 as usize].place.borrowed
    }

    fn name(&self, local: LocalId) -> &str {
        &self.locals[local.0 as usize].name
    }

    /// The mutable alias of `root` that `alias` was made through, if any
    /// (`a` in `let mut a = ps[0]; let s = a.name;`): `alias` reborrows it,
    /// so `root` cannot be used while `alias` is.
    fn through_mutable(&self, alias: LocalId, root: LocalId) -> Option<LocalId> {
        self.roots[alias.0 as usize].iter().copied().find(|&m| {
            m != root && self.is_mutable_alias(m) && self.roots[m.0 as usize].contains(&root)
        })
    }

    /// Marks the aliases of `root` (only the mutable ones, and those made
    /// through one, when `mutable_only`) as broken at `at`, except those in
    /// `except`.
    fn break_aliases(
        &self,
        marks: &mut Marks<Broken>,
        root: LocalId,
        at: Span,
        how: How,
        mutable_only: bool,
        except: &[LocalId],
    ) {
        for &alias in &self.aliases_of[root.0 as usize] {
            if except.contains(&alias) {
                continue;
            }
            if !mutable_only
                || self.is_mutable_alias(alias)
                || self.through_mutable(alias, root).is_some()
            {
                marks.entry(alias).or_insert(Broken { root, at, how });
            }
        }
    }

    /// `alias` is used at `span`: V0307 when it is marked.
    fn use_(&mut self, marks: &Marks<Broken>, alias: LocalId, span: Span, repeating: bool) {
        let Some(&Broken { root, at, how }) = marks.get(&alias) else {
            return;
        };
        if !self.reported.insert((alias, at.start, at.end)) {
            return;
        }
        let (alias_name, root_name) = (self.name(alias), self.name(root));
        let part = if self.whole[alias.0 as usize].contains(&root) {
            ""
        } else {
            "part of "
        };
        let (message, note) = match how {
            How::Changed | How::GivenAway => {
                let verb = if how == How::Changed {
                    "changed"
                } else {
                    "given away"
                };
                let rust = if how == How::Changed {
                    "changed"
                } else {
                    "moved"
                };
                (
                    format!(
                        "`{alias_name}` is another name for {part}`{root_name}`, so `{root_name}` \
                         cannot be {verb} until `{alias_name}` is no longer needed"
                    ),
                    format!(
                        "in Rust terms, `{alias_name}` is a reference that borrows `{root_name}`, \
                         and a value cannot be {rust} while it is borrowed"
                    ),
                )
            }
            How::Used => match self.through_mutable(alias, root) {
                Some(m) if !self.is_mutable_alias(alias) => (
                    format!(
                        "`{alias_name}` is another name for part of `{m}`, which can change \
                         `{root_name}`, so `{root_name}` cannot be used until `{alias_name}` is \
                         no longer needed",
                        m = self.name(m)
                    ),
                    format!(
                        "in Rust terms, `{alias_name}` is a reference taken through the mutable \
                         reference `{m}`, which borrows `{root_name}`, and a value cannot be used \
                         while it is mutably borrowed",
                        m = self.name(m)
                    ),
                ),
                _ => (
                    format!(
                        "`{alias_name}` is another name for {part}`{root_name}` that can change \
                         it, so `{root_name}` cannot be used until `{alias_name}` is no longer \
                         needed"
                    ),
                    format!(
                        "in Rust terms, `{alias_name}` is a mutable reference that borrows \
                         `{root_name}`, and a value cannot be used while it is mutably borrowed"
                    ),
                ),
            },
        };
        let label = if repeating {
            format!("`{alias_name}` is used here the next time through the loop")
        } else {
            format!("`{alias_name}` is used here later")
        };
        self.diagnostics.push(
            Diagnostic::new(codes::V0307, at, message)
                .with_label(span, label)
                .with_note(note),
        );
    }
}

impl Rule for Aliases<'_> {
    type Mark = Broken;

    fn access(
        &mut self,
        marks: &mut Marks<Broken>,
        local: LocalId,
        span: Span,
        access: Access,
        repeating: bool,
    ) {
        let borrowed = self.locals[local.0 as usize].place.borrowed;
        if access == Access::Replace && !borrowed {
            // A local that owns its value, or holds a reference it does not
            // write through, gets a new value: whatever it named before is
            // no longer used through it.
            marks.remove(&local);
        } else if !self.only_holds(local) {
            self.use_(marks, local, span, repeating);
        }
        let (how, mutable_only) = match access {
            Access::Change | Access::Replace => (How::Changed, false),
            Access::Move if moves::movable(&self.locals[local.0 as usize]) => {
                (How::GivenAway, false)
            }
            Access::Move | Access::Read => (How::Used, true),
        };
        self.break_aliases(marks, local, span, how, mutable_only, &[]);
    }

    fn bind(&mut self, marks: &mut Marks<Broken>, local: LocalId, value: &HirExpr) {
        let index = local.0 as usize;
        // Making a mutable alias changes its roots: nothing else may use
        // them while it is in use. The mutable aliases it is made through
        // are reborrowed, not broken: using one while it is in use is
        // caught as a use of its root.
        if self.is_mutable_alias(local) && !self.roots[index].is_empty() {
            let roots = self.roots[index].clone();
            for &root in &roots {
                self.break_aliases(marks, root, value.span, How::Changed, false, &roots);
            }
        }
        self.whole[index] = leaves(value)
            .into_iter()
            .filter_map(|leaf| match leaf.kind {
                HirExprKind::Local(id) => Some(id),
                _ => None,
            })
            .collect();
        marks.remove(&local);
    }

    fn bind_pattern(
        &mut self,
        marks: &mut Marks<Broken>,
        local: LocalId,
        scrutinee: &HirExpr,
        whole: bool,
    ) {
        self.bind(marks, local, scrutinee);
        if !whole {
            // A position of a variant is part of what is matched on.
            self.whole[local.0 as usize].clear();
        }
    }

    fn lent(&mut self, marks: &mut Marks<Broken>, local: LocalId, span: Span, repeating: bool) {
        if !self.only_holds(local) {
            self.use_(marks, local, span, repeating);
        }
    }

    fn held(&mut self, marks: &mut Marks<Broken>, local: LocalId, head: Head) {
        let Some(&Broken { root, at, how }) = marks.get(&local) else {
            return;
        };
        if !self.reported.insert((local, at.start, at.end)) {
            return;
        }
        let name = self.name(root);
        let part = if head.local == Some(root) {
            ""
        } else {
            "part of "
        };
        let (verb, rust) = match how {
            How::GivenAway => ("given away", "moved"),
            How::Changed | How::Used => ("changed", "changed"),
        };
        let (message, note) = match (how, self.through_mutable(local, root)) {
            (How::Used, Some(m)) => (
                format!(
                    "this loop goes over `{m}`, which can change `{name}`, so `{name}` cannot be \
                     used inside it",
                    m = self.name(m)
                ),
                format!(
                    "in Rust terms, the `for` loop borrows `{m}`, a mutable reference into \
                     `{name}`, until it ends, and a value cannot be used while it is mutably \
                     borrowed",
                    m = self.name(m)
                ),
            ),
            _ => (
                format!(
                    "this loop goes over {part}`{name}`, so `{name}` cannot be {verb} inside it"
                ),
                format!(
                    "in Rust terms, the `for` loop borrows `{name}` until it ends, and a value \
                     cannot be {rust} while it is borrowed"
                ),
            ),
        };
        self.diagnostics.push(
            Diagnostic::new(codes::V0307, at, message)
                .with_label(head.span, "the loop goes over this until it ends")
                .with_note(note),
        );
    }
}
