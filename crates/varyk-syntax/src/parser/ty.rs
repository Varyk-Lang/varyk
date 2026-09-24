//! Type parsing: milestone 1 types are bare names (`i32`, `string`,
//! `User`, ...); spec 4.1. This module also detects and reports (but keeps
//! parsing past) the two Rust habits a type name alone can carry: a
//! module-qualified name and generic arguments. `&T`/`&mut T` and lifetime
//! syntax in a parameter's type are `item.rs`'s job (`parse_param`), since
//! reporting them needs the parameter's name too, to build the fix-it.

use super::Parser;
use crate::ast::TypeExpr;
use crate::error::V0001;
use crate::span::Span;
use crate::token::TokenKind;

impl<'a> Parser<'a> {
    /// A named type: a bare identifier, optionally followed by `::name` (a
    /// module-qualified type, `V0001`: milestone 1 has no such types) or
    /// `<...>` (generic arguments, `V0001`: milestone 1 has no generics).
    /// The `::name` and `<...>` suffixes are reported and skipped rather
    /// than fatal, so a base name followed by either still yields a
    /// `TypeExpr`. The base type name itself is not optional: a missing or
    /// reserved-keyword name is a fatal `Err` (from `expect_identifier`).
    pub(super) fn parse_type(&mut self) -> Result<TypeExpr, ()> {
        let mut name = self.expect_identifier("a type name")?;
        let mut span = name.span;

        if self.peek() == Some(&TokenKind::ColonColon) {
            self.bump();
            let second = self.expect_identifier("a type name after `::`")?;
            span = self.span_from(span, second.span);
            self.push_error(
                V0001,
                span,
                "module-qualified types are not supported in Varyk yet",
            );
            name = second;
        }

        if self.peek() == Some(&TokenKind::Lt) {
            let group_span = self.skip_generic_args();
            span = self.span_from(span, group_span);
            self.push_error(V0001, span, "generics are not supported in Varyk");
        }

        Ok(TypeExpr { name, span })
    }

    /// Consumes a `<...>` group whose opening `<` is already confirmed
    /// present (`peek() == Some(&TokenKind::Lt)`), matching nested `<`/`>`
    /// by depth, and returns the span of the whole group. Best-effort: an
    /// unclosed group consumes the rest of the token stream rather than
    /// reporting a second error, since every call site already reports one
    /// for the group itself.
    pub(super) fn skip_generic_args(&mut self) -> Span {
        let lt = self.bump().expect("caller confirmed `<` is present");
        let mut depth = 1u32;
        let mut last_span = lt.span;
        while depth > 0 {
            match self.peek() {
                Some(TokenKind::Lt) => {
                    depth += 1;
                    last_span = self.bump().expect("peek just confirmed a token").span;
                }
                Some(TokenKind::Gt) => {
                    depth -= 1;
                    last_span = self.bump().expect("peek just confirmed a token").span;
                }
                Some(_) => {
                    last_span = self.bump().expect("peek just confirmed a token").span;
                }
                None => break,
            }
        }
        self.span_from(lt.span, last_span)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SyntaxError;
    use crate::lex;
    use crate::source::SourceFile;
    use crate::span::FileId;

    fn parse_type(src: &str) -> (Result<TypeExpr, ()>, Vec<SyntaxError>) {
        let file = SourceFile::new(FileId(0), "test.vr", src);
        let (tokens, lex_errors) = lex(&file);
        assert!(
            lex_errors.is_empty(),
            "unexpected lex errors: {lex_errors:?}"
        );
        let mut parser = Parser::new(&tokens, FileId(0));
        let ty = parser.parse_type();
        (ty, parser.errors().to_vec())
    }

    #[test]
    fn bare_identifier_type() {
        let (ty, errors) = parse_type("i32");
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(ty.unwrap().name.name, "i32");
    }

    #[test]
    fn module_qualified_type_is_v0001() {
        let (ty, errors) = parse_type("math::Point");
        assert!(errors.iter().any(|e| e.code == V0001), "errors: {errors:?}");
        assert_eq!(ty.unwrap().name.name, "Point");
    }

    #[test]
    fn generic_type_is_v0001() {
        let (ty, errors) = parse_type("Vec<i32>");
        assert!(errors.iter().any(|e| e.code == V0001), "errors: {errors:?}");
        assert_eq!(ty.unwrap().name.name, "Vec");
    }
}
