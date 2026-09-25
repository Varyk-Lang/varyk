//! Item parsing: `fn`, `struct`, `enum`, `impl`, and `mod` declarations
//! (spec 4.1, 2.2, 2.5), plus [`Parser::parse_program`], the driver that
//! reads a whole file's items.

use super::Parser;
use crate::ast::{
    EnumDecl, EnumVariant, FieldDecl, Function, ImplBlock, Item, ModDecl, Param, Program, SelfMode,
    StructDecl,
};
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
            Some(TokenKind::Fn) => self
                .parse_function(start_span, is_pub, false)
                .map(Item::Function),
            Some(TokenKind::Struct) => self.parse_struct(start_span, is_pub).map(Item::Struct),
            Some(TokenKind::Enum) => self.parse_enum(start_span, is_pub).map(Item::Enum),
            Some(TokenKind::Impl) if !is_pub => self.parse_impl(start_span).map(Item::Impl),
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
                self.push_error(
                    V0002,
                    span,
                    "expected an item (`fn`, `struct`, `enum`, `impl`, or `mod`)",
                );
                Err(())
            }
        }
    }

    /// `in_impl` is set for a function parsed inside an `impl` block
    /// (`parse_impl`), the only place `self` may open the parameter list
    /// (spec 2.5).
    fn parse_function(
        &mut self,
        start_span: Span,
        is_pub: bool,
        in_impl: bool,
    ) -> Result<Function, ()> {
        self.bump(); // `fn`
        let name = self.expect_name_identifier("a function name")?;
        self.skip_fn_generics_if_present();
        self.expect(TokenKind::LParen, "`(` after the function name")?;
        let (self_mode, self_span) = if in_impl {
            self.parse_self_receiver()?
        } else {
            (SelfMode::None, None)
        };
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
            self_mode,
            self_span,
            params,
            return_type,
            body,
            span,
        })
    }

    /// Reads an optional `self` receiver right after a method's `(`, in one
    /// of the forms spec 2.5 allows (`self`, `mut self`) or the three Rust
    /// habits it names (`&self`, `&mut self`, `self: T`), consuming a
    /// trailing `,` too so the caller's parameter loop starts clean.
    /// Returns `SelfMode::None`, consuming nothing, when the parameter list
    /// does not open with any of these — including plain `self` used
    /// nowhere near the first position, which is left for `parse_param`
    /// (via `expect_name_identifier`) to reject as `V0001`. The span is
    /// the `self` keyword's.
    fn parse_self_receiver(&mut self) -> Result<(SelfMode, Option<Span>), ()> {
        if self.peek() == Some(&TokenKind::Amp) {
            let has_mut = matches!(self.peek_at(1), Some(TokenKind::Mut))
                && matches!(self.peek_at(2), Some(TokenKind::SelfKw));
            let has_self = matches!(self.peek_at(1), Some(TokenKind::SelfKw));
            if !has_mut && !has_self {
                return Ok((SelfMode::None, None));
            }
            let amp = self.bump().expect("peek confirmed `&`");
            let mutable = self.bump_if(&TokenKind::Mut);
            let self_token = self.bump().expect("peek confirmed `self`");
            let span = self.span_from(amp.span, self_token.span);
            let replacement = if mutable { "mut self" } else { "self" };
            self.push_error_with_fix_it(
                V0011,
                span,
                "Varyk's `self` has no `&`; write `self` to only read it, or `mut self` to change it",
                FixIt {
                    span,
                    replacement: replacement.to_string(),
                },
            );
            self.bump_if(&TokenKind::Comma);
            let mode = if mutable {
                SelfMode::Mutable
            } else {
                SelfMode::Shared
            };
            return Ok((mode, Some(self_token.span)));
        }

        if self.peek() == Some(&TokenKind::Mut)
            && matches!(self.peek_at(1), Some(TokenKind::SelfKw))
        {
            self.bump(); // `mut`
            let self_token = self.bump().expect("peek confirmed `self`");
            self.bump_if(&TokenKind::Comma);
            return Ok((SelfMode::Mutable, Some(self_token.span)));
        }

        if self.peek() == Some(&TokenKind::SelfKw) {
            let self_token = self.bump().expect("peek confirmed `self`");
            if self.peek() == Some(&TokenKind::Colon) {
                self.bump(); // `:`
                let ty = self.parse_type()?;
                let span = self.span_from(self_token.span, ty.span);
                self.push_error_with_fix_it(
                    V0001,
                    span,
                    "`self` has no type annotation in Varyk",
                    FixIt {
                        span,
                        replacement: "self".to_string(),
                    },
                );
                self.bump_if(&TokenKind::Comma);
                return Ok((SelfMode::Shared, Some(self_token.span)));
            }
            self.bump_if(&TokenKind::Comma);
            return Ok((SelfMode::Shared, Some(self_token.span)));
        }

        Ok((SelfMode::None, None))
    }

    /// `<T>` or `<'a>` right after a function name: Varyk has
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
            let ty_display = ty.display_name();
            let replacement = if mutable {
                format!("mut {}: {}", name.name, ty_display)
            } else {
                format!("{}: {}", name.name, ty_display)
            };
            let (param, ty_name) = (&name.name, &ty_display);
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
    /// visibility (spec 2.10: field-level `pub` is milestone 3), reported
    /// and skipped so the field still parses.
    fn parse_field(&mut self) -> Result<FieldDecl, ()> {
        let start_span = self.current_span();
        if self.peek() == Some(&TokenKind::Pub) {
            let pub_span = self.current_span();
            self.bump();
            self.push_error(
                V0001,
                pub_span,
                "field-level `pub` is not supported until milestone 3; make the whole struct `pub` instead",
            );
        }
        let name = self.expect_name_identifier("a field name")?;
        self.expect(TokenKind::Colon, "`:` after the field name")?;
        let ty = self.parse_type()?;
        let span = self.span_from(start_span, ty.span);
        Ok(FieldDecl { name, ty, span })
    }

    /// `enum Name { variants... }` (spec 2.2). At least one variant is
    /// required: an empty `{ }` is `V0002`, naming what was expected.
    fn parse_enum(&mut self, start_span: Span, is_pub: bool) -> Result<EnumDecl, ()> {
        self.bump(); // `enum`
        let name = self.expect_name_identifier("an enum name")?;
        self.expect(TokenKind::LBrace, "`{` after the enum name")?;
        let mut variants = Vec::new();
        if self.peek() != Some(&TokenKind::RBrace) {
            loop {
                variants.push(self.parse_enum_variant()?);
                if self.bump_if(&TokenKind::Comma) {
                    if self.peek() == Some(&TokenKind::RBrace) {
                        break;
                    }
                    continue;
                }
                break;
            }
        }
        let rbrace = self.expect(TokenKind::RBrace, "`}` after the enum's variants")?;
        if variants.is_empty() {
            self.push_error(V0002, rbrace.span, "expected at least one variant");
            return Err(());
        }
        let span = self.span_from(start_span, rbrace.span);
        Ok(EnumDecl {
            name,
            is_pub,
            variants,
            span,
        })
    }

    /// A unit variant (`Point`), a tuple variant (`Circle(f64, f64)`), or a
    /// named-field variant (`Circle { x: f64 }`), the last of which is
    /// `V0001` naming milestone 4: it is reported and its field group is
    /// discarded, still yielding a variant with no fields, so the enum
    /// keeps parsing.
    fn parse_enum_variant(&mut self) -> Result<EnumVariant, ()> {
        let name = self.expect_name_identifier("a variant name")?;
        let mut span = name.span;
        let mut fields = Vec::new();

        if self.peek() == Some(&TokenKind::LParen) {
            self.bump();
            if self.peek() != Some(&TokenKind::RParen) {
                loop {
                    fields.push(self.parse_type()?);
                    if self.bump_if(&TokenKind::Comma) {
                        if self.peek() == Some(&TokenKind::RParen) {
                            break;
                        }
                        continue;
                    }
                    break;
                }
            }
            let rparen = self.expect(TokenKind::RParen, "`)` after the variant's fields")?;
            span = self.span_from(name.span, rparen.span);
        } else if self.peek() == Some(&TokenKind::LBrace) {
            let group_span = self.skip_brace_group();
            span = self.span_from(name.span, group_span);
            self.push_error(
                V0001,
                span,
                "enum variants with named fields are not supported until milestone 4; use a tuple variant instead",
            );
        }

        Ok(EnumVariant { name, fields, span })
    }

    /// Consumes a `{...}` group whose opening `{` is already confirmed
    /// present, matching nested braces by depth, and returns the span of
    /// the whole group without building anything from its contents. Mirrors
    /// `ty.rs`'s `skip_generic_args`; best-effort, an unclosed group
    /// consumes the rest of the token stream.
    pub(super) fn skip_brace_group(&mut self) -> Span {
        let lbrace = self.bump().expect("caller confirmed `{` is present");
        let mut depth = 1u32;
        let mut last_span = lbrace.span;
        while depth > 0 {
            match self.peek() {
                Some(TokenKind::LBrace) => {
                    depth += 1;
                    last_span = self.bump().expect("peek just confirmed a token").span;
                }
                Some(TokenKind::RBrace) => {
                    depth -= 1;
                    last_span = self.bump().expect("peek just confirmed a token").span;
                }
                Some(_) => {
                    last_span = self.bump().expect("peek just confirmed a token").span;
                }
                None => break,
            }
        }
        self.span_from(lbrace.span, last_span)
    }

    /// `impl Name { fns... }` (spec 2.5). `impl Trait for Type`, the Rust
    /// trait-impl shape milestone 2 does not have, is `V0001` with a fix-it
    /// dropping the trait name and `for`; parsing continues with `Type` as
    /// the impl's target. Whether `Name` actually names a struct or enum
    /// declared in this file is the compiler's `resolve` module's job.
    fn parse_impl(&mut self, start_span: Span) -> Result<ImplBlock, ()> {
        self.bump(); // `impl`
        let mut type_name = self.expect_name_identifier("a type name")?;
        if self.peek() == Some(&TokenKind::For) {
            self.bump(); // `for`
            let actual = self.expect_name_identifier("a type name after `for`")?;
            let removal_end = actual.span.start;
            let removal_span = Span::new(type_name.span.file, type_name.span.start, removal_end);
            self.push_error_with_fix_it(
                V0001,
                removal_span,
                "trait implementations are not supported in Varyk yet; implement methods directly on the type",
                FixIt {
                    span: removal_span,
                    replacement: String::new(),
                },
            );
            type_name = actual;
        }
        self.expect(TokenKind::LBrace, "`{` after the impl type")?;
        let mut functions = Vec::new();
        while self.peek().is_some() && self.peek() != Some(&TokenKind::RBrace) {
            let fn_start = self.current_span();
            let is_pub = self.bump_if(&TokenKind::Pub);
            match self.peek() {
                Some(TokenKind::Fn) => {
                    functions.push(self.parse_function(fn_start, is_pub, true)?);
                }
                _ => {
                    let span = self.current_span();
                    self.push_error(V0002, span, "expected a function in the `impl` block");
                    return Err(());
                }
            }
        }
        let rbrace = self.expect(TokenKind::RBrace, "`}` after the impl's functions")?;
        let span = self.span_from(start_span, rbrace.span);
        Ok(ImplBlock {
            type_name,
            functions,
            span,
        })
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
    fn amp_generic_param_type_fix_it_keeps_generic_args() {
        let (_, errors) = parse_program("fn total(v: &Vec<i32>) { }");
        let v0011 = errors
            .iter()
            .find(|e| e.code == V0011)
            .expect("expected a V0011");
        let fix_it = v0011.fix_it.as_ref().expect("V0011 carries a fix-it");
        assert_eq!(fix_it.replacement, "v: Vec<i32>");
        assert!(
            v0011.message.contains("Vec<i32>"),
            "message: {}",
            v0011.message
        );
    }

    #[test]
    fn amp_mut_module_qualified_param_type_fix_it_keeps_module_path() {
        let (_, errors) = parse_program("fn f(u: &mut m::T) { }");
        let v0011 = errors
            .iter()
            .find(|e| e.code == V0011)
            .expect("expected a V0011");
        let fix_it = v0011.fix_it.as_ref().expect("V0011 carries a fix-it");
        assert_eq!(fix_it.replacement, "mut u: m::T");
        assert!(v0011.message.contains("m::T"), "message: {}", v0011.message);
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
        let (program, errors) = parse_program("loop");
        assert!(program.items.is_empty());
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, V0001);
        assert!(errors[0].message.contains("loop"), "{}", errors[0].message);
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
        let v0001 = errors
            .iter()
            .find(|e| e.code == V0001)
            .expect("expected a V0001");
        assert_eq!(
            v0001.message,
            "field-level `pub` is not supported until milestone 3; make the whole struct `pub` instead"
        );
        match &program.items[0] {
            Item::Struct(s) => assert_eq!(s.fields[0].name.name, "name"),
            other => panic!("expected a struct, got {other:?}"),
        }
    }

    // --- Enums (spec 2.2) --------------------------------------------------

    fn only_enum(program: &Program) -> &EnumDecl {
        assert_eq!(program.items.len(), 1);
        match &program.items[0] {
            Item::Enum(e) => e,
            other => panic!("expected an enum, got {other:?}"),
        }
    }

    #[test]
    fn enum_with_three_variants() {
        let program =
            parse_program_ok("enum Shape {\n    Circle(f64),\n    Rect(f64, f64),\n    Point,\n}");
        let e = only_enum(&program);
        assert_eq!(e.name.name, "Shape");
        assert!(!e.is_pub);
        assert_eq!(e.variants.len(), 3);
        assert_eq!(e.variants[0].name.name, "Circle");
        assert_eq!(e.variants[0].fields.len(), 1);
        assert_eq!(e.variants[0].fields[0].name.name, "f64");
        assert_eq!(e.variants[1].name.name, "Rect");
        assert_eq!(e.variants[1].fields.len(), 2);
        assert_eq!(e.variants[2].name.name, "Point");
        assert!(e.variants[2].fields.is_empty());
    }

    #[test]
    fn pub_enum() {
        let program = parse_program_ok("pub enum Shape {\n    Point,\n}");
        let e = only_enum(&program);
        assert!(e.is_pub);
    }

    #[test]
    fn enum_with_no_variants_is_v0002() {
        let (program, errors) = parse_program("enum Shape { }");
        assert!(program.items.is_empty(), "V0002 must stop parsing");
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].code, V0002);
        assert_eq!(errors[0].message, "expected at least one variant");
    }

    #[test]
    fn named_field_variant_is_v0001_naming_milestone_4() {
        let (program, errors) = parse_program("enum Shape {\n    Circle { radius: f64 },\n}");
        assert!(!program.items.is_empty(), "V0001 must not stop parsing");
        let v0001 = errors
            .iter()
            .find(|e| e.code == V0001)
            .expect("expected a V0001");
        assert!(v0001.message.contains("milestone 4"), "{}", v0001.message);
        let e = only_enum(&program);
        assert_eq!(e.variants.len(), 1);
        assert_eq!(e.variants[0].name.name, "Circle");
        assert!(e.variants[0].fields.is_empty());
    }

    // --- Impl blocks and methods (spec 2.5) ---------------------------------

    fn only_impl(program: &Program) -> &ImplBlock {
        assert_eq!(program.items.len(), 1);
        match &program.items[0] {
            Item::Impl(i) => i,
            other => panic!("expected an impl block, got {other:?}"),
        }
    }

    #[test]
    fn impl_block_with_new_self_and_mut_self() {
        let program = parse_program_ok(
            r#"
            struct Counter { count: i32 }
            impl Counter {
                fn new() -> Counter {
                    Counter { count: 0 }
                }
                fn add(mut self, by: i32) {
                    self.count = self.count + by;
                }
                fn value(self) -> i32 {
                    self.count
                }
            }
            "#,
        );
        assert_eq!(program.items.len(), 2);
        let i = match &program.items[1] {
            Item::Impl(i) => i,
            other => panic!("expected an impl block, got {other:?}"),
        };
        assert_eq!(i.type_name.name, "Counter");
        assert_eq!(i.functions.len(), 3);
        assert_eq!(i.functions[0].name.name, "new");
        assert!(matches!(i.functions[0].self_mode, SelfMode::None));
        assert_eq!(i.functions[1].name.name, "add");
        assert!(matches!(i.functions[1].self_mode, SelfMode::Mutable));
        assert_eq!(i.functions[1].params.len(), 1);
        assert_eq!(i.functions[1].params[0].name.name, "by");
        assert_eq!(i.functions[2].name.name, "value");
        assert!(matches!(i.functions[2].self_mode, SelfMode::Shared));
    }

    #[test]
    fn self_span_is_the_self_keyword() {
        let src = "impl S {\n    fn a() {}\n    fn b(mut self) {}\n    fn c(self, x: i32) {}\n}";
        let program = parse_program_ok(src);
        let i = only_impl(&program);
        let at = |needle: &str| {
            let start = src.find(needle).expect("in source") as u32;
            let offset = needle.find("self").expect("self in needle") as u32;
            Some((start + offset, start + offset + 4))
        };
        let spans: Vec<Option<(u32, u32)>> = i
            .functions
            .iter()
            .map(|f| f.self_span.map(|s| (s.start, s.end)))
            .collect();
        assert_eq!(spans, vec![None, at("(mut self)"), at("(self, x")]);
    }

    #[test]
    fn amp_self_is_v0011_with_fix_it() {
        let (program, errors) = parse_program("impl S {\n    fn f(&self) { }\n}");
        let i = only_impl(&program);
        assert!(matches!(i.functions[0].self_mode, SelfMode::Shared));
        let v0011 = errors
            .iter()
            .find(|e| e.code == V0011)
            .expect("expected a V0011");
        let fix_it = v0011.fix_it.as_ref().expect("V0011 carries a fix-it");
        assert_eq!(fix_it.replacement, "self");
    }

    #[test]
    fn amp_mut_self_is_v0011_with_fix_it() {
        let (program, errors) = parse_program("impl S {\n    fn f(&mut self) { }\n}");
        let i = only_impl(&program);
        assert!(matches!(i.functions[0].self_mode, SelfMode::Mutable));
        let v0011 = errors
            .iter()
            .find(|e| e.code == V0011)
            .expect("expected a V0011");
        let fix_it = v0011.fix_it.as_ref().expect("V0011 carries a fix-it");
        assert_eq!(fix_it.replacement, "mut self");
    }

    #[test]
    fn self_with_type_annotation_is_v0001_with_fix_it() {
        let (program, errors) = parse_program("impl S {\n    fn f(self: S) { }\n}");
        let i = only_impl(&program);
        assert!(matches!(i.functions[0].self_mode, SelfMode::Shared));
        let v0001 = errors
            .iter()
            .find(|e| e.code == V0001)
            .expect("expected a V0001");
        let fix_it = v0001.fix_it.as_ref().expect("V0001 carries a fix-it");
        assert_eq!(fix_it.replacement, "self");
    }

    #[test]
    fn impl_trait_for_type_is_v0001_with_fix_it() {
        let (program, errors) = parse_program("impl Greet for S {\n    fn f(self) { }\n}");
        let i = only_impl(&program);
        assert_eq!(i.type_name.name, "S");
        let v0001 = errors
            .iter()
            .find(|e| e.code == V0001)
            .expect("expected a V0001");
        assert!(v0001.message.contains("trait"), "{}", v0001.message);
        let fix_it = v0001.fix_it.as_ref().expect("V0001 carries a fix-it");
        assert_eq!(fix_it.replacement, "");
    }

    #[test]
    fn self_in_a_non_receiver_parameter_position_is_v0001() {
        let (_, errors) = parse_program("impl S {\n    fn f(x: i32, self: S) { }\n}");
        assert!(
            errors
                .iter()
                .any(|e| e.code == V0001 && e.message.contains("keyword")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn self_as_top_level_function_receiver_is_v0001() {
        let (_, errors) = parse_program("fn f(self) { }");
        assert!(
            errors
                .iter()
                .any(|e| e.code == V0001 && e.message.contains("keyword")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn self_as_a_function_name_is_v0001() {
        let (_, errors) = parse_program("fn self() { }");
        assert!(
            errors
                .iter()
                .any(|e| e.code == V0001 && e.message.contains("keyword")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn self_as_a_let_name_is_v0001() {
        let (_, errors) = parse_program("fn f() { let self = 1; }");
        assert!(
            errors
                .iter()
                .any(|e| e.code == V0001 && e.message.contains("keyword")),
            "errors: {errors:?}"
        );
    }
}
