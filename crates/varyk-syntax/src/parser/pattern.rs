//! Pattern parsing (spec 2.3, M4 spec 2.5): the patterns of a `match` arm,
//! an `if let`, and a `while let`, nested to any depth.

use super::Parser;
use crate::ast::{Ident, Literal, PathStart, Pattern};
use crate::error::{V0001, V0002};
use crate::span::Span;
use crate::token::TokenKind;

impl<'a> Parser<'a> {
    /// A pattern: `_`, a name, a literal (`1`, `-1`, `"yes"`, `true`), a
    /// range of two literals (`0..=9`), or a variant path with values by
    /// position (`Some(p)`) or named fields (`Event::Click { x, y: 0 }`),
    /// whose own patterns are parsed the same way.
    pub(super) fn parse_pattern(&mut self) -> Result<Pattern, ()> {
        match self.peek() {
            Some(TokenKind::Identifier(name)) if name == "_" => {
                let token = self.bump().expect("peek just confirmed a token is present");
                Ok(Pattern::Wildcard(token.span))
            }
            Some(TokenKind::CrateKw | TokenKind::SelfKw | TokenKind::SuperKw)
                if self.peek_at(1) == Some(&TokenKind::ColonColon) =>
            {
                self.parse_path_pattern()
            }
            Some(TokenKind::Identifier(_)) => self.parse_path_pattern(),
            Some(
                TokenKind::IntegerLiteral(_)
                | TokenKind::FloatLiteral(_)
                | TokenKind::StringLiteral(_)
                | TokenKind::BoolLiteral(_)
                | TokenKind::Minus,
            ) => self.parse_literal_pattern(),
            Some(TokenKind::DotDot) => {
                let span = self.current_span();
                self.bump();
                self.push_error(
                    V0001,
                    span,
                    "rest patterns (`..`) are not supported in Varyk",
                );
                Err(())
            }
            Some(TokenKind::Mut) => {
                let span = self.current_span();
                self.bump();
                self.push_error(
                    V0001,
                    span,
                    "`mut` is not supported in a pattern in Varyk; write `let mut x = x;` \
                     inside the arm instead",
                );
                Err(())
            }
            Some(TokenKind::ReservedKeyword(word)) if word == "ref" => {
                let span = self.current_span();
                self.bump();
                self.push_error(
                    V0001,
                    span,
                    "`ref` is not supported in a pattern in Varyk; write `let mut x = x;` \
                     inside the arm instead",
                );
                Err(())
            }
            Some(TokenKind::Hash) => {
                self.push_expected("a pattern");
                Err(())
            }
            _ => {
                let span = self.current_span();
                self.bump();
                self.push_error(
                    V0001,
                    span,
                    "only `_`, a name, a literal, a range, or a variant can be a pattern in Varyk",
                );
                Err(())
            }
        }
    }

    /// V0001 for a `|` right after a pattern: alternatives (`a | b`) are
    /// not in Varyk (M4 spec 2.5).
    pub(super) fn reject_alternatives(&mut self) -> Result<(), ()> {
        if self.peek() != Some(&TokenKind::Pipe) {
            return Ok(());
        }
        let span = self.current_span();
        self.push_error(
            V0001,
            span,
            "alternatives (`a | b`) in a pattern are not supported in Varyk; write one arm for \
             each",
        );
        Err(())
    }

    /// A literal pattern, or a range pattern when `..=` follows it.
    fn parse_literal_pattern(&mut self) -> Result<Pattern, ()> {
        let (start, start_span) = self.parse_pattern_literal()?;
        match self.peek() {
            Some(TokenKind::DotDotEq) => {
                self.bump();
                let (end, end_span) = self.parse_pattern_literal()?;
                Ok(Pattern::Range {
                    start,
                    end,
                    span: self.span_from(start_span, end_span),
                })
            }
            Some(TokenKind::DotDot) => {
                let dotdot_span = self.current_span();
                self.bump();
                self.push_error(
                    V0001,
                    self.span_from(start_span, dotdot_span),
                    "a range pattern includes both ends in Varyk; write it with `..=`, as in \
                     `0..=9`",
                );
                Err(())
            }
            _ => Ok(Pattern::Literal(start, start_span)),
        }
    }

    /// One literal of a pattern: a number with an optional `-`, a string,
    /// or a `bool`, with its span.
    pub(super) fn parse_pattern_literal(&mut self) -> Result<(Literal, Span), ()> {
        let start = self.current_span();
        let negative = self.bump_if(&TokenKind::Minus);
        let literal = match self.peek() {
            Some(TokenKind::IntegerLiteral(text)) => Some(Literal::Int {
                text: text.clone(),
                negative,
            }),
            Some(TokenKind::FloatLiteral(text)) => Some(Literal::Float {
                text: text.clone(),
                negative,
            }),
            Some(TokenKind::StringLiteral(text)) if !negative => Some(Literal::Str(text.clone())),
            Some(TokenKind::BoolLiteral(value)) if !negative => Some(Literal::Bool(*value)),
            _ => None,
        };
        let span = self.current_span();
        let Some(literal) = literal else {
            let what = if negative {
                "a number after `-`"
            } else {
                "a literal"
            };
            self.push_error(V0002, span, format!("expected {what}"));
            return Err(());
        };
        self.bump();
        Ok((literal, self.span_from(start, span)))
    }

    /// A path pattern: a bare name when it is one plain segment with
    /// nothing after it, else a variant, with values by position in
    /// parentheses or named fields in braces (spec 2.3, M4 spec 2.5):
    /// `n`, `Point`, `Some(p)`, `geo::Shape::Point`,
    /// `Event::Click { x, y: 0 }`.
    fn parse_path_pattern(&mut self) -> Result<Pattern, ()> {
        let dotted = self.parse_dotted_path("a pattern")?;
        let path_span = dotted.span;
        match self.peek() {
            Some(TokenKind::LBrace) => {
                let (fields, group_span) = self.parse_field_patterns()?;
                let (path, name) = self.path_and_name(dotted);
                Ok(Pattern::Struct {
                    path,
                    name,
                    fields,
                    span: self.span_from(path_span, group_span),
                })
            }
            Some(TokenKind::LParen) => {
                let (fields, group_span) = self.parse_positional_patterns()?;
                let (path, name) = self.path_and_name(dotted);
                Ok(Pattern::Variant {
                    path,
                    name,
                    fields,
                    span: self.span_from(path_span, group_span),
                })
            }
            _ if dotted.leading == PathStart::None && dotted.segments.len() == 1 => {
                let mut segments = dotted.segments;
                Ok(Pattern::Name(segments.pop().expect("one segment")))
            }
            _ => {
                let (path, name) = self.path_and_name(dotted);
                Ok(Pattern::Variant {
                    path,
                    name,
                    fields: Vec::new(),
                    span: path_span,
                })
            }
        }
    }

    /// `(pattern, pattern, ...)` right after a variant pattern's name,
    /// whose opening `(` is already confirmed present. Returns the
    /// patterns plus the span of the whole group.
    fn parse_positional_patterns(&mut self) -> Result<(Vec<Pattern>, Span), ()> {
        let lparen = self.bump().expect("caller confirmed `(`");
        let mut fields = Vec::new();
        if self.peek() != Some(&TokenKind::RParen) {
            loop {
                fields.push(self.parse_pattern()?);
                self.reject_alternatives()?;
                if self.bump_if(&TokenKind::Comma) {
                    if self.peek() == Some(&TokenKind::RParen) {
                        break;
                    }
                    continue;
                }
                break;
            }
        }
        let rparen = self.expect(TokenKind::RParen, "`)`")?;
        Ok((fields, self.span_from(lparen.span, rparen.span)))
    }

    /// `{ name, name: pattern, ... }` right after a variant pattern's name,
    /// whose opening `{` is already confirmed present. A field written as
    /// `name` alone binds it: `(name, Pattern::Name(name))`. There is no
    /// `..` (M4 spec 2.5): every field is named. Returns the fields plus
    /// the span of the whole group.
    fn parse_field_patterns(&mut self) -> Result<(Vec<(Ident, Pattern)>, Span), ()> {
        let lbrace = self.bump().expect("caller confirmed `{`");
        let mut fields = Vec::new();
        if self.peek() != Some(&TokenKind::RBrace) {
            loop {
                if self.peek() == Some(&TokenKind::DotDot) {
                    let span = self.current_span();
                    self.push_error(
                        V0001,
                        span,
                        "`..` is not supported in a pattern with named fields; name every \
                         field, and write `name: _` for one you do not need",
                    );
                    return Err(());
                }
                let name = self.expect_identifier("a field name")?;
                let pattern = if self.bump_if(&TokenKind::Colon) {
                    self.parse_pattern()?
                } else {
                    Pattern::Name(name.clone())
                };
                self.reject_alternatives()?;
                fields.push((name, pattern));
                if self.bump_if(&TokenKind::Comma) {
                    if self.peek() == Some(&TokenKind::RBrace) {
                        break;
                    }
                    continue;
                }
                break;
            }
        }
        let rbrace = self.expect(TokenKind::RBrace, "`}`")?;
        Ok((fields, self.span_from(lbrace.span, rbrace.span)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Expr, ExprKind, Literal, Path};
    use crate::error::SyntaxError;
    use crate::lex;
    use crate::source::SourceFile;
    use crate::span::FileId;

    fn parse(src: &str) -> (Result<Expr, ()>, Vec<SyntaxError>) {
        let file = SourceFile::new(FileId(0), "test.vr", src);
        let (tokens, lex_errors) = lex(&file);
        assert!(
            lex_errors.is_empty(),
            "unexpected lex errors: {lex_errors:?}"
        );
        let mut parser = Parser::new(&tokens, FileId(0));
        let expr = parser.parse_expr();
        (expr, parser.errors().to_vec())
    }

    fn parse_ok(src: &str) -> Expr {
        let (expr, errors) = parse(src);
        assert!(
            errors.is_empty(),
            "unexpected errors parsing {src:?}: {errors:?}"
        );
        expr.unwrap_or_else(|()| panic!("expected {src:?} to parse"))
    }

    /// The pattern of the first arm of `match x { <pattern> => 0, _ => 1 }`.
    fn first_pattern(pattern: &str) -> Pattern {
        let expr = parse_ok(&format!("match x {{ {pattern} => 0, _ => 1 }}"));
        match expr.kind {
            ExprKind::Match { mut arms, .. } => arms.remove(0).pattern,
            other => panic!("expected a match, got {other:?}"),
        }
    }

    /// The one error of `match x { <pattern> => 0, _ => 1 }`, which must
    /// not parse.
    fn pattern_error(pattern: &str) -> SyntaxError {
        let (expr, errors) = parse(&format!("match x {{ {pattern} => 0, _ => 1 }}"));
        assert!(expr.is_err(), "{pattern} must not parse");
        assert_eq!(errors.len(), 1, "{pattern}: {errors:?}");
        errors[0].clone()
    }

    fn path_name(expr: &Expr) -> &str {
        match &expr.kind {
            ExprKind::Path { name, .. } => &name.name,
            other => panic!("expected a Path, got {other:?}"),
        }
    }

    /// The leading keyword and plain segments of a path, as
    /// `("crate", ["shop", "Cart"])`.
    fn prefix_and_segments(path: &Path) -> (&'static str, Vec<&str>) {
        let leading = match path.leading {
            PathStart::Crate => "crate",
            PathStart::SelfMod => "self",
            PathStart::Super => "super",
            PathStart::None => "",
        };
        (
            leading,
            path.segments.iter().map(|s| s.name.as_str()).collect(),
        )
    }

    fn int(text: &str, negative: bool) -> Literal {
        Literal::Int {
            text: text.to_string(),
            negative,
        }
    }

    #[test]
    fn match_with_wildcard_and_name_pattern() {
        let src = "match x { _ => 1, n => n, }";
        let expr = parse_ok(src);
        assert_eq!(expr.span.start, 0);
        assert_eq!(expr.span.end, src.len() as u32);
        match &expr.kind {
            ExprKind::Match { scrutinee, arms } => {
                assert_eq!(path_name(scrutinee), "x");
                assert_eq!(arms.len(), 2);
                assert!(matches!(arms[0].pattern, Pattern::Wildcard(_)));
                match &arms[1].pattern {
                    Pattern::Name(name) => assert_eq!(name.name, "n"),
                    other => panic!("expected a name pattern, got {other:?}"),
                }
            }
            other => panic!("expected a match, got {other:?}"),
        }
    }

    #[test]
    fn match_variant_pattern_with_fields() {
        let expr = parse_ok("match shape { Shape::Circle(r) => 1, Shape::Point => 0 }");
        match &expr.kind {
            ExprKind::Match { arms, .. } => {
                assert_eq!(arms.len(), 2);
                match &arms[0].pattern {
                    Pattern::Variant {
                        path, name, fields, ..
                    } => {
                        let path = path.as_ref().expect("one segment: the type");
                        assert_eq!(path.segments.len(), 1);
                        assert_eq!(path.segments[0].name, "Shape");
                        assert_eq!(name.name, "Circle");
                        assert_eq!(fields.len(), 1);
                        match &fields[0] {
                            Pattern::Name(name) => assert_eq!(name.name, "r"),
                            other => panic!("expected a name pattern, got {other:?}"),
                        }
                    }
                    other => panic!("expected a variant pattern, got {other:?}"),
                }
                match &arms[1].pattern {
                    Pattern::Variant {
                        name, fields, span, ..
                    } => {
                        assert_eq!(name.name, "Point");
                        assert!(fields.is_empty());
                        assert_eq!(span.end, name.span.end, "no parentheses");
                    }
                    other => panic!("expected a variant pattern, got {other:?}"),
                }
            }
            other => panic!("expected a match, got {other:?}"),
        }
    }

    #[test]
    fn match_unqualified_variant_pattern_with_wildcard_field() {
        // `Some(_)`: a single-segment variant pattern, told apart from a
        // name pattern by the parentheses that follow it.
        match first_pattern("Some(_)") {
            Pattern::Variant {
                path, name, fields, ..
            } => {
                assert!(path.is_none());
                assert_eq!(name.name, "Some");
                assert!(matches!(fields[0], Pattern::Wildcard(_)));
            }
            other => panic!("expected a variant pattern, got {other:?}"),
        }
    }

    #[test]
    fn match_module_qualified_variant_pattern() {
        match first_pattern("geo::Shape::Point") {
            Pattern::Variant { path, name, .. } => {
                let path = path.expect("two segments");
                let names: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
                assert_eq!(names, vec!["geo", "Shape"]);
                assert_eq!(name.name, "Point");
            }
            other => panic!("expected a variant pattern, got {other:?}"),
        }
    }

    #[test]
    fn keyword_prefixed_variant_pattern_parses() {
        let expr = parse_ok("match k { crate::shop::Kind::Word => 0, super::Kind::Other(n) => n }");
        let ExprKind::Match { arms, .. } = &expr.kind else {
            panic!("expected a match, got {expr:?}");
        };
        let expected = [
            ("crate", vec!["shop", "Kind"], "Word", 0),
            ("super", vec!["Kind"], "Other", 1),
        ];
        for (arm, (leading, segments, variant, count)) in arms.iter().zip(expected) {
            let Pattern::Variant {
                path, name, fields, ..
            } = &arm.pattern
            else {
                panic!("expected a variant pattern, got {:?}", arm.pattern);
            };
            let path = path.as_ref().expect("a module path");
            assert_eq!(prefix_and_segments(path), (leading, segments));
            assert_eq!(name.name, variant);
            assert_eq!(fields.len(), count);
        }
    }

    #[test]
    fn nested_variant_pattern() {
        let Pattern::Variant { name, fields, .. } = first_pattern("Some(Shape::Circle(r))") else {
            panic!("expected a variant pattern");
        };
        assert_eq!(name.name, "Some");
        let Pattern::Variant {
            path, name, fields, ..
        } = &fields[0]
        else {
            panic!("expected a nested variant pattern, got {fields:?}");
        };
        assert_eq!(path.as_ref().map(|p| p.segments.len()), Some(1));
        assert_eq!(name.name, "Circle");
        assert!(matches!(&fields[0], Pattern::Name(n) if n.name == "r"));
    }

    #[test]
    fn named_field_variant_pattern_with_a_literal_and_a_shorthand() {
        let src = "Event::Click { x: 0, y }";
        let Pattern::Struct {
            path,
            name,
            fields,
            span,
        } = first_pattern(src)
        else {
            panic!("expected a struct pattern");
        };
        assert_eq!(path.map(|p| p.segments.len()), Some(1));
        assert_eq!(name.name, "Click");
        assert_eq!(span.end - span.start, src.len() as u32);
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].0.name, "x");
        assert!(matches!(&fields[0].1, Pattern::Literal(lit, _) if *lit == int("0", false)));
        assert_eq!(fields[1].0.name, "y");
        assert!(matches!(&fields[1].1, Pattern::Name(n) if n.name == "y"));
    }

    #[test]
    fn named_field_variant_pattern_with_wildcards() {
        let Pattern::Struct { fields, .. } = first_pattern("Event::Click { x: _, y: _ }") else {
            panic!("expected a struct pattern");
        };
        assert_eq!(fields.len(), 2);
        assert!(
            fields
                .iter()
                .all(|(_, p)| matches!(p, Pattern::Wildcard(_)))
        );
    }

    #[test]
    fn literal_patterns() {
        let cases = [
            ("\"yes\"", Literal::Str("yes".to_string())),
            ("-1", int("1", true)),
            ("42", int("42", false)),
            ("true", Literal::Bool(true)),
            ("false", Literal::Bool(false)),
            (
                "1.5",
                Literal::Float {
                    text: "1.5".to_string(),
                    negative: false,
                },
            ),
        ];
        for (src, expected) in cases {
            match first_pattern(src) {
                Pattern::Literal(literal, span) => {
                    assert_eq!(literal, expected, "{src}");
                    assert_eq!(span.end - span.start, src.len() as u32, "{src}");
                }
                other => panic!("{src}: expected a literal pattern, got {other:?}"),
            }
        }
    }

    #[test]
    fn range_pattern() {
        match first_pattern("90..=100") {
            Pattern::Range { start, end, span } => {
                assert_eq!(start, int("90", false));
                assert_eq!(end, int("100", false));
                assert_eq!(span.end - span.start, 8);
            }
            other => panic!("expected a range pattern, got {other:?}"),
        }
        match first_pattern("-5..=-1") {
            Pattern::Range { start, end, .. } => {
                assert_eq!(start, int("5", true));
                assert_eq!(end, int("1", true));
            }
            other => panic!("expected a range pattern, got {other:?}"),
        }
    }

    #[test]
    fn range_pattern_inside_a_variant() {
        let Pattern::Variant { fields, .. } = first_pattern("Some(0..=9)") else {
            panic!("expected a variant pattern");
        };
        assert!(matches!(&fields[0], Pattern::Range { .. }));
    }

    #[test]
    fn exclusive_range_pattern_is_v0001_naming_the_inclusive_one() {
        let error = pattern_error("0..5");
        assert_eq!(error.code, V0001);
        assert_eq!(
            error.message,
            "a range pattern includes both ends in Varyk; write it with `..=`, as in `0..=9`"
        );
    }

    #[test]
    fn match_mut_field_is_v0001_naming_mut() {
        let error = pattern_error("Some(mut x)");
        assert_eq!(error.code, V0001);
        assert!(
            error.message.contains("mut") && error.message.contains("let mut x = x;"),
            "{error:?}"
        );
    }

    #[test]
    fn match_ref_field_is_v0001_naming_ref() {
        let error = pattern_error("Some(ref x)");
        assert_eq!(error.code, V0001);
        assert!(
            error.message.contains("ref") && error.message.contains("let mut x = x;"),
            "{error:?}"
        );
    }

    #[test]
    fn match_rest_pattern_is_v0001() {
        for pattern in ["..", "Some(..)"] {
            let error = pattern_error(pattern);
            assert_eq!(error.code, V0001);
            assert!(error.message.contains("rest"), "{pattern}: {error:?}");
        }
    }

    #[test]
    fn rest_in_a_named_field_pattern_is_v0001() {
        let error = pattern_error("Event::Click { x, .. }");
        assert_eq!(error.code, V0001);
        assert_eq!(
            error.message,
            "`..` is not supported in a pattern with named fields; name every field, and \
             write `name: _` for one you do not need"
        );
    }

    #[test]
    fn match_guard_is_v0001() {
        let error = pattern_error("Some(n) if n > 0");
        assert_eq!(error.code, V0001);
        assert_eq!(
            error.message,
            "match guards (`pattern if condition`) are not supported in Varyk; write the \
             condition as an `if` inside the arm"
        );
    }

    #[test]
    fn alternatives_are_v0001() {
        let expected = "alternatives (`a | b`) in a pattern are not supported in Varyk; write \
                        one arm for each";
        for pattern in ["1 | 2", "Some(1 | 2)", "Event::Click { x: 0 | 1, y }"] {
            let error = pattern_error(pattern);
            assert_eq!(error.code, V0001);
            assert_eq!(error.message, expected, "{pattern}");
        }
    }
}
