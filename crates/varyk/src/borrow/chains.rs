//! Chains (M4 spec 2.3, 3.3): what the items of each chain are, followed
//! through the first pass (`places`) as it walks the calls of a chain.
//!
//! Every item is borrowed, a copy, or owned ([`ItemKind`]), decided at
//! the source and changed only by `map`. A source reads a place (or, for
//! `split`, a string literal): its items are borrowed with the root of its
//! receiver, or copies when the item type is Copy. `filter` keeps the
//! kind; `map` gives what its closure returns, classified as a function
//! body is (M4 spec 3.1, 3.2). Each closure parameter is the item as its
//! row hands it over, which is where the generated Rust dereferences it
//! (`LocalKind::ClosureParam { deref }`). `collect` needs owned items or
//! copies, and a `find` on borrowed items is looked into where it is made
//! (M4 spec 2.8).
//!
//! The table is keyed by the span of the chain call whose items it holds,
//! a source or an adapter, and carried out of the walk for `strings` and
//! for a `for` over a chain.

use varyk_syntax::{FixIt, Span};

use super::returns::Classification;
use super::slots::{BORROWED_NOTE, clones};
use super::{FnAnalyzer, borrowed_receiver, place_root, push_unique, roots};
use crate::builtins::{Builtin, Owner, ResultKind};
use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{
    HirBlock, HirExpr, HirExprKind, LocalId, LocalKind, MethodRef, Origin, PlaceInfo, StringRepr,
    leaves,
};
use crate::types::Ty;

/// What the items of a chain are (M4 spec 3.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ItemKind {
    /// Other names for parts of stored values, read-only, rooted at these
    /// locals (none for the pieces of a string literal).
    Borrowed(Vec<LocalId>),
    /// Numbers or `bool`s, copied as they are read.
    Copy,
    /// New values a `map` made.
    Owned,
}

/// A chain's items as the first pass follows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Items {
    pub(super) kind: ItemKind,
    /// The generated Rust form of a `string` item: `&str` for a piece of
    /// text or a part a `map` returns, `&String` for an element of a
    /// `Vec<string>` or a map, `String` for an owned one.
    pub(super) repr: Option<StringRepr>,
    /// What a borrowed item derives from, for diagnostics.
    pub(super) origin: Option<Origin>,
}

impl Items {
    /// Items of type `ty` that are new: copies or owned.
    fn new(ty: &Ty) -> Items {
        if ty.is_copy() {
            return Items {
                kind: ItemKind::Copy,
                repr: None,
                origin: None,
            };
        }
        Items {
            kind: ItemKind::Owned,
            repr: (*ty == Ty::String).then_some(StringRepr::Owned),
            origin: None,
        }
    }

    /// The locals borrowed items are rooted at; none for other items.
    pub(super) fn roots(&self) -> &[LocalId] {
        match &self.kind {
            ItemKind::Borrowed(roots) => roots,
            ItemKind::Copy | ItemKind::Owned => &[],
        }
    }
}

/// The row of `method` when it is a call of a chain: a source (a row
/// whose result is a chain) or a row of a chain.
pub(super) fn chain_row(method: MethodRef) -> Option<&'static Builtin> {
    match method {
        MethodRef::Builtin(id) => {
            let entry = id.get();
            (entry.owner == Owner::Chain || entry.result_kind == ResultKind::Chain).then_some(entry)
        }
        MethodRef::Varyk(_) | MethodRef::Imported(_) => None,
    }
}

/// The receiver of the source of `chain`, a chain: what it goes over.
pub(super) fn source_receiver(chain: &HirExpr) -> Option<&HirExpr> {
    let HirExprKind::MethodCall {
        receiver, method, ..
    } = &chain.kind
    else {
        return None;
    };
    match chain_row(*method) {
        Some(row) if row.owner == Owner::Chain => source_receiver(receiver),
        Some(_) => Some(receiver),
        None => None,
    }
}

/// The item type of a chain of type `ty`.
fn item_type(ty: &Ty) -> Ty {
    match ty {
        Ty::Chain(item) => (**item).clone(),
        _ => Ty::Unit,
    }
}

impl FnAnalyzer<'_> {
    /// The items of `chain`, the receiver of a row of a chain.
    pub(super) fn items_of(&self, chain: &HirExpr) -> Items {
        self.items
            .get(&chain.span)
            .cloned()
            .unwrap_or_else(|| Items::new(&item_type(&chain.ty)))
    }

    /// Every local `chain`, the head of a `for`, reads while the loop runs
    /// (M4 spec 3.3): the roots of its source's receiver and argument, and
    /// every capture of its closures, which Rust's iterator keeps borrowed
    /// until the loop ends.
    pub(super) fn head_reads(&self, chain: &HirExpr) -> Vec<LocalId> {
        let mut reads = Vec::new();
        let mut call = chain;
        while let HirExprKind::MethodCall {
            receiver,
            method,
            args,
            ..
        } = &call.kind
        {
            let Some(row) = chain_row(*method) else {
                break;
            };
            for arg in args {
                match &arg.kind {
                    HirExprKind::Closure { captures, .. } => {
                        for &capture in captures {
                            push_unique(&mut reads, capture);
                        }
                    }
                    _ => {
                        // Every local the argument may borrow from.
                        let rooted = leaves(arg).into_iter().filter_map(place_root);
                        for root in roots(arg).into_iter().chain(rooted) {
                            push_unique(&mut reads, root);
                        }
                    }
                }
            }
            if row.owner != Owner::Chain {
                // The source: what it reads is its receiver.
                for root in borrowed_receiver(self.cx, receiver).unwrap_or_default() {
                    push_unique(&mut reads, root);
                }
                break;
            }
            call = receiver;
        }
        reads
    }

    /// Whether `expr` is a looked-into result: a looked-into `get`, or a
    /// `find` on borrowed items (M4 spec 2.8), which this pass decides
    /// before the HIR records it.
    pub(super) fn looked_into(&self, expr: &HirExpr) -> bool {
        crate::hir::is_looked_into(expr)
            || (matches!(expr.kind, HirExprKind::MethodCall { .. })
                && self.finds.contains(&expr.span))
    }

    /// The items a looked-into `find` holds one of, or `None` for anything
    /// else.
    pub(super) fn found_items(&self, expr: &HirExpr) -> Option<Items> {
        match &expr.kind {
            HirExprKind::MethodCall { receiver, .. } if self.finds.contains(&expr.span) => {
                Some(self.items_of(receiver))
            }
            _ => None,
        }
    }

    /// Makes `param`, the parameter of a closure of the chain row `row`
    /// over `items`, the item as the row hands it (M4 spec 3.3): `map`
    /// gets the item itself; `filter` and `find` look at it through one
    /// more reference, a read-only borrowed place (rootless over owned
    /// items); `any` and `all` get it by value, an owned local of the
    /// closure that is never given away over owned items; a copy is a
    /// copy. Records where the generated Rust dereferences it at entry.
    pub(super) fn chain_param(
        &mut self,
        param: LocalId,
        row: &Builtin,
        items: &Items,
        body: &HirBlock,
    ) {
        let root = items.roots().first().copied();
        self.closure_param(param, body, root);
        let looks = matches!(row.name, "filter" | "find");
        let index = param.0 as usize;
        let (borrowed, origin, deref) = match &items.kind {
            ItemKind::Borrowed(roots) => {
                self.refers[index] = (false, roots.clone());
                (true, items.origin, looks)
            }
            ItemKind::Copy => (false, None, looks),
            ItemKind::Owned if looks => (true, None, false),
            ItemKind::Owned => {
                if matches!(row.name, "any" | "all") {
                    self.never_given_away.insert(param);
                }
                (false, None, false)
            }
        };
        self.locals[index].place = PlaceInfo {
            borrowed,
            mutable: false,
            origin,
        };
        self.locals[index].kind = LocalKind::ClosureParam { deref };
    }

    /// Records the items of `call`, a call of a chain on `receiver` (with
    /// `incoming` its items when `call` is a row of a chain), and checks
    /// what its row asks of them: a source's receiver must be stored
    /// (V0001), `collect` needs items it can own (V0304), and a `find` on
    /// borrowed items is looked into. `mapped` is how a `map`'s closure
    /// returns were classified; `None` when they were in error.
    pub(super) fn chain_call(
        &mut self,
        call: &HirExpr,
        receiver: &HirExpr,
        row: &Builtin,
        incoming: Option<Items>,
        mapped: Option<Classification>,
    ) {
        let item = item_type(&call.ty);
        let Some(incoming) = incoming else {
            // A source.
            let roots = match borrowed_receiver(self.cx, receiver) {
                Ok(roots) => roots,
                Err(diagnostic) => {
                    self.diagnostics.push(diagnostic);
                    Vec::new()
                }
            };
            let items = if item.is_copy() {
                Items::new(&item)
            } else {
                let repr = match (&item, row.name) {
                    (Ty::String, "split") => Some(StringRepr::Str),
                    (Ty::String, _) => Some(StringRepr::RefOwned),
                    _ => None,
                };
                Items {
                    kind: ItemKind::Borrowed(roots),
                    repr,
                    origin: self.head_origin(receiver),
                }
            };
            self.items.insert(call.span, items);
            return;
        };
        match row.name {
            "filter" => {
                self.items.insert(call.span, incoming);
            }
            "map" => {
                let items = self.mapped(call, &incoming, mapped, &item);
                self.items.insert(call.span, items);
            }
            "collect" => self.collected(call, receiver, &incoming),
            "find" if matches!(incoming.kind, ItemKind::Borrowed(_)) => {
                self.finds.insert(call.span);
            }
            _ => {}
        }
    }

    /// The items of `call`, a `map` over `incoming` whose closure's
    /// returns were classified `mapped`, making items of type `item`: new
    /// ones, or borrowed from what the closure returns part of, its
    /// parameter's root or a captured name (M4 spec 3.3).
    fn mapped(
        &mut self,
        call: &HirExpr,
        incoming: &Items,
        mapped: Option<Classification>,
        item: &Ty,
    ) -> Items {
        let HirExprKind::MethodCall { args, .. } = &call.kind else {
            return Items::new(item);
        };
        let Some(HirExprKind::Closure { param, .. }) = args.first().map(|arg| &arg.kind) else {
            return Items::new(item);
        };
        let Some(Classification::Part(root)) = mapped else {
            return Items::new(item);
        };
        if let Some(closure) = args.first() {
            self.parts.insert(closure.span);
        }
        let repr = (*item == Ty::String).then_some(StringRepr::Str);
        if root == *param {
            return Items {
                kind: incoming.kind.clone(),
                repr,
                origin: incoming.origin,
            };
        }
        // A captured name: the root the function's aliases follow on.
        let origin = if self.captured(root) {
            Some(Origin::Captured(root))
        } else {
            let place = self.local(root).place;
            if place.borrowed {
                place.origin
            } else {
                Some(Origin::Local(root))
            }
        };
        Items {
            kind: ItemKind::Borrowed(vec![root]),
            repr,
            origin,
        }
    }

    /// V0304 for `call`, a `collect` of `incoming`, when the items are
    /// borrowed: a `Vec` of borrowed values has no Varyk type (M4 spec
    /// 3.3). Text and `Bytes` are copied with `.map(|w| w.clone())`, the
    /// fix-it.
    fn collected(&mut self, call: &HirExpr, receiver: &HirExpr, incoming: &Items) {
        let ItemKind::Borrowed(roots) = &incoming.kind else {
            return;
        };
        let parts = match roots.first() {
            Some(root) => format!("parts of `{}`", self.local(*root).name),
            None => "parts of the text they come from".to_string(),
        };
        let at = Span::new(call.span.file, receiver.span.end, call.span.end);
        let copied = clones(&item_type(&receiver.ty));
        let (message, note) = if copied {
            (
                format!(
                    "these items are {parts}, and a `Vec` must own what it holds; copy them \
                     first with `.map(|w| w.clone())`"
                ),
                "`collect` makes a `Vec` of the items as they are",
            )
        } else {
            (
                format!(
                    "these items are {parts}, and a `Vec` must own what it holds; make new \
                     values from them with `map` first"
                ),
                "`collect` makes a `Vec` of the items as they are",
            )
        };
        let mut diagnostic = Diagnostic::new(codes::V0304, at, message)
            .with_note(note)
            .with_note(BORROWED_NOTE);
        if copied {
            let end = Span::new(call.span.file, receiver.span.end, receiver.span.end);
            diagnostic = diagnostic.with_fix_it(FixIt {
                span: end,
                replacement: ".map(|w| w.clone())".to_string(),
            });
        }
        self.diagnostics.push(diagnostic);
    }
}
