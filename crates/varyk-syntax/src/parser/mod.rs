//! The parser: a hand-written recursive-descent / Pratt parser over a
//! token slice. This module holds the token-cursor primitives shared by
//! every parsing function; `expr.rs` holds expression parsing, `item.rs`
//! item parsing (`fn`, `struct`, `mod`) and the top-level [`Parser::parse_program`],
//! `stmt.rs` statement parsing, and `ty.rs` type parsing.

mod expr;
mod item;
mod stmt;
mod ty;

use crate::ast::Program;
use crate::error::{FixIt, SyntaxError, V0001, V0002};
use crate::span::{FileId, Span};
use crate::token::{Token, TokenKind};

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
}

impl<'a> Parser<'a> {
    pub(crate) fn new(tokens: &'a [Token], file: FileId) -> Self {
        Self {
            tokens,
            pos: 0,
            file,
            errors: Vec::new(),
            in_condition: false,
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

    /// Consumes the current token if its kind equals `expected`, recording
    /// `V0002` and returning `Err` otherwise.
    fn expect(&mut self, expected: TokenKind, what: &str) -> Result<Token, ()> {
        if self.peek() == Some(&expected) {
            Ok(self.bump().expect("peek just confirmed a token is present"))
        } else {
            let span = self.current_span();
            self.push_error(V0002, span, format!("expected {what}"));
            Err(())
        }
    }

    /// Consumes an [`crate::token::TokenKind::Identifier`], recording
    /// `V0002` and returning `Err` for anything else, or `V0001` for a
    /// reserved Rust keyword (spec 4.1).
    fn expect_identifier(&mut self, what: &str) -> Result<crate::ast::Ident, ()> {
        match self.peek() {
            Some(TokenKind::ReservedKeyword(word)) => {
                let message =
                    format!("`{word}` is a Rust keyword and cannot be used as a name in Varyk");
                let span = self.current_span();
                self.bump();
                self.push_error(V0001, span, message);
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
                let span = self.current_span();
                self.push_error(V0002, span, format!("expected {what}"));
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
