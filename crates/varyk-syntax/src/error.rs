//! Diagnostic codes and the syntax errors the lexer and parser produce.
//!
//! This crate defines exactly the codes used by its own passes plus the
//! ones reserved for the parser: `V0001` through `V0003`
//! and `V0010` through `V0012`. A code, once assigned, is never reused for
//! a different meaning (spec section 3).

use crate::span::Span;

/// Unsupported construct, naming it.
pub const V0001: &str = "V0001";
/// Unexpected token or malformed syntax.
pub const V0002: &str = "V0002";
/// Bad string escape, or an unterminated string.
pub const V0003: &str = "V0003";
/// `&x` or `&mut x` written at a call site.
pub const V0010: &str = "V0010";
/// `&T` or `&mut T` written in a parameter type.
pub const V0011: &str = "V0011";
/// Lifetime syntax, such as `<'a>` or `&'a T`.
pub const V0012: &str = "V0012";

/// A suggested edit attached to a [`SyntaxError`]: replace the text at
/// `span` with `replacement`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixIt {
    pub span: Span,
    pub replacement: String,
}

/// A lexical or syntactic error, carrying a span, a message, and one of
/// the diagnostic codes defined above.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub span: Span,
    pub message: String,
    pub code: &'static str,
    pub fix_it: Option<FixIt>,
}

impl SyntaxError {
    pub fn new(span: Span, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
            code,
            fix_it: None,
        }
    }

    pub fn with_fix_it(mut self, fix_it: FixIt) -> Self {
        self.fix_it = Some(fix_it);
        self
    }
}
