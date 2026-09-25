//! Token kinds produced by the lexer.

/// The kind of a lexical token. Keywords that are part of the Varyk
/// surface (spec 4.1) each get their own variant; every other Rust keyword
/// and reserved word, including the 2024-edition ones, lexes as
/// [`TokenKind::ReservedKeyword`] carrying its text, so the parser can report
/// it by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    // Keywords (spec 4.1). `true` and `false` are bool literals, not
    // keyword tokens, so they are not listed here.
    Fn,
    Pub,
    Let,
    Mut,
    Struct,
    Mod,
    If,
    Else,
    While,
    Break,
    Continue,
    Return,
    Enum,
    Impl,
    Match,
    For,
    In,
    /// `self`, spec 2.1. `Self` is unrelated and still lexes as
    /// [`TokenKind::ReservedKeyword`]: milestone 2 has no way to write it.
    SelfKw,

    /// Any other Rust keyword or reserved word, carrying its text.
    ReservedKeyword(String),

    Identifier(String),

    /// Raw source text of an integer literal, digits only.
    IntegerLiteral(String),
    /// Raw source text of a float literal, digits `.` digits.
    FloatLiteral(String),
    /// Raw source text between the quotes of a string literal, unmodified.
    StringLiteral(String),
    BoolLiteral(bool),

    /// A `'ident` lifetime, carrying the identifier without the quote.
    Lifetime(String),

    // Operators and punctuation (spec 4.1).
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Eq,
    EqEq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    Bang,
    AmpAmp,
    PipePipe,
    Amp,
    ColonColon,
    Colon,
    Semi,
    Comma,
    Dot,
    DotDot,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Arrow,
    FatArrow,
    Question,
}

/// A single lexical token: its kind plus the span it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: crate::span::Span,
}
