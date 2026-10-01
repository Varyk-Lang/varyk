//! The parser: a hand-written recursive-descent / Pratt parser over a
//! token slice. This module holds the token-cursor primitives shared by
//! every parsing function; `expr.rs` holds expression parsing, `item.rs`
//! item parsing (`fn`, `struct`, `mod`) and the top-level [`Parser::parse_program`],
//! `stmt.rs` statement parsing, `pattern.rs` pattern parsing, and `ty.rs`
//! type parsing.

mod expr;
mod item;
mod pattern;
mod stmt;
mod ty;

use crate::ast::{PathStart, Program};
use crate::error::{FixIt, SyntaxError, V0001, V0002};
use crate::span::{FileId, Span};
use crate::token::{Token, TokenKind};

/// The message for a `#` where no attribute can go (M5a spec 2.2).
pub(super) const STRAY_HASH: &str = "an attribute (`#[..]`) can only go before a struct field, an enum variant, or a \
     top-level function";

/// Parses a whole token stream for a single file into a [`Program`] plus
/// every syntax error found. On the first `V0002` (or an unrecoverable
/// `V0001`, such as a reserved keyword where an item is expected), parsing
/// stops and the items parsed so far are returned alongside the errors.
pub fn parse(tokens: &[Token], file: FileId) -> (Program, Vec<SyntaxError>) {
    let mut parser = Parser::new(tokens, file);
    let program = parser.parse_program();
    (program, parser.errors().to_vec())
}

/// Parses a token slice for a single file. No error recovery beyond what
/// each parsing function documents: `V0002` always stops parsing outright,
/// since without a reliable recovery point there is nothing safe to guess.
/// Most other diagnostic codes are recorded and parsing continues (for
/// example the `&` / `&mut` sigil's `V0010`), but a few `V0001`s have no
/// operand left to fall back to — a reserved keyword or an unsupported
/// macro name where an expression is expected — and stop parsing too, per
/// the call sites that produce them.
pub(crate) struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    file: FileId,
    errors: Vec<SyntaxError>,
    /// Set while parsing an `if` or `while` condition, so
    /// that a `{` right after a path is treated as the start of the
    /// block, not a struct literal `Name { ... }` — the same ambiguity
    /// Rust resolves the same way.
    in_condition: bool,
    /// Set right before parsing a `for` loop's head starts, and consumed
    /// by the very next `(` reached as a primary expression (`parse_primary`),
    /// if any: it never survives past that first primary. Lets a range
    /// found directly inside that group (`for i in (0..3)`) get a message
    /// naming the parentheses themselves, rather than the generic "only in
    /// the head of a `for` loop" one, since this group IS the head.
    for_head_leading_paren: bool,
}

impl<'a> Parser<'a> {
    pub(crate) fn new(tokens: &'a [Token], file: FileId) -> Self {
        Self {
            tokens,
            pos: 0,
            file,
            errors: Vec::new(),
            in_condition: false,
            for_head_leading_paren: false,
        }
    }

    /// Every syntax error recorded so far, in the order they were found.
    pub(crate) fn errors(&self) -> &[SyntaxError] {
        &self.errors
    }

    // --- Cursor -----------------------------------------------------------

    fn peek(&self) -> Option<&TokenKind> {
        self.tokens.get(self.pos).map(|t| &t.kind)
    }

    /// The kind of the token `offset` positions past the current one,
    /// without consuming anything. `peek_at(0)` is [`Parser::peek`].
    fn peek_at(&self, offset: usize) -> Option<&TokenKind> {
        self.tokens.get(self.pos + offset).map(|t| &t.kind)
    }

    fn peek_token(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    /// Consumes and returns the current token, or `None` at the end of the
    /// stream.
    fn bump(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.pos).cloned();
        if token.is_some() {
            self.pos += 1;
        }
        token
    }

    /// The span an error should carry when there is no current token to
    /// point at: a zero-width span right after the last token (or at byte
    /// 0 of an empty file).
    fn eof_span(&self) -> Span {
        let end = self.tokens.last().map(|t| t.span.end).unwrap_or(0);
        Span::new(self.file, end, end)
    }

    /// The span of the current token, or [`Parser::eof_span`] if the
    /// stream is exhausted.
    fn current_span(&self) -> Span {
        self.peek_token()
            .map(|t| t.span)
            .unwrap_or_else(|| self.eof_span())
    }

    /// A span running from the start of `from` to the end of `to`, in this
    /// parser's file.
    fn span_from(&self, from: Span, to: Span) -> Span {
        Span::new(self.file, from.start, to.end)
    }

    /// Consumes a path-start keyword (`crate`, `self`, `super`) and the
    /// `::` right after it, when the current token is one of the three
    /// AND is immediately followed by `::` (spec 3.1, 3.3); a keyword used
    /// any other way (a method receiver, a bare misuse) is left alone, for
    /// the caller to handle as it already does. Consumes nothing and
    /// returns [`PathStart::None`] otherwise.
    fn take_path_start(&mut self) -> PathStart {
        let leading = match self.peek() {
            Some(TokenKind::CrateKw) => PathStart::Crate,
            Some(TokenKind::SelfKw) => PathStart::SelfMod,
            Some(TokenKind::SuperKw) => PathStart::Super,
            _ => return PathStart::None,
        };
        if self.peek_at(1) != Some(&TokenKind::ColonColon) {
            return PathStart::None;
        }
        self.bump(); // the keyword
        self.bump(); // `::`
        leading
    }

    /// Consumes the current token if its kind equals `kind`, without
    /// recording an error either way if it does not match.
    fn bump_if(&mut self, kind: &TokenKind) -> bool {
        if self.peek() == Some(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    /// Runs `f` with the `in_condition` flag temporarily cleared,
    /// restoring it afterwards regardless of whether `f` succeeds. Used
    /// wherever a nested context — parentheses, call arguments, a block's
    /// body — already disambiguates struct-literal syntax on its own, so
    /// the enclosing condition's suppression must not leak into it (a
    /// leak would make `if f(Point { x: 1 }) { }` or
    /// `if { Point { x: 1 } } { }` fail to parse, even though the
    /// parentheses/braces already remove the ambiguity `if`'s own
    /// condition has).
    fn without_condition<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let was_in_condition = self.in_condition;
        self.in_condition = false;
        let result = f(self);
        self.in_condition = was_in_condition;
        result
    }

    fn push_error(&mut self, code: &'static str, span: Span, message: impl Into<String>) {
        self.errors.push(SyntaxError::new(span, code, message));
    }

    fn push_error_with_fix_it(
        &mut self,
        code: &'static str,
        span: Span,
        message: impl Into<String>,
        fix_it: FixIt,
    ) {
        self.errors
            .push(SyntaxError::new(span, code, message).with_fix_it(fix_it));
    }

    /// Records `V0002` at the current token, "expected `what`", or
    /// [`STRAY_HASH`] when that token is a `#`.
    fn push_expected(&mut self, what: &str) {
        let span = self.current_span();
        if self.peek() == Some(&TokenKind::Hash) {
            self.push_error(V0002, span, STRAY_HASH);
        } else {
            self.push_error(V0002, span, format!("expected {what}"));
        }
    }

    /// Consumes the current token if its kind equals `expected`, recording
    /// `V0002` and returning `Err` otherwise.
    fn expect(&mut self, expected: TokenKind, what: &str) -> Result<Token, ()> {
        if self.peek() == Some(&expected) {
            Ok(self.bump().expect("peek just confirmed a token is present"))
        } else {
            self.push_expected(what);
            Err(())
        }
    }

    /// Consumes an [`crate::token::TokenKind::Identifier`], recording
    /// `V0002` and returning `Err` for anything else, or `V0001` for a
    /// reserved Rust keyword (spec 4.1) or for `self` (spec 2.5): `self` is
    /// a keyword everywhere except a method's own receiver position, which
    /// `item.rs`'s `parse_self_receiver` reads before this function ever
    /// sees it.
    fn expect_identifier(&mut self, what: &str) -> Result<crate::ast::Ident, ()> {
        // `as` is a Varyk keyword but not a Varyk name, so it reads as the
        // Rust keyword it also is.
        let keyword = match self.peek() {
            Some(TokenKind::ReservedKeyword(word)) => Some(word.as_str()),
            Some(TokenKind::As) => Some("as"),
            Some(TokenKind::Async) => Some("async"),
            Some(TokenKind::Await) => Some("await"),
            _ => None,
        };
        if let Some(word) = keyword {
            let message =
                format!("`{word}` is a Rust keyword and cannot be used as a name in Varyk");
            let span = self.current_span();
            self.bump();
            self.push_error(V0001, span, message);
            return Err(());
        }
        match self.peek() {
            Some(TokenKind::SelfKw) => {
                let span = self.current_span();
                self.bump();
                self.push_error(
                    V0001,
                    span,
                    "`self` is a keyword and can only be used as a method's first parameter, \
                     or to start a path (`self::name`)",
                );
                Err(())
            }
            Some(TokenKind::CrateKw) => {
                let span = self.current_span();
                self.bump();
                self.push_error(
                    V0001,
                    span,
                    "`crate` is a keyword and can only start a path, followed by `::`",
                );
                Err(())
            }
            Some(TokenKind::SuperKw) => {
                let span = self.current_span();
                let after_super = self.pos >= 2
                    && self.tokens[self.pos - 1].kind == TokenKind::ColonColon
                    && self.tokens[self.pos - 2].kind == TokenKind::SuperKw;
                self.bump();
                let message = if after_super {
                    "`super::super` is not supported; write the path from `crate::` instead"
                } else {
                    "`super` is a keyword and can only start a path, followed by `::`"
                };
                self.push_error(V0001, span, message);
                Err(())
            }
            Some(TokenKind::UseKw) => {
                let span = self.current_span();
                self.bump();
                self.push_error(
                    V0001,
                    span,
                    "`use` is a keyword and can only start a `use` item",
                );
                Err(())
            }
            Some(TokenKind::Identifier(_)) => {
                let token = self.bump().expect("peek just confirmed a token is present");
                let name = match token.kind {
                    TokenKind::Identifier(name) => name,
                    _ => unreachable!("matched above"),
                };
                Ok(crate::ast::Ident {
                    name,
                    span: token.span,
                })
            }
            _ => {
                self.push_expected(what);
                Err(())
            }
        }
    }

    /// Like [`Parser::expect_identifier`], but also rejects `_`: valid as a
    /// `let` binding's discard, but not as a function, struct, module,
    /// field, or parameter name (rustc itself rejects `fn _`).
    fn expect_name_identifier(&mut self, what: &str) -> Result<crate::ast::Ident, ()> {
        let ident = self.expect_identifier(what)?;
        if ident.name == "_" {
            self.push_error(V0002, ident.span, "`_` cannot be used as a name here");
            return Err(());
        }
        Ok(ident)
    }
}
