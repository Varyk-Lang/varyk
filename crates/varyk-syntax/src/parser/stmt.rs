//! Statement parsing: `let`, assignment, `return`, `while`, `break`,
//! `continue`, and expression statements (spec 4.1), plus
//! [`Parser::parse_block_body`], which `parser::expr`'s `parse_block` uses
//! for everything between a block's braces.

use super::Parser;
use crate::ast::{Expr, ExprKind, ForHead, Stmt};
use crate::error::{V0001, V0002};
use crate::token::TokenKind;

impl<'a> Parser<'a> {
    /// Everything between a block's braces: the statements, in order, and
    /// its optional tail expression. Never called in condition position;
    /// the caller (`parser::expr::Parser::parse_block`) already clears the
    /// flag around the whole body.
    pub(super) fn parse_block_body(&mut self) -> Result<(Vec<Stmt>, Option<Expr>), ()> {
        let mut stmts = Vec::new();
        let mut tail = None;
        while self.peek().is_some() && self.peek() != Some(&TokenKind::RBrace) {
            match self.peek() {
                Some(TokenKind::Let) => stmts.push(self.parse_let_stmt()?),
                Some(TokenKind::Return) => stmts.push(self.parse_return_stmt()?),
                Some(TokenKind::While) => stmts.push(self.parse_while_stmt()?),
                Some(TokenKind::For) => stmts.push(self.parse_for_stmt()?),
                Some(TokenKind::Break) => stmts.push(self.parse_break_stmt()?),
                Some(TokenKind::Continue) => stmts.push(self.parse_continue_stmt()?),
                _ => {
                    let expr = self.parse_expr()?;
                    if self.peek() == Some(&TokenKind::Eq) {
                        stmts.push(self.parse_assign_stmt(expr)?);
                        continue;
                    }
                    if self.peek() == Some(&TokenKind::Semi) {
                        let semi_span = self.current_span();
                        self.bump();
                        let span = self.span_from(expr.span, semi_span);
                        stmts.push(Stmt::Expr {
                            expr,
                            span,
                            has_semi: true,
                        });
                        continue;
                    }
                    if self.peek().is_none() || self.peek() == Some(&TokenKind::RBrace) {
                        tail = Some(expr);
                        break;
                    }
                    // No `;`, and not the block's last item: only a
                    // block-like expression (`if`, a bare block) can stand
                    // as a statement without one, since its own closing
                    // `}` already ends it visually.
                    if matches!(
                        expr.kind,
                        ExprKind::If { .. } | ExprKind::Block(_) | ExprKind::Match { .. }
                    ) {
                        let span = expr.span;
                        stmts.push(Stmt::Expr {
                            expr,
                            span,
                            has_semi: false,
                        });
                    } else {
                        let err_span = self.current_span();
                        self.push_error(V0002, err_span, "expected `;` after this expression");
                        return Err(());
                    }
                }
            }
        }
        Ok((stmts, tail))
    }

    fn parse_let_stmt(&mut self) -> Result<Stmt, ()> {
        let let_token = self.bump().expect("peek confirmed `let`");
        let mut_span = self.current_span();
        let mutable = self.bump_if(&TokenKind::Mut);
        let name = self.expect_identifier("a variable name")?;
        if mutable && name.name == "_" {
            self.push_error(V0002, mut_span, "`_` cannot be marked `mut`");
            return Err(());
        }
        let ty = if self.bump_if(&TokenKind::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect(TokenKind::Eq, "`=` in a `let` binding")?;
        let value = self.parse_expr()?;
        let semi = self.expect(TokenKind::Semi, "`;` after a `let` binding")?;
        let span = self.span_from(let_token.span, semi.span);
        Ok(Stmt::Let {
            name,
            mutable,
            ty,
            value,
            span,
        })
    }

    fn parse_return_stmt(&mut self) -> Result<Stmt, ()> {
        let return_token = self.bump().expect("peek confirmed `return`");
        let value = if self.peek() == Some(&TokenKind::Semi) {
            None
        } else {
            Some(self.parse_expr()?)
        };
        let semi = self.expect(TokenKind::Semi, "`;` after `return`")?;
        let span = self.span_from(return_token.span, semi.span);
        Ok(Stmt::Return { value, span })
    }

    /// `while cond { body }`. The condition is parsed in condition
    /// position, the same way `if`'s is (`parser::expr::parse_if`), so
    /// `while i < n { }` parses `i < n` as a comparison rather than
    /// treating `n { }` as a struct literal.
    fn parse_while_stmt(&mut self) -> Result<Stmt, ()> {
        let while_token = self.bump().expect("peek confirmed `while`");
        if self.peek() == Some(&TokenKind::Let) {
            let let_token = self.bump().expect("peek confirmed `let`");
            let span = self.span_from(while_token.span, let_token.span);
            self.push_error(
                V0001,
                span,
                "`while let` is not supported in Varyk; milestone 2 has no `while let`",
            );
            return Err(());
        }
        let was_in_condition = self.in_condition;
        self.in_condition = true;
        let cond = self.parse_expr();
        self.in_condition = was_in_condition;
        let cond = cond?;
        let body = self.parse_block()?;
        let span = self.span_from(while_token.span, body.span);
        Ok(Stmt::While { cond, body, span })
    }

    /// `for var in head { body }` (spec 2.4). `head` is parsed in
    /// condition position, like `while`'s condition, so `for x in xs { }`
    /// parses `xs` as the head rather than swallowing `{ }` as a struct
    /// literal.
    fn parse_for_stmt(&mut self) -> Result<Stmt, ()> {
        let for_token = self.bump().expect("peek confirmed `for`");
        let var = self.expect_identifier("a loop variable name")?;
        self.expect(TokenKind::In, "`in` after the loop variable")?;
        let was_in_condition = self.in_condition;
        self.in_condition = true;
        let head = self.parse_for_head();
        self.in_condition = was_in_condition;
        let head = head?;
        let body = self.parse_block()?;
        let span = self.span_from(for_token.span, body.span);
        Ok(Stmt::For {
            var,
            head,
            body,
            span,
        })
    }

    /// `a..b`, or a plain expression, right after `for var in`. Calls
    /// `parse_binary` directly (`parser::expr::Parser::parse_binary`)
    /// rather than `parse_expr`, so its stray-range check does not fire on
    /// the very range this function is meant to parse. `..=` is `V0001`
    /// naming milestone 4.
    fn parse_for_head(&mut self) -> Result<ForHead, ()> {
        self.for_head_leading_paren = self.peek() == Some(&TokenKind::LParen);
        let start = self.parse_binary(0)?;
        self.for_head_leading_paren = false;
        if self.peek() != Some(&TokenKind::DotDot) {
            return Ok(ForHead::Expr(Box::new(start)));
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
        let end = self.parse_binary(0)?;
        Ok(ForHead::Range {
            start: Box::new(start),
            end: Box::new(end),
        })
    }

    fn parse_break_stmt(&mut self) -> Result<Stmt, ()> {
        let break_token = self.bump().expect("peek confirmed `break`");
        let semi = self.expect(TokenKind::Semi, "`;` after `break`")?;
        let span = self.span_from(break_token.span, semi.span);
        Ok(Stmt::Break { span })
    }

    fn parse_continue_stmt(&mut self) -> Result<Stmt, ()> {
        let continue_token = self.bump().expect("peek confirmed `continue`");
        let semi = self.expect(TokenKind::Semi, "`;` after `continue`")?;
        let span = self.span_from(continue_token.span, semi.span);
        Ok(Stmt::Continue { span })
    }

    /// `target = value;`, called right after `target` has been parsed as
    /// an expression and the next token is `=`. `target` must be a place
    /// expression, a path (a variable), a field access, or an index (spec
    /// 2.6); anything else is `V0002`.
    fn parse_assign_stmt(&mut self, target: Expr) -> Result<Stmt, ()> {
        if !matches!(
            target.kind,
            ExprKind::Path { .. } | ExprKind::Field { .. } | ExprKind::Index { .. }
        ) {
            let span = target.span;
            self.push_error(
                V0002,
                span,
                "the left side of an assignment must be a variable, a field, or an index",
            );
            return Err(());
        }
        self.bump(); // `=`
        let value = self.parse_expr()?;
        let semi = self.expect(TokenKind::Semi, "`;` after an assignment")?;
        let span = self.span_from(target.span, semi.span);
        Ok(Stmt::Assign {
            target,
            value,
            span,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{BinaryOp, Block};
    use crate::error::SyntaxError;
    use crate::lex;
    use crate::source::SourceFile;
    use crate::span::FileId;

    fn parse_block(src: &str) -> (Result<Block, ()>, Vec<SyntaxError>) {
        let file = SourceFile::new(FileId(0), "test.vr", src);
        let (tokens, lex_errors) = lex(&file);
        assert!(
            lex_errors.is_empty(),
            "unexpected lex errors: {lex_errors:?}"
        );
        let mut parser = Parser::new(&tokens, FileId(0));
        let block = parser.parse_block();
        (block, parser.errors().to_vec())
    }

    fn parse_block_ok(src: &str) -> Block {
        let (block, errors) = parse_block(src);
        assert!(
            errors.is_empty(),
            "unexpected errors parsing {src:?}: {errors:?}"
        );
        block.unwrap_or_else(|()| panic!("expected {src:?} to parse"))
    }

    #[test]
    fn let_binding() {
        let block = parse_block_ok("{ let x = 5; }");
        assert_eq!(block.stmts.len(), 1);
        match &block.stmts[0] {
            Stmt::Let {
                name, mutable, ty, ..
            } => {
                assert_eq!(name.name, "x");
                assert!(!mutable);
                assert!(ty.is_none());
            }
            other => panic!("expected a let, got {other:?}"),
        }
    }

    #[test]
    fn reserved_keyword_as_let_name_is_v0001() {
        let (_, errors) = parse_block("{ let type = 1; }");
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].code, crate::error::V0001);
        assert_eq!(
            errors[0].message,
            "`type` is a Rust keyword and cannot be used as a name in Varyk"
        );
    }

    #[test]
    fn let_mut_discard_is_v0002_at_mut() {
        let (_, errors) = parse_block("{ let mut _ = 0; }");
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].code, V0002);
        assert_eq!(errors[0].message, "`_` cannot be marked `mut`");
        assert_eq!(errors[0].span.start, 6);
        assert_eq!(errors[0].span.end, 9);
    }

    #[test]
    fn let_mut_binding() {
        let block = parse_block_ok("{ let mut x = 5; }");
        match &block.stmts[0] {
            Stmt::Let { mutable, .. } => assert!(mutable),
            other => panic!("expected a let, got {other:?}"),
        }
    }

    #[test]
    fn let_with_type_annotation() {
        let block = parse_block_ok("{ let x: i64 = 5; }");
        match &block.stmts[0] {
            Stmt::Let { ty, .. } => assert_eq!(ty.as_ref().unwrap().name.name, "i64"),
            other => panic!("expected a let, got {other:?}"),
        }
    }

    #[test]
    fn assignment_to_variable() {
        let block = parse_block_ok("{ x = 1; }");
        match &block.stmts[0] {
            Stmt::Assign { target, .. } => match &target.kind {
                ExprKind::Path { name, .. } => assert_eq!(name.name, "x"),
                other => panic!("expected a path target, got {other:?}"),
            },
            other => panic!("expected an assignment, got {other:?}"),
        }
    }

    #[test]
    fn assignment_to_field() {
        let block = parse_block_ok(r#"{ user.name = "Bob"; }"#);
        match &block.stmts[0] {
            Stmt::Assign { target, .. } => match &target.kind {
                ExprKind::Field { name, .. } => assert_eq!(name.name, "name"),
                other => panic!("expected a field target, got {other:?}"),
            },
            other => panic!("expected an assignment, got {other:?}"),
        }
    }

    #[test]
    fn assignment_to_non_place_is_v0002() {
        let (block, errors) = parse_block("{ 1 + 1 = 2; }");
        assert!(block.is_err());
        assert!(errors.iter().any(|e| e.code == V0002), "errors: {errors:?}");
    }

    #[test]
    fn while_with_break_and_continue() {
        let block = parse_block_ok("{ while true { break; continue; } }");
        match &block.stmts[0] {
            Stmt::While { body, .. } => {
                assert_eq!(body.stmts.len(), 2);
                assert!(matches!(body.stmts[0], Stmt::Break { .. }));
                assert!(matches!(body.stmts[1], Stmt::Continue { .. }));
            }
            other => panic!("expected a while, got {other:?}"),
        }
    }

    #[test]
    fn while_condition_has_no_struct_literal() {
        // `while i < n { }` must parse `i < n` as a comparison followed by
        // an empty block, not `n { }` as a struct literal, mirroring
        // `if`'s rule (spec 4.1).
        let block = parse_block_ok("{ while i < n { } }");
        match &block.stmts[0] {
            Stmt::While { cond, body, .. } => {
                assert!(matches!(
                    cond.kind,
                    ExprKind::Binary {
                        op: BinaryOp::Lt,
                        ..
                    }
                ));
                assert!(body.stmts.is_empty() && body.tail.is_none());
            }
            other => panic!("expected a while, got {other:?}"),
        }
    }

    #[test]
    fn return_with_value() {
        let block = parse_block_ok("{ return 1; }");
        match &block.stmts[0] {
            Stmt::Return { value, .. } => assert!(value.is_some()),
            other => panic!("expected a return, got {other:?}"),
        }
    }

    #[test]
    fn return_without_value() {
        let block = parse_block_ok("{ return; }");
        match &block.stmts[0] {
            Stmt::Return { value, .. } => assert!(value.is_none()),
            other => panic!("expected a return, got {other:?}"),
        }
    }

    #[test]
    fn expression_statement_needs_semicolon() {
        let block = parse_block_ok("{ f(); 1 }");
        assert_eq!(block.stmts.len(), 1);
        assert!(matches!(block.stmts[0], Stmt::Expr { has_semi: true, .. }));
        assert!(block.tail.is_some());
    }

    #[test]
    fn missing_semicolon_after_expression_statement_is_v0002_and_stops() {
        let (block, errors) = parse_block("{ f() g() }");
        assert!(block.is_err());
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, V0002);
    }

    #[test]
    fn block_like_statement_needs_no_semicolon() {
        let block = parse_block_ok("{ if true { } while false { } 1 }");
        assert_eq!(block.stmts.len(), 2);
        assert!(matches!(
            block.stmts[0],
            Stmt::Expr {
                has_semi: false,
                ..
            }
        ));
        assert!(matches!(block.stmts[1], Stmt::While { .. }));
        assert!(block.tail.is_some());
    }

    #[test]
    fn match_statement_needs_no_semicolon() {
        // `match` may stand as a statement without `;`, the same as `if`
        // and a bare block (spec 2.3).
        let block = parse_block_ok("{ match x { _ => 1 } 2 }");
        assert_eq!(block.stmts.len(), 1);
        assert!(matches!(
            block.stmts[0],
            Stmt::Expr {
                has_semi: false,
                ..
            }
        ));
        assert!(block.tail.is_some());
    }

    // --- `for` and ranges (spec 2.4) ----------------------------------------

    #[test]
    fn for_over_a_range() {
        let for_src = "for i in 0..n { }";
        let block = parse_block_ok(&format!("{{ {for_src} }}"));
        match &block.stmts[0] {
            Stmt::For {
                var,
                head,
                body,
                span,
            } => {
                assert_eq!(var.name, "i");
                // The node spans the whole `for` statement, from `for` to
                // the body's closing `}`.
                assert_eq!(span.start, 2);
                assert_eq!(span.end, 2 + for_src.len() as u32);
                match head {
                    ForHead::Range { start, end } => {
                        assert!(matches!(start.kind, ExprKind::Integer(_)));
                        assert!(matches!(end.kind, ExprKind::Path { .. }));
                    }
                    other => panic!("expected a range head, got {other:?}"),
                }
                assert!(body.stmts.is_empty() && body.tail.is_none());
            }
            other => panic!("expected a for, got {other:?}"),
        }
    }

    #[test]
    fn for_over_a_name() {
        let block = parse_block_ok("{ for item in items { } }");
        match &block.stmts[0] {
            Stmt::For { var, head, .. } => {
                assert_eq!(var.name, "item");
                match head {
                    ForHead::Expr(expr) => assert!(matches!(expr.kind, ExprKind::Path { .. })),
                    other => panic!("expected an expression head, got {other:?}"),
                }
            }
            other => panic!("expected a for, got {other:?}"),
        }
    }

    #[test]
    fn for_condition_has_no_struct_literal() {
        // `for x in xs { }` must parse `xs` as the head followed by an
        // empty block, not `xs { }` as a struct literal, mirroring
        // `while`'s rule.
        let block = parse_block_ok("{ for x in xs { } }");
        match &block.stmts[0] {
            Stmt::For { body, .. } => assert!(body.stmts.is_empty() && body.tail.is_none()),
            other => panic!("expected a for, got {other:?}"),
        }
    }

    #[test]
    fn for_head_inclusive_range_is_v0001() {
        let (block, errors) = parse_block("{ for i in 0..=n { } }");
        assert!(block.is_err());
        assert!(
            errors
                .iter()
                .any(|e| e.code == V0001 && e.message.contains("..=")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn for_head_range_in_parens_says_to_drop_them() {
        let (block, errors) = parse_block("{ for i in (0..3) { } }");
        assert!(block.is_err());
        assert!(
            errors
                .iter()
                .any(|e| e.code == V0001 && e.message.contains("drop the parentheses")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn while_let_is_v0001() {
        let (block, errors) = parse_block("{ while let Some(x) = y { } }");
        assert!(block.is_err());
        assert!(
            errors
                .iter()
                .any(|e| e.code == V0001 && e.message.contains("while let")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn assignment_to_index() {
        let block = parse_block_ok("{ v[0] = 1; }");
        match &block.stmts[0] {
            Stmt::Assign { target, .. } => {
                assert!(matches!(target.kind, ExprKind::Index { .. }))
            }
            other => panic!("expected an assignment, got {other:?}"),
        }
    }
}
