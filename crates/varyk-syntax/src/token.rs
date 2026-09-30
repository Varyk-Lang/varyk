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
    /// `self`, spec 2.1, also a path prefix (spec 3.1, 3.3). `Self` is
    /// unrelated and still lexes as [`TokenKind::ReservedKeyword`]: Varyk
    /// has no way to write it.
    SelfKw,
    /// `crate`, a path prefix naming the crate root (spec 3.1, 3.3).
    CrateKw,
    /// `super`, a path prefix naming the parent module (spec 3.1, 3.3).
    SuperKw,
    /// `use`, spec 3.3.
    UseKw,
    /// `as`: a cast, `expr as T` (M4 spec 2.9), and the alias in
    /// `use path as name;` (spec 3.3).
    As,

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
    /// A lone `|`, around a closure's parameters (M4 spec 2.2).
    Pipe,
    Amp,
    ColonColon,
    Colon,
    Semi,
    Comma,
    Dot,
    DotDot,
    /// `..=`, an inclusive range (M4 spec 2.5, 2.11).
    DotDotEq,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Arrow,
    FatArrow,
    Question,
    /// `#`, which starts an attribute, `#[name]` or `#[name(literal)]`
    /// (M5a spec 2.1, 2.2).
    Hash,
}

/// A single lexical token: its kind plus the span it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: crate::span::Span,
}
