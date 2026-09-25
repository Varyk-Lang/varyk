//! Type parsing (spec 2.6, 2.10): a type is a bare name (`i32`, `string`,
//! `User`, ...), optionally preceded by a module segment (`m::User`) and
//! followed by generic arguments (`Vec<i32>`, `Result<User, string>`,
//! nested freely, spec 2.6). Resolving either the module segment or the
//! generic arguments is milestone 2 task 5's job; this parser only builds
//! the syntax tree (`TypeExpr::module`, `TypeExpr::args`). `&T`/`&mut T` and
//! lifetime syntax in a parameter's type are `item.rs`'s job
//! (`parse_param`), since reporting them needs the parameter's name too, to
//! build the fix-it.

use super::Parser;
use crate::ast::TypeExpr;
use crate::span::Span;
use crate::token::TokenKind;

impl<'a> Parser<'a> {
    /// A named type: `name`, `m::name`, `name<args>`, or `m::name<args>`.
    /// The base type name is not optional: a missing or reserved-keyword
    /// name is a fatal `Err` (from `expect_identifier`), and so is a
    /// missing name after `::`.
    pub(super) fn parse_type(&mut self) -> Result<TypeExpr, ()> {
        let first = self.expect_identifier("a type name")?;
        let mut span = first.span;

        let (module, name) = if self.peek() == Some(&TokenKind::ColonColon) {
            self.bump();
            let second = self.expect_identifier("a type name after `::`")?;
            span = self.span_from(span, second.span);
            (Some(first), second)
        } else {
            (None, first)
        };

        let args = if self.peek() == Some(&TokenKind::Lt) {
            let (args, group_span) = self.parse_generic_args()?;
            span = self.span_from(span, group_span);
            args
        } else {
            Vec::new()
        };

        Ok(TypeExpr {
            module,
            name,
            args,
            span,
        })
    }

    /// `<T, U, ...>` right after a type name, whose opening `<` is already
    /// confirmed present. Returns the parsed arguments plus the span of the
    /// whole group.
    fn parse_generic_args(&mut self) -> Result<(Vec<TypeExpr>, Span), ()> {
        let lt = self.bump().expect("caller confirmed `<` is present");
        let mut args = Vec::new();
        loop {
            args.push(self.parse_type()?);
            if self.bump_if(&TokenKind::Comma) {
                if self.peek() == Some(&TokenKind::Gt) {
                    break;
                }
                continue;
            }
            break;
        }
        let gt = self.expect(TokenKind::Gt, "`>` after the generic arguments")?;
        Ok((args, self.span_from(lt.span, gt.span)))
    }

    /// Consumes a `<...>` group whose opening `<` is already confirmed
    /// present (`peek() == Some(&TokenKind::Lt)`), matching nested `<`/`>`
    /// by depth, and returns the span of the whole group, without building
    /// anything from its contents. Used where a generic-looking group must
    /// be discarded rather than parsed, such as a function's own `<T>`
    /// (`item.rs`'s `skip_fn_generics_if_present`). Best-effort: an
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

    fn parse_type_ok(src: &str) -> TypeExpr {
        let (ty, errors) = parse_type(src);
        assert!(
            errors.is_empty(),
            "unexpected errors parsing {src:?}: {errors:?}"
        );
        ty.unwrap_or_else(|()| panic!("expected {src:?} to parse"))
    }

    #[test]
    fn bare_identifier_type() {
        let ty = parse_type_ok("i32");
        assert_eq!(ty.name.name, "i32");
        assert!(ty.module.is_none());
        assert!(ty.args.is_empty());
    }

    #[test]
    fn module_qualified_type_parses() {
        let ty = parse_type_ok("m::User");
        assert_eq!(ty.module.as_ref().unwrap().name, "m");
        assert_eq!(ty.name.name, "User");
        assert!(ty.args.is_empty());
    }

    #[test]
    fn generic_type_with_one_argument_parses() {
        let ty = parse_type_ok("Vec<i32>");
        assert_eq!(ty.name.name, "Vec");
        assert_eq!(ty.args.len(), 1);
        assert_eq!(ty.args[0].name.name, "i32");
    }

    #[test]
    fn generic_type_with_two_arguments_parses() {
        let ty = parse_type_ok("Result<User, string>");
        assert_eq!(ty.name.name, "Result");
        assert_eq!(ty.args.len(), 2);
        assert_eq!(ty.args[0].name.name, "User");
        assert_eq!(ty.args[1].name.name, "string");
    }

    #[test]
    fn nested_generic_arguments_parse() {
        let ty = parse_type_ok("Vec<Option<Shape>>");
        assert_eq!(ty.name.name, "Vec");
        assert_eq!(ty.args.len(), 1);
        let inner = &ty.args[0];
        assert_eq!(inner.name.name, "Option");
        assert_eq!(inner.args.len(), 1);
        assert_eq!(inner.args[0].name.name, "Shape");
    }

    #[test]
    fn module_qualified_generic_type_parses() {
        let ty = parse_type_ok("m::Vec<i32>");
        assert_eq!(ty.module.as_ref().unwrap().name, "m");
        assert_eq!(ty.name.name, "Vec");
        assert_eq!(ty.args.len(), 1);
    }
}
