//! Expression parsing: a Pratt parser using Rust's binary-operator
//! precedence, plus the call/field/path postfix chain, struct literals,
//! blocks, `if`, and the `println!` intrinsic.

use super::Parser;
use crate::ast::{BinaryOp, Block, Expr, ExprKind, Ident, UnaryOp};
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
    /// already recorded in [`Parser::errors`].
    pub(crate) fn parse_expr(&mut self) -> Result<Expr, ()> {
        self.parse_binary(0)
    }

    /// Pratt-parses a binary expression: everything at binding power at
    /// least `min_bp`. Comparisons are non-associative: once one has been
    /// parsed at this level, seeing another comparison operator right
    /// after is `V0002`, not left-associative chaining.
    fn parse_binary(&mut self, min_bp: u8) -> Result<Expr, ()> {
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

    /// Postfix `.field`, `(args)`, applied left-to-right onto whatever
    /// primary expression starts the chain.
    fn parse_postfix(&mut self) -> Result<Expr, ()> {
        let mut expr = self.parse_primary()?;
        loop {
            match self.peek() {
                Some(TokenKind::Dot) => {
                    self.bump();
                    let name = self.expect_identifier("a field name after `.`")?;
                    let span = self.span_from(expr.span, name.span);
                    expr = Expr {
                        kind: ExprKind::Field {
                            base: Box::new(expr),
                            name,
                        },
                        span,
                    };
                }
                Some(TokenKind::LParen) => {
                    if !matches!(expr.kind, ExprKind::Path { .. }) {
                        self.push_error(
                            V0001,
                            expr.span,
                            "method calls are not supported in Varyk yet; call a function by its path instead",
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
                self.bump();
                // Parentheses disambiguate, so a struct literal is fine
                // again immediately inside them even in condition
                // position: `if (Point { x: 1 }).x == 1 { }`.
                let inner = self.without_condition(Self::parse_expr)?;
                self.expect(TokenKind::RParen, "`)`")?;
                Ok(inner)
            }

            Some(TokenKind::Identifier(name)) if self.peek_at(1) == Some(&TokenKind::Bang) => {
                let name = name.clone();
                self.parse_macro_like(name)
            }

            Some(TokenKind::Identifier(_)) => self.parse_path_or_struct_lit(),

            Some(TokenKind::If) => self.parse_if(),

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

    /// `name!(...)`: recognized only for `println!`; every other
    /// `name!` is `V0001` naming macros (spec 6.3, 4.1).
    fn parse_macro_like(&mut self, name: String) -> Result<Expr, ()> {
        let name_token = self.bump().expect("peek confirmed an identifier");
        let bang_token = self.bump().expect("peek confirmed a `!`");
        if name != "println" {
            let span = self.span_from(name_token.span, bang_token.span);
            self.push_error(
                V0001,
                span,
                format!("macro `{name}!` is not supported in Varyk; only `println!` is"),
            );
            return Err(());
        }
        let name_ident = Ident {
            name,
            span: name_token.span,
        };

        if self.peek() != Some(&TokenKind::LParen) {
            let span = self.current_span();
            self.push_error(V0002, span, "expected `(` after `println!`");
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
                    "expected a format string as `println!`'s first argument",
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

    /// A path (`name`, `module::name`, or, `V0001`-reported, a longer
    /// chain), optionally followed by a struct literal when the parser is
    /// not in condition position.
    fn parse_path_or_struct_lit(&mut self) -> Result<Expr, ()> {
        let mut segments = vec![self.expect_identifier("a name")?];
        while self.peek() == Some(&TokenKind::ColonColon) {
            self.bump();
            segments.push(self.expect_identifier("a name after `::`")?);
        }
        let path_span = self.span_from(
            segments[0].span,
            segments.last().expect("at least one segment").span,
        );
        if segments.len() > 2 {
            self.push_error(
                V0001,
                path_span,
                "nested module paths are not supported in Varyk; use a single `module::name` path",
            );
        }
        let name = segments.pop().expect("at least one segment");
        let had_module = !segments.is_empty();
        let module = if had_module {
            Some(segments.remove(0))
        } else {
            None
        };
        let path_expr = Expr {
            kind: ExprKind::Path { module, name },
            span: path_span,
        };

        if self.peek() == Some(&TokenKind::LBrace) && !self.in_condition {
            return self.parse_struct_lit(path_expr, had_module);
        }
        Ok(path_expr)
    }

    /// `Name { field: expr, ... }`, called right after `path` has been
    /// parsed as a `Path` and the next token is `{`. `had_module` is
    /// whether that path had a module segment, which milestone 1 does not
    /// allow on a struct literal (`V0001`); the fields still parse so the
    /// caller sees a complete AST either way.
    fn parse_struct_lit(&mut self, path: Expr, had_module: bool) -> Result<Expr, ()> {
        // Captured before `path.kind` is moved out below, so the node this
        // function returns can span the whole path, including a `module::`
        // prefix, not just the struct name.
        let path_span = path.span;
        let name = match path.kind {
            ExprKind::Path { name, .. } => name,
            _ => unreachable!("only called right after building a Path"),
        };
        if had_module {
            self.push_error(
                V0001,
                path_span,
                "struct literals are named without a module path in Varyk; module-qualified struct literals are not supported",
            );
        }
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
            kind: ExprKind::StructLit { name, fields },
            span,
        })
    }

    /// `if cond { ... }` with an optional `else { ... }` or `else if ...`.
    /// The condition is parsed in condition position so `if a == b { }`
    /// parses as a comparison followed by an empty block, not a struct
    /// literal named `b`.
    fn parse_if(&mut self) -> Result<Expr, ()> {
        let if_token = self.bump().expect("peek confirmed `if`");

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
                    ExprKind::Path { module, name } => {
                        assert_eq!(module.as_ref().unwrap().name, "greet");
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
    fn three_segment_path_is_v0001() {
        let (_expr, errors) = parse("a::b::c");
        assert!(errors.iter().any(|e| e.code == V0001));
    }

    #[test]
    fn method_call_syntax_is_v0001() {
        let (_expr, errors) = parse("x.f()");
        assert!(
            errors
                .iter()
                .any(|e| e.code == V0001 && e.message.contains("method calls")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn struct_literal_with_trailing_comma() {
        let expr = parse_ok("Point { x: 1, y: 2, }");
        match &expr.kind {
            ExprKind::StructLit { name, fields } => {
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
    fn module_qualified_struct_literal_is_v0001() {
        let src = "math::Point { }";
        let (expr, errors) = parse(src);
        assert!(errors.iter().any(|e| e.code == V0001), "errors: {errors:?}");
        // The node still spans the whole path, `math::Point { }`, not just
        // `Point { }`: the diagnostic points at the module prefix, but the
        // AST node it builds is not truncated.
        let expr = expr.unwrap_or_else(|()| panic!("V0001 must not stop parsing"));
        assert_eq!(expr.span.start, 0);
        assert_eq!(expr.span.end, src.len() as u32);
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
        let (_expr, errors) = parse(r#"vec!(1, 2)"#);
        assert!(
            errors
                .iter()
                .any(|e| e.code == V0001 && e.message.contains("`vec!`")),
            "errors: {errors:?}"
        );
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
        let (expr, errors) = parse("match");
        assert!(expr.is_err());
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, V0001);
        assert!(errors[0].message.contains("match"), "{}", errors[0].message);
    }
}
