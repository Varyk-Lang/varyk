//! The lexer: a hand-written scanner over the source text's bytes, using
//! the longest-match rule for multi-character operators.

use crate::error::{SyntaxError, V0001, V0003};
use crate::source::SourceFile;
use crate::span::Span;
use crate::token::{Token, TokenKind};

/// Varyk's own keywords (spec 4.1). `true` and `false` are bool literals,
/// handled separately.
fn keyword_kind(word: &str) -> Option<TokenKind> {
    Some(match word {
        "fn" => TokenKind::Fn,
        "pub" => TokenKind::Pub,
        "let" => TokenKind::Let,
        "mut" => TokenKind::Mut,
        "struct" => TokenKind::Struct,
        "mod" => TokenKind::Mod,
        "if" => TokenKind::If,
        "else" => TokenKind::Else,
        "while" => TokenKind::While,
        "break" => TokenKind::Break,
        "continue" => TokenKind::Continue,
        "return" => TokenKind::Return,
        "true" => TokenKind::BoolLiteral(true),
        "false" => TokenKind::BoolLiteral(false),
        _ => return None,
    })
}

/// Every other Rust keyword and reserved word, including the 2024-edition
/// ones.
const RESERVED_KEYWORDS: &[&str] = &[
    "as", "async", "await", "const", "crate", "dyn", "enum", "extern", "for", "impl", "in", "loop",
    "match", "move", "ref", "self", "Self", "static", "super", "trait", "type", "unsafe", "use",
    "where", "abstract", "become", "box", "do", "final", "gen", "macro", "override", "priv", "try",
    "typeof", "unsized", "virtual", "yield",
];

/// Lexes `file` into tokens and any syntax errors found along the way.
/// Lexing never stops at the first error: an unknown character or a bad
/// string is recorded and scanning continues after it.
pub fn lex(file: &SourceFile) -> (Vec<Token>, Vec<SyntaxError>) {
    Lexer::new(file).run()
}

struct Lexer<'a> {
    file_id: crate::span::FileId,
    text: &'a str,
    chars: std::iter::Peekable<std::str::CharIndices<'a>>,
    tokens: Vec<Token>,
    errors: Vec<SyntaxError>,
}

impl<'a> Lexer<'a> {
    fn new(file: &'a SourceFile) -> Self {
        Self {
            file_id: file.id,
            text: &file.text,
            chars: file.text.char_indices().peekable(),
            tokens: Vec::new(),
            errors: Vec::new(),
        }
    }

    fn run(mut self) -> (Vec<Token>, Vec<SyntaxError>) {
        while let Some(&(start, c)) = self.chars.peek() {
            match c {
                ' ' | '\t' | '\r' | '\n' => {
                    self.chars.next();
                }
                '/' => {
                    self.chars.next();
                    if self.eat_if('/') {
                        self.skip_line_comment();
                    } else {
                        self.push(TokenKind::Slash, start, start + 1);
                    }
                }
                '"' => {
                    self.chars.next();
                    self.scan_string(start);
                }
                '\'' => {
                    self.chars.next();
                    self.scan_lifetime_or_unknown(start);
                }
                c if c.is_ascii_digit() => self.scan_number(start),
                c if c.is_ascii_alphabetic() || c == '_' => self.scan_identifier(start, c),
                '+' => self.single(TokenKind::Plus, start),
                '-' => {
                    self.chars.next();
                    if self.eat_if('>') {
                        self.push(TokenKind::Arrow, start, start + 2);
                    } else {
                        self.push(TokenKind::Minus, start, start + 1);
                    }
                }
                '*' => self.single(TokenKind::Star, start),
                '%' => self.single(TokenKind::Percent, start),
                '=' => {
                    self.chars.next();
                    if self.eat_if('=') {
                        self.push(TokenKind::EqEq, start, start + 2);
                    } else {
                        self.push(TokenKind::Eq, start, start + 1);
                    }
                }
                '!' => {
                    self.chars.next();
                    if self.eat_if('=') {
                        self.push(TokenKind::NotEq, start, start + 2);
                    } else {
                        self.push(TokenKind::Bang, start, start + 1);
                    }
                }
                '<' => {
                    self.chars.next();
                    if self.eat_if('=') {
                        self.push(TokenKind::LtEq, start, start + 2);
                    } else {
                        self.push(TokenKind::Lt, start, start + 1);
                    }
                }
                '>' => {
                    self.chars.next();
                    if self.eat_if('=') {
                        self.push(TokenKind::GtEq, start, start + 2);
                    } else {
                        self.push(TokenKind::Gt, start, start + 1);
                    }
                }
                '&' => {
                    self.chars.next();
                    if self.eat_if('&') {
                        self.push(TokenKind::AmpAmp, start, start + 2);
                    } else {
                        self.push(TokenKind::Amp, start, start + 1);
                    }
                }
                '|' => {
                    self.chars.next();
                    if self.eat_if('|') {
                        self.push(TokenKind::PipePipe, start, start + 2);
                    } else {
                        self.unknown_char(start, c);
                    }
                }
                ':' => {
                    self.chars.next();
                    if self.eat_if(':') {
                        self.push(TokenKind::ColonColon, start, start + 2);
                    } else {
                        self.push(TokenKind::Colon, start, start + 1);
                    }
                }
                ';' => self.single(TokenKind::Semi, start),
                ',' => self.single(TokenKind::Comma, start),
                '.' => self.single(TokenKind::Dot, start),
                '(' => self.single(TokenKind::LParen, start),
                ')' => self.single(TokenKind::RParen, start),
                '{' => self.single(TokenKind::LBrace, start),
                '}' => self.single(TokenKind::RBrace, start),
                other => {
                    self.chars.next();
                    self.unknown_char(start, other);
                }
            }
        }
        (self.tokens, self.errors)
    }

    /// Consumes the current (already-peeked) single-byte character and
    /// emits `kind` spanning it.
    fn single(&mut self, kind: TokenKind, start: usize) {
        self.chars.next();
        self.push(kind, start, start + 1);
    }

    /// Consumes the next character if it equals `expected`, without
    /// consuming anything otherwise.
    fn eat_if(&mut self, expected: char) -> bool {
        if self.chars.peek().is_some_and(|&(_, c)| c == expected) {
            self.chars.next();
            return true;
        }
        false
    }

    fn push(&mut self, kind: TokenKind, start: usize, end: usize) {
        self.tokens.push(Token {
            kind,
            span: Span::new(self.file_id, start as u32, end as u32),
        });
    }

    fn skip_line_comment(&mut self) {
        while let Some(&(_, c)) = self.chars.peek() {
            if c == '\n' {
                break;
            }
            self.chars.next();
        }
    }

    /// `start` is the byte offset of `first`, the already-peeked but not
    /// yet consumed identifier-start character.
    fn scan_identifier(&mut self, start: usize, first: char) {
        self.chars.next();
        let mut end = start + first.len_utf8();
        while let Some(&(idx, c)) = self.chars.peek() {
            if c.is_ascii_alphanumeric() || c == '_' {
                self.chars.next();
                end = idx + c.len_utf8();
            } else {
                break;
            }
        }
        let word = &self.text[start..end];
        let kind = keyword_kind(word).unwrap_or_else(|| {
            if RESERVED_KEYWORDS.contains(&word) {
                TokenKind::ReservedKeyword(word.to_string())
            } else {
                TokenKind::Identifier(word.to_string())
            }
        });
        self.push(kind, start, end);
    }

    fn scan_number(&mut self, start: usize) {
        // The first character is already known to be an ASCII digit (the
        // caller only reaches here on one). `_` is accepted as a digit
        // separator anywhere after it, validated only in the sense that it
        // must follow a digit; the raw text, underscores included, is kept
        // unmodified, as with every other literal.
        let mut end = start;
        while let Some(&(idx, c)) = self.chars.peek() {
            if c.is_ascii_digit() || c == '_' {
                self.chars.next();
                end = idx + 1;
            } else {
                break;
            }
        }

        let mut is_float = false;
        if let Some(&(dot_idx, '.')) = self.chars.peek() {
            let mut lookahead = self.chars.clone();
            lookahead.next();
            if matches!(lookahead.next(), Some((_, next_c)) if next_c.is_ascii_digit()) {
                is_float = true;
                self.chars.next();
                end = dot_idx + 1;
                while let Some(&(idx2, c2)) = self.chars.peek() {
                    if c2.is_ascii_digit() || c2 == '_' {
                        self.chars.next();
                        end = idx2 + 1;
                    } else {
                        break;
                    }
                }
            }
        }

        let text = self.text[start..end].to_string();
        let kind = if is_float {
            TokenKind::FloatLiteral(text)
        } else {
            TokenKind::IntegerLiteral(text)
        };
        self.push(kind, start, end);
    }

    /// `start` is the byte offset of the opening quote, already consumed.
    fn scan_string(&mut self, start: usize) {
        let content_start = start + 1;
        let mut closed = false;
        let mut content_end = content_start;

        while let Some(&(idx, c)) = self.chars.peek() {
            match c {
                '"' => {
                    self.chars.next();
                    content_end = idx;
                    closed = true;
                    break;
                }
                '\\' => {
                    self.chars.next();
                    match self.chars.peek().copied() {
                        Some((_, 'n' | 't' | '\\' | '"' | '0')) => {
                            self.chars.next();
                        }
                        Some((esc_idx, esc_ch)) => {
                            self.chars.next();
                            let span = Span::new(
                                self.file_id,
                                idx as u32,
                                (esc_idx + esc_ch.len_utf8()) as u32,
                            );
                            self.errors.push(SyntaxError::new(
                                span,
                                V0003,
                                format!(
                                    "`\\{esc_ch}` is not a known escape; use `\\n`, `\\t`, `\\\\`, `\\\"`, or `\\0`"
                                ),
                            ));
                        }
                        None => {
                            // A trailing backslash right at end of file:
                            // the string is also unterminated, and the
                            // "no closing quote" (V0003) error below
                            // already covers this exact problem, so no
                            // separate escape error is reported here.
                        }
                    }
                }
                '\r' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' => {
                    // rustc rejects these inside a string literal (a bare
                    // carriage return, and text-direction controls under
                    // its `text_direction_codepoint_in_literal` lint), and
                    // the text is emitted as it is.
                    self.chars.next();
                    let bare_cr = c == '\r' && !matches!(self.chars.peek(), Some((_, '\n')));
                    if bare_cr || c != '\r' {
                        let what = if c == '\r' {
                            "a carriage return"
                        } else {
                            "a text-direction control character"
                        };
                        let span = Span::new(self.file_id, idx as u32, (idx + c.len_utf8()) as u32);
                        self.errors.push(SyntaxError::new(
                            span,
                            V0003,
                            format!("{what} is not allowed inside a string"),
                        ));
                    }
                }
                _ => {
                    self.chars.next();
                }
            }
        }

        if closed {
            let text = self.text[content_start..content_end].to_string();
            self.push(TokenKind::StringLiteral(text), start, content_end + 1);
        } else {
            let span = Span::new(self.file_id, start as u32, (start + 1) as u32);
            self.errors.push(SyntaxError::new(
                span,
                V0003,
                "this string has no closing `\"`",
            ));
            // No token: the string never closed, so there is nothing
            // meaningful to hand the parser.
        }
    }

    /// `start` is the byte offset of the opening `'`, already consumed.
    fn scan_lifetime_or_unknown(&mut self, start: usize) {
        if let Some(&(ident_start, _)) = self
            .chars
            .peek()
            .filter(|&&(_, c)| c.is_ascii_alphabetic() || c == '_')
        {
            let mut end = ident_start;
            while let Some(&(idx, c2)) = self.chars.peek() {
                if c2.is_ascii_alphanumeric() || c2 == '_' {
                    self.chars.next();
                    end = idx + c2.len_utf8();
                } else {
                    break;
                }
            }
            let ident = self.text[ident_start..end].to_string();
            self.push(TokenKind::Lifetime(ident), start, end);
            return;
        }
        self.unknown_char(start, '\'');
    }

    fn unknown_char(&mut self, start: usize, c: char) {
        let span = Span::new(self.file_id, start as u32, (start + c.len_utf8()) as u32);
        let message = match c {
            '#' => "attributes (`#`) are not supported in Varyk yet".to_string(),
            '[' | ']' => format!("arrays and indexing (`{c}`) are not supported in Varyk yet"),
            '|' => "closures (`|`) are not supported in Varyk yet".to_string(),
            '?' => "the `?` operator is not supported in Varyk yet".to_string(),
            c if !c.is_ascii() && c.is_alphanumeric() => {
                format!("non-ASCII names are not supported in Varyk yet: `{c}`")
            }
            _ => format!("unknown character `{c}`"),
        };
        self.errors.push(SyntaxError::new(span, V0001, message));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::FileId;

    fn file(text: &str) -> SourceFile {
        SourceFile::new(FileId(0), "test.vr", text)
    }

    fn lex_kinds(text: &str) -> Vec<TokenKind> {
        let f = file(text);
        let (tokens, errors) = lex(&f);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        tokens.into_iter().map(|t| t.kind).collect()
    }

    // --- Keywords and reserved words -------------------------------------

    #[test]
    fn each_varyk_keyword_lexes_to_its_kind() {
        let cases: &[(&str, TokenKind)] = &[
            ("fn", TokenKind::Fn),
            ("pub", TokenKind::Pub),
            ("let", TokenKind::Let),
            ("mut", TokenKind::Mut),
            ("struct", TokenKind::Struct),
            ("mod", TokenKind::Mod),
            ("if", TokenKind::If),
            ("else", TokenKind::Else),
            ("while", TokenKind::While),
            ("break", TokenKind::Break),
            ("continue", TokenKind::Continue),
            ("return", TokenKind::Return),
            ("true", TokenKind::BoolLiteral(true)),
            ("false", TokenKind::BoolLiteral(false)),
        ];
        for (word, expected) in cases {
            let kinds = lex_kinds(word);
            assert_eq!(kinds, vec![expected.clone()], "lexing {word:?}");
        }
    }

    #[test]
    fn every_other_reserved_word_lexes_as_reserved_keyword() {
        for word in RESERVED_KEYWORDS {
            let kinds = lex_kinds(word);
            assert_eq!(
                kinds,
                vec![TokenKind::ReservedKeyword((*word).to_string())],
                "lexing {word:?}"
            );
        }
    }

    #[test]
    fn plain_identifier_is_not_a_keyword() {
        assert_eq!(
            lex_kinds("hello_world"),
            vec![TokenKind::Identifier("hello_world".to_string())]
        );
    }

    // --- Spans ------------------------------------------------------------

    #[test]
    fn spans_on_the_second_line_are_byte_offsets() {
        let f = file("let x;\nlet y;\n");
        let (tokens, errors) = lex(&f);
        assert!(errors.is_empty());
        // Find the identifier `y` on the second line.
        let y = tokens
            .iter()
            .find(|t| matches!(&t.kind, TokenKind::Identifier(name) if name == "y"))
            .expect("token `y` not found");
        assert_eq!(y.span, Span::new(f.id, 11, 12));
    }

    // --- String literals and escapes ---------------------------------------

    #[test]
    fn string_literal_preserves_raw_text_of_valid_escapes() {
        let f = file(r#""a\nb\tc\\d\"e\0f""#);
        let (tokens, errors) = lex(&f);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(tokens.len(), 1);
        match &tokens[0].kind {
            TokenKind::StringLiteral(text) => {
                assert_eq!(text, r#"a\nb\tc\\d\"e\0f"#);
            }
            other => panic!("expected StringLiteral, got {other:?}"),
        }
    }

    #[test]
    fn bare_carriage_return_in_a_string_is_v0003() {
        let f = file("\"a\rb\"");
        let (_tokens, errors) = lex(&f);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, V0003);
        assert_eq!(errors[0].span.start, 2);
        // A CRLF inside a string is fine: rustc normalizes it like Varyk does.
        let f = file("\"a\r\nb\"");
        let (_tokens, errors) = lex(&f);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn text_direction_control_in_a_string_is_v0003() {
        let f = file("\"ab\u{202E}cd\"");
        let (_tokens, errors) = lex(&f);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, V0003);
        assert_eq!(errors[0].span.start, 3);
    }

    #[test]
    fn unknown_escape_yields_v0003_at_the_escape() {
        let f = file(r#""bad \q escape""#);
        let (_tokens, errors) = lex(&f);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, V0003);
        // The backslash of `\q` is at byte offset 5.
        assert_eq!(errors[0].span.start, 5);
    }

    #[test]
    fn unterminated_string_yields_v0003_at_the_opening_quote() {
        let f = file("\"unterminated");
        let (_tokens, errors) = lex(&f);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, V0003);
        assert_eq!(errors[0].span.start, 0);
        assert_eq!(errors[0].span.end, 1);
    }

    #[test]
    fn trailing_backslash_before_eof_yields_a_single_v0003() {
        // The dangling backslash and the missing closing quote are the
        // same underlying problem: the string never closed. That must be
        // reported once, not as a bad-escape error plus an unterminated
        // error.
        let f = file("\"abc\\");
        let (_tokens, errors) = lex(&f);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, V0003);
        assert_eq!(errors[0].span.start, 0);
        assert_eq!(errors[0].span.end, 1);
    }

    #[test]
    fn lexing_continues_after_an_unterminated_string() {
        // An unterminated string has no closing quote to stop at, so it
        // necessarily consumes the rest of the file; what "continues"
        // means here is that the tokens scanned before it are kept and
        // the pass finishes cleanly rather than aborting.
        let f = file("fn \"unterminated");
        let (tokens, errors) = lex(&f);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, V0003);
        assert_eq!(
            tokens,
            vec![Token {
                kind: TokenKind::Fn,
                span: Span::new(f.id, 0, 2),
            }]
        );
    }

    #[test]
    fn lexing_continues_after_a_bad_escape() {
        // Unlike an unterminated string, a bad escape does not stop
        // scanning the string: the rest of the file still lexes normally.
        let f = file(r#""bad \q escape" fn"#);
        let (tokens, errors) = lex(&f);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, V0003);
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Fn));
    }

    // --- Operators and punctuation ------------------------------------------

    #[test]
    fn amp_and_amp_amp() {
        assert_eq!(lex_kinds("&"), vec![TokenKind::Amp]);
        assert_eq!(lex_kinds("&&"), vec![TokenKind::AmpAmp]);
    }

    #[test]
    fn amp_mut_is_two_tokens() {
        assert_eq!(
            lex_kinds("&mut"),
            vec![TokenKind::Amp, TokenKind::Mut],
            "&mut is not one token; & then mut"
        );
    }

    #[test]
    fn lifetime_token() {
        assert_eq!(lex_kinds("'a"), vec![TokenKind::Lifetime("a".to_string())]);
    }

    #[test]
    fn line_comments_are_skipped() {
        assert_eq!(lex_kinds("// a comment\nfn"), vec![TokenKind::Fn]);
        assert_eq!(lex_kinds("fn // trailing comment"), vec![TokenKind::Fn]);
    }

    #[test]
    fn remaining_operators_and_punctuation() {
        assert_eq!(
            lex_kinds("+ - * / % = == != < <= > >= ! || :: : ; , . ( ) { } ->"),
            vec![
                TokenKind::Plus,
                TokenKind::Minus,
                TokenKind::Star,
                TokenKind::Slash,
                TokenKind::Percent,
                TokenKind::Eq,
                TokenKind::EqEq,
                TokenKind::NotEq,
                TokenKind::Lt,
                TokenKind::LtEq,
                TokenKind::Gt,
                TokenKind::GtEq,
                TokenKind::Bang,
                TokenKind::PipePipe,
                TokenKind::ColonColon,
                TokenKind::Colon,
                TokenKind::Semi,
                TokenKind::Comma,
                TokenKind::Dot,
                TokenKind::LParen,
                TokenKind::RParen,
                TokenKind::LBrace,
                TokenKind::RBrace,
                TokenKind::Arrow,
            ]
        );
    }

    // --- Number literals ------------------------------------------------------

    #[test]
    fn integer_and_float_literals() {
        assert_eq!(
            lex_kinds("42"),
            vec![TokenKind::IntegerLiteral("42".to_string())]
        );
        assert_eq!(
            lex_kinds("3.14"),
            vec![TokenKind::FloatLiteral("3.14".to_string())]
        );
    }

    #[test]
    fn numeric_literal_underscore_separators() {
        assert_eq!(
            lex_kinds("1_000"),
            vec![TokenKind::IntegerLiteral("1_000".to_string())],
        );
        assert_eq!(
            lex_kinds("1_000.5"),
            vec![TokenKind::FloatLiteral("1_000.5".to_string())],
        );
    }

    // --- Unknown characters --------------------------------------------------

    #[test]
    fn unknown_character_yields_v0001_naming_it_and_continues() {
        let f = file("a @ b");
        let (tokens, errors) = lex(&f);
        assert_eq!(
            tokens.iter().map(|t| &t.kind).collect::<Vec<_>>(),
            vec![
                &TokenKind::Identifier("a".to_string()),
                &TokenKind::Identifier("b".to_string()),
            ]
        );
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, V0001);
        assert_eq!(errors[0].message, "unknown character `@`");
    }

    #[test]
    fn non_ascii_name_character_yields_v0001_naming_it_and_continues() {
        let f = file("café = été");
        let (tokens, errors) = lex(&f);
        assert_eq!(
            tokens.iter().map(|t| &t.kind).collect::<Vec<_>>(),
            vec![
                &TokenKind::Identifier("caf".to_string()),
                &TokenKind::Eq,
                &TokenKind::Identifier("t".to_string()),
            ]
        );
        assert_eq!(errors.len(), 3);
        assert!(errors.iter().all(|e| e.code == V0001));
        assert_eq!(
            errors[0].message,
            "non-ASCII names are not supported in Varyk yet: `é`"
        );
    }

    #[test]
    fn non_ascii_text_in_a_string_literal_is_kept() {
        assert_eq!(
            lex_kinds("\"café →\""),
            vec![TokenKind::StringLiteral("café →".to_string())]
        );
    }

    #[test]
    fn lone_pipe_names_closures() {
        let f = file("|");
        let (tokens, errors) = lex(&f);
        assert!(tokens.is_empty());
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, V0001);
        assert!(errors[0].message.contains("closures"));
    }

    #[test]
    fn hash_names_attributes() {
        let f = file("#");
        let (_tokens, errors) = lex(&f);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, V0001);
        assert_eq!(
            errors[0].message,
            "attributes (`#`) are not supported in Varyk yet"
        );
    }

    #[test]
    fn bracket_names_arrays_and_indexing() {
        let f = file("[");
        let (_tokens, errors) = lex(&f);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, V0001);
        assert_eq!(
            errors[0].message,
            "arrays and indexing (`[`) are not supported in Varyk yet"
        );
    }
}
