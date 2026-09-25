//! Expression parsing: a Pratt parser using Rust's binary-operator
//! precedence, plus the call/field/path postfix chain, struct literals,
//! blocks, `if`, and the `println!` intrinsic.

use super::Parser;
use crate::ast::{
    BinaryOp, Block, Expr, ExprKind, Ident, MatchArm, Pattern, SubPattern, UnaryOp, VariantPattern,
};
use crate::error::{V0001, V0002};
use crate::span::Span;
use crate::token::TokenKind;

/// A binary operator's (kind, left binding power, right binding power).
/// Higher binds tighter. Non-associative operators (the comparisons) use
/// `left == right - 1` like every other left-associative operator here;
/// what makes them non-associative is [`Parser::parse_binary`] refusing to
/// loop back into another operator at the same precedence level.
fn binary_op(kind: &TokenKind) -> Option<(BinaryOp, u8, u8)> {
    Some(match kind {
        TokenKind::PipePipe => (BinaryOp::Or, 1, 2),
        TokenKind::AmpAmp => (BinaryOp::And, 3, 4),
        TokenKind::EqEq => (BinaryOp::Eq, 5, 6),
        TokenKind::NotEq => (BinaryOp::Ne, 5, 6),
        TokenKind::Lt => (BinaryOp::Lt, 5, 6),
        TokenKind::LtEq => (BinaryOp::Le, 5, 6),
        TokenKind::Gt => (BinaryOp::Gt, 5, 6),
        TokenKind::GtEq => (BinaryOp::Ge, 5, 6),
        TokenKind::Plus => (BinaryOp::Add, 7, 8),
        TokenKind::Minus => (BinaryOp::Sub, 7, 8),
        TokenKind::Star => (BinaryOp::Mul, 9, 10),
        TokenKind::Slash => (BinaryOp::Div, 9, 10),
        TokenKind::Percent => (BinaryOp::Rem, 9, 10),
        _ => return None,
    })
}

/// The binding power comparisons sit at (5), used to detect a second
/// comparison chained onto the first, which `a < b < c` is and Rust (and
/// Varyk) does not allow.
const COMPARISON_BP: u8 = 5;

impl<'a> Parser<'a> {
    /// Parses one expression, stopping at the first `V0002`. The error
    /// carries no data: every diagnostic, with its span and message, is
    /// already recorded in [`Parser::errors`]. A `..`/`..=` immediately
    /// following the expression is always `V0001` here: a range exists
    /// only in the head of a `for` loop
    /// (`parser::stmt::Parser::parse_for_head`, which calls
    /// [`Parser::parse_binary`] directly to bypass this check).
    pub(crate) fn parse_expr(&mut self) -> Result<Expr, ()> {
        let lhs = self.parse_binary(0)?;
        self.reject_stray_range(lhs)
    }

    fn reject_stray_range(&mut self, lhs: Expr) -> Result<Expr, ()> {
        self.reject_stray_range_in(lhs, false)
    }

    /// `reject_stray_range`, except when `in_for_head_parens` is set: the
    /// caller (`parse_primary`'s `(` case) already knows this range sits
    /// directly inside a `for` head's own parentheses, so the message
    /// names them instead of claiming the range is not in the head at all.
    fn reject_stray_range_in(&mut self, lhs: Expr, in_for_head_parens: bool) -> Result<Expr, ()> {
        if self.peek() != Some(&TokenKind::DotDot) {
            return Ok(lhs);
        }
        let dotdot_span = self.current_span();
        self.bump();
        if self.peek() == Some(&TokenKind::Eq) {
            let eq_span = self.current_span();
            self.bump();
            let span = self.span_from(dotdot_span, eq_span);
            self.push_error(
                V0001,
                span,
                "`..=` is not supported in Varyk yet; milestone 4 adds inclusive ranges",
            );
            return Err(());
        }
        let span = self.span_from(lhs.span, dotdot_span);
        let message = if in_for_head_parens {
            "a range exists only in the head of a `for` loop; drop the parentheses around it"
        } else {
            "a range exists only in the head of a `for` loop"
        };
        self.push_error(V0001, span, message);
        Err(())
    }

    /// Pratt-parses a binary expression: everything at binding power at
    /// least `min_bp`. Comparisons are non-associative: once one has been
    /// parsed at this level, seeing another comparison operator right
    /// after is `V0002`, not left-associative chaining. Called directly
    /// (bypassing [`Parser::parse_expr`]'s stray-range check) by
    /// `parser::stmt::Parser::parse_for_head` to parse a range's
    /// endpoints.
    pub(super) fn parse_binary(&mut self, min_bp: u8) -> Result<Expr, ()> {
        let mut lhs = self.parse_unary()?;
        let mut last_was_comparison = false;
        while let Some((op, l_bp, r_bp)) = self.peek().and_then(binary_op) {
            if l_bp < min_bp {
                break;
            }
            if last_was_comparison && l_bp == COMPARISON_BP {
                let span = self.current_span();
                self.push_error(
                    V0002,
                    span,
                    "comparisons cannot be chained; add parentheses",
                );
                return Err(());
            }
            if self.compound_assignment(op) {
                return Err(());
            }
            self.bump();
            let rhs = self.parse_binary(r_bp)?;
            let span = self.span_from(lhs.span, rhs.span);
            lhs = Expr {
                kind: ExprKind::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            };
            last_was_comparison = l_bp == COMPARISON_BP;
        }
        Ok(lhs)
    }

    /// At an arithmetic operator immediately followed by `=` (`x += 1`),
    /// records V0001 for the unsupported compound assignment and returns
    /// `true` so the caller stops as it would on V0002.
    fn compound_assignment(&mut self, op: BinaryOp) -> bool {
        let symbol = match op {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Rem => "%",
            _ => return false,
        };
        let op_span = self.current_span();
        let Some(eq) = self.tokens.get(self.pos + 1) else {
            return false;
        };
        if eq.kind != TokenKind::Eq || eq.span.start != op_span.end {
            return false;
        }
        let span = self.span_from(op_span, eq.span);
        self.push_error(
            V0001,
            span,
            format!("compound assignment `{symbol}=` is not supported; write `x = x {symbol} 1`"),
        );
        true
    }

    /// Unary `-` and `!`, which bind tighter than any binary operator, and
    /// the `&` / `&mut` Rust-habit sigil, which is not an operator at all:
    /// it is reported and the operand is parsed and returned as if it were
    /// absent (spec 6.3, 6.6).
    fn parse_unary(&mut self) -> Result<Expr, ()> {
        match self.peek() {
            Some(TokenKind::Minus) => {
                let minus = self.bump().expect("peek just confirmed a token is present");
                let operand = self.parse_unary()?;
                let span = self.span_from(minus.span, operand.span);
                Ok(Expr {
                    kind: ExprKind::Unary {
                        op: UnaryOp::Neg,
                        operand: Box::new(operand),
                    },
                    span,
                })
            }
            Some(TokenKind::Bang) => {
                let bang = self.bump().expect("peek just confirmed a token is present");
                let operand = self.parse_unary()?;
                let span = self.span_from(bang.span, operand.span);
                Ok(Expr {
                    kind: ExprKind::Unary {
                        op: UnaryOp::Not,
                        operand: Box::new(operand),
                    },
                    span,
                })
            }
            Some(TokenKind::Amp) => {
                let amp = self.bump().expect("peek just confirmed a token is present");
                self.bump_if(&TokenKind::Mut);
                // The sigil's fix-it (and error) span runs up to the start
                // of whatever comes next, so it swallows the separating
                // whitespace along with `&`/`&mut` and leaves no stray
                // space behind: `&x` -> `x`, `&mut x` -> `x`.
                let sigil_end = self
                    .peek_token()
                    .map(|t| t.span.start)
                    .unwrap_or_else(|| self.eof_span().start);
                let sigil_span =
                    self.span_from(amp.span, Span::new(amp.span.file, sigil_end, sigil_end));
                self.push_error_with_fix_it(
                    crate::error::V0010,
                    sigil_span,
                    "Varyk infers references; remove the `&`",
                    crate::error::FixIt {
                        span: sigil_span,
                        replacement: String::new(),
                    },
                );
                self.parse_unary()
            }
            _ => self.parse_postfix(),
        }
    }

    /// Postfix `.field`, `.method(args)`, `(args)`, `[index]`, and `?`,
    /// applied left-to-right onto whatever primary expression starts the
    /// chain (spec 2.5, 2.6, 2.8).
    fn parse_postfix(&mut self) -> Result<Expr, ()> {
        let mut expr = self.parse_primary()?;
        loop {
            match self.peek() {
                Some(TokenKind::Dot) => {
                    self.bump();
                    let name = self.expect_identifier("a field or method name after `.`")?;
                    if self.peek() == Some(&TokenKind::LParen) {
                        let (args, rparen_span) = self.parse_call_args()?;
                        let span = self.span_from(expr.span, rparen_span);
                        expr = Expr {
                            kind: ExprKind::MethodCall {
                                receiver: Box::new(expr),
                                method: name,
                                args,
                            },
                            span,
                        };
                    } else {
                        let span = self.span_from(expr.span, name.span);
                        expr = Expr {
                            kind: ExprKind::Field {
                                base: Box::new(expr),
                                name,
                            },
                            span,
                        };
                    }
                }
                Some(TokenKind::LParen) => {
                    if !matches!(expr.kind, ExprKind::Path { .. }) {
                        self.push_error(
                            V0002,
                            expr.span,
                            "only a function's name can be followed by `(...)`",
                        );
                    }
                    let (args, rparen_span) = self.parse_call_args()?;
                    let span = self.span_from(expr.span, rparen_span);
                    expr = Expr {
                        kind: ExprKind::Call {
                            callee: Box::new(expr),
                            args,
                        },
                        span,
                    };
                }
                Some(TokenKind::LBracket) => {
                    self.bump();
                    let index = self.without_condition(Self::parse_expr)?;
                    let rbracket = self.expect(TokenKind::RBracket, "`]`")?;
                    let span = self.span_from(expr.span, rbracket.span);
                    expr = Expr {
                        kind: ExprKind::Index {
                            base: Box::new(expr),
                            index: Box::new(index),
                        },
                        span,
                    };
                }
                Some(TokenKind::Question) => {
                    let question = self.bump().expect("peek just confirmed a token is present");
                    let span = self.span_from(expr.span, question.span);
                    expr = Expr {
                        kind: ExprKind::Try {
                            operand: Box::new(expr),
                        },
                        span,
                    };
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    /// Parses `(` already-peeked-but-not-consumed `expr, expr, ... )`,
    /// with an optional trailing comma, and returns the arguments plus the
    /// closing paren's span. The parentheses disambiguate, the same as a
    /// grouping expression's do, so a struct literal parses fine as an
    /// argument even in condition position: `if f(Point { x: 1 }) { }`.
    fn parse_call_args(&mut self) -> Result<(Vec<Expr>, Span), ()> {
        self.bump(); // '('
        let args = self.without_condition(|p| {
            let mut args = Vec::new();
            if p.peek() != Some(&TokenKind::RParen) {
                loop {
                    args.push(p.parse_expr()?);
                    if p.bump_if(&TokenKind::Comma) {
                        if p.peek() == Some(&TokenKind::RParen) {
                            break;
                        }
                        continue;
                    }
                    break;
                }
            }
            Ok(args)
        })?;
        let rparen = self.expect(TokenKind::RParen, "`)`")?;
        Ok((args, rparen.span))
    }

    fn parse_primary(&mut self) -> Result<Expr, ()> {
        match self.peek() {
            Some(TokenKind::IntegerLiteral(_))
            | Some(TokenKind::FloatLiteral(_))
            | Some(TokenKind::StringLiteral(_))
            | Some(TokenKind::BoolLiteral(_)) => Ok(self.parse_literal()),

            Some(TokenKind::LParen) => {
                let for_head_leading = std::mem::take(&mut self.for_head_leading_paren);
                self.bump();
                // Parentheses disambiguate, so a struct literal is fine
                // again immediately inside them even in condition
                // position: `if (Point { x: 1 }).x == 1 { }`.
                let inner = self.without_condition(|p| {
                    let lhs = p.parse_binary(0)?;
                    p.reject_stray_range_in(lhs, for_head_leading)
                })?;
                self.expect(TokenKind::RParen, "`)`")?;
                Ok(inner)
            }

            Some(TokenKind::Identifier(name)) if self.peek_at(1) == Some(&TokenKind::Bang) => {
                let name = name.clone();
                self.parse_macro_like(name)
            }

            Some(TokenKind::Identifier(_)) => self.parse_path_or_struct_lit(),

            Some(TokenKind::SelfKw) => self.parse_self(),

            Some(TokenKind::If) => self.parse_if(),

            Some(TokenKind::Match) => self.parse_match(),

            Some(TokenKind::LBracket) => {
                let span = self.current_span();
                self.bump();
                self.push_error(
                    V0001,
                    span,
                    "array literals are not supported in Varyk; use `vec![...]` instead",
                );
                Err(())
            }

            Some(TokenKind::In) => {
                let span = self.current_span();
                self.bump();
                self.push_error(V0001, span, "`in` only follows a `for` loop's variable");
                Err(())
            }

            // A range with no start (`..5`): still a range, so it gets the
            // same "only in a `for` head" diagnostic as a stray `a..b`
            // (`Parser::reject_stray_range`), rather than falling through
            // to the generic "expected an expression".
            Some(TokenKind::DotDot) => {
                let dotdot_span = self.current_span();
                self.bump();
                if self.peek() == Some(&TokenKind::Eq) {
                    let eq_span = self.current_span();
                    self.bump();
                    let span = self.span_from(dotdot_span, eq_span);
                    self.push_error(
                        V0001,
                        span,
                        "`..=` is not supported in Varyk yet; milestone 4 adds inclusive ranges",
                    );
                    return Err(());
                }
                self.push_error(
                    V0001,
                    dotdot_span,
                    "a range exists only in the head of a `for` loop",
                );
                Err(())
            }

            Some(TokenKind::LBrace) => {
                let block = self.parse_block()?;
                let span = block.span;
                Ok(Expr {
                    kind: ExprKind::Block(block),
                    span,
                })
            }

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

            Some(TokenKind::Lifetime(_)) => {
                let span = self.current_span();
                self.bump();
                self.push_error(
                    V0002,
                    span,
                    "a lifetime is not allowed where an expression is expected",
                );
                Err(())
            }

            _ => {
                let span = self.current_span();
                self.push_error(V0002, span, "expected an expression");
                Err(())
            }
        }
    }

    fn parse_literal(&mut self) -> Expr {
        let token = self
            .bump()
            .expect("parse_primary only calls this when peek() matched a literal");
        let span = token.span;
        let kind = match token.kind {
            TokenKind::IntegerLiteral(text) => ExprKind::Integer(text),
            TokenKind::FloatLiteral(text) => ExprKind::Float(text),
            TokenKind::StringLiteral(text) => ExprKind::String(text),
            TokenKind::BoolLiteral(value) => ExprKind::Bool(value),
            other => {
                unreachable!("parse_primary only calls this for literal tokens, got {other:?}")
            }
        };
        Expr { kind, span }
    }

    /// `name!(...)`: recognized for `println!` and `format!` (spec 4.1,
    /// 2.9), which share the format-string-plus-args shape, and `vec!`
    /// (spec 2.9), which has its own bracketed shape; every other `name!`
    /// is `V0001` naming macros (spec 6.3).
    fn parse_macro_like(&mut self, name: String) -> Result<Expr, ()> {
        let name_token = self.bump().expect("peek confirmed an identifier");
        let bang_token = self.bump().expect("peek confirmed a `!`");
        if name == "vec" {
            return self.parse_vec_lit(name_token.span);
        }
        if name != "println" && name != "format" {
            let span = self.span_from(name_token.span, bang_token.span);
            self.push_error(
                V0001,
                span,
                format!(
                    "macro `{name}!` is not supported in Varyk; only `println!`, `format!`, and `vec!` are"
                ),
            );
            return Err(());
        }
        let name_ident = Ident {
            name: name.clone(),
            span: name_token.span,
        };

        if self.peek() != Some(&TokenKind::LParen) {
            let span = self.current_span();
            self.push_error(V0002, span, format!("expected `(` after `{name}!`"));
            return Err(());
        }
        self.bump(); // '('

        let format = match self.peek() {
            Some(TokenKind::StringLiteral(_)) => {
                let token = self
                    .bump()
                    .expect("peek just confirmed a string literal is present");
                let text = match token.kind {
                    TokenKind::StringLiteral(text) => text,
                    _ => unreachable!("matched above"),
                };
                (text, token.span)
            }
            _ => {
                let span = self.current_span();
                self.push_error(
                    V0002,
                    span,
                    format!("expected a format string as `{name}!`'s first argument"),
                );
                return Err(());
            }
        };

        let mut args = Vec::new();
        while self.bump_if(&TokenKind::Comma) {
            if self.peek() == Some(&TokenKind::RParen) {
                break;
            }
            args.push(self.parse_expr()?);
        }
        let rparen = self.expect(TokenKind::RParen, "`)`")?;
        let span = self.span_from(name_ident.span, rparen.span);
        Ok(Expr {
            kind: ExprKind::Intrinsic {
                name: name_ident,
                format,
                args,
            },
            span,
        })
    }

    /// `vec![a, b, c]` (spec 2.9), right after `vec!` has been consumed:
    /// a bracketed, comma-separated, optionally empty list of elements,
    /// with an optional trailing comma.
    fn parse_vec_lit(&mut self, name_span: Span) -> Result<Expr, ()> {
        if self.peek() != Some(&TokenKind::LBracket) {
            let span = self.current_span();
            self.push_error(V0002, span, "expected `[` after `vec!`");
            return Err(());
        }
        self.bump(); // '['
        let elements = self.without_condition(|p| {
            let mut elements = Vec::new();
            if p.peek() != Some(&TokenKind::RBracket) {
                loop {
                    elements.push(p.parse_expr()?);
                    if p.bump_if(&TokenKind::Comma) {
                        if p.peek() == Some(&TokenKind::RBracket) {
                            break;
                        }
                        continue;
                    }
                    break;
                }
            }
            Ok(elements)
        })?;
        let rbracket = self.expect(TokenKind::RBracket, "`]`")?;
        let span = self.span_from(name_span, rbracket.span);
        Ok(Expr {
            kind: ExprKind::VecLit(elements),
            span,
        })
    }

    /// `self` in expression position (spec 2.5): a `Path` whose name is
    /// `self`, the same node a plain variable produces, so `self.count` and
    /// the rest of a method body parse like any other place expression.
    /// `self::x` is not a path Varyk has (`self` is never a module), so it
    /// is `V0001` rather than a `Path` with a module segment.
    fn parse_self(&mut self) -> Result<Expr, ()> {
        let self_token = self.bump().expect("peek confirmed `self`");
        if self.peek() == Some(&TokenKind::ColonColon) {
            self.bump();
            let end_span = match self.peek() {
                Some(TokenKind::Identifier(_)) => {
                    self.bump().expect("peek just confirmed a token").span
                }
                _ => self.current_span(),
            };
            let span = self.span_from(self_token.span, end_span);
            self.push_error(
                V0001,
                span,
                "`self` is a keyword and cannot be used as a module in a path",
            );
            return Err(());
        }
        Ok(Expr {
            kind: ExprKind::Path {
                module: None,
                type_: None,
                name: Ident {
                    name: "self".to_string(),
                    span: self_token.span,
                },
            },
            span: self_token.span,
        })
    }

    /// Parses `name(::name)*`: at least one segment, chased through every
    /// `::` found. Shared by expression paths (`parse_path_or_struct_lit`)
    /// and variant patterns (`parse_pattern`), which chase the same
    /// `name`, `module::name`, `module::Type::name` shape (spec 2.5,
    /// 2.10).
    fn parse_dotted_path(&mut self, what: &str) -> Result<(Vec<Ident>, Span), ()> {
        let mut segments = vec![self.expect_identifier(what)?];
        while self.peek() == Some(&TokenKind::ColonColon) {
            self.bump();
            segments.push(self.expect_identifier("a name after `::`")?);
        }
        let span = self.span_from(
            segments[0].span,
            segments.last().expect("at least one segment").span,
        );
        Ok((segments, span))
    }

    /// A path (`name`, `module::name`, `module::Type::name`, or,
    /// `V0001`-reported, a longer chain), optionally followed by a struct
    /// literal when the parser is not in condition position.
    fn parse_path_or_struct_lit(&mut self) -> Result<Expr, ()> {
        let (mut segments, path_span) = self.parse_dotted_path("a name")?;

        if self.peek() == Some(&TokenKind::LBrace) && !self.in_condition {
            return self.parse_struct_lit(segments, path_span);
        }

        if segments.len() > 3 {
            self.push_error(
                V0001,
                path_span,
                "a path longer than three segments (`module::Type::name`) is not supported in Varyk",
            );
        }
        let name = segments.pop().expect("at least one segment");
        // A four-or-more-segment path (already `V0001`-reported above)
        // still needs a `Path` built from something: the first two
        // segments, dropping the rest, so later passes have something to
        // work with.
        let type_ = if segments.len() >= 2 {
            Some(segments.remove(1))
        } else {
            None
        };
        let module = if !segments.is_empty() {
            Some(segments.remove(0))
        } else {
            None
        };
        Ok(Expr {
            kind: ExprKind::Path {
                module,
                type_,
                name,
            },
            span: path_span,
        })
    }

    /// `Name { field: expr, ... }` or `module::Name { field: expr, ... }`
    /// (spec 2.10), called right after `segments` has been parsed as a
    /// dotted path and the next token is `{`. A struct literal supports at
    /// most one module segment; a longer path is `V0001`, and the fields
    /// still parse so the caller sees a complete AST either way, using the
    /// segment closest to the name as the effective module.
    fn parse_struct_lit(&mut self, mut segments: Vec<Ident>, path_span: Span) -> Result<Expr, ()> {
        if segments.len() > 2 {
            self.push_error(
                V0001,
                path_span,
                "a struct literal is named with at most one module (`module::Name`)",
            );
        }
        let name = segments.pop().expect("at least one segment");
        let module = segments.pop();
        self.bump(); // '{'
        let mut fields = Vec::new();
        if self.peek() != Some(&TokenKind::RBrace) {
            loop {
                let field_name = self.expect_identifier("a field name")?;
                self.expect(TokenKind::Colon, "`:` after the field name")?;
                let value = self.parse_expr()?;
                fields.push((field_name, value));
                if self.bump_if(&TokenKind::Comma) {
                    if self.peek() == Some(&TokenKind::RBrace) {
                        break;
                    }
                    continue;
                }
                break;
            }
        }
        let rbrace = self.expect(TokenKind::RBrace, "`}`")?;
        let span = self.span_from(path_span, rbrace.span);
        Ok(Expr {
            kind: ExprKind::StructLit {
                module,
                name,
                fields,
            },
            span,
        })
    }

    /// `if cond { ... }` with an optional `else { ... }` or `else if ...`.
    /// The condition is parsed in condition position so `if a == b { }`
    /// parses as a comparison followed by an empty block, not a struct
    /// literal named `b`.
    fn parse_if(&mut self) -> Result<Expr, ()> {
        let if_token = self.bump().expect("peek confirmed `if`");

        if self.peek() == Some(&TokenKind::Let) {
            let let_token = self.bump().expect("peek confirmed `let`");
            let span = self.span_from(if_token.span, let_token.span);
            self.push_error(
                V0001,
                span,
                "`if let` is not supported in Varyk; use `match` instead",
            );
            return Err(());
        }

        let was_in_condition = self.in_condition;
        self.in_condition = true;
        let cond = self.parse_expr();
        self.in_condition = was_in_condition;
        let cond = cond?;

        let then = self.parse_block()?;
        let mut span = self.span_from(if_token.span, then.span);

        let else_ = if self.peek() == Some(&TokenKind::Else) {
            self.bump();
            match self.peek() {
                Some(TokenKind::LBrace) => {
                    let block = self.parse_block()?;
                    span = self.span_from(if_token.span, block.span);
                    Some(block)
                }
                Some(TokenKind::If) => {
                    // `else if ...` is represented as an `else` block
                    // whose only content is the nested `if` as its tail.
                    let nested = self.parse_if()?;
                    let block_span = nested.span;
                    span = self.span_from(if_token.span, block_span);
                    Some(Block {
                        stmts: Vec::new(),
                        tail: Some(Box::new(nested)),
                        span: block_span,
                    })
                }
                _ => {
                    let err_span = self.current_span();
                    self.push_error(V0002, err_span, "expected `{` or `if` after `else`");
                    return Err(());
                }
            }
        } else {
            None
        };

        Ok(Expr {
            kind: ExprKind::If {
                cond: Box::new(cond),
                then,
                else_,
            },
            span,
        })
    }

    /// `match scrutinee { arms... }` (spec 2.3). The scrutinee is parsed in
    /// condition position, the same as `if`'s and `while`'s, so
    /// `match x { }` parses `x` as the value and `{ }` as the arm list,
    /// not `x { }` as a struct literal.
    fn parse_match(&mut self) -> Result<Expr, ()> {
        let match_token = self.bump().expect("peek confirmed `match`");

        let was_in_condition = self.in_condition;
        self.in_condition = true;
        let scrutinee = self.parse_expr();
        self.in_condition = was_in_condition;
        let scrutinee = scrutinee?;

        self.expect(TokenKind::LBrace, "`{` after the value being matched")?;
        let mut arms = Vec::new();
        while self.peek() != Some(&TokenKind::RBrace) && self.peek().is_some() {
            let arm = self.without_condition(Self::parse_match_arm)?;
            let body_is_block = matches!(arm.body.kind, ExprKind::Block(_));
            arms.push(arm);
            if self.peek() == Some(&TokenKind::RBrace) {
                break;
            }
            if self.bump_if(&TokenKind::Comma) {
                continue;
            }
            if body_is_block {
                // The comma may be left out after a block-bodied arm; its
                // own closing `}` already ends it visually.
                continue;
            }
            let span = self.current_span();
            self.push_error(V0002, span, "expected `,` after this match arm");
            return Err(());
        }
        let rbrace = self.expect(TokenKind::RBrace, "`}`")?;
        let span = self.span_from(match_token.span, rbrace.span);
        Ok(Expr {
            kind: ExprKind::Match {
                scrutinee: Box::new(scrutinee),
                arms,
            },
            span,
        })
    }

    fn parse_match_arm(&mut self) -> Result<MatchArm, ()> {
        let pattern = self.parse_pattern()?;
        if self.peek() == Some(&TokenKind::DotDot) {
            let dotdot_span = self.current_span();
            self.bump();
            let span = self.span_from(pattern.span(), dotdot_span);
            self.push_error(V0001, span, "range patterns are not supported in Varyk");
            return Err(());
        }
        if self.peek() == Some(&TokenKind::If) {
            let span = self.current_span();
            self.push_error(
                V0001,
                span,
                "match guards (`pattern if condition`) are not supported in Varyk",
            );
            return Err(());
        }
        self.expect(TokenKind::FatArrow, "`=>` after a match pattern")?;
        let body = self.parse_match_arm_body()?;
        let span = self.span_from(pattern.span(), body.span);
        Ok(MatchArm {
            pattern,
            body,
            span,
        })
    }

    /// An arm's body, right after `=>`. A bare `return`, `break`, or
    /// `continue`, or an assignment, here is a Rust habit rustc allows but
    /// Varyk does not (spec 2.3): it is `V0001` naming the fix, wrapping it
    /// in a block (`{ return value; }`, `{ t = value; }`).
    fn parse_match_arm_body(&mut self) -> Result<Expr, ()> {
        let keyword = match self.peek() {
            Some(TokenKind::Return) => Some("return ..."),
            Some(TokenKind::Break) => Some("break"),
            Some(TokenKind::Continue) => Some("continue"),
            _ => None,
        };
        if let Some(statement) = keyword {
            let span = self.current_span();
            let word = statement.split(' ').next().expect("a keyword");
            self.push_error(
                V0001,
                span,
                format!(
                    "a bare `{word}` cannot be a match arm's body in Varyk; wrap it in a block: `{{ {statement}; }}`"
                ),
            );
            return Err(());
        }
        let body = self.parse_expr()?;
        if self.peek() == Some(&TokenKind::Eq) {
            let span = self.current_span();
            let target = match &body.kind {
                ExprKind::Path {
                    module: None,
                    type_: None,
                    name,
                } => name.name.as_str(),
                _ => "...",
            };
            self.push_error(
                V0001,
                span,
                format!(
                    "an assignment cannot be a match arm's body; wrap it in a block: `{{ {target} = ...; }}`"
                ),
            );
            return Err(());
        }
        Ok(body)
    }

    /// A pattern, one level deep (spec 2.3): `_`, a name, or a variant
    /// path with an optional parenthesized list of sub-patterns.
    fn parse_pattern(&mut self) -> Result<Pattern, ()> {
        match self.peek() {
            Some(TokenKind::Identifier(name)) if name == "_" => {
                let token = self.bump().expect("peek just confirmed a token is present");
                Ok(Pattern::Wildcard(token.span))
            }
            Some(TokenKind::Identifier(_)) => {
                let (mut segments, path_span) = self.parse_dotted_path("a pattern")?;
                if segments.len() > 3 {
                    self.push_error(
                        V0001,
                        path_span,
                        "a variant pattern longer than three segments (`module::Type::Variant`) is not supported in Varyk",
                    );
                }
                if self.peek() == Some(&TokenKind::LBrace) {
                    let group_span = self.skip_brace_group();
                    let span = self.span_from(path_span, group_span);
                    self.push_error(
                        V0001,
                        span,
                        "enum variants with named fields are not supported until milestone 4; \
                         use a tuple variant instead",
                    );
                    return Err(());
                }
                let has_subpatterns = self.peek() == Some(&TokenKind::LParen);
                if segments.len() == 1 && !has_subpatterns {
                    return Ok(Pattern::Name(segments.pop().expect("one segment")));
                }
                let (subpatterns, sub_span) = if has_subpatterns {
                    let (subpatterns, sub_span) = self.parse_subpatterns()?;
                    (subpatterns, Some(sub_span))
                } else {
                    (Vec::new(), None)
                };
                let name = segments.pop().expect("at least one segment");
                // Unlike `ExprKind::Path`, a pattern's single remaining
                // segment is unambiguously the type (`Shape::Point`), not
                // a module: patterns never call anything, so there is no
                // bare-module case to weigh against it.
                let type_ = segments.pop();
                let module = segments.pop();
                let span = match sub_span {
                    Some(sub_span) => self.span_from(path_span, sub_span),
                    None => path_span,
                };
                Ok(Pattern::Variant(VariantPattern {
                    module,
                    type_,
                    name,
                    subpatterns,
                    span,
                }))
            }
            Some(TokenKind::DotDot) => {
                let span = self.current_span();
                self.bump();
                self.push_error(
                    V0001,
                    span,
                    "rest patterns (`..`) are not supported in Varyk",
                );
                Err(())
            }
            _ => {
                let span = self.current_span();
                self.bump();
                // `0..5`: the literal alone would be a "literal pattern"
                // error, but what follows it names this a range pattern
                // instead, which is the more useful thing to say.
                if self.peek() == Some(&TokenKind::DotDot) {
                    let dotdot_span = self.current_span();
                    self.bump();
                    let range_span = self.span_from(span, dotdot_span);
                    self.push_error(
                        V0001,
                        range_span,
                        "range patterns are not supported in Varyk",
                    );
                    return Err(());
                }
                self.push_error(
                    V0001,
                    span,
                    "only a name, `_`, or a variant is supported as a pattern in Varyk; literal patterns are not supported",
                );
                Err(())
            }
        }
    }

    /// `(sub, sub, ...)` right after a variant pattern's name, whose
    /// opening `(` is already confirmed present. Returns the sub-patterns
    /// plus the span of the whole group.
    fn parse_subpatterns(&mut self) -> Result<(Vec<SubPattern>, Span), ()> {
        let lparen = self.bump().expect("caller confirmed `(`");
        let mut subpatterns = Vec::new();
        if self.peek() != Some(&TokenKind::RParen) {
            loop {
                subpatterns.push(self.parse_subpattern()?);
                if self.bump_if(&TokenKind::Comma) {
                    if self.peek() == Some(&TokenKind::RParen) {
                        break;
                    }
                    continue;
                }
                break;
            }
        }
        let rparen = self.expect(TokenKind::RParen, "`)`")?;
        Ok((subpatterns, self.span_from(lparen.span, rparen.span)))
    }

    /// A sub-pattern inside a variant pattern's parentheses (spec 2.3): a
    /// name or `_`. Anything else, including a nested variant pattern
    /// (`Some(Shape::Point)`) or a literal, is `V0001`.
    fn parse_subpattern(&mut self) -> Result<SubPattern, ()> {
        match self.peek() {
            Some(TokenKind::Identifier(name)) if name == "_" => {
                let token = self.bump().expect("peek just confirmed a token is present");
                Ok(SubPattern::Wildcard(token.span))
            }
            Some(TokenKind::Identifier(_)) => {
                let name = self.expect_identifier("a name")?;
                if matches!(
                    self.peek(),
                    Some(TokenKind::LParen) | Some(TokenKind::ColonColon)
                ) {
                    self.push_error(
                        V0001,
                        name.span,
                        "nested patterns are not supported in Varyk; match the inner value in the arm's body instead",
                    );
                    return Err(());
                }
                Ok(SubPattern::Name(name))
            }
            Some(TokenKind::DotDot) => {
                let span = self.current_span();
                self.bump();
                self.push_error(
                    V0001,
                    span,
                    "rest patterns (`..`) are not supported in Varyk",
                );
                Err(())
            }
            Some(TokenKind::Mut) => {
                let span = self.current_span();
                self.bump();
                self.push_error(
                    V0001,
                    span,
                    "`mut` is not supported in a pattern in Varyk; write `let mut x = x;` \
                     inside the arm instead",
                );
                Err(())
            }
            Some(TokenKind::ReservedKeyword(word)) if word == "ref" => {
                let span = self.current_span();
                self.bump();
                self.push_error(
                    V0001,
                    span,
                    "`ref` is not supported in a pattern in Varyk; write `let mut x = x;` \
                     inside the arm instead",
                );
                Err(())
            }
            _ => {
                let span = self.current_span();
                self.bump();
                self.push_error(
                    V0001,
                    span,
                    "only a name or `_` is supported inside a variant pattern in Varyk; literal patterns are not supported",
                );
                Err(())
            }
        }
    }

    /// `{ stmts...; tail? }`. A block's own body is never in condition
    /// position, even when the block itself sits inside an `if`/`while`
    /// condition (`if { Point { x: 1 } } { }`): the braces already
    /// disambiguate, the same as a grouping expression's parentheses do.
    pub(super) fn parse_block(&mut self) -> Result<Block, ()> {
        let lbrace = self.expect(TokenKind::LBrace, "`{`")?;
        let (stmts, tail) = self.without_condition(Self::parse_block_body)?;
        let rbrace = self.expect(TokenKind::RBrace, "`}`")?;
        Ok(Block {
            stmts,
            tail: tail.map(Box::new),
            span: self.span_from(lbrace.span, rbrace.span),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SyntaxError;
    use crate::lex;
    use crate::source::SourceFile;
    use crate::span::FileId;

    fn parse(src: &str) -> (Result<Expr, ()>, Vec<SyntaxError>) {
        let file = SourceFile::new(FileId(0), "test.vr", src);
        let (tokens, lex_errors) = lex(&file);
        assert!(
            lex_errors.is_empty(),
            "unexpected lex errors: {lex_errors:?}"
        );
        let mut parser = Parser::new(&tokens, FileId(0));
        let expr = parser.parse_expr();
        (expr, parser.errors().to_vec())
    }

    fn parse_ok(src: &str) -> Expr {
        let (expr, errors) = parse(src);
        assert!(
            errors.is_empty(),
            "unexpected errors parsing {src:?}: {errors:?}"
        );
        expr.unwrap_or_else(|()| panic!("expected {src:?} to parse"))
    }

    fn path_name(expr: &Expr) -> &str {
        match &expr.kind {
            ExprKind::Path { name, .. } => &name.name,
            other => panic!("expected a Path, got {other:?}"),
        }
    }

    #[test]
    fn addition_binds_looser_than_multiplication() {
        let expr = parse_ok("a + b * c");
        match &expr.kind {
            ExprKind::Binary {
                op: BinaryOp::Add,
                lhs,
                rhs,
            } => {
                assert_eq!(path_name(lhs), "a");
                match &rhs.kind {
                    ExprKind::Binary {
                        op: BinaryOp::Mul,
                        lhs,
                        rhs,
                    } => {
                        assert_eq!(path_name(lhs), "b");
                        assert_eq!(path_name(rhs), "c");
                    }
                    other => panic!("expected b * c, got {other:?}"),
                }
            }
            other => panic!("expected a + (b * c), got {other:?}"),
        }
    }

    #[test]
    fn unary_binds_tighter_than_multiplication() {
        let expr = parse_ok("-a * b");
        match &expr.kind {
            ExprKind::Binary {
                op: BinaryOp::Mul,
                lhs,
                rhs,
            } => {
                match &lhs.kind {
                    ExprKind::Unary {
                        op: UnaryOp::Neg,
                        operand,
                    } => {
                        assert_eq!(path_name(operand), "a");
                    }
                    other => panic!("expected -a, got {other:?}"),
                }
                assert_eq!(path_name(rhs), "b");
            }
            other => panic!("expected (-a) * b, got {other:?}"),
        }
    }

    #[test]
    fn and_binds_tighter_than_or_and_not_binds_tighter_than_and() {
        let expr = parse_ok("!a && b || c");
        match &expr.kind {
            ExprKind::Binary {
                op: BinaryOp::Or,
                lhs,
                rhs,
            } => {
                assert_eq!(path_name(rhs), "c");
                match &lhs.kind {
                    ExprKind::Binary {
                        op: BinaryOp::And,
                        lhs,
                        rhs,
                    } => {
                        match &lhs.kind {
                            ExprKind::Unary {
                                op: UnaryOp::Not,
                                operand,
                            } => {
                                assert_eq!(path_name(operand), "a");
                            }
                            other => panic!("expected !a, got {other:?}"),
                        }
                        assert_eq!(path_name(rhs), "b");
                    }
                    other => panic!("expected !a && b, got {other:?}"),
                }
            }
            other => panic!("expected (!a && b) || c, got {other:?}"),
        }
    }

    #[test]
    fn compound_assignment_is_v0001_and_stops() {
        for (src, symbol) in [("x += 1", "+"), ("x -= 1", "-"), ("x %= 2", "%")] {
            let (expr, errors) = parse(src);
            assert!(expr.is_err(), "{src} must not parse");
            assert_eq!(errors.len(), 1, "{src}: {errors:?}");
            assert_eq!(errors[0].code, V0001);
            assert_eq!(
                errors[0].message,
                format!(
                    "compound assignment `{symbol}=` is not supported; write `x = x {symbol} 1`"
                )
            );
        }
    }

    #[test]
    fn comparisons_are_non_associative() {
        let (expr, errors) = parse("a < b < c");
        assert!(expr.is_err(), "a < b < c must not parse");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, V0002);
    }

    #[test]
    fn parenthesized_groups_override_precedence() {
        let expr = parse_ok("(a + b) * c");
        match &expr.kind {
            ExprKind::Binary {
                op: BinaryOp::Mul,
                lhs,
                rhs,
            } => {
                match &lhs.kind {
                    ExprKind::Binary {
                        op: BinaryOp::Add, ..
                    } => {}
                    other => panic!("expected a + b, got {other:?}"),
                }
                assert_eq!(path_name(rhs), "c");
            }
            other => panic!("expected (a + b) * c, got {other:?}"),
        }
    }

    #[test]
    fn call_with_zero_arguments() {
        let expr = parse_ok("f()");
        match &expr.kind {
            ExprKind::Call { callee, args } => {
                assert_eq!(path_name(callee), "f");
                assert!(args.is_empty());
            }
            other => panic!("expected a call, got {other:?}"),
        }
    }

    #[test]
    fn call_with_several_arguments() {
        let expr = parse_ok("f(a, b, c)");
        match &expr.kind {
            ExprKind::Call { callee, args } => {
                assert_eq!(path_name(callee), "f");
                assert_eq!(args.len(), 3);
                assert_eq!(path_name(&args[0]), "a");
                assert_eq!(path_name(&args[2]), "c");
            }
            other => panic!("expected a call, got {other:?}"),
        }
    }

    #[test]
    fn nested_field_access() {
        let expr = parse_ok("a.b.c");
        match &expr.kind {
            ExprKind::Field { base, name } => {
                assert_eq!(name.name, "c");
                match &base.kind {
                    ExprKind::Field { base, name } => {
                        assert_eq!(name.name, "b");
                        assert_eq!(path_name(base), "a");
                    }
                    other => panic!("expected a.b, got {other:?}"),
                }
            }
            other => panic!("expected a.b.c, got {other:?}"),
        }
    }

    #[test]
    fn path_call() {
        let expr = parse_ok("greet::hello(x)");
        match &expr.kind {
            ExprKind::Call { callee, args } => {
                match &callee.kind {
                    ExprKind::Path {
                        module,
                        type_,
                        name,
                    } => {
                        assert_eq!(module.as_ref().unwrap().name, "greet");
                        assert!(type_.is_none());
                        assert_eq!(name.name, "hello");
                    }
                    other => panic!("expected a path callee, got {other:?}"),
                }
                assert_eq!(args.len(), 1);
            }
            other => panic!("expected a call, got {other:?}"),
        }
    }

    #[test]
    fn three_segment_path_fills_module_and_type() {
        let expr = parse_ok("m::Counter::new");
        match &expr.kind {
            ExprKind::Path {
                module,
                type_,
                name,
            } => {
                assert_eq!(module.as_ref().unwrap().name, "m");
                assert_eq!(type_.as_ref().unwrap().name, "Counter");
                assert_eq!(name.name, "new");
            }
            other => panic!("expected a path, got {other:?}"),
        }
    }

    #[test]
    fn four_segment_path_is_v0001() {
        let (_expr, errors) = parse("a::b::c::d");
        assert!(errors.iter().any(|e| e.code == V0001), "errors: {errors:?}");
    }

    #[test]
    fn method_call_parses() {
        let src = "x.f()";
        let expr = parse_ok(src);
        // The node spans the whole call, `x.f()`, not just `x` or `f`.
        assert_eq!(expr.span.start, 0);
        assert_eq!(expr.span.end, src.len() as u32);
        match &expr.kind {
            ExprKind::MethodCall {
                receiver,
                method,
                args,
            } => {
                assert_eq!(path_name(receiver), "x");
                assert_eq!(method.name, "f");
                assert!(args.is_empty());
            }
            other => panic!("expected a method call, got {other:?}"),
        }
    }

    #[test]
    fn method_call_with_arguments_and_field_receiver() {
        // `a.b(c)`, spec's own example.
        let expr = parse_ok("a.b(c)");
        match &expr.kind {
            ExprKind::MethodCall {
                receiver,
                method,
                args,
            } => {
                assert_eq!(path_name(receiver), "a");
                assert_eq!(method.name, "b");
                assert_eq!(args.len(), 1);
                assert_eq!(path_name(&args[0]), "c");
            }
            other => panic!("expected a method call, got {other:?}"),
        }
    }

    #[test]
    fn index_expression_parses() {
        let src = "v[i]";
        let expr = parse_ok(src);
        assert_eq!(expr.span.start, 0);
        assert_eq!(expr.span.end, src.len() as u32);
        match &expr.kind {
            ExprKind::Index { base, index } => {
                assert_eq!(path_name(base), "v");
                assert_eq!(path_name(index), "i");
            }
            other => panic!("expected an index, got {other:?}"),
        }
    }

    #[test]
    fn try_operator_parses() {
        let src = "x?";
        let expr = parse_ok(src);
        assert_eq!(expr.span.start, 0);
        assert_eq!(expr.span.end, src.len() as u32);
        match &expr.kind {
            ExprKind::Try { operand } => assert_eq!(path_name(operand), "x"),
            other => panic!("expected a try, got {other:?}"),
        }
    }

    #[test]
    fn non_callable_callee_is_v0002() {
        let (_expr, errors) = parse("(a + b)()");
        assert!(errors.iter().any(|e| e.code == V0002), "errors: {errors:?}");
    }

    #[test]
    fn array_literal_is_v0001_naming_vec() {
        let (_expr, errors) = parse("[1, 2, 3]");
        assert!(
            errors
                .iter()
                .any(|e| e.code == V0001 && e.message.contains("vec!")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn struct_literal_with_trailing_comma() {
        let expr = parse_ok("Point { x: 1, y: 2, }");
        match &expr.kind {
            ExprKind::StructLit {
                module,
                name,
                fields,
            } => {
                assert!(module.is_none());
                assert_eq!(name.name, "Point");
                assert_eq!(fields.len(), 2);
                assert_eq!(fields[0].0.name, "x");
                assert_eq!(fields[1].0.name, "y");
            }
            other => panic!("expected a struct literal, got {other:?}"),
        }
    }

    #[test]
    fn block_with_tail_expression() {
        let expr = parse_ok("{ 42 }");
        match &expr.kind {
            ExprKind::Block(Block { stmts, tail, .. }) => {
                assert!(stmts.is_empty());
                match tail.as_deref().map(|e| &e.kind) {
                    Some(ExprKind::Integer(text)) => assert_eq!(text, "42"),
                    other => panic!("expected a tail of 42, got {other:?}"),
                }
            }
            other => panic!("expected a block, got {other:?}"),
        }
    }

    #[test]
    fn if_without_else() {
        let expr = parse_ok("if a { 1 }");
        match &expr.kind {
            ExprKind::If { cond, then, else_ } => {
                assert_eq!(path_name(cond), "a");
                assert!(then.tail.is_some());
                assert!(else_.is_none());
            }
            other => panic!("expected an if, got {other:?}"),
        }
    }

    #[test]
    fn if_with_else() {
        let expr = parse_ok("if a { 1 } else { 2 }");
        match &expr.kind {
            ExprKind::If { else_, .. } => {
                assert!(else_.is_some());
            }
            other => panic!("expected an if, got {other:?}"),
        }
    }

    #[test]
    fn else_if_chains() {
        let expr = parse_ok("if a { 1 } else if b { 2 } else { 3 }");
        match &expr.kind {
            ExprKind::If { else_, .. } => {
                let else_block = else_.as_ref().unwrap();
                match else_block.tail.as_deref().map(|e| &e.kind) {
                    Some(ExprKind::If { cond, else_, .. }) => {
                        assert_eq!(path_name(cond), "b");
                        assert!(else_.is_some());
                    }
                    other => {
                        panic!("expected the else block's tail to be a nested if, got {other:?}")
                    }
                }
            }
            other => panic!("expected an if, got {other:?}"),
        }
    }

    #[test]
    fn no_struct_literal_in_if_condition() {
        // `if a == b { }` must parse as a comparison followed by an empty
        // block, not as a struct literal named `b`.
        let expr = parse_ok("if a == b { }");
        match &expr.kind {
            ExprKind::If { cond, then, .. } => {
                match &cond.kind {
                    ExprKind::Binary {
                        op: BinaryOp::Eq, ..
                    } => {}
                    other => panic!("expected a == b, got {other:?}"),
                }
                assert!(then.tail.is_none());
            }
            other => panic!("expected an if, got {other:?}"),
        }
    }

    #[test]
    fn module_qualified_struct_literal_parses() {
        let src = "math::Point { x: 1 }";
        let expr = parse_ok(src);
        assert_eq!(expr.span.start, 0);
        assert_eq!(expr.span.end, src.len() as u32);
        match &expr.kind {
            ExprKind::StructLit { module, name, .. } => {
                assert_eq!(module.as_ref().unwrap().name, "math");
                assert_eq!(name.name, "Point");
            }
            other => panic!("expected a struct literal, got {other:?}"),
        }
    }

    #[test]
    fn three_segment_struct_literal_is_v0001() {
        let (_expr, errors) = parse("a::b::Point { }");
        assert!(errors.iter().any(|e| e.code == V0001), "errors: {errors:?}");
    }

    #[test]
    fn struct_literal_in_call_argument_inside_if_condition() {
        // The parentheses around the call's arguments already disambiguate,
        // so `in_condition` must not leak into them.
        let expr = parse_ok("if f(Point { x: 1 }) { }");
        match &expr.kind {
            ExprKind::If { cond, .. } => match &cond.kind {
                ExprKind::Call { args, .. } => match &args[0].kind {
                    ExprKind::StructLit { name, .. } => assert_eq!(name.name, "Point"),
                    other => panic!("expected a struct literal argument, got {other:?}"),
                },
                other => panic!("expected a call, got {other:?}"),
            },
            other => panic!("expected an if, got {other:?}"),
        }
    }

    #[test]
    fn struct_literal_in_block_expression_inside_if_condition() {
        // The braces of a block expression already disambiguate, so
        // `in_condition` must not leak into the block's body either.
        let expr = parse_ok("if { Point { x: 1 } } { }");
        match &expr.kind {
            ExprKind::If { cond, .. } => match &cond.kind {
                ExprKind::Block(Block { tail, .. }) => match tail.as_deref().map(|e| &e.kind) {
                    Some(ExprKind::StructLit { name, .. }) => assert_eq!(name.name, "Point"),
                    other => panic!("expected a struct literal tail, got {other:?}"),
                },
                other => panic!("expected a block, got {other:?}"),
            },
            other => panic!("expected an if, got {other:?}"),
        }
    }

    #[test]
    fn println_intrinsic_with_format_and_args() {
        let expr = parse_ok(r#"println!("{} and {}", a, b)"#);
        match &expr.kind {
            ExprKind::Intrinsic { name, format, args } => {
                assert_eq!(name.name, "println");
                assert_eq!(format.0, "{} and {}");
                assert_eq!(args.len(), 2);
                assert_eq!(path_name(&args[0]), "a");
                assert_eq!(path_name(&args[1]), "b");
            }
            other => panic!("expected an intrinsic, got {other:?}"),
        }
    }

    #[test]
    fn other_macro_name_is_v0001() {
        let (_expr, errors) = parse(r#"assert!(true)"#);
        assert!(
            errors
                .iter()
                .any(|e| e.code == V0001 && e.message.contains("`assert!`")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn format_intrinsic_with_format_and_args() {
        let expr = parse_ok(r#"format!("{} and {}", a, b)"#);
        match &expr.kind {
            ExprKind::Intrinsic { name, format, args } => {
                assert_eq!(name.name, "format");
                assert_eq!(format.0, "{} and {}");
                assert_eq!(args.len(), 2);
            }
            other => panic!("expected an intrinsic, got {other:?}"),
        }
    }

    #[test]
    fn vec_lit_with_elements_parses() {
        let src = "vec![a, b, c]";
        let expr = parse_ok(src);
        assert_eq!(expr.span.start, 0);
        assert_eq!(expr.span.end, src.len() as u32);
        match &expr.kind {
            ExprKind::VecLit(elements) => {
                assert_eq!(elements.len(), 3);
                assert_eq!(path_name(&elements[0]), "a");
                assert_eq!(path_name(&elements[2]), "c");
            }
            other => panic!("expected a vec literal, got {other:?}"),
        }
    }

    #[test]
    fn empty_vec_lit_parses() {
        let expr = parse_ok("vec![]");
        match &expr.kind {
            ExprKind::VecLit(elements) => assert!(elements.is_empty()),
            other => panic!("expected a vec literal, got {other:?}"),
        }
    }

    #[test]
    fn vec_lit_with_trailing_comma_parses() {
        let expr = parse_ok("vec![a, b,]");
        match &expr.kind {
            ExprKind::VecLit(elements) => assert_eq!(elements.len(), 2),
            other => panic!("expected a vec literal, got {other:?}"),
        }
    }

    #[test]
    fn amp_before_expression_is_v0010_and_parsing_continues() {
        let (expr, errors) = parse("&x");
        let expr = expr.unwrap_or_else(|()| panic!("parsing must continue past &"));
        assert_eq!(path_name(&expr), "x");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, crate::error::V0010);
        let fix_it = errors[0].fix_it.as_ref().expect("V0010 carries a fix-it");
        assert_eq!(fix_it.replacement, "");
        // The fix-it span covers exactly the `&` (no trailing space, since
        // `x` starts right after it).
        assert_eq!(fix_it.span.start, 0);
        assert_eq!(fix_it.span.end, 1);
    }

    #[test]
    fn amp_mut_before_expression_is_v0010_and_parsing_continues() {
        let (expr, errors) = parse("&mut x");
        let expr = expr.unwrap_or_else(|()| panic!("parsing must continue past &mut"));
        assert_eq!(path_name(&expr), "x");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, crate::error::V0010);
        let fix_it = errors[0].fix_it.as_ref().expect("V0010 carries a fix-it");
        // Covers "&mut " including the separating space, so removing it
        // leaves exactly `x` behind.
        assert_eq!(fix_it.span.start, 0);
        assert_eq!(fix_it.span.end, 5);
    }

    #[test]
    fn reserved_keyword_where_expression_expected_is_v0001() {
        let (expr, errors) = parse("loop");
        assert!(expr.is_err());
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, V0001);
        assert!(errors[0].message.contains("loop"), "{}", errors[0].message);
    }

    // --- `self` (spec 2.5) --------------------------------------------------

    #[test]
    fn self_parses_as_a_path() {
        let expr = parse_ok("self");
        assert_eq!(path_name(&expr), "self");
    }

    #[test]
    fn self_field_access_parses() {
        let expr = parse_ok("self.count");
        match &expr.kind {
            ExprKind::Field { base, name } => {
                assert_eq!(name.name, "count");
                assert_eq!(path_name(base), "self");
            }
            other => panic!("expected a field access, got {other:?}"),
        }
    }

    #[test]
    fn self_colon_colon_x_is_v0001() {
        let (expr, errors) = parse("self::x");
        assert!(expr.is_err());
        assert!(
            errors
                .iter()
                .any(|e| e.code == V0001 && e.message.contains("self")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn module_colon_colon_self_is_v0001() {
        let (_expr, errors) = parse("m::self");
        assert!(
            errors
                .iter()
                .any(|e| e.code == V0001 && e.message.contains("keyword")),
            "errors: {errors:?}"
        );
    }

    // --- `match` and patterns (spec 2.3) ------------------------------------

    #[test]
    fn match_with_wildcard_and_name_pattern() {
        let src = "match x { _ => 1, n => n, }";
        let expr = parse_ok(src);
        assert_eq!(expr.span.start, 0);
        assert_eq!(expr.span.end, src.len() as u32);
        match &expr.kind {
            ExprKind::Match { scrutinee, arms } => {
                assert_eq!(path_name(scrutinee), "x");
                assert_eq!(arms.len(), 2);
                assert!(matches!(arms[0].pattern, Pattern::Wildcard(_)));
                match &arms[1].pattern {
                    Pattern::Name(name) => assert_eq!(name.name, "n"),
                    other => panic!("expected a name pattern, got {other:?}"),
                }
            }
            other => panic!("expected a match, got {other:?}"),
        }
    }

    #[test]
    fn match_variant_pattern_with_subpatterns() {
        let expr = parse_ok("match shape { Shape::Circle(r) => 1, Shape::Point => 0 }");
        match &expr.kind {
            ExprKind::Match { arms, .. } => {
                assert_eq!(arms.len(), 2);
                match &arms[0].pattern {
                    Pattern::Variant(variant) => {
                        assert!(variant.module.is_none());
                        assert_eq!(variant.type_.as_ref().unwrap().name, "Shape");
                        assert_eq!(variant.name.name, "Circle");
                        assert_eq!(variant.subpatterns.len(), 1);
                        match &variant.subpatterns[0] {
                            SubPattern::Name(name) => assert_eq!(name.name, "r"),
                            other => panic!("expected a name subpattern, got {other:?}"),
                        }
                    }
                    other => panic!("expected a variant pattern, got {other:?}"),
                }
                match &arms[1].pattern {
                    Pattern::Variant(variant) => {
                        assert_eq!(variant.name.name, "Point");
                        assert!(variant.subpatterns.is_empty());
                    }
                    other => panic!("expected a variant pattern, got {other:?}"),
                }
            }
            other => panic!("expected a match, got {other:?}"),
        }
    }

    #[test]
    fn match_unqualified_variant_pattern_with_wildcard_subpattern() {
        // `Some(_)`: a single-segment variant pattern, told apart from a
        // name pattern by the parentheses that follow it.
        let expr = parse_ok("match x { Some(_) => 1, None => 0 }");
        match &expr.kind {
            ExprKind::Match { arms, .. } => match &arms[0].pattern {
                Pattern::Variant(variant) => {
                    assert!(variant.module.is_none());
                    assert!(variant.type_.is_none());
                    assert_eq!(variant.name.name, "Some");
                    assert!(matches!(variant.subpatterns[0], SubPattern::Wildcard(_)));
                }
                other => panic!("expected a variant pattern, got {other:?}"),
            },
            other => panic!("expected a match, got {other:?}"),
        }
    }

    #[test]
    fn match_module_qualified_variant_pattern() {
        let expr = parse_ok("match shape { geo::Shape::Point => 0, _ => 1 }");
        match &expr.kind {
            ExprKind::Match { arms, .. } => match &arms[0].pattern {
                Pattern::Variant(variant) => {
                    assert_eq!(variant.module.as_ref().unwrap().name, "geo");
                    assert_eq!(variant.type_.as_ref().unwrap().name, "Shape");
                    assert_eq!(variant.name.name, "Point");
                }
                other => panic!("expected a variant pattern, got {other:?}"),
            },
            other => panic!("expected a match, got {other:?}"),
        }
    }

    #[test]
    fn match_arm_with_block_body_needs_no_trailing_comma() {
        let expr = parse_ok("match x { _ => { 1 } n => 2 }");
        match &expr.kind {
            ExprKind::Match { arms, .. } => assert_eq!(arms.len(), 2),
            other => panic!("expected a match, got {other:?}"),
        }
    }

    #[test]
    fn match_arm_without_comma_or_block_body_is_v0002() {
        let (_expr, errors) = parse("match x { _ => 1 n => 2 }");
        assert!(errors.iter().any(|e| e.code == V0002), "errors: {errors:?}");
    }

    #[test]
    fn match_variant_pattern_wrong_kind_of_subpattern_is_v0001() {
        let (_expr, errors) = parse("match x { Some(1) => 1, _ => 0 }");
        assert!(errors.iter().any(|e| e.code == V0001), "errors: {errors:?}");
    }

    #[test]
    fn match_named_field_variant_pattern_is_v0001_naming_milestone_4() {
        let (_expr, errors) = parse("match s { Shape::Circle { r } => 1, _ => 0 }");
        assert!(
            errors.iter().any(|e| e.code == V0001
                && e.message
                    == "enum variants with named fields are not supported until milestone 4; \
                        use a tuple variant instead"),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn match_nested_variant_pattern_is_v0001() {
        let (_expr, errors) = parse("match x { Some(Shape::Point) => 1, _ => 0 }");
        assert!(
            errors
                .iter()
                .any(|e| e.code == V0001 && e.message.contains("nested")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn match_literal_pattern_is_v0001() {
        let (_expr, errors) = parse("match x { 1 => 1, _ => 0 }");
        assert!(errors.iter().any(|e| e.code == V0001), "errors: {errors:?}");
    }

    #[test]
    fn match_mut_subpattern_is_v0001_naming_mut() {
        let (_expr, errors) = parse("match x { Some(mut x) => 1, _ => 0 }");
        assert!(
            errors.iter().any(|e| e.code == V0001
                && e.message.contains("mut")
                && e.message.contains("let mut x = x;")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn match_ref_subpattern_is_v0001_naming_ref() {
        let (_expr, errors) = parse("match x { Some(ref x) => 1, _ => 0 }");
        assert!(
            errors.iter().any(|e| e.code == V0001
                && e.message.contains("ref")
                && e.message.contains("let mut x = x;")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn match_rest_pattern_is_v0001() {
        let (_expr, errors) = parse("match x { .. => 1, _ => 0 }");
        assert!(
            errors
                .iter()
                .any(|e| e.code == V0001 && e.message.contains("rest")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn match_rest_subpattern_is_v0001() {
        // `Some(..)`: a rest pattern inside a variant's parentheses is a
        // distinct case from a bare `..` pattern, and must not be called a
        // literal pattern either.
        let (_expr, errors) = parse("match x { Some(..) => 1, _ => 0 }");
        assert!(
            errors
                .iter()
                .any(|e| e.code == V0001 && e.message.contains("rest")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn match_range_pattern_from_a_literal_is_v0001() {
        // `0..5`: must be named a range pattern, not a literal pattern.
        let (_expr, errors) = parse("match x { 0..5 => 1, _ => 0 }");
        assert!(
            errors
                .iter()
                .any(|e| e.code == V0001 && e.message.contains("range")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn match_range_pattern_after_a_name_is_v0001() {
        // `n..5`: the start parses fine as a name pattern, so the `..`
        // must be caught right after, in `parse_match_arm`.
        let (_expr, errors) = parse("match x { n..5 => 1, m => m }");
        assert!(
            errors
                .iter()
                .any(|e| e.code == V0001 && e.message.contains("range")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn match_guard_is_v0001() {
        let (_expr, errors) = parse("match x { n if n > 0 => 1, _ => 0 }");
        assert!(
            errors
                .iter()
                .any(|e| e.code == V0001 && e.message.contains("guard")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn match_arm_bare_return_body_is_v0001() {
        let (_expr, errors) = parse("match x { _ => return 1, n => n }");
        assert!(
            errors
                .iter()
                .any(|e| e.code == V0001 && e.message.contains("return")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn match_arm_bare_break_or_continue_body_is_v0001() {
        for keyword in ["break", "continue"] {
            let (_expr, errors) = parse(&format!("match x {{ Some(y) => {keyword}, None => 0 }}"));
            assert_eq!(errors.len(), 1, "errors: {errors:?}");
            assert_eq!(errors[0].code, V0001, "errors: {errors:?}");
            assert!(
                errors[0]
                    .message
                    .contains(&format!("a bare `{keyword}` cannot be a match arm's body")),
                "errors: {errors:?}"
            );
            assert!(
                errors[0].message.contains(&format!("`{{ {keyword}; }}`")),
                "errors: {errors:?}"
            );
        }
    }

    #[test]
    fn match_arm_assignment_body_is_v0001() {
        let (_expr, errors) = parse("match x { Some(y) => t = t + y, None => {} }");
        assert_eq!(errors.len(), 1, "errors: {errors:?}");
        assert_eq!(errors[0].code, V0001, "errors: {errors:?}");
        assert!(
            errors[0].message.contains(
                "an assignment cannot be a match arm's body; wrap it in a block: `{ t = ...; }`"
            ),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn if_let_is_v0001() {
        let (_expr, errors) = parse("if let Some(x) = y { }");
        assert!(
            errors
                .iter()
                .any(|e| e.code == V0001 && e.message.contains("if let")),
            "errors: {errors:?}"
        );
    }

    // --- Ranges (spec 2.4), valid only in a `for` head ----------------------

    #[test]
    fn range_outside_for_is_v0001() {
        let (_expr, errors) = parse("0..10");
        assert!(
            errors
                .iter()
                .any(|e| e.code == V0001 && e.message.contains("`for`")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn inclusive_range_is_v0001() {
        let (_expr, errors) = parse("0..=10");
        assert!(
            errors
                .iter()
                .any(|e| e.code == V0001 && e.message.contains("..=")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn range_with_no_start_is_v0001() {
        // `..5`: a leading `..` with nothing before it must still be
        // named a range, not fall through to a generic "expected an
        // expression".
        let (_expr, errors) = parse("..5");
        assert!(
            errors
                .iter()
                .any(|e| e.code == V0001 && e.message.contains("`for`")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn inclusive_range_with_no_start_is_v0001() {
        let (_expr, errors) = parse("..=5");
        assert!(
            errors
                .iter()
                .any(|e| e.code == V0001 && e.message.contains("..=")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn calling_a_method_calls_result_is_v0002() {
        // `a.b()(c)`: the outer call's callee is a `MethodCall`, not a
        // `Path`, so it is rejected the same as any other non-callable
        // shape rather than being accepted.
        let (_expr, errors) = parse("a.b()(c)");
        assert!(errors.iter().any(|e| e.code == V0002), "errors: {errors:?}");
    }
}
