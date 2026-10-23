//! Statements: `const`, `var`, `return`, `if`, `for`, `switch`, `break`,
//! `continue`, `defer`, assignment, and expression statements.
//!
//! `parse_block` lives here too, because a block is a list of statements and
//! the seam Task 4 left for it is closed by this module.

use yashy_common::Span;
use yashy_lexer::TokenKind;

use crate::ast::{BindingKeyword, BinOp, Block, Expr, Literal, Pattern, Stmt, SwitchArm};
use crate::error::ParseError;
use crate::parser::{cover, Parser};

impl<'src> Parser<'src> {
    pub fn parse_block(&mut self) -> Result<Block, ParseError> {
        let open = self.expect(TokenKind::LBrace)?;
        let _ = self.push_open(TokenKind::LBrace, open.span)?;

        let mut stmts = Vec::new();
        while !self.check(TokenKind::RBrace) && !self.at_eof() {
            if self.eat(TokenKind::Semi) {
                continue;
            }
            stmts.push(self.parse_stmt()?);
        }

        let close = self.expect(TokenKind::RBrace)?;
        self.close_open(TokenKind::RBrace);
        Ok(Block {
            stmts,
            span: cover(open.span, close.span),
        })
    }

    pub fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        match self.peek() {
            TokenKind::Const => self.parse_const(),
            TokenKind::Var => self.parse_var(),
            TokenKind::Let => self.parse_let(),
            TokenKind::Return => self.parse_return(),
            TokenKind::If => self.parse_if(),
            TokenKind::For => self.parse_for(),
            TokenKind::While => self.parse_while(),
            TokenKind::Switch => self.parse_switch_stmt(),
            TokenKind::Break => self.parse_break(),
            TokenKind::Continue => self.parse_continue(),
            TokenKind::Defer => self.parse_defer(),
            TokenKind::Ident if self.peek_second() == TokenKind::ColonEq => self.parse_binding_stmt(),
            _ => self.parse_expr_stmt(),
        }
    }

    fn parse_const(&mut self) -> Result<Stmt, ParseError> {
        let kw = self.expect(TokenKind::Const)?;
        if self.check(TokenKind::LParen) {
            return self.parse_destructure(BindingKeyword::Const, kw.span);
        }
        let (name, _) = self.identifier()?;
        let ty = if self.eat(TokenKind::Colon) {
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        self.expect(TokenKind::Eq)?;
        let init = self.parse_expr()?;
        let semi = self.expect(TokenKind::Semi)?;
        Ok(Stmt::Const {
            name,
            ty,
            init,
            span: cover(kw.span, semi.span),
        })
    }

    fn parse_var(&mut self) -> Result<Stmt, ParseError> {
        let kw = self.expect(TokenKind::Var)?;
        if self.check(TokenKind::LParen) {
            return self.parse_destructure(BindingKeyword::Var, kw.span);
        }
        let (name, _) = self.identifier()?;
        let ty = if self.eat(TokenKind::Colon) {
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        self.expect(TokenKind::Eq)?;
        let init = self.parse_expr()?;
        let semi = self.expect(TokenKind::Semi)?;
        Ok(Stmt::Var {
            name,
            ty,
            init,
            span: cover(kw.span, semi.span),
        })
    }

    fn parse_let(&mut self) -> Result<Stmt, ParseError> {
        let kw = self.expect(TokenKind::Let)?;
        if self.check(TokenKind::LParen) {
            return self.parse_destructure(BindingKeyword::Let, kw.span);
        }
        let (name, _) = self.identifier()?;
        let ty = if self.eat(TokenKind::Colon) {
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        self.expect(TokenKind::Eq)?;
        let init = self.parse_expr()?;
        let semi = self.expect(TokenKind::Semi)?;
        Ok(Stmt::Let {
            name,
            ty,
            init,
            span: cover(kw.span, semi.span),
        })
    }

    /// The rest of `const (a, b) = pair;` once the keyword is consumed.
    ///
    /// Only names and `_` are accepted: the elements are bound positionally,
    /// so there is nothing else a pattern could say here. The list must not be
    /// empty -- `const () = x;` has no binding to make and no tuple to
    /// destructure, since `()` is the unit value, not a zero-element tuple.
    fn parse_destructure(
        &mut self,
        kind: BindingKeyword,
        keyword_span: Span,
    ) -> Result<Stmt, ParseError> {
        let open = self.expect(TokenKind::LParen)?;
        let _ = self.push_open(TokenKind::LParen, open.span)?;
        let mut names = Vec::new();
        while !self.check(TokenKind::RParen) && !self.at_eof() {
            let span = self.peek_span();
            if self.check(TokenKind::Ident) {
                let (name, _) = self.identifier()?;
                names.push(if name == "_" { None } else { Some(name) });
            } else {
                return Err(self.error(
                    span,
                    "a destructuring pattern is a list of names: each element of the tuple binds \
                     to one name, or to `_` to skip it"
                        .to_string(),
                ));
            }
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        let close = self.expect(TokenKind::RParen)?;
        self.close_open(TokenKind::RParen);
        if names.is_empty() {
            return Err(self.error(
                cover(open.span, close.span),
                "a destructuring pattern needs at least one name".to_string(),
            ));
        }
        self.expect(TokenKind::Eq)?;
        let init = self.parse_expr()?;
        let semi = self.expect(TokenKind::Semi)?;
        Ok(Stmt::Destructure {
            kind,
            names,
            init,
            span: cover(keyword_span, semi.span),
        })
    }

    fn parse_binding_stmt(&mut self) -> Result<Stmt, ParseError> {
        let (name, name_span) = self.identifier()?;
        self.expect(TokenKind::ColonEq)?;
        let init = self.parse_expr()?;
        let span = cover(name_span, crate::exprs::expr_span(&init));
        // Consumed as part of a statement context that provides the semicolon,
        // e.g. for-loop init or if-binding. Standalone `x := y;` also works.
        // If a semicolon follows, consume it.
        if self.eat(TokenKind::Semi) {
            // span already; fine
        }
        Ok(Stmt::Const {
            name,
            ty: None,
            init,
            span,
        })
    }

    fn parse_return(&mut self) -> Result<Stmt, ParseError> {
        let kw = self.expect(TokenKind::Return)?;
        if self.check(TokenKind::Semi) {
            let semi = self.expect(TokenKind::Semi)?;
            return Ok(Stmt::Return(None, cover(kw.span, semi.span)));
        }
        let expr = self.parse_expr()?;
        let semi = self.expect(TokenKind::Semi)?;
        Ok(Stmt::Return(Some(expr), cover(kw.span, semi.span)))
    }

    fn parse_if(&mut self) -> Result<Stmt, ParseError> {
        let kw = self.expect(TokenKind::If)?;
        let init = if self.check(TokenKind::Ident) && self.peek_second() == TokenKind::ColonEq {
            Some(Box::new(self.parse_binding_stmt()?))
        } else {
            None
        };
        let cond = self.parse_expr()?;
        let then_block = self.parse_block()?;
        let else_block = if self.eat(TokenKind::Else) {
            Some(self.parse_block()?)
        } else {
            None
        };
        let end_span = match &else_block {
            Some(b) => b.span,
            None => then_block.span,
        };
        Ok(Stmt::If {
            init,
            cond,
            then_block,
            else_block,
            span: cover(kw.span, end_span),
        })
    }

    fn parse_for(&mut self) -> Result<Stmt, ParseError> {
        let kw = self.expect(TokenKind::For)?;
        // Range form: for <ident> in <expr> .. <expr> { }
        // Slice form: for <ident> in <expr> { } -- the head is the same up to
        // the token after the first expression.
        if self.check(TokenKind::Ident) && self.peek_second() == TokenKind::In {
            let (name, _) = self.identifier()?;
            self.expect(TokenKind::In)?;
            let first = self.parse_expr()?;
            if !self.check(TokenKind::DotDot) {
                let body = self.parse_block()?;
                let body_span = body.span;
                return Ok(Stmt::ForIn {
                    name,
                    iterable: first,
                    body,
                    span: cover(kw.span, body_span),
                });
            }
            self.expect(TokenKind::DotDot)?;
            let end = self.parse_expr()?;
            let body = self.parse_block()?;
            let body_span = body.span;
            return Ok(Stmt::ForRange {
                name,
                start: first,
                end,
                body,
                span: cover(kw.span, body_span),
            });
        }
        // C form
        let (init, semi_consumed) = if self.check(TokenKind::Semi) {
            (None, false)
        } else if self.check(TokenKind::Ident) && self.peek_second() == TokenKind::ColonEq {
            (Some(Box::new(self.parse_binding_stmt()?)), true)
        } else {
            let e = self.parse_expr()?;
            let es = crate::exprs::expr_span(&e);
            (Some(Box::new(Stmt::Expr(e, es))), false)
        };
        if !semi_consumed {
            self.expect(TokenKind::Semi)?;
        }
        let cond = if self.check(TokenKind::Semi) {
            None
        } else {
            Some(self.parse_expr()?)
        };
        self.expect(TokenKind::Semi)?;
        let update = if self.check(TokenKind::LBrace) {
            None
        } else {
            Some(self.parse_expr()?)
        };
        let body = self.parse_block()?;
        let body_span = body.span;
        Ok(Stmt::ForC {
            init,
            cond,
            update,
            body,
            span: cover(kw.span, body_span),
        })
    }

    /// `while <cond> { ... }` -- the condition-only loop.
    ///
    /// Sugar for the C form with no init and no update, `for ; <cond>; { }`.
    /// It earns its own keyword because that is what most loops actually are --
    /// "keep going while there is something to read" -- and `for ;;` with the
    /// exit buried in the body says the wrong thing twice: nothing about the
    /// loop is a `for`, and the condition is at the bottom rather than at the
    /// top, where a reader looks for it. The AST is unchanged: the range form
    /// and the C form already carry every loop the backend knows how to lower.
    fn parse_while(&mut self) -> Result<Stmt, ParseError> {
        let kw = self.expect(TokenKind::While)?;
        let cond = self.parse_expr()?;
        let body = self.parse_block()?;
        let body_span = body.span;
        Ok(Stmt::ForC {
            init: None,
            cond: Some(cond),
            update: None,
            body,
            span: cover(kw.span, body_span),
        })
    }

    fn parse_switch_stmt(&mut self) -> Result<Stmt, ParseError> {
        let kw = self.expect(TokenKind::Switch)?;
        let subject = self.parse_expr()?;
        let (arms, close_span) = self.parse_switch_arms()?;
        Ok(Stmt::Switch {
            subject,
            arms,
            span: cover(kw.span, close_span),
        })
    }

    fn parse_switch_arms(&mut self) -> Result<(Vec<SwitchArm>, Span), ParseError> {
        let open = self.expect(TokenKind::LBrace)?;
        let _ = self.push_open(TokenKind::LBrace, open.span)?;
        let mut arms = Vec::new();
        while !self.check(TokenKind::RBrace) && !self.at_eof() {
            let arm_start = self.peek_span();
            let is_default = if self.eat(TokenKind::Default) {
                true
            } else {
                self.expect(TokenKind::Case)?;
                false
            };
            let pattern = if is_default {
                Pattern::Wildcard(arm_start)
            } else {
                self.parse_pattern()?
            };
            // `if` cannot start anything else after a pattern, so it is
            // unambiguous here. The condition may be any boolean expression --
            // it is checked after the pattern's names are in scope, which is
            // what makes `case Some(v) if v > 0` work.
            let guard = if self.eat(TokenKind::If) {
                Some(self.parse_expr()?)
            } else {
                None
            };
            let body = self.parse_switch_arm_body()?;
            let arm_span = cover(crate::exprs::pattern_span(&pattern), crate::exprs::expr_span(&body));
            // extend to braces? body already includes. fine.
            let _ = arm_start;
            arms.push(SwitchArm {
                pattern,
                guard,
                body,
                is_default,
                span: arm_span,
            });
        }
        let close = self.expect(TokenKind::RBrace)?;
        self.close_open(TokenKind::RBrace);
        Ok((arms, close.span))
    }

    fn parse_switch_arm_body(&mut self) -> Result<Expr, ParseError> {
        if self.check(TokenKind::LBrace) {
            let open = self.expect(TokenKind::LBrace)?;
            let _ = self.push_open(TokenKind::LBrace, open.span)?;
            let expr = self.parse_expr()?;
            // A trailing `;` is allowed (and ignored) so that a statement-shaped
            // arm -- `case Some(v) { print(v); }` -- parses. The arm's value
            // is still the expression's value; only the punctuation changes.
            // Full multi-statement arm bodies need block expressions, which are
            // a larger change.
            let _ = self.eat(TokenKind::Semi);
            self.expect(TokenKind::RBrace)?;
            self.close_open(TokenKind::RBrace);
            Ok(expr)
        } else {
            self.parse_expr()
        }
    }

    pub(crate) fn parse_pattern(&mut self) -> Result<Pattern, ParseError> {
        // Wildcard
        if self.check(TokenKind::Ident) {
            let (name, span) = self.identifier()?;
            if name == "_" {
                return Ok(Pattern::Wildcard(span));
            }
            // Tuple variant
            if self.check(TokenKind::LParen) {
                let open = self.expect(TokenKind::LParen)?;
                let _ = self.push_open(TokenKind::LParen, open.span)?;
                let mut args = Vec::new();
                while !self.check(TokenKind::RParen) && !self.at_eof() {
                    args.push(self.parse_pattern()?);
                    if !self.eat(TokenKind::Comma) {
                        break;
                    }
                }
                let close = self.expect(TokenKind::RParen)?;
                self.close_open(TokenKind::RParen);
                return Ok(Pattern::TupleVariant {
                    path: vec![name],
                    args,
                    span: cover(span, close.span),
                });
            }
            // Struct variant: only when the brace actually opens a field list.
            // A unit variant followed by an arm body (`case None { 0 }`) also
            // sees `{` next, but its brace holds an expression, not
            // `field: pattern` pairs. Requiring `ident :` or `..` after the
            // `{` keeps the two apart without changing the AST.
            if self.check(TokenKind::LBrace)
                && (self.peek_second() == TokenKind::DotDot
                    || (self.peek_second() == TokenKind::Ident
                        && self.peek_third() == TokenKind::Colon))
            {
                let open = self.expect(TokenKind::LBrace)?;
                let _ = self.push_open(TokenKind::LBrace, open.span)?;
                let mut fields = Vec::new();
                while !self.check(TokenKind::RBrace) && !self.at_eof() {
                    if self.eat(TokenKind::DotDot) {
                        // rest pattern; ignore
                        break;
                    }
                    let (fname, fspan) = self.identifier()?;
                    self.expect(TokenKind::Colon)?;
                    let sub = self.parse_pattern()?;
                    fields.push((fname, sub, fspan));
                    if !self.eat(TokenKind::Comma) {
                        break;
                    }
                }
                let close = self.expect(TokenKind::RBrace)?;
                self.close_open(TokenKind::RBrace);
                return Ok(Pattern::StructVariant {
                    path: vec![name],
                    fields,
                    span: cover(span, close.span),
                });
            }
            // Unit variant vs binding: uppercase -> variant
            let is_variant = name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
            if is_variant {
                // dotted path support
                let mut path = vec![name];
                while self.eat(TokenKind::Dot) {
                    if self.check(TokenKind::Ident) {
                        let (seg, _) = self.identifier()?;
                        path.push(seg);
                    } else {
                        break;
                    }
                }
                return Ok(Pattern::UnitVariant { path, span });
            }
            return Ok(Pattern::Binding { name, span });
        }
        // Literal patterns
        if self.check(TokenKind::I64) {
            let tok = self.advance();
            let val = self.integer_literal(tok.span)?;
            return Ok(Pattern::Literal(Literal::I64(val), tok.span));
        }
        if self.check(TokenKind::F64) {
            let tok = self.advance();
            let val = self.float_literal(tok.span)?;
            return Ok(Pattern::Literal(Literal::F64(val), tok.span));
        }
        if self.check(TokenKind::True) {
            let tok = self.advance();
            return Ok(Pattern::Literal(Literal::Bool(true), tok.span));
        }
        if self.check(TokenKind::False) {
            let tok = self.advance();
            return Ok(Pattern::Literal(Literal::Bool(false), tok.span));
        }
        if self.check(TokenKind::Str) {
            let tok = self.advance();
            let raw = self.text(tok.span)?;
            let interpolated = yashy_lexer::split_interpolation(&raw)
                .map(|pieces| {
                    pieces
                        .iter()
                        .any(|p| matches!(p, yashy_lexer::StringPiece::Interpolation { .. }))
                })
                .unwrap_or(false);
            if interpolated {
                return Err(self.error(
                    tok.span,
                    "a `case` pattern cannot be a `${ ... }` string: a pattern is matched against a value, not built at run time",
                ));
            }
            let lit = yashy_lexer::StringLit::decode(&raw)
                .unwrap_or(yashy_lexer::StringLit { value: String::new(), bytes: Vec::new() });
            return Ok(Pattern::Literal(Literal::Str(lit), tok.span));
        }
        Err(self.expected_error(TokenKind::Ident))
    }

    fn parse_break(&mut self) -> Result<Stmt, ParseError> {
        let kw = self.expect(TokenKind::Break)?;
        let semi = self.expect(TokenKind::Semi)?;
        Ok(Stmt::Break(cover(kw.span, semi.span)))
    }

    fn parse_continue(&mut self) -> Result<Stmt, ParseError> {
        let kw = self.expect(TokenKind::Continue)?;
        let semi = self.expect(TokenKind::Semi)?;
        Ok(Stmt::Continue(cover(kw.span, semi.span)))
    }

    fn parse_defer(&mut self) -> Result<Stmt, ParseError> {
        let kw = self.expect(TokenKind::Defer)?;
        let body = self.parse_block()?;
        let body_span = body.span;
        Ok(Stmt::Defer {
            body,
            span: cover(kw.span, body_span),
        })
    }

    fn parse_expr_stmt(&mut self) -> Result<Stmt, ParseError> {
        let expr = self.parse_expr()?;
        let span = crate::exprs::expr_span(&expr);

        // `target = value;` -- the only place `=` appears in statement position.
        //
        // The left side is parsed as an expression first and only then re-read
        // as a target, rather than being recognised by its shape. That is what
        // lets the target be any expression the expression grammar can produce
        // without a second lookahead rule per target shape: `s.x = 1` needs no
        // parser change at all, now or when fields become assignable.
        //
        // `==` is a different token, so a comparison statement is untouched:
        // `x == 1;` still parses as an expression statement.
        // Compound assignment, `target op= value;`, is sugar for the plain
        // assignment of the operator applied to the target: `x += 1` *is*
        // `x = x + 1`, as one `Stmt::Assign` whose value is the binary.
        //
        // The desugar reuses the target expression unchanged -- same node, same
        // span -- so any target the expression grammar produces works here too,
        // and the checker never learns that `+=` exists: a target that cannot be
        // assigned gets the diagnostic the expanded form would get, because the
        // expanded form is what the checker sees.
        //
        // The five spelling tokens were already in the lexer and had no
        // consumer; this is the consumer.
        let compound = match self.peek() {
            TokenKind::PlusEq => Some(BinOp::Add),
            TokenKind::MinusEq => Some(BinOp::Sub),
            TokenKind::StarEq => Some(BinOp::Mul),
            TokenKind::SlashEq => Some(BinOp::Div),
            TokenKind::PercentEq => Some(BinOp::Rem),
            _ => None,
        };
        if let Some(op) = compound {
            self.advance();
            let rhs = self.parse_expr()?;
            let semi = self.expect(TokenKind::Semi)?;
            let value = Expr::Binary {
                op,
                lhs: Box::new(expr.clone()),
                rhs: Box::new(rhs),
                span,
            };
            return Ok(Stmt::Assign {
                target: expr,
                value,
                span: cover(span, semi.span),
            });
        }

        if self.check(TokenKind::Eq) {
            self.expect(TokenKind::Eq)?;
            let value = self.parse_expr()?;
            let semi = self.expect(TokenKind::Semi)?;
            return Ok(Stmt::Assign {
                target: expr,
                value,
                span: cover(span, semi.span),
            });
        }

        let semi = self.expect(TokenKind::Semi)?;
        Ok(Stmt::Expr(expr, cover(span, semi.span)))
    }

    // Shared switch parsing used by both statement and expression forms.
    // Returns (subject, arms, span) for expression? We'll implement separately.
}



