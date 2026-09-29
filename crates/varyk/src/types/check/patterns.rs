//! Typing of `match`, `if let`, and `while let` and their patterns (spec
//! 2.3, M4 spec 2.4, 2.5): the value looked at must be an enum, an
//! `Option`, a `Result`, a number, a `bool`, or a string, stored somewhere
//! or made right there; each pattern is typed against it at every depth;
//! a `match` must handle every value and every arm must be reachable
//! (`exhaustive`); and the arms have one type, as the branches of an `if`
//! do.

use std::collections::HashMap;

use varyk_syntax::{Block, Expr, Ident, Literal, MatchArm, Path, Pattern, Span};

use super::{
    FnChecker, Scope, exhaustive, expr_diverges, int_range, is_builtin_variant, opaque_variant,
    unfinished_chain,
};
use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{
    HirArm, HirBlock, HirExpr, HirExprKind, HirLiteral, HirPattern, HirStmt, VariantRef,
    is_block_like, is_place,
};
use crate::resolve::{
    LookupError, UserType, VariantFieldsDef, display_path, path_text, split_last,
};
use crate::types::{IntKind, Ty};

/// The note of a V0205 for a pattern of another type than its value.
const TYPE_NOTE: &str =
    "in Rust terms, the pattern's type does not match the type of the value matched on";

/// A variant of the type a pattern looks at: its name as written in a
/// pattern (`Shape::Point`, `Some`), what it is, and what it holds.
struct Variant {
    name: String,
    variant: VariantRef,
    fields: VariantFieldsDef,
}

/// The names a pattern binds so far, with where each is first bound.
type Seen = HashMap<String, Span>;

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
        self.check_matchable(&head, "match")?;

        let mut failed = false;
        let mut lowered = Vec::new();
        let mut value_ty: Option<(Ty, Span)> = None;
        for arm in arms {
            self.scopes.push(Scope::new());
            let pattern = self.lower_pattern(&arm.pattern, &head.ty);
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

        let at = Span::new(span.file, span.start, head.span.end);
        let patterns: Vec<(&HirPattern, Span)> =
            lowered.iter().map(|arm| (&arm.pattern, arm.span)).collect();
        let found = exhaustive::check(self.symbols, &head.ty, &patterns, at);
        if !found.is_empty() {
            self.diagnostics.extend(found);
            return None;
        }

        let ty = match value_ty {
            Some((ty, _)) => ty,
            // Every arm always leaves early: the `match` fits wherever it
            // is used, like a block that always returns.
            None => lowered
                .first()
                .map_or(expected.unwrap_or(Ty::Unit), |arm| arm.body.ty.clone()),
        };
        let head_is_str = head.ty == Ty::String;
        Some(HirExpr {
            kind: HirExprKind::Match {
                scrutinee: Box::new(head),
                arms: lowered,
                head_is_str,
            },
            ty,
            span,
        })
    }

    /// `if let pattern = value { then } else { else_ }` (M4 spec 2.4):
    /// the head follows the rules of `match`'s, the pattern's bindings
    /// live in `then` alone, and the whole is typed like `if`.
    pub(super) fn if_let(
        &mut self,
        pattern: &Pattern,
        value: &Expr,
        then: &Block,
        else_: Option<&Block>,
        expected: Option<Ty>,
        span: Span,
    ) -> Option<HirExpr> {
        let head = self.let_head(value, "if let");
        let mut lowered = None;
        let branches = self.branches(
            |checker, expected| {
                checker.scopes.push(Scope::new());
                if let Some(head) = &head {
                    lowered = checker.lower_pattern(pattern, &head.ty);
                } else {
                    checker.poison_names(pattern);
                }
                let then = checker.block(then, expected);
                checker.scopes.pop();
                then
            },
            else_,
            expected,
        );
        let (head, pattern, (then, else_, ty)) = (head?, lowered?, branches?);
        let head_is_str = head.ty == Ty::String;
        Some(HirExpr {
            kind: HirExprKind::IfLet {
                pattern: Box::new(pattern),
                value: Box::new(head),
                then,
                else_,
                head_is_str,
            },
            ty,
            span,
        })
    }

    /// `while let pattern = value { body }` (M4 spec 2.4): a loop for
    /// `break` and `continue`, its head following the rules of `match`'s.
    pub(super) fn while_let(
        &mut self,
        pattern: &Pattern,
        value: &Expr,
        body: &Block,
        span: Span,
    ) -> Option<HirStmt> {
        let head = self.let_head(value, "while let");
        self.scopes.push(Scope::new());
        let lowered = match &head {
            Some(head) => self.lower_pattern(pattern, &head.ty),
            None => {
                self.poison_names(pattern);
                None
            }
        };
        self.loop_depth += 1;
        let body = self.block(body, Some(Ty::Unit));
        self.loop_depth -= 1;
        self.scopes.pop();
        let (head, pattern, body) = (head?, lowered?, body?);
        if body.ty != Ty::Unit {
            let tail = body.tail.as_deref()?;
            self.mismatch(tail.span, &Ty::Unit, &tail.ty);
            return None;
        }
        let head_is_str = head.ty == Ty::String;
        Some(HirStmt::WhileLet {
            pattern,
            value: head,
            body,
            head_is_str,
            span,
        })
    }

    /// The value of an `if let` or `while let` (`keyword`), checked as a
    /// `match` head.
    fn let_head(&mut self, value: &Expr, keyword: &str) -> Option<HirExpr> {
        let head = self.expr(value, None)?;
        let subject = format!("the value of `{keyword}`");
        self.check_head(&head, &subject, "look at")?;
        self.check_matchable(&head, keyword)?;
        Some(head)
    }

    /// The branches of an `if` or `if let`: `then`, checked by `then_block`
    /// with the type it should have, and `else_`, both of one type unless
    /// one always leaves early; without `else_`, `then` has type `()`.
    /// Returns them with the whole's type.
    pub(super) fn branches<F>(
        &mut self,
        then_block: F,
        else_: Option<&Block>,
        expected: Option<Ty>,
    ) -> Option<(HirBlock, Option<HirBlock>, Ty)>
    where
        F: FnOnce(&mut Self, Option<Ty>) -> Option<HirBlock>,
    {
        let Some(else_) = else_ else {
            let then = then_block(self, Some(Ty::Unit))?;
            if then.ty != Ty::Unit {
                let tail = then.tail.as_deref()?;
                let message = format!(
                    "`if` without `else` must have type `()`, found `{}`",
                    self.ty_name(&tail.ty)
                );
                self.diagnostics
                    .push(Diagnostic::new(codes::V0200, tail.span, message));
                return None;
            }
            return Some((then, None, Ty::Unit));
        };

        let then = then_block(self, expected.clone());
        // The then-branch types the else-branch's literals, unless it
        // always leaves early and so says nothing about the value.
        let then_diverges = then.as_ref().is_some_and(super::block_diverges);
        let else_expected = match &then {
            Some(then) if !then_diverges => expected.clone().or(Some(then.ty.clone())),
            _ => expected,
        };
        let else_block = self.block(else_, else_expected);
        let (then, else_block) = (then?, else_block?);

        let ty = if then_diverges {
            else_block.ty.clone()
        } else if super::block_diverges(&else_block) || else_block.ty == then.ty {
            then.ty.clone()
        } else {
            let message = format!(
                "`if` and `else` have incompatible types: expected `{}`, found `{}`",
                self.ty_name(&then.ty),
                self.ty_name(&else_block.ty)
            );
            self.diagnostics
                .push(Diagnostic::new(codes::V0200, else_block.span, message));
            return None;
        };
        Some((then, Some(else_block), ty))
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

    /// V0205 unless `head`, looked at by `keyword`, is an enum, an
    /// `Option`, a `Result`, a number, a `bool`, or a string (M4 spec
    /// 2.5); an unfinished chain is V0208 instead (M4 spec 2.3).
    fn check_matchable(&mut self, head: &HirExpr, keyword: &str) -> Option<()> {
        if head.ty.has_chain() {
            self.diagnostics.push(unfinished_chain(head.span));
            return None;
        }
        if matches!(
            head.ty,
            Ty::Enum(_)
                | Ty::Option(_)
                | Ty::Result(..)
                | Ty::Int(_)
                | Ty::Float(_)
                | Ty::Bool
                | Ty::String
        ) {
            return Some(());
        }
        let message = format!(
            "`{keyword}` looks at an enum, an `Option`, a `Result`, a number, a `bool`, or a \
             string, and this is `{}`",
            self.ty_name(&head.ty)
        );
        self.diagnostics
            .push(Diagnostic::new(codes::V0205, head.span, message).with_note(
                "to look at the fields of a struct or the elements of a `Vec`, use `if` and `for`",
            ));
        None
    }

    /// Types `pattern` against `ty`, the whole value looked at, binding
    /// its names in the current scope. When it does not check, its names
    /// are bound as failed, so their uses stay quiet.
    pub(super) fn lower_pattern(&mut self, pattern: &Pattern, ty: &Ty) -> Option<HirPattern> {
        let mut seen = Seen::new();
        self.sub_pattern(pattern, ty, true, &mut seen)
    }

    /// Binds every name of `pattern` as failed.
    fn poison_names(&mut self, pattern: &Pattern) {
        let mut names = Vec::new();
        bound_names(pattern, &mut names);
        for name in names {
            self.bind_poisoned(name);
        }
    }

    /// [`Self::lower_pattern`] for `pattern`, the whole pattern when
    /// `top`, looking at a value of type `ty`.
    fn sub_pattern(
        &mut self,
        pattern: &Pattern,
        ty: &Ty,
        top: bool,
        seen: &mut Seen,
    ) -> Option<HirPattern> {
        let lowered = self.sub_pattern_unbound(pattern, ty, top, seen);
        if lowered.is_none() {
            self.poison_names(pattern);
        }
        lowered
    }

    fn sub_pattern_unbound(
        &mut self,
        pattern: &Pattern,
        ty: &Ty,
        top: bool,
        seen: &mut Seen,
    ) -> Option<HirPattern> {
        match pattern {
            Pattern::Wildcard(_) => Some(HirPattern::Wildcard),
            Pattern::Name(name) if !is_builtin_variant(&name.name) => {
                if let Some(&first) = seen.get(&name.name) {
                    let diagnostic = Diagnostic::new(
                        codes::V0103,
                        name.span,
                        format!(
                            "`{}` is bound more than once in this pattern; give each position \
                             its own name, or `_`",
                            name.name
                        ),
                    )
                    .with_label(first, format!("`{}` is first bound here", name.name));
                    self.diagnostics.push(diagnostic);
                    return None;
                }
                seen.insert(name.name.clone(), name.span);
                Some(HirPattern::Binding(self.bind(name, ty.clone(), false)))
            }
            Pattern::Name(name) => {
                let variants = self.variants_of(ty);
                let index = self.builtin_pattern(&name.name, ty, &variants, name.span)?;
                self.positional(&variants[index], &[], false, name.span, seen)
            }
            Pattern::Variant {
                path,
                name,
                fields,
                span,
            } => {
                let variants = self.variants_of(ty);
                let index = self.variant_index(path.as_ref(), name, ty, &variants, *span)?;
                let parens = span.end > name.span.end;
                self.positional(&variants[index], fields, parens, *span, seen)
            }
            Pattern::Struct {
                path,
                name,
                fields,
                span,
            } => {
                if let Ok(UserType::Struct(_)) =
                    self.symbols
                        .lookup_type(self.module, path.as_ref(), &name.name)
                {
                    let message = format!(
                        "a pattern cannot take apart the struct `{}`; bind it to a name, then \
                         look at its fields with `if`",
                        name.name
                    );
                    self.diagnostics
                        .push(Diagnostic::new(codes::V0001, *span, message));
                    return None;
                }
                let variants = self.variants_of(ty);
                let index = self.variant_index(path.as_ref(), name, ty, &variants, *span)?;
                self.named(&variants[index], fields, *span, seen)
            }
            Pattern::Literal(literal, span) => self.literal_pattern(literal, *span, ty, top),
            Pattern::Range { start, end, span } => self.range_pattern(start, end, *span, ty),
        }
    }

    /// The index of the variant a pattern names (`path::name`, or a bare
    /// `Some`, `None`, `Ok`, or `Err`) among `variants`, those of `ty`:
    /// V0205 for a variant of another type.
    fn variant_index(
        &mut self,
        path: Option<&Path>,
        name: &Ident,
        ty: &Ty,
        variants: &[Variant],
        span: Span,
    ) -> Option<usize> {
        match path {
            None if is_builtin_variant(&name.name) => {
                self.builtin_pattern(&name.name, ty, variants, span)
            }
            None => {
                let variant = &name.name;
                let mut message = format!("`{variant}` is not a variant of `{}`", self.ty_name(ty));
                if let Some(found) = variants
                    .iter()
                    .find(|v| v.name.ends_with(&format!("::{variant}")))
                {
                    message.push_str(&format!(
                        "; a variant is written with its enum's name, as in `{}`",
                        found.name
                    ));
                }
                self.diagnostics
                    .push(Diagnostic::new(codes::V0205, span, message).with_note(TYPE_NOTE));
                None
            }
            Some(path) => self.user_pattern(path, name, ty, span),
        }
    }

    /// A variant pattern with values by position (`parens` when written
    /// with parentheses): V0205 for a variant with named fields or the
    /// wrong number of positions; each position typed against its value.
    fn positional(
        &mut self,
        variant: &Variant,
        subpatterns: &[Pattern],
        parens: bool,
        span: Span,
        seen: &mut Seen,
    ) -> Option<HirPattern> {
        let name = &variant.name;
        let types = match &variant.fields {
            VariantFieldsDef::Tuple(types) => types,
            VariantFieldsDef::Named(fields) => {
                let names: Vec<&str> = fields.iter().map(|(field, _)| field.as_str()).collect();
                let message = format!(
                    "`{name}` has named fields; write them in braces: `{name} {{ {} }}`",
                    names.join(", ")
                );
                self.diagnostics.push(
                    Diagnostic::new(codes::V0205, span, message)
                        .with_note("in Rust terms, a struct variant's pattern names its fields"),
                );
                return None;
            }
        };
        let n = types.len();
        let values = if n == 1 { "value" } else { "values" };
        let message = if n == 0 && parens {
            format!("`{name}` holds no values; write it without parentheses: `{name}`")
        } else if n > 0 && !parens {
            format!(
                "`{name}` holds {n} {values}; write a pattern for each in parentheses: \
                 `{name}({})`",
                vec!["_"; n].join(", ")
            )
        } else if subpatterns.len() != n {
            format!(
                "`{name}` holds {n} {values}, so its pattern needs {n} position{}, but this one \
                 has {}",
                if n == 1 { "" } else { "s" },
                subpatterns.len()
            )
        } else {
            let mut fields = Vec::new();
            let mut failed = false;
            for (sub, ty) in subpatterns.iter().zip(types) {
                match self.sub_pattern(sub, ty, false, seen) {
                    Some(field) => fields.push(field),
                    None => failed = true,
                }
            }
            return (!failed).then_some(HirPattern::Variant {
                variant: variant.variant,
                fields,
            });
        };
        self.diagnostics.push(
            Diagnostic::new(codes::V0205, span, message).with_note(
                "in Rust terms, a tuple variant's pattern has one sub-pattern per field",
            ),
        );
        None
    }

    /// A variant pattern with named fields (M4 spec 2.5): every field
    /// named once (V0102 for an unknown one, V0103 for one named twice,
    /// V0205 for one left out), each typed against its value; V0205 for a
    /// variant with values by position.
    fn named(
        &mut self,
        variant: &Variant,
        written: &[(Ident, Pattern)],
        span: Span,
        seen: &mut Seen,
    ) -> Option<HirPattern> {
        let name = &variant.name;
        let declared = match &variant.fields {
            VariantFieldsDef::Named(fields) => fields,
            VariantFieldsDef::Tuple(types) => {
                let message = if types.is_empty() {
                    format!("`{name}` holds no values; write it without braces: `{name}`")
                } else {
                    format!(
                        "`{name}` holds values by position; write a pattern for each in \
                         parentheses: `{name}({})`",
                        vec!["_"; types.len()].join(", ")
                    )
                };
                self.diagnostics.push(
                    Diagnostic::new(codes::V0205, span, message)
                        .with_note("in Rust terms, a tuple variant's pattern lists its fields"),
                );
                return None;
            }
        };
        let mut failed = false;
        let mut found: Vec<Option<HirPattern>> = vec![None; declared.len()];
        let mut named: HashMap<usize, Span> = HashMap::new();
        for (field, pattern) in written {
            let Some(index) = declared.iter().position(|(f, _)| *f == field.name) else {
                let message = format!("`{name}` has no field `{}`", field.name);
                self.diagnostics
                    .push(Diagnostic::new(codes::V0102, field.span, message));
                self.poison_names(pattern);
                failed = true;
                continue;
            };
            if let Some(&first) = named.get(&index) {
                let diagnostic = Diagnostic::new(
                    codes::V0103,
                    field.span,
                    format!("field `{}` is named more than once", field.name),
                )
                .with_label(first, format!("`{}` is first named here", field.name));
                self.diagnostics.push(diagnostic);
                self.poison_names(pattern);
                failed = true;
                continue;
            }
            named.insert(index, field.span);
            match self.sub_pattern(pattern, &declared[index].1, false, seen) {
                Some(lowered) => found[index] = Some(lowered),
                None => failed = true,
            }
        }
        if failed {
            return None;
        }
        let missing: Vec<String> = declared
            .iter()
            .enumerate()
            .filter(|(index, _)| !named.contains_key(index))
            .map(|(_, (field, _))| format!("`{field}`"))
            .collect();
        if !missing.is_empty() {
            let message = format!(
                "this pattern leaves out {} of `{name}`; a pattern names every field, with \
                 `name: _` for one it does not need",
                if missing.len() == 1 {
                    format!("the field {}", missing[0])
                } else {
                    format!("the fields {}", missing.join(", "))
                }
            );
            self.diagnostics.push(
                Diagnostic::new(codes::V0205, span, message)
                    .with_note("Varyk has no `..` in patterns"),
            );
            return None;
        }
        let fields = declared
            .iter()
            .zip(found)
            .filter_map(|((field, _), pattern)| Some((field.clone(), pattern?)))
            .collect();
        Some(HirPattern::Struct {
            variant: variant.variant,
            fields,
        })
    }

    /// A literal pattern (M4 spec 2.5) looking at a value of type `ty`:
    /// an integer that fits it, a `bool`, or, as the whole pattern
    /// (`top`), a string; anything else is V0205.
    fn literal_pattern(
        &mut self,
        literal: &Literal,
        span: Span,
        ty: &Ty,
        top: bool,
    ) -> Option<HirPattern> {
        let literal = match (literal, ty) {
            (Literal::Float { .. }, _) | (Literal::Int { .. }, Ty::Float(_)) => {
                self.float_pattern(span);
                return None;
            }
            (Literal::Int { text, negative }, Ty::Int(kind)) => {
                HirLiteral::Int(self.pattern_int(text, *negative, *kind, span)?)
            }
            (Literal::Str(text), Ty::String) if top => HirLiteral::Str(text.clone()),
            (Literal::Str(_), Ty::String) => {
                let message = "a string literal can only be the whole pattern of a `match`, \
                               `if let`, or `while let` on a string; bind this position to a \
                               name and compare it with `==` in the arm";
                self.diagnostics
                    .push(Diagnostic::new(codes::V0205, span, message).with_note(
                        "in Rust terms, a `String` inside another value only matches a string \
                         literal through `.as_str()`",
                    ));
                return None;
            }
            (Literal::Bool(value), Ty::Bool) => HirLiteral::Bool(*value),
            (literal, ty) => {
                let what = match literal {
                    Literal::Int { .. } => "an integer",
                    Literal::Str(_) => "a `string`",
                    Literal::Bool(_) | Literal::Float { .. } => "a `bool`",
                };
                self.pattern_mismatch(span, what, ty);
                return None;
            }
        };
        Some(HirPattern::Literal(literal))
    }

    /// `start..=end` (M4 spec 2.5): two integers that fit `ty`, the first
    /// not above the second; anything else is V0205.
    fn range_pattern(
        &mut self,
        start: &Literal,
        end: &Literal,
        span: Span,
        ty: &Ty,
    ) -> Option<HirPattern> {
        let ends = match (start, end) {
            (Literal::Float { .. }, _) | (_, Literal::Float { .. }) => None,
            (
                Literal::Int {
                    text: lo,
                    negative: lo_neg,
                },
                Literal::Int {
                    text: hi,
                    negative: hi_neg,
                },
            ) => Some(((lo, *lo_neg), (hi, *hi_neg))),
            _ => {
                let message = "a range pattern goes between two integers, as in `0..=9`";
                self.diagnostics
                    .push(Diagnostic::new(codes::V0205, span, message));
                return None;
            }
        };
        let (Some(((lo, lo_neg), (hi, hi_neg))), Ty::Int(kind)) = (ends, ty) else {
            if ends.is_none() || matches!(ty, Ty::Float(_)) {
                self.float_pattern(span);
            } else {
                self.pattern_mismatch(span, "a range of integers", ty);
            }
            return None;
        };
        let lo_value = self.pattern_int(lo, lo_neg, *kind, span)?;
        let hi_value = self.pattern_int(hi, hi_neg, *kind, span)?;
        if lo_value > hi_value {
            let message = format!(
                "the range `{lo_value}..={hi_value}` is empty, because it starts above where it \
                 ends; write the smaller end first"
            );
            self.diagnostics
                .push(Diagnostic::new(codes::V0205, span, message));
            return None;
        }
        Some(HirPattern::Range {
            lo: lo_value,
            hi: hi_value,
        })
    }

    /// The value of the integer `text` (negated when `negative`) in a
    /// pattern on a `kind`: V0205 at `span` unless it fits, as rustc
    /// refuses one that does not even inside generated code.
    fn pattern_int(
        &mut self,
        text: &str,
        negative: bool,
        kind: IntKind,
        span: Span,
    ) -> Option<i128> {
        let (min, max) = int_range(kind);
        let value = text
            .replace('_', "")
            .parse::<u128>()
            .ok()
            .filter(|value| *value <= u128::from(u64::MAX))
            .and_then(|value| i128::try_from(value).ok())
            .map(|value| if negative { -value } else { value });
        let sign = if negative { "-" } else { "" };
        match value {
            // A `-` on an unsigned type is refused even before `0`.
            Some(value) if min <= value && value <= max && !(negative && min == 0) => {
                return Some(value);
            }
            _ => {}
        }
        let message = format!(
            "`{sign}{text}` does not fit in `{}` ({min} to {max})",
            kind.name()
        );
        self.diagnostics.push(
            Diagnostic::new(codes::V0205, span, message)
                .with_note("in Rust terms, the literal is out of range for the pattern's type"),
        );
        None
    }

    /// V0205 at `span` for a pattern matching a float.
    fn float_pattern(&mut self, span: Span) {
        let message = "a number with a fractional part cannot be matched with a pattern; \
                       compare it with `if` instead";
        self.diagnostics.push(
            Diagnostic::new(codes::V0205, span, message)
                .with_note("in Rust terms, floating-point literal patterns are not allowed"),
        );
    }

    /// V0205 at `span` for a pattern that is `what` looking at a `ty`.
    fn pattern_mismatch(&mut self, span: Span, what: &str, ty: &Ty) {
        let message = format!(
            "this pattern is {what}, but the value it looks at is `{}`",
            self.ty_name(ty)
        );
        self.diagnostics
            .push(Diagnostic::new(codes::V0205, span, message).with_note(TYPE_NOTE));
    }

    /// The variants of `ty`; none when it is not an enum, an `Option`, or
    /// a `Result`.
    fn variants_of(&self, ty: &Ty) -> Vec<Variant> {
        let variant = |name: &str, variant, types: Vec<Ty>| Variant {
            name: name.to_string(),
            variant,
            fields: VariantFieldsDef::Tuple(types),
        };
        match ty {
            Ty::Enum(id) => {
                let def = &self.symbols.enums[id.0 as usize];
                def.variants
                    .iter()
                    .enumerate()
                    .map(|(index, v)| Variant {
                        name: format!("{}::{}", def.name, v.name),
                        variant: VariantRef::User(*id, index),
                        fields: v.fields.clone(),
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
            _ => Vec::new(),
        }
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
                .push(Diagnostic::new(codes::V0205, span, message).with_note(TYPE_NOTE));
        }
        found
    }

    /// The index of the variant `path::name` (`type_::name`, or
    /// `module::type_::name` with a module path of any depth and prefix,
    /// `path`'s last segment always the type) among those of `ty`: V0100
    /// or V0105 for a type that cannot be found or seen, V0111 for `super`
    /// in the crate root, V0205 for a variant of another type.
    fn user_pattern(&mut self, path: &Path, name: &Ident, ty: &Ty, span: Span) -> Option<usize> {
        let Some((module, type_)) = split_last(path) else {
            // `crate::Word`: a keyword alone before the variant.
            let message = format!(
                "`{}` names no enum; a pattern names a variant through its enum, as in \
                 `Shape::Point`",
                display_path(path, &name.name)
            );
            self.diagnostics
                .push(Diagnostic::new(codes::V0100, span, message));
            return None;
        };
        let full_path = path_text(path);
        let type_span = Span::new(span.file, path.span.start, type_.span.end);
        let found = self
            .symbols
            .lookup_type(self.module, module.as_ref(), &type_.name);
        let id = match found {
            Ok(UserType::Enum(id)) => id,
            Ok(UserType::Struct(_)) => {
                let message =
                    format!("`{full_path}` is a struct, not an enum, so it has no variants");
                self.diagnostics
                    .push(Diagnostic::new(codes::V0205, type_span, message).with_note(TYPE_NOTE));
                return None;
            }
            Err(error) => {
                // `m::Wrap` for a `.rs` enum Varyk did not import (M3
                // spec 4.3): V0101 saying why, same as the type, value,
                // and call positions.
                let skipped = self.symbols.skipped_item(
                    self.module,
                    module.as_ref(),
                    &type_.name,
                    &full_path,
                    type_span,
                );
                match skipped {
                    Some(diagnostic) if error == LookupError::Unknown => {
                        self.diagnostics.push(diagnostic);
                    }
                    _ => self.lookup_error(error, "type", &full_path, type_span, Some(&type_.name)),
                }
                return None;
            }
        };
        let def = &self.symbols.enums[id.0 as usize];
        let Some(index) = def.variant(&name.name) else {
            let message = format!("enum `{full_path}` has no variant `{}`", name.name);
            self.diagnostics
                .push(Diagnostic::new(codes::V0100, name.span, message));
            return None;
        };
        if *ty != Ty::Enum(id) {
            let message = format!(
                "`{full_path}::{}` is a variant of `{full_path}`, but this value is `{}`",
                name.name,
                self.ty_name(ty)
            );
            self.diagnostics
                .push(Diagnostic::new(codes::V0205, span, message).with_note(TYPE_NOTE));
            return None;
        }
        if let Some(reason) = &def.opaque {
            let full = format!("{full_path}::{}", name.name);
            self.diagnostics
                .push(opaque_variant(&full, &def.name, reason, span));
            return None;
        }
        Some(index)
    }
}

/// Every name `pattern` would bind, at any depth: each `Name` that is not
/// a built-in variant (`None`).
fn bound_names<'p>(pattern: &'p Pattern, names: &mut Vec<&'p Ident>) {
    match pattern {
        Pattern::Name(name) if !is_builtin_variant(&name.name) => names.push(name),
        Pattern::Variant { fields, .. } => {
            for field in fields {
                bound_names(field, names);
            }
        }
        Pattern::Struct { fields, .. } => {
            for (_, field) in fields {
                bound_names(field, names);
            }
        }
        Pattern::Name(_) | Pattern::Wildcard(_) | Pattern::Literal(..) | Pattern::Range { .. } => {}
    }
}
