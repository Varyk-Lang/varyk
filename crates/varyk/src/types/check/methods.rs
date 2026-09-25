//! Typing of method calls, indexing, and `?` (spec 2.5, 2.6, 2.8): a
//! method of a user type's `impl` blocks or a row of the built-in table,
//! `v[i]`, and `r?`.

use varyk_syntax::{Expr, Ident, Span};

use super::{FnChecker, RANGE_USIZE_NOTE, usize_note};
use crate::builtins::{self, Owner};
use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{HirExpr, HirExprKind, MethodRef};
use crate::resolve::{LookupError, UserType, not_visible};
use crate::types::{IntKind, Ty};

impl FnChecker<'_> {
    /// `receiver.method(args)` (spec 2.5, 2.6): a method of the receiver's
    /// type from its `impl` blocks, or a row of the built-in table for a
    /// `Vec` or a `string`. A method the type does not have is V0100
    /// naming the type (and, for a built-in type, listing its methods); a
    /// non-`pub` method of another module's type is V0105.
    pub(super) fn method_call(
        &mut self,
        receiver: &Expr,
        method: &Ident,
        args: &[Expr],
        span: Span,
    ) -> Option<HirExpr> {
        let receiver = self.expr(receiver, None)?;
        let type_name = self.ty_name(&receiver.ty);
        let no_method = format!("type `{type_name}` has no method `{}`", method.name);
        let (found, params, ret): (MethodRef, Vec<Ty>, Ty) = match &receiver.ty {
            Ty::Struct(_) | Ty::Enum(_) => {
                let owner = match receiver.ty {
                    Ty::Struct(id) => UserType::Struct(id),
                    Ty::Enum(id) => UserType::Enum(id),
                    _ => unreachable!("matched above"),
                };
                let path = format!("{type_name}::{}", method.name);
                let id = match self.symbols.lookup_member(self.module, owner, &method.name) {
                    Ok(id) => id,
                    Err(LookupError::NotVisible { decl, keyword }) => {
                        self.diagnostics.push(not_visible(
                            "method",
                            &path,
                            method.span,
                            decl,
                            keyword,
                        ));
                        return None;
                    }
                    Err(LookupError::Unknown) => {
                        self.diagnostics.push(Diagnostic::new(
                            codes::V0100,
                            method.span,
                            no_method,
                        ));
                        return None;
                    }
                };
                let sig = &self.symbols.fns[id.0 as usize];
                if sig.self_mode.is_none() {
                    let message = format!(
                        "`{path}` has no `self`, so it is not called on a value; call it as \
                         `{path}(..)`"
                    );
                    self.diagnostics
                        .push(Diagnostic::new(codes::V0100, method.span, message));
                    return None;
                }
                let params = sig.params.iter().map(|p| p.1.clone()).collect();
                (MethodRef::Varyk(id), params, sig.ret.clone())
            }
            Ty::Vec(_) | Ty::String => {
                let (owner, element) = match &receiver.ty {
                    Ty::Vec(element) => (Owner::Vec, (**element).clone()),
                    _ => (Owner::String, Ty::String),
                };
                let Some(id) = builtins::lookup(owner, &method.name, true) else {
                    let message = format!(
                        "{no_method}; the methods of a `{}` are {}",
                        owner.name(),
                        listing(&builtins::names(owner, true))
                    );
                    self.diagnostics
                        .push(Diagnostic::new(codes::V0100, method.span, message));
                    return None;
                };
                let entry = id.get();
                let params = entry.params.iter().map(|p| p.ty(&element)).collect();
                (MethodRef::Builtin(id), params, entry.result.ty(&element))
            }
            other => {
                let mut diagnostic = Diagnostic::new(
                    codes::V0100,
                    method.span,
                    format!("type `{type_name}` has no methods"),
                );
                if matches!(other, Ty::Option(_) | Ty::Result(..)) {
                    diagnostic = diagnostic.with_note("use `match` to look at the value inside it");
                }
                self.diagnostics.push(diagnostic);
                return None;
            }
        };
        let args = self.arguments(&method.name, &params, args, span)?;
        Some(HirExpr {
            kind: HirExprKind::MethodCall {
                receiver: Box::new(receiver),
                method: found,
                args,
            },
            ty: ret,
            span,
        })
    }

    /// `base[index]` (spec 2.6): `base` must be a `Vec` and `index` a
    /// `usize` (V0200 otherwise); the element is a place.
    pub(super) fn index(&mut self, base: &Expr, index: &Expr, span: Span) -> Option<HirExpr> {
        let base = self.expr(base, None);
        let usize = Ty::Int(IntKind::Usize);
        let index = self.expr(index, Some(usize.clone())).and_then(|index| {
            let range_var =
                matches!(index.kind, HirExprKind::Local(id) if self.range_vars.contains(&id));
            if range_var && usize_note(&usize, &index.ty).is_some() {
                let message = format!(
                    "mismatched types: expected `usize`, found `{}`",
                    self.ty_name(&index.ty)
                );
                self.diagnostics.push(
                    Diagnostic::new(codes::V0200, index.span, message).with_note(RANGE_USIZE_NOTE),
                );
                return None;
            }
            self.expect(index, &usize)
        });
        let base = base?;
        let Ty::Vec(element) = &base.ty else {
            let message = format!(
                "only a `Vec` can be indexed, and this is `{}`",
                self.ty_name(&base.ty)
            );
            self.diagnostics
                .push(Diagnostic::new(codes::V0200, span, message));
            return None;
        };
        let ty = (**element).clone();
        Some(HirExpr {
            kind: HirExprKind::Index {
                base: Box::new(base),
                index: Box::new(index?),
            },
            ty,
            span,
        })
    }

    /// `operand?` (spec 2.8): the function returns `Result<T, E>` and
    /// `operand` is a `Result<U, E>` with the same `E`; the value is the
    /// `U`. Anything else is V0206 naming the function's return type and
    /// the operand's type: there is no error conversion, and `?` on an
    /// `Option` is a later milestone.
    pub(super) fn try_(&mut self, operand: &Expr, span: Span) -> Option<HirExpr> {
        let operand = self.expr(operand, None)?;
        let found = self.ty_name(&operand.ty);
        let name = self.name;
        let (message, note) = match (&self.ret, &operand.ty) {
            (Ty::Result(_, want), Ty::Result(ok, err)) if err == want => {
                let ty = (**ok).clone();
                return Some(HirExpr {
                    kind: HirExprKind::Try(Box::new(operand)),
                    ty,
                    span,
                });
            }
            (Ty::Result(_, want), Ty::Result(..)) => (
                format!(
                    "`?` hands the error of this `{found}` back to the caller, but `{name}` \
                     returns `{}`, whose error type is `{}`",
                    self.ty_name(&self.ret),
                    self.ty_name(want)
                ),
                "the error types must be the same: an error is never converted into \
                 another type",
            ),
            (Ty::Result(..), other) => (
                format!(
                    "`?` works on a `Result`, and this is `{found}`; `{name}` returns `{}`",
                    self.ty_name(&self.ret)
                ),
                if matches!(other, Ty::Option(_)) {
                    "`?` on an `Option` comes in a later milestone; use `match` to look inside it"
                } else {
                    "`?` takes the value out of an `Ok`, or returns the error of an `Err`"
                },
            ),
            (Ty::Unit, _) => (
                format!(
                    "`?` needs a function that returns a `Result`, but `{name}` returns \
                     nothing (this value is `{found}`)"
                ),
                RESULT_NOTE,
            ),
            (ret, _) => (
                format!(
                    "`?` needs a function that returns a `Result`, but `{name}` returns `{}` \
                     (this value is `{found}`)",
                    self.ty_name(ret)
                ),
                RESULT_NOTE,
            ),
        };
        self.diagnostics.push(
            Diagnostic::new(codes::V0206, span, message)
                .with_note(note)
                .with_note("in Rust terms, `?` returns early with the `Err`, so the function's return type must be a `Result` with the same error type"),
        );
        None
    }
}

/// Why `?` needs a function returning a `Result` (spec 2.8).
const RESULT_NOTE: &str = "on an error, `?` returns it from the function at once, so the \
                           function must return a `Result`; `main` never does, so use `?` in a \
                           helper function and `match` on that function's result";

/// `names` as a list in words: "`a`, `b`, and `c`", or "`a` and `b`".
fn listing(names: &[&str]) -> String {
    let quoted: Vec<String> = names.iter().map(|name| format!("`{name}`")).collect();
    match quoted.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] if init.len() == 1 => format!("{} and {last}", init[0]),
        [init @ .., last] => format!("{}, and {last}", init.join(", ")),
    }
}
