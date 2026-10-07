//! Typing of method calls, indexing, and `?` (spec 2.5, 2.6, 2.8): a
//! method of a user type's `impl` blocks or a row of the built-in table,
//! `v[i]`, and `r?`.

use varyk_syntax::{Expr, ExprKind, Ident, Span};

use super::asyncs::{TaskPlace, map_value_spans, started_ty};
use super::values::RESULT_HOLE;
use super::{
    ArgRules, FnChecker, RANGE_USIZE_NOTE, closures, unsupported_rust_signature, usize_note,
};
use crate::builtins::{self, BuiltinId, ClosureResult, Owner, ResultKind, Subst};
use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{HirExpr, HirExprKind, MethodRef, TryKind, VariantRef};
use crate::resolve::{Callee, LookupError, UserType, not_visible};
use crate::types::{IntKind, Ty};

impl FnChecker<'_> {
    /// `receiver.method(args)` (spec 2.5, 2.6): a method of the receiver's
    /// type from its `impl` blocks, or a row of the built-in table for a
    /// `Vec`, a `string`, an `Option`, a `Result`, a `HashMap` (M4 spec
    /// 2.7), or an `Error` (M5a spec 2.3), whose element rule the receiver must meet (V0200) and whose
    /// `parse` takes its type from `expected`. A method the type does not
    /// have is V0100 naming the type (and, for a built-in type, listing
    /// its methods); a non-`pub` method of another module's type is V0105.
    /// `ok_or` takes its error type from an expected `Result`, else from
    /// its argument; a looked-into row whose payload is not Copy, and a
    /// borrowed row, is `rooted` at its receiver.
    pub(super) fn method_call(
        &mut self,
        receiver: &Expr,
        method: &Ident,
        args: &[Expr],
        expected: Option<Ty>,
        span: Span,
    ) -> Option<HirExpr> {
        // Where a task may be made (milestone 5b1 spec 2.3): the receiver
        // of `.detach()`, and the value of the closure of the `map` that
        // `collect` is called on.
        let places = self.task_places.len();
        if method.name == "detach" && args.is_empty() {
            self.task_places.push((receiver.span, TaskPlace::Detach));
        }
        if let (
            "collect",
            [],
            ExprKind::MethodCall {
                method: map,
                args: map_args,
                ..
            },
        ) = (method.name.as_str(), args, &receiver.kind)
        {
            if let ("map", [closure]) = (map.name.as_str(), map_args.as_slice()) {
                for at in map_value_spans(closure) {
                    self.task_places.push((at, TaskPlace::MapValue));
                }
            }
        }
        let receiver = self.expr(receiver, None);
        self.task_places.truncate(places);
        let receiver = receiver?;
        // A route call is a statement of its own (milestone 5b4 spec 2.1).
        if let Some(diagnostic) = self.route_as_value(&receiver, method, span) {
            self.diagnostics.push(diagnostic);
            return None;
        }
        self.method_on(receiver, method, args, expected, span)
    }

    /// [`FnChecker::method_call`] once `receiver` is checked.
    pub(super) fn method_on(
        &mut self,
        receiver: HirExpr,
        method: &Ident,
        args: &[Expr],
        expected: Option<Ty>,
        span: Span,
    ) -> Option<HirExpr> {
        if method.name == "clone" && self.derived_clone(&receiver.ty) {
            return self.clone_call(receiver, method, args, span);
        }
        // A `Shared`'s methods are its struct's, but for `clone`, which
        // gives another handle (milestone 5b1 spec 2.6).
        let reached = match &receiver.ty {
            Ty::Shared(_) if method.name == "clone" => &receiver.ty,
            ty => ty.reached(),
        };
        let type_name = self.ty_name(reached);
        let no_method = format!("type `{type_name}` has no method `{}`", method.name);
        // Arguments typed before the parameter types were known.
        let mut typed = None;
        let mut started = false;
        let mut type_arg = None;
        let (found, params, ret): (MethodRef, Vec<Ty>, Ty) = match reached {
            Ty::Struct(_) | Ty::Enum(_) => {
                let owner = match *reached {
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
                    Err(LookupError::PrivateModule { module }) => {
                        let diagnostic =
                            self.symbols
                                .private_module(module, "method", &path, method.span);
                        self.diagnostics.push(diagnostic);
                        return None;
                    }
                    Err(LookupError::Unknown) => {
                        let mut diagnostic = Diagnostic::new(codes::V0100, method.span, no_method);
                        if let Some(note) = self.symbols.skipped_member_note(owner, &method.name) {
                            diagnostic = diagnostic.with_note(note);
                        }
                        self.diagnostics.push(diagnostic);
                        return None;
                    }
                    Err(LookupError::NoParent { .. } | LookupError::Dependency { .. }) => {
                        unreachable!("a member lookup follows no path")
                    }
                };
                let (self_mode, params, ret, found) = match id {
                    Callee::Varyk(id) => {
                        let sig = &self.symbols.fns[id.0 as usize];
                        let params = sig.params.iter().map(|p| p.1.clone()).collect();
                        (sig.self_mode, params, sig.ret.clone(), MethodRef::Varyk(id))
                    }
                    Callee::Imported(id) => {
                        let sig = &self.symbols.imported[id.0 as usize];
                        if !sig.callable || !self.symbols.usable_from(sig.within, self.module) {
                            self.diagnostics.push(unsupported_rust_signature(
                                &path,
                                sig,
                                method.span,
                            ));
                            return None;
                        }
                        // The call names `varyk-std` (milestone 5b3 spec 2.4).
                        self.uses_std |= sig.names_std;
                        let params = sig.params.iter().map(|p| p.0.clone()).collect();
                        (
                            sig.self_mode,
                            params,
                            sig.ret.clone(),
                            MethodRef::Imported(id),
                        )
                    }
                    Callee::Builtin(_) => unreachable!("a type's members are never built-ins"),
                };
                if self_mode.is_none() {
                    let message = format!(
                        "`{path}` has no `self`, so it is not called on a value; call it as \
                         `{path}(..)`"
                    );
                    self.diagnostics
                        .push(Diagnostic::new(codes::V0100, method.span, message));
                    return None;
                }
                match found {
                    MethodRef::Varyk(id) if self.symbols.fns[id.0 as usize].is_async => {
                        started = self.async_call(Some(id), &path, span)?;
                    }
                    MethodRef::Imported(id) if self.symbols.imported[id.0 as usize].is_async => {
                        started = self.async_call(None, &path, span)?;
                    }
                    _ => {}
                }
                // A type hole is filled from where the result goes
                // (milestone 5b3 spec 2.1).
                let imported = match found {
                    MethodRef::Imported(id) => Some(id),
                    _ => None,
                };
                let written = Span::new(span.file, receiver.span.start, method.span.end);
                let filled = self.filled(imported, written, started, expected.as_ref(), span)?;
                match filled {
                    Some((ret, t)) => {
                        type_arg = Some(t);
                        (found, params, ret)
                    }
                    None => (found, params, ret),
                }
            }
            other => {
                let Some((owner, subst)) = Owner::of(other) else {
                    self.diagnostics.push(Diagnostic::new(
                        codes::V0100,
                        method.span,
                        format!("type `{type_name}` has no methods"),
                    ));
                    return None;
                };
                let Some(id) = builtins::lookup(owner, &method.name, true) else {
                    let calls = listing(&builtins::names(owner, true));
                    let message = if owner == Owner::Chain {
                        format!(
                            "a chain has no call `{}`; the calls of a chain are {calls}",
                            method.name
                        )
                    } else {
                        format!(
                            "{no_method}; the methods of {} are {calls}",
                            owner_words(owner)
                        )
                    };
                    let mut diagnostic = Diagnostic::new(codes::V0100, method.span, message);
                    if matches!(owner, Owner::Option | Owner::Result) {
                        diagnostic =
                            diagnostic.with_note("use `match` to look at the value inside it");
                    }
                    self.diagnostics.push(diagnostic);
                    return None;
                };
                let entry = id.get();
                self.uses_std |= entry.uses_std();
                if !entry.element.accepts(&subst.t) {
                    let message = if owner == Owner::Chain {
                        format!(
                            "`{}` adds up numbers, and these items are `{}`",
                            entry.name,
                            self.ty_name(&subst.t)
                        )
                    } else {
                        format!(
                            "`{}` needs {}, and this is `{type_name}`",
                            entry.name,
                            entry.element.wanted()
                        )
                    };
                    self.diagnostics
                        .push(Diagnostic::new(codes::V0200, method.span, message));
                    return None;
                }
                let subst = match (entry.result, &expected, args) {
                    (builtins::Shape::ResultOfExpected, ..) => {
                        let expected = self.parsed_type(expected.as_ref(), span)?;
                        builtins::Subst { expected, ..subst }
                    }
                    // `ok_or(e)`: `E` from an expected `Result<_, E>`, and
                    // otherwise from the argument, typed first with
                    // nothing expected.
                    (builtins::Shape::ResultOfTAndE, Some(Ty::Result(_, e)), _) => {
                        builtins::Subst {
                            e: (**e).clone(),
                            ..subst
                        }
                    }
                    (builtins::Shape::ResultOfTAndE, _, [arg]) => {
                        let arg = self.expr(arg, None)?;
                        let e = arg.ty.clone();
                        typed = Some(vec![arg]);
                        builtins::Subst { e, ..subst }
                    }
                    _ => subst,
                };
                if entry.params.iter().any(|shape| shape.is_closure()) {
                    let (args, subst) = self.closure_arguments(id, subst, args, expected, span)?;
                    typed = Some(args);
                    (MethodRef::Builtin(id), Vec::new(), entry.result.ty(&subst))
                } else {
                    let params = entry.params.iter().map(|p| p.ty(&subst)).collect();
                    (MethodRef::Builtin(id), params, entry.result.ty(&subst))
                }
            }
        };
        let rules = match found {
            MethodRef::Imported(id) => ArgRules::of(&self.symbols.imported[id.0 as usize]),
            _ => ArgRules::default(),
        };
        let (args, trailing) = match typed {
            Some(args) => (args, Vec::new()),
            None => self.arguments(&method.name, &params, &rules, args, span)?,
        };
        // A looked-into result holds part of its receiver, unless its
        // payload is Copy, when it is a plain `Option` of a copy (M4 spec
        // 2.8). A chain's `find` is looked into by its items, which borrow
        // analysis decides (M4 spec 3.3).
        let rooted = match (found, &ret) {
            (MethodRef::Builtin(id), Ty::Option(payload))
                if id.get().result_kind == ResultKind::LookInside
                    && id.get().owner != Owner::Chain
                    && !payload.is_copy() =>
            {
                Some(0)
            }
            // A borrowed row's result is part of its receiver (M4 spec 3.1).
            (MethodRef::Builtin(id), _) if id.get().result_kind == ResultKind::Borrowed => Some(0),
            _ => None,
        };
        let call = HirExpr {
            kind: HirExprKind::MethodCall {
                receiver: Box::new(receiver),
                method: found,
                args,
                trailing,
                type_arg,
                rooted,
                looked_into: false,
                started,
            },
            ty: started_ty(started, ret),
            span,
        };
        // A collected `map` of started calls is a `Vec` of tasks.
        self.tasks_made(call)
    }

    /// Whether `.clone()` on a value of type `ty` is the derived copy of
    /// M4 spec 2.10 rather than a method of the type: on a number, `bool`,
    /// `Option`, `Result`, `Vec`, `HashMap`, or `Error`, and on a struct or enum
    /// with no member of that name. `string` has its own row, and a chain
    /// no `clone`.
    fn derived_clone(&self, ty: &Ty) -> bool {
        let owner = match ty {
            Ty::Struct(id) => UserType::Struct(*id),
            Ty::Enum(id) => UserType::Enum(*id),
            Ty::Bool
            | Ty::Int(_)
            | Ty::Float(_)
            | Ty::Option(_)
            | Ty::Result(..)
            | Ty::Vec(_)
            | Ty::HashMap(..)
            | Ty::Error => return true,
            _ => return false,
        };
        matches!(
            self.symbols.lookup_member(self.module, owner, "clone"),
            Err(LookupError::Unknown)
        )
    }

    /// `receiver.clone()` on a type that is not `string` (M4 spec 2.10): a
    /// new value of the receiver's type, lowered as the `clone` row, which
    /// reads its receiver. On a number or `bool` it is V0100, since those
    /// are copied on use; on a type that cannot be cloned V0203, naming
    /// what prevents it.
    fn clone_call(
        &mut self,
        receiver: HirExpr,
        method: &Ident,
        args: &[Expr],
        span: Span,
    ) -> Option<HirExpr> {
        let ty = receiver.ty.clone();
        if ty.is_copy() {
            let message = format!(
                "`{}` needs no `.clone()`: numbers and `bool` are copied on use; drop \
                 `.clone()`",
                self.ty_name(&ty)
            );
            self.diagnostics
                .push(Diagnostic::new(codes::V0100, method.span, message));
            return None;
        }
        if let Err(blocker) = self.derives.can_clone(&ty) {
            let headline = format!("`{}` cannot be copied with `.clone()`", self.ty_name(&ty));
            let rust = "in Rust terms, `.clone()` needs the type to implement `Clone`, which \
                        Varyk derives for a struct or enum whose every field and payload has \
                        it, and reads from `#[derive(..)]` on a Rust type";
            self.blocked(span, headline, blocker, rust);
            return None;
        }
        let (args, _) = self.arguments(&method.name, &[], &ArgRules::default(), args, span)?;
        // The row is always in the table.
        let id = builtins::lookup(Owner::String, "clone", true)?;
        Some(HirExpr {
            kind: HirExprKind::MethodCall {
                receiver: Box::new(receiver),
                method: MethodRef::Builtin(id),
                args,
                trailing: Vec::new(),
                type_arg: None,
                rooted: None,
                looked_into: false,
                started: false,
            },
            ty,
            span,
        })
    }

    /// The arguments of row `id`, one of whose parameters is a closure
    /// (M4 spec 2.2): each closure is typed at the call, handed the type
    /// its row says, with its body expected to give what the call's
    /// `expected` type holds in `R`'s place; `R` is then what the body
    /// gives. Returns the arguments and `subst` with `R` filled in.
    fn closure_arguments(
        &mut self,
        id: BuiltinId,
        mut subst: Subst,
        args: &[Expr],
        expected: Option<Ty>,
        span: Span,
    ) -> Option<(Vec<HirExpr>, Subst)> {
        let entry = id.get();
        if args.len() != entry.params.len() {
            // The count is all `arguments` looks at here.
            let params = vec![Ty::Unit; entry.params.len()];
            return self
                .arguments(entry.name, &params, &ArgRules::default(), args, span)
                .map(|(args, _)| (args, subst));
        }
        let wanted = match (entry.result, expected) {
            (builtins::Shape::OptionOfR, Some(Ty::Option(r))) => Some(*r),
            (builtins::Shape::ResultOfTAndR, Some(Ty::Result(_, r))) => Some(*r),
            _ => None,
        };
        let mut checked = Vec::new();
        for (arg, shape) in args.iter().zip(entry.params) {
            let builtins::Shape::Closure(closure) = shape else {
                let ty = shape.ty(&subst);
                checked.push(
                    self.expr(arg, Some(ty.clone()))
                        .and_then(|arg| self.expect(arg, &ty)),
                );
                continue;
            };
            let param = closure.param.ty(&subst);
            let result = match closure.result {
                ClosureResult::Bool => Some(Ty::Bool),
                ClosureResult::Any => wanted.clone(),
            };
            let Some((closure_expr, ty)) = closures::check(self, id, arg, param, result.clone())
            else {
                checked.push(None);
                continue;
            };
            if let (Some(Ty::Bool), ClosureResult::Bool) = (&result, closure.result) {
                if ty != Ty::Bool {
                    let at = closure_expr_tail_span(&closure_expr);
                    self.mismatch(at, &Ty::Bool, &ty);
                    checked.push(None);
                    continue;
                }
            }
            subst.r = ty;
            checked.push(Some(closure_expr));
        }
        let args: Option<Vec<HirExpr>> = checked.into_iter().collect();
        Some((args?, subst))
    }

    /// The type `parse()` (at `span`) reads, taken from `expected` as
    /// `None`'s is (M4 spec 2.7, M5a spec 2.8): the `X` of an expected
    /// `Result<X, _>`, a number type or `bool` (V0200 otherwise). The
    /// result is always `Result<X, Error>`, so another error type is a
    /// mismatch, or V0206 under `?`. V0206 when it is the operand of `?` in
    /// a function returning `Option`, and V0207 with nothing expected, each
    /// showing the two statements that make an `Option` of it.
    fn parsed_type(&mut self, expected: Option<&Ty>, span: Span) -> Option<Ty> {
        // Two statements, since the checker never types a receiver from
        // the call made on it (M5a spec 2.8).
        let shape = "let parsed: Result<i32, Error> = text.parse();` then `parsed.ok()";
        match expected {
            Some(Ty::Result(inner, _))
                if matches!(**inner, Ty::Int(_) | Ty::Float(_) | Ty::Bool) =>
            {
                Some((**inner).clone())
            }
            Some(Ty::Result(inner, _)) => {
                let message = format!(
                    "`parse` reads a number or `bool` from text, and cannot make a `{}`",
                    self.ty_name(inner)
                );
                self.diagnostics
                    .push(Diagnostic::new(codes::V0200, span, message));
                None
            }
            // `text.parse()?` in a function returning `Option`: the
            // expected `Option` came through `?`, and `parse` gives a
            // `Result`.
            Some(Ty::Option(..)) if self.option_try_operand == Some(span) => {
                self.result_under_option_question(span, "parse", shape);
                None
            }
            _ => {
                let what = "the type `parse()` reads";
                self.type_hole(span, expected, RESULT_HOLE, what, shape);
                None
            }
        }
    }

    /// V0206 at `span` for `call(..)?`, whose `Result<_, Error>` meets `?`
    /// in a function returning `Option`; the note shows the two
    /// statements of `shape`.
    pub(super) fn result_under_option_question(&mut self, span: Span, call: &str, shape: &str) {
        let message = format!(
            "`?` works on an `Option`, and this is `Result<_, Error>`; `{}` returns `{}`",
            self.name,
            self.ty_name(&self.ret)
        );
        self.diagnostics.push(
            Diagnostic::new(codes::V0206, span, message)
                .with_note(format!(
                    "`{call}` gives a `Result`; in a function returning `Option`, write two \
                     statements: `{shape}?`"
                ))
                .with_note(
                    "in Rust terms, `?` returns early with the `Err` or the `None`, so the \
                     function's return type must be a `Result` with the same error type, or an \
                     `Option`",
                ),
        );
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

    /// `operand?` (spec 2.8, M4 spec 2.6). In a function returning
    /// `Result<T, E>` the operand is a `Result<U, E>` with the same `E`; in
    /// one returning `Option<T>` it is an `Option<U>`; the value is the `U`.
    /// Anything else is V0206 naming the function's return type and the
    /// operand's type: there is no conversion of any kind.
    ///
    /// `expected` is the type wanted of the whole `operand?`; it becomes
    /// the operand's `U`. With nothing expected an `Ok(x)` operand takes
    /// its `U` from `x` and the function's error type, and `Some(x)` from
    /// `x`; `Err(e)?` and `None?` have nothing to take a `U` from (V0207).
    pub(super) fn try_(
        &mut self,
        operand: &Expr,
        expected: Option<Ty>,
        span: Span,
    ) -> Option<HirExpr> {
        self.leaves_closure("`?`", span)?;
        let ret = self.ret.clone();
        // A constructor of the other family (or in a function that has none)
        // is a misuse whatever its payload: decided before any expected
        // type goes down, so it is V0206 and never a hole or a mismatch.
        let family = constructor_family(operand);
        let misplaced = match (&ret, &family) {
            (Ty::Option(_), Some(Shape::Result(_))) => true,
            (Ty::Result(..), Some(Shape::Option)) => true,
            (Ty::Option(_) | Ty::Result(..), _) => false,
            (_, family) => family.is_some(),
        };
        let wanted = match (&ret, expected) {
            (_, _) if misplaced => None,
            (Ty::Option(_), Some(ty)) => Some(Ty::Option(Box::new(ty))),
            (Ty::Result(_, err), Some(ty)) => Some(Ty::Result(Box::new(ty), err.clone())),
            _ => None,
        };
        let typed = if misplaced {
            None
        } else {
            Some(match (&wanted, &ret, ok_argument(operand)) {
                (None, Ty::Result(_, err), Some(arg)) => {
                    let err = err.clone();
                    let arg = self.expr(arg, None)?;
                    let ty = Ty::Result(Box::new(arg.ty.clone()), err);
                    HirExpr {
                        kind: HirExprKind::EnumLit {
                            variant: VariantRef::Ok,
                            args: vec![arg],
                            fields: None,
                            write_full_type: false,
                        },
                        ty,
                        span: operand.span,
                    }
                }
                _ => {
                    let before = self.diagnostics.len();
                    let outer = self.option_try_operand.take();
                    if let Ty::Option(..) = ret {
                        self.option_try_operand = Some(operand.span);
                    }
                    let checked = self.expr(operand, wanted);
                    self.option_try_operand = outer;
                    let Some(checked) = checked else {
                        // `Err(e)?;` has no value type to take from anywhere.
                        if is_err_call(operand) {
                            if let Some(hole) = self.diagnostics.get_mut(before) {
                                if hole.code == codes::V0207 {
                                    hole.notes.push(
                                        "to give up with an error, write `return Err(..);`"
                                            .to_string(),
                                    );
                                }
                            }
                        }
                        return None;
                    };
                    checked
                }
            })
        };
        let (found, shape) = match (&typed, &family) {
            (Some(operand), _) => (
                self.ty_name(&operand.ty),
                match &operand.ty {
                    Ty::Result(_, err) => Shape::Result(Some((**err).clone())),
                    Ty::Option(_) => Shape::Option,
                    _ => Shape::Other,
                },
            ),
            (None, Some(Shape::Option)) => ("Option<_>".to_string(), Shape::Option),
            (None, _) => ("Result<_, _>".to_string(), Shape::Result(None)),
        };
        let name = self.name;
        let (message, note) = match (&ret, shape) {
            (Ty::Result(_, want), Shape::Result(Some(err))) if err == *want.as_ref() => {
                let operand = typed?;
                let Ty::Result(ok, _) = &operand.ty else {
                    return None;
                };
                let ty = (**ok).clone();
                return Some(self.question(operand, TryKind::Result, ty, span));
            }
            (Ty::Option(_), Shape::Option) => {
                let operand = typed?;
                let Ty::Option(inner) = &operand.ty else {
                    return None;
                };
                let ty = (**inner).clone();
                return Some(self.question(operand, TryKind::Option, ty, span));
            }
            (Ty::Result(_, want), Shape::Result(_)) => (
                format!(
                    "`?` hands the error of this `{found}` back to the caller, but `{name}` \
                     returns `{}`, whose error type is `{}`",
                    self.ty_name(&ret),
                    self.ty_name(want)
                ),
                "the error types must be the same: an error is never converted into \
                 another type",
            ),
            (Ty::Result(..), other) => (
                format!(
                    "`?` works on a `Result`, and this is `{found}`; `{name}` returns `{}`",
                    self.ty_name(&ret)
                ),
                if matches!(other, Shape::Option) {
                    "`?` on an `Option` needs a function that returns an `Option`; use \
                     `match` to look inside it"
                } else {
                    "`?` takes the value out of an `Ok`, or returns the error of an `Err`"
                },
            ),
            (Ty::Option(_), other) => (
                format!(
                    "`?` works on an `Option`, and this is `{found}`; `{name}` returns `{}`",
                    self.ty_name(&ret)
                ),
                if matches!(other, Shape::Result(_)) {
                    "`?` on a `Result` needs a function that returns a `Result`; use \
                     `match` to look inside it"
                } else {
                    "`?` takes the value out of a `Some`, or returns `None` from the function"
                },
            ),
            (Ty::Unit, _) => (
                format!(
                    "`?` needs a function that returns a `Result` or an `Option`, but `{name}` \
                     returns nothing (this value is `{found}`)"
                ),
                RESULT_NOTE,
            ),
            (ret, _) => (
                format!(
                    "`?` needs a function that returns a `Result` or an `Option`, but `{name}` \
                     returns `{}` (this value is `{found}`)",
                    self.ty_name(ret)
                ),
                RESULT_NOTE,
            ),
        };
        self.diagnostics.push(
            Diagnostic::new(codes::V0206, span, message)
                .with_note(note)
                .with_note("in Rust terms, `?` returns early with the `Err` or the `None`, so the function's return type must be a `Result` with the same error type, or an `Option`"),
        );
        None
    }

    /// `operand?` of type `ty`. A built-in constructor written directly
    /// under `?` has type arguments rustc cannot infer, so the backend
    /// writes them out.
    fn question(&self, mut operand: HirExpr, kind: TryKind, ty: Ty, span: Span) -> HirExpr {
        if let HirExprKind::EnumLit {
            variant: VariantRef::Ok | VariantRef::Err | VariantRef::None,
            write_full_type,
            ..
        } = &mut operand.kind
        {
            *write_full_type = true;
        }
        HirExpr {
            kind: HirExprKind::Try {
                operand: Box::new(operand),
                kind,
            },
            ty,
            span,
        }
    }
}

/// Where a closure's value is given: its body's tail, or the whole
/// closure for a body without one.
fn closure_expr_tail_span(closure: &HirExpr) -> Span {
    match &closure.kind {
        HirExprKind::Closure { body, .. } => body.tail.as_ref().map_or(body.span, |tail| tail.span),
        _ => closure.span,
    }
}

/// The kind of value a `?` operand is, as far as it is known.
#[derive(Clone)]
enum Shape {
    Result(Option<Ty>),
    Option,
    Other,
}

/// The family of the built-in constructor `expr` is (`Ok(..)`, `Err(..)`,
/// `Some(..)`, `None`), if it is one.
fn constructor_family(expr: &Expr) -> Option<Shape> {
    let name = match &expr.kind {
        ExprKind::Call { callee, .. } => match &callee.kind {
            ExprKind::Path { path: None, name } => name,
            _ => return None,
        },
        ExprKind::Path { path: None, name } => name,
        _ => return None,
    };
    match name.name.as_str() {
        "Some" | "None" => Some(Shape::Option),
        "Ok" | "Err" => Some(Shape::Result(None)),
        _ => None,
    }
}

/// Whether `expr` is a call of `Err`.
fn is_err_call(expr: &Expr) -> bool {
    let ExprKind::Call { callee, .. } = &expr.kind else {
        return false;
    };
    matches!(&callee.kind, ExprKind::Path { path: None, name } if name.name == "Err")
}

/// The argument of `expr` when it is `Ok(argument)`: the one built-in
/// constructor whose value type `?` can take from its argument alone.
fn ok_argument(expr: &Expr) -> Option<&Expr> {
    let ExprKind::Call { callee, args } = &expr.kind else {
        return None;
    };
    let ExprKind::Path { path: None, name } = &callee.kind else {
        return None;
    };
    match args.as_slice() {
        [arg] if name.name == "Ok" => Some(arg),
        _ => None,
    }
}

/// Why `?` needs a function returning a `Result` or an `Option` (spec 2.8).
const RESULT_NOTE: &str = "on an error, `?` returns it from the function at once, so the \
                           function must return a `Result` (or, for an `Option`, an `Option`); \
                           `main` may return a `Result` whose error is `Error`";

/// The built-in type `owner` with its article, as a message says it:
/// "a `Vec`", "an `Option`", "a chain".
pub(super) fn owner_words(owner: Owner) -> String {
    match owner {
        Owner::Chain => "a chain".to_string(),
        Owner::Option | Owner::Error => format!("an `{}`", owner.name()),
        _ => format!("a `{}`", owner.name()),
    }
}

/// `names` as a list in words: "`a`, `b`, and `c`", or "`a` and `b`".
pub(super) fn listing(names: &[&str]) -> String {
    let quoted: Vec<String> = names.iter().map(|name| format!("`{name}`")).collect();
    match quoted.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] if init.len() == 1 => format!("{} and {last}", init[0]),
        [init @ .., last] => format!("{}, and {last}", init.join(", ")),
    }
}
