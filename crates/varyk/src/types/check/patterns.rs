//! Typing of `match` and its patterns (spec 2.3): the value matched on
//! must be an enum, an `Option`, or a `Result`, stored somewhere or made
//! right there; each pattern is typed against it; every variant must have
//! an arm unless the last arm is a catch-all; and the arms have one type,
//! as the branches of an `if` do.

use std::collections::HashMap;

use varyk_syntax::{Expr, Ident, MatchArm, Pattern, Span, SubPattern};

use super::{FnChecker, Scope, expr_diverges, is_builtin_variant};
use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{HirArm, HirExpr, HirExprKind, HirPattern, VariantRef, is_block_like, is_place};
use crate::resolve::UserType;
use crate::types::Ty;

/// A variant of the type matched on: its name as written in a pattern
/// (`Shape::Point`, `Some`), what it is, and its payload types.
struct Variant {
    name: String,
    variant: VariantRef,
    payload: Vec<Ty>,
}

/// A pattern once its variant is known, before its names are bound.
enum Shape<'p> {
    Wildcard,
    Name(&'p Ident),
    /// The variant's index into the scrutinee's variants, and its
    /// positions.
    Variant(usize, &'p [SubPattern]),
}

impl FnChecker<'_> {
    /// `match scrutinee { arms }` (spec 2.3). Each arm's body is checked
    /// with the `match`'s expected type, or else the type of the first
    /// arm that does not always leave early, which it must then have.
    pub(super) fn match_expr(
        &mut self,
        scrutinee: &Expr,
        arms: &[MatchArm],
        expected: Option<Ty>,
        span: Span,
    ) -> Option<HirExpr> {
        let head = self.expr(scrutinee, None)?;
        self.check_head(&head, "the value matched on", "match on")?;
        let Some(variants) = self.variants_of(&head.ty) else {
            let message = format!(
                "`match` looks at an enum, an `Option`, or a `Result`, and this is `{}`; to \
                 compare numbers, `bool`s, or strings, use `if`",
                self.ty_name(&head.ty)
            );
            self.diagnostics
                .push(Diagnostic::new(codes::V0205, head.span, message).with_note("in Rust terms, `match` on other types needs literal patterns, which come in a later milestone"));
            return None;
        };

        let mut failed = false;
        let mut shapes = Vec::new();
        for (index, arm) in arms.iter().enumerate() {
            let shape = self.pattern_shape(&arm.pattern, &head.ty, &variants);
            if let Some(Shape::Wildcard | Shape::Name(_)) = shape {
                if index + 1 < arms.len() {
                    self.diagnostics.push(
                        Diagnostic::new(
                            codes::V0205,
                            arm.span,
                            "this arm matches every value, so the arms after it could never \
                             run; make it the last arm",
                        )
                        .with_note("in Rust terms, the later patterns are unreachable"),
                    );
                    failed = true;
                }
            }
            failed |= shape.is_none();
            shapes.push(shape);
        }

        let mut lowered = Vec::new();
        let mut value_ty: Option<(Ty, Span)> = None;
        for (arm, shape) in arms.iter().zip(&shapes) {
            self.scopes.push(Scope::new());
            let pattern = self.bind_pattern(&arm.pattern, shape.as_ref(), &head.ty, &variants);
            let arm_expected = expected
                .clone()
                .or(value_ty.as_ref().map(|(ty, _)| ty.clone()));
            let body = self.expr(&arm.body, arm_expected);
            self.scopes.pop();
            let (Some(pattern), Some(body)) = (pattern, body) else {
                failed = true;
                continue;
            };
            if !expr_diverges(&body) {
                match &value_ty {
                    None => value_ty = Some((body.ty.clone(), body.span)),
                    Some((ty, first)) if *ty != body.ty => {
                        let message = format!(
                            "`match` arms have different types: expected `{}`, found `{}`",
                            self.ty_name(ty),
                            self.ty_name(&body.ty)
                        );
                        let label = format!("this arm is `{}`", self.ty_name(ty));
                        self.diagnostics.push(
                            Diagnostic::new(codes::V0200, body.span, message)
                                .with_label(*first, label),
                        );
                        failed = true;
                    }
                    Some(_) => {}
                }
            }
            lowered.push(HirArm {
                pattern,
                body,
                span: arm.span,
            });
        }
        if failed {
            return None;
        }

        let catch_all = matches!(shapes.last(), Some(Some(Shape::Wildcard | Shape::Name(_))));
        if !catch_all {
            let missing: Vec<&str> = (0..variants.len())
                .filter(|index| {
                    !shapes
                        .iter()
                        .any(|shape| matches!(shape, Some(Shape::Variant(i, _)) if i == index))
                })
                .map(|index| variants[index].name.as_str())
                .collect();
            if !missing.is_empty() {
                let at = Span::new(span.file, span.start, head.span.end);
                let message = format!("this `match` does not handle {}", either(&missing));
                let diagnostic = Diagnostic::new(codes::V0204, at, message).with_note(
                    "add an arm for each missing variant, or end with `_ => ...` to handle every \
                     other value; in Rust terms, the patterns are not exhaustive",
                );
                self.diagnostics.push(diagnostic);
                return None;
            }
        }

        let ty = match value_ty {
            Some((ty, _)) => ty,
            // Every arm always leaves early: the `match` fits wherever it
            // is used, like a block that always returns.
            None => lowered
                .first()
                .map_or(expected.unwrap_or(Ty::Unit), |arm| arm.body.ty.clone()),
        };
        Some(HirExpr {
            kind: HirExprKind::Match {
                scrutinee: Box::new(head),
                arms: lowered,
            },
            ty,
            span,
        })
    }

    /// V0001 for a value matched on or looped over (`subject`) that is
    /// neither a place nor a temporary (spec 2.3, 2.4): an `if`, a block,
    /// or a `match`, or a field or element of anything but a place. `verb`
    /// says what to do with the stored value instead.
    pub(super) fn check_head(&mut self, head: &HirExpr, subject: &str, verb: &str) -> Option<()> {
        let message = if is_block_like(head) {
            format!(
                "{subject} cannot be an `if`, a block, or a `match`; store it with `let` first, \
                 then {verb} that"
            )
        } else if matches!(
            head.kind,
            HirExprKind::Field { .. } | HirExprKind::Index { .. }
        ) && !is_place(head)
        {
            format!(
                "{subject} cannot be a part of a value that is not stored anywhere; store that \
                 value with `let` first, then {verb} its part"
            )
        } else {
            return Some(());
        };
        self.diagnostics
            .push(Diagnostic::new(codes::V0001, head.span, message));
        None
    }

    /// The variants of `ty`, or `None` when it is not an enum, an
    /// `Option`, or a `Result`.
    fn variants_of(&self, ty: &Ty) -> Option<Vec<Variant>> {
        let variant = |name: &str, variant, payload| Variant {
            name: name.to_string(),
            variant,
            payload,
        };
        Some(match ty {
            Ty::Enum(id) => {
                let def = &self.symbols.enums[id.0 as usize];
                def.variants
                    .iter()
                    .enumerate()
                    .map(|(index, (name, payload))| {
                        let name = format!("{}::{name}", def.name);
                        variant(&name, VariantRef::User(*id, index), payload.clone())
                    })
                    .collect()
            }
            Ty::Option(inner) => vec![
                variant("Some", VariantRef::Some, vec![(**inner).clone()]),
                variant("None", VariantRef::None, Vec::new()),
            ],
            Ty::Result(ok, err) => vec![
                variant("Ok", VariantRef::Ok, vec![(**ok).clone()]),
                variant("Err", VariantRef::Err, vec![(**err).clone()]),
            ],
            _ => return None,
        })
    }

    /// Which of `variants` (those of `ty`) `pattern` names, with its
    /// positions checked against the variant's payload: V0205 for a
    /// variant of another type or the wrong number of positions.
    fn pattern_shape<'p>(
        &mut self,
        pattern: &'p Pattern,
        ty: &Ty,
        variants: &[Variant],
    ) -> Option<Shape<'p>> {
        let (index, subpatterns, parens, span) = match pattern {
            Pattern::Wildcard(_) => return Some(Shape::Wildcard),
            Pattern::Name(name) if !is_builtin_variant(&name.name) => {
                return Some(Shape::Name(name));
            }
            Pattern::Name(name) => (
                self.builtin_pattern(&name.name, ty, variants, name.span)?,
                &[][..],
                false,
                name.span,
            ),
            Pattern::Variant(pattern) => {
                let index = match (&pattern.module, &pattern.type_) {
                    (None, None) if is_builtin_variant(&pattern.name.name) => {
                        self.builtin_pattern(&pattern.name.name, ty, variants, pattern.span)?
                    }
                    (None, None) => {
                        let name = &pattern.name.name;
                        let mut message =
                            format!("`{name}` is not a variant of `{}`", self.ty_name(ty));
                        if let Some(found) = variants
                            .iter()
                            .find(|v| v.name.ends_with(&format!("::{name}")))
                        {
                            message.push_str(&format!(
                                "; a variant is written with its enum's name, as in `{}`",
                                found.name
                            ));
                        }
                        self.diagnostics
                            .push(Diagnostic::new(codes::V0205, pattern.span, message).with_note("in Rust terms, the pattern's type does not match the type of the value matched on"));
                        return None;
                    }
                    (module, Some(type_)) => {
                        self.user_pattern(module.as_ref(), type_, &pattern.name, ty, pattern.span)?
                    }
                    (Some(_), None) => unreachable!("a pattern's one segment fills `type_`"),
                };
                let parens = pattern.span.end > pattern.name.span.end;
                (index, &pattern.subpatterns[..], parens, pattern.span)
            }
        };

        let variant = &variants[index];
        let n = variant.payload.len();
        let values = if n == 1 { "value" } else { "values" };
        let message = if n == 0 && parens {
            format!(
                "`{0}` holds no values; write it without parentheses: `{0}`",
                variant.name
            )
        } else if n > 0 && !parens {
            format!(
                "`{}` holds {n} {values}; write a name or `_` for each in parentheses: `{}({})`",
                variant.name,
                variant.name,
                vec!["_"; n].join(", ")
            )
        } else if subpatterns.len() != n {
            format!(
                "`{}` holds {n} {values}, so its pattern needs {n} position{}, but this one has {}",
                variant.name,
                if n == 1 { "" } else { "s" },
                subpatterns.len()
            )
        } else {
            return Some(Shape::Variant(index, subpatterns));
        };
        self.diagnostics.push(
            Diagnostic::new(codes::V0205, span, message).with_note(
                "in Rust terms, a tuple variant's pattern has one sub-pattern per field",
            ),
        );
        None
    }

    /// The index of `Some`, `None`, `Ok`, or `Err` (`name`, at `span`)
    /// among `variants`, those of `ty`: V0205 unless `ty` has it.
    fn builtin_pattern(
        &mut self,
        name: &str,
        ty: &Ty,
        variants: &[Variant],
        span: Span,
    ) -> Option<usize> {
        let found = variants
            .iter()
            .position(|v| v.name == name && !matches!(v.variant, VariantRef::User(..)));
        if found.is_none() {
            let owner = if matches!(name, "Some" | "None") {
                "Option"
            } else {
                "Result"
            };
            let message = format!(
                "`{name}` is a variant of `{owner}`, but this value is `{}`",
                self.ty_name(ty)
            );
            self.diagnostics
                .push(Diagnostic::new(codes::V0205, span, message).with_note(
                "in Rust terms, the pattern's type does not match the type of the value matched on",
            ));
        }
        found
    }

    /// The index of the variant `type_::name` (or `module::type_::name`)
    /// among those of `ty`: V0100 or V0105 for a type that cannot be found
    /// or seen, V0205 for a variant of another type.
    fn user_pattern(
        &mut self,
        module: Option<&Ident>,
        type_: &Ident,
        name: &Ident,
        ty: &Ty,
        span: Span,
    ) -> Option<usize> {
        let path = match module {
            Some(module) => format!("{}::{}", module.name, type_.name),
            None => type_.name.clone(),
        };
        let type_span = Span::new(
            span.file,
            module.unwrap_or(type_).span.start,
            type_.span.end,
        );
        let found =
            self.symbols
                .lookup_type(self.module, module.map(|m| m.name.as_str()), &type_.name);
        let id = match found {
            Ok(UserType::Enum(id)) => id,
            Ok(UserType::Struct(_)) => {
                let message = format!("`{path}` is a struct, not an enum, so it has no variants");
                self.diagnostics
                    .push(Diagnostic::new(codes::V0205, type_span, message).with_note("in Rust terms, the pattern's type does not match the type of the value matched on"));
                return None;
            }
            Err(error) => {
                self.lookup_error(error, "type", &path, type_span, Some(&type_.name));
                return None;
            }
        };
        let def = &self.symbols.enums[id.0 as usize];
        let Some(index) = def.variant(&name.name) else {
            let message = format!("enum `{path}` has no variant `{}`", name.name);
            self.diagnostics
                .push(Diagnostic::new(codes::V0100, name.span, message));
            return None;
        };
        if *ty != Ty::Enum(id) {
            let message = format!(
                "`{path}::{}` is a variant of `{path}`, but this value is `{}`",
                name.name,
                self.ty_name(ty)
            );
            self.diagnostics
                .push(Diagnostic::new(codes::V0205, span, message).with_note(
                "in Rust terms, the pattern's type does not match the type of the value matched on",
            ));
            return None;
        }
        Some(index)
    }

    /// Binds the names of `pattern` (whose shape is `shape`, `None` when
    /// it did not check) in the arm's scope and lowers it; a name bound
    /// twice is V0103. Names of a pattern that did not check are bound as
    /// failed, so their uses in the arm stay quiet.
    fn bind_pattern(
        &mut self,
        pattern: &Pattern,
        shape: Option<&Shape>,
        ty: &Ty,
        variants: &[Variant],
    ) -> Option<HirPattern> {
        let Some(shape) = shape else {
            if let Pattern::Variant(pattern) = pattern {
                for sub in &pattern.subpatterns {
                    if let SubPattern::Name(name) = sub {
                        self.bind_poisoned(name);
                    }
                }
            }
            return None;
        };
        match shape {
            Shape::Wildcard => Some(HirPattern::Wildcard),
            Shape::Name(name) => Some(HirPattern::Binding(self.bind(name, ty.clone(), false))),
            Shape::Variant(index, subpatterns) => {
                let variant = &variants[*index];
                let mut seen: HashMap<&str, Span> = HashMap::new();
                let mut bindings = Vec::new();
                let mut failed = false;
                for (sub, ty) in subpatterns.iter().zip(&variant.payload) {
                    let SubPattern::Name(name) = sub else {
                        bindings.push(None);
                        continue;
                    };
                    if let Some(&first) = seen.get(name.name.as_str()) {
                        let diagnostic = Diagnostic::new(
                            codes::V0103,
                            name.span,
                            format!(
                                "`{}` is bound more than once in this pattern; give each \
                                 position its own name, or `_`",
                                name.name
                            ),
                        )
                        .with_label(first, format!("`{}` is first bound here", name.name));
                        self.diagnostics.push(diagnostic);
                        failed = true;
                        continue;
                    }
                    seen.insert(&name.name, name.span);
                    bindings.push(Some(self.bind(name, ty.clone(), false)));
                }
                (!failed).then_some(HirPattern::Variant {
                    variant: variant.variant,
                    bindings,
                })
            }
        }
    }
}

/// `names` as alternatives in words: "`a`", "`a` or `b`", or "`a`, `b`,
/// or `c`".
fn either(names: &[&str]) -> String {
    let quoted: Vec<String> = names.iter().map(|name| format!("`{name}`")).collect();
    match quoted.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [first, second] => format!("{first} or {second}"),
        [init @ .., last] => format!("{}, or {last}", init.join(", ")),
    }
}
