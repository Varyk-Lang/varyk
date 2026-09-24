//! Item parsing: `fn`, `struct`, and `mod` declarations (spec 4.1), plus
//! [`Parser::parse_program`], the driver that reads a whole file's items.

use super::Parser;
use crate::ast::{FieldDecl, Function, Item, ModDecl, Param, Program, StructDecl};
use crate::error::{FixIt, V0001, V0002, V0011, V0012};
use crate::span::Span;
use crate::token::TokenKind;

impl<'a> Parser<'a> {
    /// Parses every item in the token stream, stopping at the first
    /// unrecoverable error (`V0002`, or the `V0001` a reserved keyword
    /// produces where an item is expected) and returning whatever items
    /// parsed cleanly before it, per the crate-wide `V0002`-stops rule.
    pub(super) fn parse_program(&mut self) -> Program {
        let mut items = Vec::new();
        while self.peek().is_some() {
            match self.parse_item() {
                Ok(item) => items.push(item),
                Err(()) => break,
            }
        }
        Program { items }
    }

    fn parse_item(&mut self) -> Result<Item, ()> {
        let start_span = self.current_span();
        let is_pub = self.bump_if(&TokenKind::Pub);
        match self.peek() {
            Some(TokenKind::Fn) => self.parse_function(start_span, is_pub).map(Item::Function),
            Some(TokenKind::Struct) => self.parse_struct(start_span, is_pub).map(Item::Struct),
            Some(TokenKind::Mod) => self.parse_mod(start_span, is_pub).map(Item::Mod),
            Some(TokenKind::ReservedKeyword(word)) => {
                let word = word.clone();
                let span = self.current_span();
                self.bump();
                self.push_error(
                    V0001,
                    span,
                    format!("`{word}` is not supported in Varyk yet"),
                );
                Err(())
            }
            _ => {
                let span = self.current_span();
                self.push_error(V0002, span, "expected an item (`fn`, `struct`, or `mod`)");
                Err(())
            }
        }
    }

    fn parse_function(&mut self, start_span: Span, is_pub: bool) -> Result<Function, ()> {
        self.bump(); // `fn`
        let name = self.expect_name_identifier("a function name")?;
        self.skip_fn_generics_if_present();
        self.expect(TokenKind::LParen, "`(` after the function name")?;
        let mut params = Vec::new();
        if self.peek() != Some(&TokenKind::RParen) {
            loop {
                params.push(self.parse_param()?);
                if self.bump_if(&TokenKind::Comma) {
                    if self.peek() == Some(&TokenKind::RParen) {
                        break;
                    }
                    continue;
                }
                break;
            }
        }
        self.expect(TokenKind::RParen, "`)` after the parameters")?;
        let return_type = if self.bump_if(&TokenKind::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };
        let body = self.parse_block()?;
        let span = self.span_from(start_span, body.span);
        Ok(Function {
            name,
            is_pub,
            params,
            return_type,
            body,
            span,
        })
    }

    /// `<T>` or `<'a>` right after a function name: milestone 1 has
    /// neither. Reports and discards the whole group rather than building
    /// anything from it. A leading lifetime is the Rust-habit `V0012`
    /// ("lifetimes are inferred") with a fix-it that removes the group;
    /// anything else is `V0001` naming generics (spec 6.6).
    fn skip_fn_generics_if_present(&mut self) {
        if self.peek() != Some(&TokenKind::Lt) {
            return;
        }
        let is_lifetime = matches!(self.peek_at(1), Some(TokenKind::Lifetime(_)));
        let group_span = self.skip_generic_args();
        if is_lifetime {
            self.push_error_with_fix_it(
                V0012,
                group_span,
                "lifetimes are inferred",
                FixIt {
                    span: group_span,
                    replacement: String::new(),
                },
            );
        } else {
            self.push_error(V0001, group_span, "generics are not supported in Varyk");
        }
    }

    /// `mut? name : T`, where `T` may itself be written the Rust way,
    /// `&T`/`&mut T` (optionally with a lifetime): that is `V0011`, with a
    /// fix-it rewriting the whole parameter to `name: T` or `mut name: T`
    /// (spec 4.2, 6.6). `mut` before the name already means "mutable
    /// borrow" in Varyk, so a `&mut` sigil in the type position sets the
    /// same flag.
    fn parse_param(&mut self) -> Result<Param, ()> {
        let start_span = self.current_span();
        let mut mutable = self.bump_if(&TokenKind::Mut);
        let name = self.expect_name_identifier("a parameter name")?;
        self.expect(TokenKind::Colon, "`:` after the parameter name")?;

        let ty = if self.peek() == Some(&TokenKind::Amp) {
            self.bump(); // `&`
            if matches!(self.peek(), Some(TokenKind::Lifetime(_))) {
                let lifetime = self.bump().expect("peek just confirmed a lifetime");
                let next_start = self
                    .peek_token()
                    .map(|t| t.span.start)
                    .unwrap_or_else(|| self.eof_span().start);
                let removal_span = self.span_from(
                    lifetime.span,
                    Span::new(lifetime.span.file, next_start, next_start),
                );
                self.push_error_with_fix_it(
                    V0012,
                    removal_span,
                    "lifetimes are inferred",
                    FixIt {
                        span: removal_span,
                        replacement: String::new(),
                    },
                );
            }
            if self.bump_if(&TokenKind::Mut) {
                mutable = true;
            }
            let ty = self.parse_type()?;
            let whole_span = self.span_from(start_span, ty.span);
            let replacement = if mutable {
                format!("mut {}: {}", name.name, ty.name.name)
            } else {
                format!("{}: {}", name.name, ty.name.name)
            };
            let (param, ty_name) = (&name.name, &ty.name.name);
            let message = format!(
                "parameter types have no `&` in Varyk; write `{param}: {ty_name}` to only \
                 read `{param}`, or `mut {param}: {ty_name}` to change it"
            );
            self.push_error_with_fix_it(
                V0011,
                whole_span,
                message,
                FixIt {
                    span: whole_span,
                    replacement,
                },
            );
            ty
        } else {
            self.parse_type()?
        };

        let span = self.span_from(start_span, ty.span);
        Ok(Param {
            name,
            mutable,
            ty,
            span,
        })
    }

    fn parse_struct(&mut self, start_span: Span, is_pub: bool) -> Result<StructDecl, ()> {
        self.bump(); // `struct`
        let name = self.expect_name_identifier("a struct name")?;
        self.expect(TokenKind::LBrace, "`{` after the struct name")?;
        let mut fields = Vec::new();
        if self.peek() != Some(&TokenKind::RBrace) {
            loop {
                fields.push(self.parse_field()?);
                if self.bump_if(&TokenKind::Comma) {
                    if self.peek() == Some(&TokenKind::RBrace) {
                        break;
                    }
                    continue;
                }
                break;
            }
        }
        let rbrace = self.expect(TokenKind::RBrace, "`}` after the struct's fields")?;
        let span = self.span_from(start_span, rbrace.span);
        Ok(StructDecl {
            name,
            is_pub,
            fields,
            span,
        })
    }

    /// A field: `name: T`. A leading `pub` is `V0001` naming field-level
    /// visibility (spec 4.1: only whole structs are `pub` in milestone 1,
    /// field-level `pub` is milestone 2), reported and skipped so the
    /// field still parses.
    fn parse_field(&mut self) -> Result<FieldDecl, ()> {
        let start_span = self.current_span();
        if self.peek() == Some(&TokenKind::Pub) {
            let pub_span = self.current_span();
            self.bump();
            self.push_error(
                V0001,
                pub_span,
                "field-level `pub` is not supported in Varyk; make the whole struct `pub` instead",
            );
        }
        let name = self.expect_name_identifier("a field name")?;
        self.expect(TokenKind::Colon, "`:` after the field name")?;
        let ty = self.parse_type()?;
        let span = self.span_from(start_span, ty.span);
        Ok(FieldDecl { name, ty, span })
    }

    fn parse_mod(&mut self, start_span: Span, is_pub: bool) -> Result<ModDecl, ()> {
        self.bump(); // `mod`
        let name = self.expect_name_identifier("a module name")?;
        let semi = self.expect(TokenKind::Semi, "`;` after the module name")?;
        let span = self.span_from(start_span, semi.span);
        Ok(ModDecl { name, is_pub, span })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SyntaxError;
    use crate::lex;
    use crate::source::SourceFile;
    use crate::span::FileId;

    fn parse_program(src: &str) -> (Program, Vec<SyntaxError>) {
        let file = SourceFile::new(FileId(0), "test.vr", src);
        let (tokens, lex_errors) = lex(&file);
        assert!(
            lex_errors.is_empty(),
            "unexpected lex errors: {lex_errors:?}"
        );
        crate::parser::parse(&tokens, FileId(0))
    }

    fn parse_program_ok(src: &str) -> Program {
        let (program, errors) = parse_program(src);
        assert!(
            errors.is_empty(),
            "unexpected errors parsing {src:?}: {errors:?}"
        );
        program
    }

    fn only_function(program: &Program) -> &Function {
        assert_eq!(program.items.len(), 1);
        match &program.items[0] {
            Item::Function(f) => f,
            other => panic!("expected a function, got {other:?}"),
        }
    }

    #[test]
    fn function_with_no_params() {
        let program = parse_program_ok("fn main() { }");
        let f = only_function(&program);
        assert_eq!(f.name.name, "main");
        assert!(f.params.is_empty());
        assert!(!f.is_pub);
        assert!(f.return_type.is_none());
    }

    #[test]
    fn function_with_params() {
        let program = parse_program_ok("fn add(a: i32, b: i32) -> i32 { a }");
        let f = only_function(&program);
        assert_eq!(f.params.len(), 2);
        assert_eq!(f.params[0].name.name, "a");
        assert_eq!(f.params[0].ty.name.name, "i32");
        assert!(!f.params[0].mutable);
        assert_eq!(f.return_type.as_ref().unwrap().name.name, "i32");
    }

    #[test]
    fn function_with_mut_param() {
        let program = parse_program_ok("fn rename(mut user: User) { }");
        let f = only_function(&program);
        assert!(f.params[0].mutable);
        assert_eq!(f.params[0].ty.name.name, "User");
    }

    #[test]
    fn pub_function() {
        let program = parse_program_ok("pub fn square(x: i32) -> i32 { x }");
        let f = only_function(&program);
        assert!(f.is_pub);
    }

    #[test]
    fn struct_with_fields() {
        let program = parse_program_ok("struct User { name: string, age: i32 }");
        match &program.items[0] {
            Item::Struct(s) => {
                assert_eq!(s.name.name, "User");
                assert!(!s.is_pub);
                assert_eq!(s.fields.len(), 2);
                assert_eq!(s.fields[0].name.name, "name");
                assert_eq!(s.fields[0].ty.name.name, "string");
                assert_eq!(s.fields[1].name.name, "age");
            }
            other => panic!("expected a struct, got {other:?}"),
        }
    }

    #[test]
    fn pub_struct() {
        let program = parse_program_ok("pub struct User { name: string }");
        match &program.items[0] {
            Item::Struct(s) => assert!(s.is_pub),
            other => panic!("expected a struct, got {other:?}"),
        }
    }

    #[test]
    fn mod_declaration() {
        let program = parse_program_ok("mod greet;");
        match &program.items[0] {
            Item::Mod(m) => {
                assert_eq!(m.name.name, "greet");
                assert!(!m.is_pub);
            }
            other => panic!("expected a mod, got {other:?}"),
        }
    }

    #[test]
    fn pub_mod_declaration() {
        let program = parse_program_ok("pub mod greet;");
        match &program.items[0] {
            Item::Mod(m) => assert!(m.is_pub),
            other => panic!("expected a mod, got {other:?}"),
        }
    }

    #[test]
    fn amp_param_type_is_v0011_with_fix_it() {
        let (program, errors) = parse_program("fn f(user: &User) { }");
        assert!(!program.items.is_empty(), "V0011 must not stop parsing");
        let v0011 = errors
            .iter()
            .find(|e| e.code == V0011)
            .expect("expected a V0011");
        let fix_it = v0011.fix_it.as_ref().expect("V0011 carries a fix-it");
        assert_eq!(fix_it.replacement, "user: User");
        let f = only_function(&program);
        assert!(!f.params[0].mutable);
        assert_eq!(f.params[0].ty.name.name, "User");
    }

    #[test]
    fn amp_mut_param_type_is_v0011_with_fix_it() {
        let (program, errors) = parse_program("fn f(user: &mut User) { }");
        let v0011 = errors
            .iter()
            .find(|e| e.code == V0011)
            .expect("expected a V0011");
        let fix_it = v0011.fix_it.as_ref().expect("V0011 carries a fix-it");
        assert_eq!(fix_it.replacement, "mut user: User");
        let f = only_function(&program);
        assert!(f.params[0].mutable);
    }

    #[test]
    fn lifetime_on_function_is_v0012_with_fix_it() {
        let (program, errors) = parse_program("fn f<'a>() { }");
        assert!(!program.items.is_empty(), "V0012 must not stop parsing");
        let v0012 = errors
            .iter()
            .find(|e| e.code == V0012)
            .expect("expected a V0012");
        let fix_it = v0012.fix_it.as_ref().expect("V0012 carries a fix-it");
        assert_eq!(fix_it.replacement, "");
    }

    #[test]
    fn lifetime_in_param_type_is_v0012_and_v0011() {
        let (program, errors) = parse_program("fn f(user: &'a User) { }");
        assert!(errors.iter().any(|e| e.code == V0012), "errors: {errors:?}");
        assert!(errors.iter().any(|e| e.code == V0011), "errors: {errors:?}");
        let f = only_function(&program);
        assert_eq!(f.params[0].ty.name.name, "User");
    }

    #[test]
    fn reserved_keyword_where_item_expected_is_v0001_and_stops() {
        let (program, errors) = parse_program("match");
        assert!(program.items.is_empty());
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, V0001);
        assert!(errors[0].message.contains("match"), "{}", errors[0].message);
    }

    #[test]
    fn reserved_keyword_as_mod_name_is_v0001() {
        let (_, errors) = parse_program("mod gen;");
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].code, V0001);
        assert_eq!(
            errors[0].message,
            "`gen` is a Rust keyword and cannot be used as a name in Varyk"
        );
    }

    #[test]
    fn function_generics_are_v0001() {
        let (program, errors) = parse_program("fn f<T>() { }");
        assert!(
            !program.items.is_empty(),
            "V0001 generics must not stop parsing"
        );
        assert!(errors.iter().any(|e| e.code == V0001), "errors: {errors:?}");
    }

    #[test]
    fn underscore_function_name_is_v0002() {
        let (program, errors) = parse_program("fn _() { }");
        assert!(program.items.is_empty());
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].code, V0002);
        assert_eq!(errors[0].message, "`_` cannot be used as a name here");
    }

    #[test]
    fn underscore_param_name_is_v0002() {
        let (program, errors) = parse_program("fn f(_: i32) { }");
        assert!(program.items.is_empty());
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].code, V0002);
        assert_eq!(errors[0].message, "`_` cannot be used as a name here");
    }

    #[test]
    fn underscore_let_binding_still_parses() {
        let program = parse_program_ok("fn f() { let _ = 1; }");
        only_function(&program);
    }

    #[test]
    fn pub_field_is_v0001() {
        let (program, errors) = parse_program("struct User { pub name: string }");
        assert!(!program.items.is_empty(), "V0001 must not stop parsing");
        assert!(errors.iter().any(|e| e.code == V0001), "errors: {errors:?}");
        match &program.items[0] {
            Item::Struct(s) => assert_eq!(s.fields[0].name.name, "name"),
            other => panic!("expected a struct, got {other:?}"),
        }
    }
}
