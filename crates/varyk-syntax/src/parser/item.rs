//! Item parsing: `fn`, `struct`, `enum`, `impl`, and `mod` declarations
//! (spec 4.1, 2.2, 2.5), plus [`Parser::parse_program`], the driver that
//! reads a whole file's items.

use super::Parser;
use crate::ast::{
    AttrArg, Attribute, EnumDecl, EnumVariant, FieldDecl, Function, Ident, ImplBlock, Item,
    Literal, ModDecl, Param, Path, Program, SelfMode, StructDecl, UseDecl, VariantField,
    VariantFields,
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

    /// Every attribute (`#[name]`, `#[name(literal)]`, M5a spec 2.2) at the
    /// current position, in order; none when the current token is not `#`.
    fn parse_attributes(&mut self) -> Result<Vec<Attribute>, ()> {
        let mut attrs = Vec::new();
        while self.peek() == Some(&TokenKind::Hash) {
            attrs.push(self.parse_attribute()?);
        }
        Ok(attrs)
    }

    /// One attribute, at its `#`. A single literal between the parentheses
    /// is its argument; anything else there is kept as [`AttrArg::Other`]
    /// for the resolver to report with the attribute's name.
    fn parse_attribute(&mut self) -> Result<Attribute, ()> {
        let hash = self.current_span();
        self.bump(); // `#`
        self.expect(TokenKind::LBracket, "`[` after `#`")?;
        let name = self.expect_identifier("an attribute name")?;
        let arg = if self.peek() == Some(&TokenKind::LParen) {
            let literal_len = match self.peek_at(1) {
                Some(TokenKind::Minus) => 2,
                Some(
                    TokenKind::IntegerLiteral(_)
                    | TokenKind::FloatLiteral(_)
                    | TokenKind::StringLiteral(_)
                    | TokenKind::BoolLiteral(_),
                ) => 1,
                _ => 0,
            };
            let number_after_minus = matches!(
                self.peek_at(2),
                Some(TokenKind::IntegerLiteral(_) | TokenKind::FloatLiteral(_))
            );
            let one_literal = literal_len > 0
                && ((literal_len == 2 && !number_after_minus)
                    || self.peek_at(1 + literal_len) == Some(&TokenKind::RParen));
            if one_literal {
                self.bump(); // `(`
                let (literal, _) = self.parse_pattern_literal()?;
                self.expect(TokenKind::RParen, "`)` after the attribute's value")?;
                Some(match literal {
                    Literal::Str(text) => AttrArg::Str(text),
                    Literal::Int { text, negative } => AttrArg::Int { text, negative },
                    Literal::Float { text, negative } => AttrArg::Float { text, negative },
                    Literal::Bool(value) => AttrArg::Bool(value),
                })
            } else {
                self.skip_paren_group();
                Some(AttrArg::Other)
            }
        } else {
            None
        };
        let rbracket = self.expect(TokenKind::RBracket, "`]` after the attribute")?;
        Ok(Attribute {
            name,
            arg,
            span: self.span_from(hash, rbracket.span),
        })
    }

    fn parse_item(&mut self) -> Result<Item, ()> {
        let attrs = self.parse_attributes()?;
        let item = self.parse_item_after(!attrs.is_empty())?;
        Ok(match item {
            Item::Function(decl) => Item::Function(Function { attrs, ..decl }),
            Item::Struct(decl) => Item::Struct(StructDecl { attrs, ..decl }),
            Item::Enum(decl) => Item::Enum(EnumDecl { attrs, ..decl }),
            Item::Impl(decl) => Item::Impl(ImplBlock { attrs, ..decl }),
            Item::Mod(decl) => Item::Mod(ModDecl { attrs, ..decl }),
            Item::Use(decl) => Item::Use(UseDecl { attrs, ..decl }),
        })
    }

    /// An item, after its attributes (`after_attrs` when there were any).
    fn parse_item_after(&mut self, after_attrs: bool) -> Result<Item, ()> {
        let start_span = self.current_span();
        let is_pub = self.bump_if(&TokenKind::Pub);
        if is_pub && self.peek() == Some(&TokenKind::LParen) {
            self.reject_pub_paren(start_span);
        }
        let is_async = self.parse_async()?;
        match self.peek() {
            Some(TokenKind::Fn) => self
                .parse_function(start_span, is_pub, false)
                .map(|f| Item::Function(Function { is_async, ..f })),
            Some(TokenKind::Struct) => self.parse_struct(start_span, is_pub).map(Item::Struct),
            Some(TokenKind::Enum) => self.parse_enum(start_span, is_pub).map(Item::Enum),
            Some(TokenKind::Impl) if !is_pub => self.parse_impl(start_span).map(Item::Impl),
            Some(TokenKind::Mod) => self.parse_mod(start_span, is_pub).map(Item::Mod),
            Some(TokenKind::UseKw) => self.parse_use(start_span, is_pub).map(Item::Use),
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
                let what = if after_attrs {
                    "an item (`fn`, `struct`, `enum`, `impl`, `mod`, or `use`) after the attribute"
                } else {
                    "an item (`fn`, `struct`, `enum`, `impl`, `mod`, or `use`)"
                };
                self.push_expected(what);
                Err(())
            }
        }
    }

    /// An optional `async`, which must be followed by `fn` (milestone 5b1
    /// spec 2.2); anything else after it is `V0002`.
    fn parse_async(&mut self) -> Result<bool, ()> {
        if !self.bump_if(&TokenKind::Async) {
            return Ok(false);
        }
        if self.peek() == Some(&TokenKind::Fn) {
            Ok(true)
        } else {
            self.push_expected("`fn` after `async`");
            Err(())
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
        let return_type = if self.peek() == Some(&TokenKind::Arrow) {
            Some(self.parse_return_type()?)
        } else {
            None
        };
        let body = self.parse_block()?;
        let span = self.span_from(start_span, body.span);
        Ok(Function {
            attrs: Vec::new(),
            name,
            is_pub,
            is_async: false,
            self_mode,
            self_span,
            params,
            return_type,
            body,
            span,
        })
    }

    /// `-> T` at the `->`. A Rust-style `-> &T` or `-> &mut T`, with or
    /// without a lifetime, is `V0011` with a fix-it writing `-> T`, and
    /// `-> string` for `-> &str` (M4 spec 5): a borrowed return is worked
    /// out, never written. Parsing goes on with `T` as the return type.
    fn parse_return_type(&mut self) -> Result<crate::ast::TypeExpr, ()> {
        let arrow = self.bump().expect("caller confirmed `->`");
        if !self.bump_if(&TokenKind::Amp) {
            return self.parse_type();
        }
        if matches!(self.peek(), Some(TokenKind::Lifetime(_))) {
            self.bump();
        }
        self.bump_if(&TokenKind::Mut);
        let ty = self.parse_type()?;
        let written = ty.display_name();
        let owned = if written == "str" {
            "string".to_string()
        } else {
            written
        };
        let span = self.span_from(arrow.span, ty.span);
        self.push_error_with_fix_it(
            V0011,
            span,
            format!(
                "a return type has no `&` in Varyk; write `-> {owned}`, and Varyk works out the \
                 borrow"
            ),
            FixIt {
                span,
                replacement: format!("-> {owned}"),
            },
        );
        Ok(ty)
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
            attrs: Vec::new(),
            name,
            is_pub,
            fields,
            span,
        })
    }

    /// A field: `name: T`, optionally `pub` (spec 3.4). `pub(...)`, Rust's
    /// restricted visibility, is `V0001` (`reject_pub_paren`): Varyk has
    /// only plain `pub`.
    fn parse_field(&mut self) -> Result<FieldDecl, ()> {
        let attrs = self.parse_attributes()?;
        let start_span = self.current_span();
        let is_pub = self.bump_if(&TokenKind::Pub);
        if is_pub && self.peek() == Some(&TokenKind::LParen) {
            self.reject_pub_paren(start_span);
        }
        let name = self.expect_name_identifier("a field name")?;
        self.expect(TokenKind::Colon, "`:` after the field name")?;
        let ty = self.parse_type()?;
        let span = self.span_from(start_span, ty.span);
        Ok(FieldDecl {
            attrs,
            name,
            ty,
            is_pub,
            span,
        })
    }

    /// `pub(...)` (spec 3.2, 3.4): Rust's restricted visibility
    /// (`pub(crate)`, `pub(super)`, `pub(in path)`), which Varyk does not
    /// have. `pub_span` is the already-consumed `pub` keyword's span, and
    /// the current token is the group's opening `(`, confirmed present by
    /// the caller. Reports `V0001` naming what Varyk offers instead and
    /// consumes the parenthesized group, so parsing continues as plain
    /// `pub`.
    fn reject_pub_paren(&mut self, pub_span: Span) {
        let group_span = self.skip_paren_group();
        let span = self.span_from(pub_span, group_span);
        self.push_error(
            V0001,
            span,
            "Varyk has only `pub`; `pub(crate)`, `pub(super)`, and `pub(in path)` are not supported",
        );
    }

    /// Consumes a `(...)` group whose opening `(` is already confirmed
    /// present, matching nested parentheses by depth, and returns the span
    /// of the whole group without building anything from its contents.
    /// Mirrors `skip_brace_group`; best-effort, an unclosed group consumes
    /// the rest of the token stream.
    fn skip_paren_group(&mut self) -> Span {
        let lparen = self.bump().expect("caller confirmed `(` is present");
        let mut depth = 1u32;
        let mut last_span = lparen.span;
        while depth > 0 {
            match self.peek() {
                Some(TokenKind::LParen) => {
                    depth += 1;
                    last_span = self.bump().expect("peek just confirmed a token").span;
                }
                Some(TokenKind::RParen) => {
                    depth -= 1;
                    last_span = self.bump().expect("peek just confirmed a token").span;
                }
                Some(_) => {
                    last_span = self.bump().expect("peek just confirmed a token").span;
                }
                None => break,
            }
        }
        self.span_from(lparen.span, last_span)
    }

    /// `use path;` or `use path as name;` (spec 3.3), right after `use` has
    /// been recognized but not yet consumed; `is_pub` for a `pub use`
    /// (milestone 5b3 spec 2.5), whose `as` is `V0001`.
    fn parse_use(&mut self, start_span: Span, is_pub: bool) -> Result<UseDecl, ()> {
        self.bump(); // `use`
        let path = self.parse_use_path()?;
        let alias = if self.bump_if(&TokenKind::As) {
            let alias = self.expect_name_identifier("a name after `as`")?;
            // A re-export under another name (milestone 5b3 spec 2.5).
            if is_pub {
                let span = self.span_from(start_span, alias.span);
                self.push_error(
                    V0001,
                    span,
                    "`pub use` with `as` is not supported yet; re-export the item under its own \
                     name, or write a plain `use` to rename it inside this file",
                );
            }
            Some(alias)
        } else {
            None
        };
        let semi = self.expect(TokenKind::Semi, "`;` after the `use` path")?;
        let span = self.span_from(start_span, semi.span);
        Ok(UseDecl {
            attrs: Vec::new(),
            is_pub,
            path,
            alias,
            span,
        })
    }

    /// The path of a `use` item: an optional keyword prefix (`crate`,
    /// `self`, `super`) followed by one or more `::`-separated names.
    /// Braces and globs, Rust habits Varyk's `use` does not have (spec
    /// 3.3), are each `V0001` naming the construct, from
    /// `parse_use_segment`.
    fn parse_use_path(&mut self) -> Result<Path, ()> {
        let start_span = self.current_span();
        let leading = self.take_path_start();
        let mut segments = vec![self.parse_use_segment()?];
        while self.peek() == Some(&TokenKind::ColonColon) {
            self.bump();
            segments.push(self.parse_use_segment()?);
        }
        let end = segments.last().expect("at least one segment").span;
        Ok(Path {
            leading,
            segments,
            span: self.span_from(start_span, end),
        })
    }

    /// One name in a `use` path: a plain identifier, or `V0001` for a
    /// brace group (`{B, C}`) or a glob (`*`), each naming what Varyk
    /// offers instead of the Rust habit.
    fn parse_use_segment(&mut self) -> Result<Ident, ()> {
        match self.peek() {
            Some(TokenKind::LBrace) => {
                let span = self.skip_brace_group();
                self.push_error(
                    V0001,
                    span,
                    "`use` does not support grouped imports in Varyk; write one `use` per name",
                );
                Err(())
            }
            Some(TokenKind::Star) => {
                let span = self.current_span();
                self.bump();
                self.push_error(
                    V0001,
                    span,
                    "`use` does not support glob imports in Varyk; write the name you want",
                );
                Err(())
            }
            _ => self.expect_identifier("a name in the `use` path"),
        }
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
            attrs: Vec::new(),
            name,
            is_pub,
            variants,
            span,
        })
    }

    /// A unit variant (`Point`), a tuple variant (`Circle(f64, f64)`), or a
    /// variant with named fields (`Click { x: i32, y: i32 }`, M4 spec 2.5).
    fn parse_enum_variant(&mut self) -> Result<EnumVariant, ()> {
        let attrs = self.parse_attributes()?;
        let name = self.expect_name_identifier("a variant name")?;
        let (fields, span) = match self.peek() {
            Some(TokenKind::LParen) => {
                self.bump();
                let mut types = Vec::new();
                if self.peek() != Some(&TokenKind::RParen) {
                    loop {
                        types.push(self.parse_type()?);
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
                (
                    VariantFields::Tuple(types),
                    self.span_from(name.span, rparen.span),
                )
            }
            Some(TokenKind::LBrace) => {
                self.bump();
                let mut fields = Vec::new();
                if self.peek() != Some(&TokenKind::RBrace) {
                    loop {
                        fields.push(self.parse_variant_field()?);
                        if self.bump_if(&TokenKind::Comma) {
                            if self.peek() == Some(&TokenKind::RBrace) {
                                break;
                            }
                            continue;
                        }
                        break;
                    }
                }
                let rbrace = self.expect(TokenKind::RBrace, "`}` after the variant's fields")?;
                (
                    VariantFields::Named(fields),
                    self.span_from(name.span, rbrace.span),
                )
            }
            _ => (VariantFields::Unit, name.span),
        };
        Ok(EnumVariant {
            attrs,
            name,
            fields,
            span,
        })
    }

    /// A named field of a variant: `name: T`. `pub` (or `pub(...)`) before
    /// it is `V0001` with a fix-it removing it, since a variant's fields
    /// are as visible as the enum; parsing goes on without it.
    fn parse_variant_field(&mut self) -> Result<VariantField, ()> {
        let attrs = self.parse_attributes()?;
        if self.peek() == Some(&TokenKind::Pub) {
            let pub_span = self.current_span();
            self.bump();
            if self.peek() == Some(&TokenKind::LParen) {
                self.skip_paren_group();
            }
            let next_start = self.current_span().start;
            let span = Span::new(pub_span.file, pub_span.start, next_start);
            self.push_error_with_fix_it(
                V0001,
                span,
                "a variant's fields take no `pub` in Varyk; they are as visible as the enum",
                FixIt {
                    span,
                    replacement: String::new(),
                },
            );
        }
        let name = self.expect_name_identifier("a field name")?;
        self.expect(TokenKind::Colon, "`:` after the field name")?;
        let ty = self.parse_type()?;
        let span = self.span_from(name.span, ty.span);
        Ok(VariantField {
            attrs,
            name,
            ty,
            span,
        })
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
            let attrs = self.parse_attributes()?;
            let fn_start = self.current_span();
            let is_pub = self.bump_if(&TokenKind::Pub);
            let is_async = self.parse_async()?;
            match self.peek() {
                Some(TokenKind::Fn) => {
                    let function = self.parse_function(fn_start, is_pub, true)?;
                    functions.push(Function {
                        attrs,
                        is_async,
                        ..function
                    });
                }
                _ => {
                    self.push_expected("a function in the `impl` block");
                    return Err(());
                }
            }
        }
        let rbrace = self.expect(TokenKind::RBrace, "`}` after the impl's functions")?;
        let span = self.span_from(start_span, rbrace.span);
        Ok(ImplBlock {
            attrs: Vec::new(),
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
        Ok(ModDecl {
            attrs: Vec::new(),
            name,
            is_pub,
            span,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SyntaxError;
    use crate::lex;
    use crate::parser::STRAY_HASH;
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
    fn pub_field_parses() {
        let program = parse_program_ok("struct User { pub name: string, age: i32 }");
        match &program.items[0] {
            Item::Struct(s) => {
                assert!(s.fields[0].is_pub);
                assert!(!s.fields[1].is_pub);
            }
            other => panic!("expected a struct, got {other:?}"),
        }
    }

    #[test]
    fn pub_paren_on_a_function_is_v0001() {
        for src in ["pub(crate) fn f() { }", "pub(super) fn f() { }"] {
            let (program, errors) = parse_program(src);
            assert!(!program.items.is_empty(), "V0001 must not stop parsing");
            let v0001 = errors
                .iter()
                .find(|e| e.code == V0001)
                .unwrap_or_else(|| panic!("expected a V0001 parsing {src:?}: {errors:?}"));
            assert!(v0001.message.contains("only `pub`"), "{}", v0001.message);
            let f = only_function(&program);
            assert!(f.is_pub, "pub( ) should still leave the item `pub`");
        }
    }

    #[test]
    fn pub_paren_on_a_field_is_v0001() {
        let (program, errors) = parse_program("struct User { pub(crate) name: string }");
        assert!(!program.items.is_empty(), "V0001 must not stop parsing");
        let v0001 = errors
            .iter()
            .find(|e| e.code == V0001)
            .expect("expected a V0001");
        assert!(v0001.message.contains("only `pub`"), "{}", v0001.message);
        match &program.items[0] {
            Item::Struct(s) => {
                assert_eq!(s.fields[0].name.name, "name");
                assert!(s.fields[0].is_pub);
            }
            other => panic!("expected a struct, got {other:?}"),
        }
    }

    // --- `use` (spec 3.3) ---------------------------------------------------

    fn only_use(program: &Program) -> &UseDecl {
        assert_eq!(program.items.len(), 1);
        match &program.items[0] {
            Item::Use(u) => u,
            other => panic!("expected a use item, got {other:?}"),
        }
    }

    #[test]
    fn use_crate_path_with_several_segments() {
        let program = parse_program_ok("use crate::a::b::C;");
        let u = only_use(&program);
        assert!(matches!(u.path.leading, crate::ast::PathStart::Crate));
        let names: Vec<&str> = u.path.segments.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "C"]);
        assert!(u.alias.is_none());
    }

    #[test]
    fn use_with_alias() {
        let program = parse_program_ok("use a::B as D;");
        let u = only_use(&program);
        assert!(matches!(u.path.leading, crate::ast::PathStart::None));
        let names: Vec<&str> = u.path.segments.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["a", "B"]);
        assert_eq!(u.alias.as_ref().unwrap().name, "D");
    }

    #[test]
    fn use_self_path() {
        let program = parse_program_ok("use self::x;");
        let u = only_use(&program);
        assert!(matches!(u.path.leading, crate::ast::PathStart::SelfMod));
        assert_eq!(u.path.segments[0].name, "x");
    }

    #[test]
    fn use_super_path() {
        let program = parse_program_ok("use super::x;");
        let u = only_use(&program);
        assert!(matches!(u.path.leading, crate::ast::PathStart::Super));
        assert_eq!(u.path.segments[0].name, "x");
    }

    #[test]
    fn use_braced_group_is_v0001() {
        let (_, errors) = parse_program("use a::{B, C};");
        let v0001 = errors
            .iter()
            .find(|e| e.code == V0001)
            .expect("expected a V0001");
        assert!(
            v0001.message.contains("grouped imports"),
            "{}",
            v0001.message
        );
    }

    #[test]
    fn use_glob_is_v0001() {
        let (_, errors) = parse_program("use a::*;");
        let v0001 = errors
            .iter()
            .find(|e| e.code == V0001)
            .expect("expected a V0001");
        assert!(v0001.message.contains("glob imports"), "{}", v0001.message);
    }

    #[test]
    fn pub_use_parses_as_a_use_marked_pub() {
        let program = parse_program_ok("pub use a::b;");
        let u = only_use(&program);
        assert!(u.is_pub);
        assert!(u.alias.is_none());
        let names: Vec<&str> = u.path.segments.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b"]);
        assert!(!only_use(&parse_program_ok("use a::b;")).is_pub);
    }

    #[test]
    fn pub_use_with_as_is_v0001() {
        let (program, errors) = parse_program("pub use a::b as c;");
        let v0001 = errors
            .iter()
            .find(|e| e.code == V0001)
            .expect("expected a V0001");
        assert!(
            v0001.message.contains("not supported yet"),
            "{}",
            v0001.message
        );
        assert!(v0001.message.contains("pub use"), "{}", v0001.message);
        // Reported, but the `use` still parses: `V0001` must not stop
        // parsing.
        let u = only_use(&program);
        assert!(u.is_pub);
        assert_eq!(u.alias.as_ref().map(|a| a.name.as_str()), Some("c"));
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
        match &e.variants[0].fields {
            VariantFields::Tuple(types) => {
                assert_eq!(types.len(), 1);
                assert_eq!(types[0].name.name, "f64");
            }
            other => panic!("expected a tuple variant, got {other:?}"),
        }
        assert_eq!(e.variants[1].name.name, "Rect");
        assert!(matches!(&e.variants[1].fields, VariantFields::Tuple(types) if types.len() == 2));
        assert_eq!(e.variants[2].name.name, "Point");
        assert_eq!(e.variants[2].fields, VariantFields::Unit);
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
    fn named_field_variant() {
        let src = "Click { x: i32, y: i32 }";
        let program = parse_program_ok(&format!("enum Event {{\n    {src},\n    Quit,\n}}"));
        let e = only_enum(&program);
        assert_eq!(e.variants.len(), 2);
        let click = &e.variants[0];
        assert_eq!(click.name.name, "Click");
        assert_eq!(click.span.end - click.span.start, src.len() as u32);
        let VariantFields::Named(fields) = &click.fields else {
            panic!("expected named fields, got {:?}", click.fields);
        };
        let names: Vec<(&str, &str)> = fields
            .iter()
            .map(|f| (f.name.name.as_str(), f.ty.name.name.as_str()))
            .collect();
        assert_eq!(names, vec![("x", "i32"), ("y", "i32")]);
        assert_eq!(
            fields[0].span.end - fields[0].span.start,
            "x: i32".len() as u32
        );
        assert_eq!(e.variants[1].fields, VariantFields::Unit);
    }

    #[test]
    fn pub_on_a_variant_field_is_v0001_with_a_fix_it() {
        let (program, errors) = parse_program("enum Event {\n    Click { pub x: i32 },\n}");
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].code, V0001);
        assert_eq!(
            errors[0].message,
            "a variant's fields take no `pub` in Varyk; they are as visible as the enum"
        );
        let fix_it = errors[0].fix_it.as_ref().expect("a fix-it");
        assert_eq!(fix_it.replacement, "");
        assert_eq!((fix_it.span.start, fix_it.span.end), (25, 29));
        let e = only_enum(&program);
        assert!(matches!(&e.variants[0].fields, VariantFields::Named(fields) if fields.len() == 1));
    }

    #[test]
    fn reference_return_type_is_v0011_with_a_fix_it() {
        let cases = [
            (
                "fn f(s: string) -> &str { s }",
                "-> &str",
                "-> string",
                "str",
            ),
            (
                "fn f(u: User) -> &User { u }",
                "-> &User",
                "-> User",
                "User",
            ),
            (
                "fn f(mut u: User) -> &mut User { u }",
                "-> &mut User",
                "-> User",
                "User",
            ),
        ];
        for (src, written, replacement, parsed) in cases {
            let (program, errors) = parse_program(src);
            assert_eq!(errors.len(), 1, "{src}: {errors:?}");
            assert_eq!(errors[0].code, V0011);
            assert_eq!(
                errors[0].message,
                format!(
                    "a return type has no `&` in Varyk; write `{replacement}`, and Varyk works \
                     out the borrow"
                )
            );
            let start = src.find(written).expect("the written return type") as u32;
            let fix_it = errors[0].fix_it.as_ref().expect("a fix-it");
            assert_eq!(fix_it.replacement, replacement);
            assert_eq!(
                (fix_it.span.start, fix_it.span.end),
                (start, start + written.len() as u32)
            );
            assert_eq!(errors[0].span, fix_it.span);
            let f = only_function(&program);
            assert_eq!(
                f.return_type.as_ref().map(|t| t.name.name.as_str()),
                Some(parsed)
            );
        }
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

    // --- Attributes (M5a spec 2.2) ---------------------------------------

    fn only_struct(program: &Program) -> &StructDecl {
        assert_eq!(program.items.len(), 1);
        match &program.items[0] {
            Item::Struct(s) => s,
            other => panic!("expected a struct, got {other:?}"),
        }
    }

    fn attr_names(attrs: &[Attribute]) -> Vec<&str> {
        attrs.iter().map(|a| a.name.name.as_str()).collect()
    }

    #[test]
    fn rename_on_its_own_line_before_a_field() {
        let program = parse_program_ok(
            "struct User {\n    #[rename(\"userName\")]\n    user_name: string,\n}",
        );
        let s = only_struct(&program);
        let attrs = &s.fields[0].attrs;
        assert_eq!(attr_names(attrs), vec!["rename"]);
        assert_eq!(attrs[0].arg, Some(AttrArg::Str("userName".to_string())));
        // The field's own span still starts at its name.
        assert_eq!(s.fields[0].span.start, s.fields[0].name.span.start);
    }

    #[test]
    fn rename_inline_before_a_field() {
        let program = parse_program_ok("struct User { #[rename(\"id\")] user_id: i32 }");
        let s = only_struct(&program);
        assert_eq!(attr_names(&s.fields[0].attrs), vec!["rename"]);
        assert_eq!(s.fields[0].name.name, "user_id");
    }

    #[test]
    fn stacked_attributes_keep_their_order() {
        let program = parse_program_ok("struct C { #[skip] #[default(1)] n: i32, m: i32 }");
        let s = only_struct(&program);
        let attrs = &s.fields[0].attrs;
        assert_eq!(attr_names(attrs), vec!["skip", "default"]);
        assert_eq!(attrs[0].arg, None);
        assert_eq!(
            attrs[1].arg,
            Some(AttrArg::Int {
                text: "1".to_string(),
                negative: false
            })
        );
        assert!(s.fields[1].attrs.is_empty());
    }

    #[test]
    fn negative_float_and_bool_literals() {
        let program = parse_program_ok(
            "struct C { #[default(-1)] a: i32, #[default(1.5)] b: f64, #[default(-2.5)] c: f64, \
             #[default(true)] d: bool }",
        );
        let s = only_struct(&program);
        let args: Vec<Option<AttrArg>> = s.fields.iter().map(|f| f.attrs[0].arg.clone()).collect();
        assert_eq!(
            args,
            vec![
                Some(AttrArg::Int {
                    text: "1".to_string(),
                    negative: true
                }),
                Some(AttrArg::Float {
                    text: "1.5".to_string(),
                    negative: false
                }),
                Some(AttrArg::Float {
                    text: "2.5".to_string(),
                    negative: true
                }),
                Some(AttrArg::Bool(true)),
            ]
        );
    }

    #[test]
    fn an_argument_that_is_not_one_literal_is_kept_as_other() {
        let program = parse_program_ok("#[derive(Clone, PartialEq)]\nstruct P { x: i32 }");
        let s = only_struct(&program);
        assert_eq!(attr_names(&s.attrs), vec!["derive"]);
        assert_eq!(s.attrs[0].arg, Some(AttrArg::Other));
        // The struct's span starts at `struct`, not at the attribute.
        assert_eq!(&s.span, &Span::new(FileId(0), 28, s.span.end));
    }

    #[test]
    fn test_before_a_function() {
        let program = parse_program_ok("#[test]\nfn parses() { }");
        let f = only_function(&program);
        assert_eq!(attr_names(&f.attrs), vec!["test"]);
        assert_eq!(f.attrs[0].span, Span::new(FileId(0), 0, 7));
        assert_eq!(f.span.start, 8);
    }

    #[test]
    fn attributes_before_every_item_kind_parse() {
        let program = parse_program_ok(
            "#[a] mod m;\n#[b] use m::f;\n#[c] enum E { X }\n#[d] impl E { }\n#[e] pub fn g() { }",
        );
        let names: Vec<Vec<&str>> = program
            .items
            .iter()
            .map(|item| match item {
                Item::Mod(d) => attr_names(&d.attrs),
                Item::Use(d) => attr_names(&d.attrs),
                Item::Enum(d) => attr_names(&d.attrs),
                Item::Impl(d) => attr_names(&d.attrs),
                Item::Function(d) => attr_names(&d.attrs),
                Item::Struct(d) => attr_names(&d.attrs),
            })
            .collect();
        assert_eq!(
            names,
            vec![vec!["a"], vec!["b"], vec!["c"], vec!["d"], vec!["e"]]
        );
    }

    #[test]
    fn attributes_before_a_variant_a_variant_field_and_a_method() {
        let program = parse_program_ok(
            "enum E { #[rename(\"a\")] A, B { #[skip] x: i32 } }\n\
             impl E { #[test] fn m(self) { } }",
        );
        let Item::Enum(e) = &program.items[0] else {
            panic!("expected an enum");
        };
        assert_eq!(attr_names(&e.variants[0].attrs), vec!["rename"]);
        assert!(e.variants[1].attrs.is_empty());
        let VariantFields::Named(fields) = &e.variants[1].fields else {
            panic!("expected named fields");
        };
        assert_eq!(attr_names(&fields[0].attrs), vec!["skip"]);
        let Item::Impl(block) = &program.items[1] else {
            panic!("expected an impl");
        };
        assert_eq!(attr_names(&block.functions[0].attrs), vec!["test"]);
    }

    #[test]
    fn a_stray_hash_in_a_body_names_where_attributes_go() {
        let (_, errors) = parse_program("fn f() { #[skip] let x = 1; }");
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].code, V0002);
        assert_eq!(errors[0].message, STRAY_HASH);
    }

    #[test]
    fn a_stray_hash_in_a_type_names_where_attributes_go() {
        let (_, errors) = parse_program("struct S { x: #[a] i32 }");
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].message, STRAY_HASH);
    }

    #[test]
    fn a_stray_hash_after_an_expression_or_in_a_pattern_names_where_attributes_go() {
        for src in [
            "fn f() { let x = 1; x #[a] }",
            "fn f() { match 1 { #[a] 1 => {}, _ => {} } }",
            "fn f() { g(#[a] 1); }",
        ] {
            let (_, errors) = parse_program(src);
            assert_eq!(errors.len(), 1, "{src}: {errors:?}");
            assert_eq!(
                (errors[0].code, errors[0].message.as_str()),
                (V0002, STRAY_HASH)
            );
        }
    }

    #[test]
    fn an_attribute_with_nothing_after_it_is_v0002() {
        let (_, errors) = parse_program("fn f() { }\n#[test]");
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].code, V0002);
        assert_eq!(
            errors[0].message,
            "expected an item (`fn`, `struct`, `enum`, `impl`, `mod`, or `use`) after the attribute"
        );
    }

    #[test]
    fn hash_without_a_bracket_is_v0002() {
        let (_, errors) = parse_program("# fn f() { }");
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].code, V0002);
        assert_eq!(errors[0].message, "expected `[` after `#`");
    }

    #[test]
    fn a_minus_before_a_string_is_v0002() {
        let (_, errors) = parse_program("struct S { #[default(-\"x\")] s: string }");
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].code, V0002);
        assert_eq!(errors[0].message, "expected a number after `-`");
    }

    #[test]
    fn async_function_and_pub_async_function() {
        let program = parse_program_ok("async fn load() -> i32 { 1 }\npub async fn save() { }");
        let functions: Vec<(&str, bool, bool)> = program
            .items
            .iter()
            .map(|item| match item {
                Item::Function(f) => (f.name.name.as_str(), f.is_pub, f.is_async),
                other => panic!("expected a function, got {other:?}"),
            })
            .collect();
        assert_eq!(functions, vec![("load", false, true), ("save", true, true)]);
        let plain = parse_program_ok("fn f() { }");
        assert!(!only_function(&plain).is_async);
    }

    #[test]
    fn async_methods_in_an_impl() {
        let program = parse_program_ok(
            "impl S {\n    async fn a(self) { }\n    pub async fn b(mut self, x: i32) { }\n    fn c() { }\n}",
        );
        let i = only_impl(&program);
        let methods: Vec<(bool, bool, SelfMode)> = i
            .functions
            .iter()
            .map(|f| (f.is_pub, f.is_async, f.self_mode))
            .collect();
        assert_eq!(
            methods,
            vec![
                (false, true, SelfMode::Shared),
                (true, true, SelfMode::Mutable),
                (false, false, SelfMode::None),
            ]
        );
    }

    #[test]
    fn async_before_anything_but_fn_is_v0002() {
        for src in [
            "async struct S { }",
            "pub async mod m;",
            "async pub fn f() { }",
            "impl S { async self }",
        ] {
            let (_, errors) = parse_program(src);
            assert_eq!(errors.len(), 1, "{src}: {errors:?}");
            assert_eq!(errors[0].code, V0002, "{src}");
            assert_eq!(errors[0].message, "expected `fn` after `async`", "{src}");
        }
    }

    #[test]
    fn async_and_await_are_not_names() {
        for src in ["fn async() { }", "fn f(await: i32) { }"] {
            let (_, errors) = parse_program(src);
            assert_eq!(errors.len(), 1, "{src}: {errors:?}");
            assert_eq!(errors[0].code, V0001, "{src}");
        }
    }
}
