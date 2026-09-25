//! Typing of values and constructors (spec 2.2, 2.5, 2.6, 2.9): paths used
//! as values, variant values, associated functions (`Type::name(..)`,
//! `m::Type::name(..)`, `Vec::new()`), `Some`/`None`/`Ok`/`Err`,
//! `vec![..]`, and the type-hole rule for a value whose type comes from
//! where it goes.

use varyk_syntax::{Expr, Ident, Span};

use super::{FnChecker, is_builtin_variant};
use crate::builtins::{self, Owner};
use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{HirExpr, HirExprKind, VariantRef};
use crate::resolve::{Callee, EnumId, LookupError, UserType, not_visible};
use crate::types::Ty;

/// What the leading segments of a path `T::name` name.
#[derive(Debug, Clone, Copy)]
pub(super) enum PathOwner {
    User(UserType),
    Vec,
}

impl FnChecker<'_> {
    /// A path used as a value: a local, `None`, a unit variant
    /// (`Shape::Point`, `m::Shape::Point`), or something that is not a
    /// value on its own (a function such as `Counter::new` without its
    /// call).
    pub(super) fn path_value(
        &mut self,
        module: Option<&Ident>,
        type_: Option<&Ident>,
        name: &Ident,
        expected: Option<Ty>,
        span: Span,
    ) -> Option<HirExpr> {
        if let Some(owner) = self.path_owner(module, type_, span) {
            let (owner, path) = owner?;
            return self.type_member(owner, &path, name, None, expected, span, span);
        }
        if let Some(module) = module {
            let message = format!("cannot find value `{}::{}`", module.name, name.name);
            let mut diagnostic = Diagnostic::new(codes::V0100, span, message);
            if let Some(note) = self.symbols.did_you_mean(self.module, &module.name) {
                diagnostic = diagnostic.with_note(note);
            }
            self.diagnostics.push(diagnostic);
            return None;
        }
        if is_builtin_variant(&name.name) {
            return self.builtin_variant(&name.name, None, expected, span);
        }
        let id = self.lookup_local(name)?;
        Some(HirExpr {
            kind: HirExprKind::Local(id),
            ty: self.locals[id.0 as usize].ty.clone(),
            span,
        })
    }

    /// For a path `T::name` or `m::T::name` (at `span`) whose leading
    /// segments name a type (`Vec` included): that type, with its path as
    /// written. `None` when they name no type of this module (so `m::f` is
    /// a module path); `Some(None)` once a diagnostic is reported for an
    /// unknown or hidden type.
    pub(super) fn path_owner(
        &mut self,
        module: Option<&Ident>,
        type_: Option<&Ident>,
        span: Span,
    ) -> Option<Option<(PathOwner, String)>> {
        let (found, path, type_span) = match (module, type_) {
            (Some(module), Some(type_)) => (
                self.symbols
                    .lookup_type(self.module, Some(&module.name), &type_.name),
                format!("{}::{}", module.name, type_.name),
                Span::new(span.file, module.span.start, type_.span.end),
            ),
            (Some(type_), None) if type_.name == "Vec" => {
                return Some(Some((PathOwner::Vec, "Vec".to_string())));
            }
            // A type of this module first, then a module (spec 2.10).
            (Some(type_), None) => match self.symbols.lookup_type(self.module, None, &type_.name) {
                Ok(found) => (Ok(found), type_.name.clone(), type_.span),
                Err(_) => return None,
            },
            _ => return None,
        };
        match found {
            Ok(owner) => Some(Some((PathOwner::User(owner), path))),
            Err(error) => {
                let hint = type_.map(|t| t.name.as_str());
                self.lookup_error(error, "type", &path, type_span, hint);
                Some(None)
            }
        }
    }

    /// `path::name` (at `path_span`), where `path` names `owner`: a variant
    /// or an associated function (spec 2.5), called with `args` when
    /// written with parentheses. A method, a function without its call,
    /// and a name the type does not have are V0100; a non-`pub` function
    /// of another module's type is V0105.
    #[expect(
        clippy::too_many_arguments,
        reason = "a path's parts, as the parser hands them over"
    )]
    pub(super) fn type_member(
        &mut self,
        owner: PathOwner,
        path: &str,
        name: &Ident,
        args: Option<&[Expr]>,
        expected: Option<Ty>,
        path_span: Span,
        span: Span,
    ) -> Option<HirExpr> {
        let user = match owner {
            PathOwner::Vec => return self.vec_function(name, args, expected, path_span, span),
            PathOwner::User(user) => user,
        };
        if let UserType::Enum(id) = user {
            if self.symbols.enums[id.0 as usize]
                .variant(&name.name)
                .is_some()
            {
                return self.variant(id, path, name, args, span);
            }
        }
        let full = format!("{path}::{}", name.name);
        let id = match self.symbols.lookup_member(self.module, user, &name.name) {
            Ok(id) => id,
            Err(LookupError::NotVisible { decl, keyword }) => {
                self.diagnostics
                    .push(not_visible("function", &full, path_span, decl, keyword));
                return None;
            }
            Err(LookupError::Unknown) => {
                let message = match user {
                    UserType::Enum(_) => {
                        format!("enum `{path}` has no variant or function `{}`", name.name)
                    }
                    UserType::Struct(_) => {
                        format!("type `{path}` has no function `{}`", name.name)
                    }
                };
                self.diagnostics
                    .push(Diagnostic::new(codes::V0100, path_span, message));
                return None;
            }
        };
        let sig = &self.symbols.fns[id.0 as usize];
        let message = if sig.self_mode.is_some() {
            format!(
                "`{full}` is a method, so it is called on a value of its type, as in \
                 `x.{}()`",
                name.name
            )
        } else if args.is_none() {
            format!("`{full}` is a function, not a value; call it, as in `{full}(..)`")
        } else {
            let params: Vec<Ty> = sig.params.iter().map(|p| p.1.clone()).collect();
            let ret = sig.ret.clone();
            let args = self.arguments(&full, &params, args.unwrap_or_default(), span)?;
            return Some(HirExpr {
                kind: HirExprKind::Call {
                    callee: Callee::Varyk(id),
                    args,
                },
                ty: ret,
                span,
            });
        };
        self.diagnostics
            .push(Diagnostic::new(codes::V0100, path_span, message));
        None
    }

    /// `Vec::name(args)` (at `path_span`): an associated function of the
    /// built-in table, `Vec::new()`, whose element type comes from
    /// `expected` (spec 2.6).
    fn vec_function(
        &mut self,
        name: &Ident,
        args: Option<&[Expr]>,
        expected: Option<Ty>,
        path_span: Span,
        span: Span,
    ) -> Option<HirExpr> {
        let full = format!("Vec::{}", name.name);
        let found = builtins::lookup(Owner::Vec, &name.name, false);
        let (Some(id), Some(args)) = (found, args) else {
            let message = match found {
                None => format!(
                    "`Vec` has no function `{}`; the only one is `Vec::new()`",
                    name.name
                ),
                Some(_) => {
                    format!("`{full}` is a function, not a value; call it, as in `{full}()`")
                }
            };
            self.diagnostics
                .push(Diagnostic::new(codes::V0100, path_span, message));
            return None;
        };
        let entry = id.get();
        let Some(Ty::Vec(element)) = expected.clone() else {
            if args.len() == entry.params.len() {
                let what = "the element type of `Vec::new()`";
                let shape = "let v: Vec<i32> = Vec::new();";
                self.type_hole(span, expected.as_ref(), "Vec<_>", what, shape);
            } else {
                self.arguments(&full, &[], args, span);
            }
            return None;
        };
        let params: Vec<Ty> = entry.params.iter().map(|p| p.ty(&element)).collect();
        let args = self.arguments(&full, &params, args, span)?;
        Some(HirExpr {
            kind: HirExprKind::Call {
                callee: Callee::Builtin(id),
                args,
            },
            ty: entry.result.ty(&element),
            span,
        })
    }

    /// The variant `name` of enum `id` (written `path::name`) as a value:
    /// with `args` when written with parentheses. The payload count must
    /// match the variant (V0201), and each payload is typed against the
    /// variant's type.
    fn variant(
        &mut self,
        id: EnumId,
        path: &str,
        name: &Ident,
        args: Option<&[Expr]>,
        span: Span,
    ) -> Option<HirExpr> {
        let def = &self.symbols.enums[id.0 as usize];
        let full = format!("{path}::{}", name.name);
        let index = def
            .variant(&name.name)
            .expect("`type_member` found the variant");
        let payload = def.variants[index].1.clone();
        let args = self.payload(&full, &payload, args, span)?;
        Some(HirExpr {
            kind: HirExprKind::EnumLit {
                variant: VariantRef::User(id, index),
                args,
            },
            ty: Ty::Enum(id),
            span,
        })
    }

    /// Checks `args` (`None` when written without parentheses) against
    /// `payload`, the types a variant `name` holds: V0201 for a count that
    /// does not match.
    fn payload(
        &mut self,
        name: &str,
        payload: &[Ty],
        args: Option<&[Expr]>,
        span: Span,
    ) -> Option<Vec<HirExpr>> {
        let count = args.map_or(0, <[Expr]>::len);
        if count != payload.len() || (args.is_some() && payload.is_empty()) {
            let n = payload.len();
            let values = if n == 1 { "value" } else { "values" };
            let message = match args {
                _ if n == 0 => {
                    format!("`{name}` holds no values; write it without parentheses: `{name}`")
                }
                None => format!(
                    "`{name}` holds {n} {values}; write the {values} in parentheses after it: \
                     `{name}({})`",
                    vec!["..."; n].join(", ")
                ),
                Some(_) => format!(
                    "`{name}` holds {n} {values} but {count} {} given",
                    if count == 1 { "was" } else { "were" }
                ),
            };
            self.diagnostics
                .push(Diagnostic::new(codes::V0201, span, message));
            return None;
        }
        let mut checked = Vec::new();
        for (arg, ty) in args.unwrap_or_default().iter().zip(payload) {
            checked.push(
                self.expr(arg, Some(ty.clone()))
                    .and_then(|arg| self.expect(arg, ty)),
            );
        }
        checked.into_iter().collect()
    }

    /// `Some(x)`, `None`, `Ok(x)`, or `Err(e)` (`args` is `None` without
    /// parentheses). `Some(x)` knows its type from `x`; `None`, `Ok`'s
    /// error type, and `Err`'s value type come from `expected` (spec 2.6).
    pub(super) fn builtin_variant(
        &mut self,
        name: &str,
        args: Option<&[Expr]>,
        expected: Option<Ty>,
        span: Span,
    ) -> Option<HirExpr> {
        let (variant, count) = match name {
            "Some" => (VariantRef::Some, 1),
            "None" => (VariantRef::None, 0),
            "Ok" => (VariantRef::Ok, 1),
            _ => (VariantRef::Err, 1),
        };
        // The payload's type, as far as `expected` says.
        let known = match (&variant, &expected) {
            (VariantRef::Some, Some(Ty::Option(inner))) => Some((**inner).clone()),
            (VariantRef::Ok, Some(Ty::Result(ok, _))) => Some((**ok).clone()),
            (VariantRef::Err, Some(Ty::Result(_, err))) => Some((**err).clone()),
            _ => None,
        };
        let args = match (count, &known) {
            (0, _) => self.payload(name, &[], args, span)?,
            (_, Some(ty)) => self.payload(name, std::slice::from_ref(ty), args, span)?,
            // Nothing expected: the payload is typed on its own.
            (_, None) => match args {
                Some([arg]) => vec![self.expr(arg, None)?],
                _ => {
                    self.payload(name, &[Ty::Unit], args, span);
                    return None;
                }
            },
        };
        let ty = match (&variant, expected) {
            (VariantRef::Some, _) => Ty::Option(Box::new(args[0].ty.clone())),
            (VariantRef::None, Some(ty @ Ty::Option(_))) => ty,
            (VariantRef::Ok | VariantRef::Err, Some(ty @ Ty::Result(..))) => ty,
            (_, expected) => {
                let (found, what, shape) = match variant {
                    VariantRef::Ok => (
                        format!("Result<{}, _>", self.ty_name(&args[0].ty)),
                        "the error type of this `Ok`",
                        "let r: Result<i32, string> = Ok(1);",
                    ),
                    VariantRef::Err => (
                        format!("Result<_, {}>", self.ty_name(&args[0].ty)),
                        "the value type of this `Err`",
                        "let r: Result<i32, string> = Err(\"failed\");",
                    ),
                    _ => (
                        "Option<_>".to_string(),
                        "the type of `None`",
                        "let x: Option<i32> = None;",
                    ),
                };
                self.type_hole(span, expected.as_ref(), &found, what, shape);
                return None;
            }
        };
        Some(HirExpr {
            kind: HirExprKind::EnumLit { variant, args },
            ty,
            span,
        })
    }

    /// `vec![..]`: every element has the element type `expected` says,
    /// else the first element's (spec 2.6, 2.9).
    pub(super) fn vec_lit(
        &mut self,
        elements: &[Expr],
        expected: Option<Ty>,
        span: Span,
    ) -> Option<HirExpr> {
        let mut element_ty = match &expected {
            Some(Ty::Vec(inner)) => Some((**inner).clone()),
            _ => None,
        };
        let mut lowered = Vec::new();
        let mut failed = false;
        for element in elements {
            if failed && element_ty.is_none() {
                break;
            }
            let value = self.expr(element, element_ty.clone());
            let value = match (&element_ty, value) {
                (Some(ty), Some(value)) => self.expect(value, ty),
                (_, value) => value,
            };
            match value {
                Some(value) => {
                    element_ty.get_or_insert_with(|| value.ty.clone());
                    lowered.push(value);
                }
                None => failed = true,
            }
        }
        if failed {
            return None;
        }
        let Some(element_ty) = element_ty else {
            let (what, shape) = ("the type of an empty `vec![]`", "let v: Vec<i32> = vec![];");
            self.type_hole(span, expected.as_ref(), "Vec<_>", what, shape);
            return None;
        };
        Some(HirExpr {
            kind: HirExprKind::VecLit(lowered),
            ty: Ty::Vec(Box::new(element_ty)),
            span,
        })
    }

    /// A value whose type is not fully known from itself (`found`, with
    /// `_` for the unknown part): V0200 when something else is expected,
    /// else V0207 saying `what` cannot be worked out, with `shape`, a `let`
    /// that writes the type.
    fn type_hole(
        &mut self,
        span: Span,
        expected: Option<&Ty>,
        found: &str,
        what: &str,
        shape: &str,
    ) {
        if let Some(expected) = expected {
            let message = format!(
                "mismatched types: expected `{}`, found `{found}`",
                self.ty_name(expected)
            );
            self.diagnostics
                .push(Diagnostic::new(codes::V0200, span, message));
            return;
        }
        let message = format!("{what} cannot be worked out here; write the type, as in `{shape}`");
        let diagnostic = Diagnostic::new(codes::V0207, span, message).with_note(
            "such a value takes its type from where it goes: a `let` with a written type, a \
             parameter, a return value, a field, or a `vec!` element after one whose type is \
             known; in Rust terms, the type cannot be inferred here",
        );
        self.diagnostics.push(diagnostic);
    }
}
