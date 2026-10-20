use otter_common::Span;
use otter_lexer::{Lexer, StringPiece, Token, TokenKind};

use crate::ast::{BinOp, Expr, InterpPart, Literal, StringLit, TypeExpr, UnOp};
use crate::error::ParseError;
use crate::parser::{cover, Parser, MAX_NESTING_DEPTH};

impl<'src> Parser<'src> {
    pub fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_and()?;
        while self.eat(TokenKind::OrOr) {
            let rhs = self.parse_and()?;
            let span = cover(expr_span(&lhs), expr_span(&rhs));
            lhs = Expr::Binary { op: BinOp::Or, lhs: Box::new(lhs), rhs: Box::new(rhs), span };
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_eq()?;
        while self.eat(TokenKind::AndAnd) {
            let rhs = self.parse_eq()?;
            let span = cover(expr_span(&lhs), expr_span(&rhs));
            lhs = Expr::Binary { op: BinOp::And, lhs: Box::new(lhs), rhs: Box::new(rhs), span };
        }
        Ok(lhs)
    }

    fn parse_eq(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_cmp()?;
        loop {
            let op = if self.eat(TokenKind::EqEq) { Some(BinOp::Eq) }
                else if self.eat(TokenKind::BangEq) { Some(BinOp::Ne) }
                else { None };
            match op {
                Some(op) => {
                    let rhs = self.parse_cmp()?;
                    let span = cover(expr_span(&lhs), expr_span(&rhs));
                    lhs = Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs), span };
                }
                None => break,
            }
        }
        Ok(lhs)
    }

    fn parse_cmp(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_add()?;
        loop {
            let op = if self.eat(TokenKind::Lt) { Some(BinOp::Lt) }
                else if self.eat(TokenKind::LtEq) { Some(BinOp::Le) }
                else if self.eat(TokenKind::Gt) { Some(BinOp::Gt) }
                else if self.eat(TokenKind::GtEq) { Some(BinOp::Ge) }
                else { None };
            match op {
                Some(op) => {
                    let rhs = self.parse_add()?;
                    let span = cover(expr_span(&lhs), expr_span(&rhs));
                    lhs = Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs), span };
                }
                None => break,
            }
        }
        Ok(lhs)
    }

    fn parse_add(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_mul()?;
        loop {
            let op = if self.eat(TokenKind::Plus) { Some(BinOp::Add) }
                else if self.eat(TokenKind::Minus) { Some(BinOp::Sub) }
                else { None };
            match op {
                Some(op) => {
                    let rhs = self.parse_mul()?;
                    let span = cover(expr_span(&lhs), expr_span(&rhs));
                    lhs = Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs), span };
                }
                None => break,
            }
        }
        Ok(lhs)
    }

    fn parse_mul(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_unary()?;
        loop {
            let op = if self.eat(TokenKind::Star) { Some(BinOp::Mul) }
                else if self.eat(TokenKind::Slash) { Some(BinOp::Div) }
                else if self.eat(TokenKind::Percent) { Some(BinOp::Rem) }
                else { None };
            match op {
                Some(op) => {
                    let rhs = self.parse_unary()?;
                    let span = cover(expr_span(&lhs), expr_span(&rhs));
                    lhs = Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs), span };
                }
                None => break,
            }
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        if self.check(TokenKind::Minus) {
            let minus_span = self.peek_span();
            self.advance();
            // `-9223372036854775808` is the one integer a leading minus makes
            // readable that the digits alone are not: the digits are one past
            // `i64::MAX`, so the literal under the minus cannot stand on its own
            // and `i64::MIN` would have no spelling at all. Folding the minus
            // into the literal here is what gives it one. Every other
            // out-of-range magnitude is left to `parse_primary`, which reports
            // it; and a magnitude that fits stays a negation of that magnitude,
            // because that is what the reader wrote.
            if self.check(TokenKind::I64) {
                let digits_span = self.peek_span();
                if self.text(digits_span)?.parse::<i64>().is_err() {
                    if let Ok(magnitude) = self.text(digits_span)?.parse::<u64>() {
                        if magnitude == i64::MAX as u64 + 1 {
                            self.advance();
                            let span = cover(minus_span, digits_span);
                            return Ok(Expr::Literal(Literal::I64(i64::MIN), span));
                        }
                    }
                }
            }
            let operand = self.parse_unary()?;
            let span = cover(minus_span, expr_span(&operand));
            return Ok(Expr::Unary { op: UnOp::Neg, operand: Box::new(operand), span });
        }
        if self.eat(TokenKind::Bang) {
            let operand = self.parse_unary()?;
            let span = expr_span(&operand);
            return Ok(Expr::Unary { op: UnOp::Not, operand: Box::new(operand), span });
        }
        let base = self.parse_cast()?;
        Ok(base)
    }

    fn parse_move(&mut self) -> Result<Expr, ParseError> {
        if self.check(TokenKind::Move) {
            let move_span = self.peek_span();
            self.advance();
            let mut operand = self.parse_postfix()?;
            // A dotted name consumed as a single `Ident` is rewritten into a
            // `FieldAccess` chain here so that `move` (which is looser than
            // postfix `.`) wraps the field access rather than the whole dotted
            // path collapsing into one identifier.
            if let Expr::Ident(path, span) = operand {
                if path.len() > 1 {
                    let mut base = Expr::Ident(vec![path[0].clone()], span);
                    for seg in path.iter().skip(1) {
                        base = Expr::FieldAccess {
                            base: Box::new(base),
                            field: seg.clone(),
                            span,
                        };
                    }
                    operand = base;
                } else {
                    operand = Expr::Ident(path, span);
                }
            }
            let span = cover(move_span, expr_span(&operand));
            return Ok(Expr::Move(Box::new(operand), span));
        }
        self.parse_postfix()
    }

    fn parse_cast(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_move()?;
        while self.eat(TokenKind::As) {
            let ty = self.parse_type_expr()?;
            let span = cover(expr_span(&expr), ty_span(&ty));
            expr = Expr::Cast { expr: Box::new(expr), ty, span };
        }
        Ok(expr)
    }

    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_primary()?;
        expr = self.parse_postfix_rest(expr)?;
        Ok(expr)
    }

    fn parse_postfix_rest(&mut self, mut expr: Expr) -> Result<Expr, ParseError> {
        loop {
            if self.check(TokenKind::Question) {
                let q = self.advance();
                let span = cover(expr_span(&expr), q.span);
                expr = Expr::Try { expr: Box::new(expr), span };
                continue;
            }
            if self.check(TokenKind::Dot) {
                self.advance();
                if self.check(TokenKind::Ident) {
                    let (field, fspan) = self.identifier()?;
                    // If followed by '(', convert to call if possible
                    if self.check(TokenKind::LParen) {
                        // build callee path
                        // expr must be Ident or FieldAccess chain; flatten
                        let mut callee = Vec::new();
                        // collect from expr
                        collect_callee(&expr, &mut callee);
                        callee.push(field.clone());
                        // parse args
                        let open = self.expect(TokenKind::LParen)?;
                        let _ = self.push_open(TokenKind::LParen, open.span)?;
                        let mut args = Vec::new();
                        while !self.check(TokenKind::RParen) && !self.at_eof() {
                            args.push(self.parse_expr()?);
                            if !self.eat(TokenKind::Comma) { break; }
                        }
                        let close = self.expect(TokenKind::RParen)?;
                        self.close_open(TokenKind::RParen);
                        let span = cover(expr_span(&expr), close.span);
                        let _ = fspan;
                        expr = Expr::Call { callee, args, unsafe_call: false, span };
                        continue;
                    }
                    let span = cover(expr_span(&expr), fspan);
                    expr = Expr::FieldAccess { base: Box::new(expr), field, span };
                    continue;
                }
                break;
            }
            if self.check(TokenKind::LParen) {
                // call on ident path
                let mut callee = Vec::new();
                if !collect_callee(&expr, &mut callee) {
                    break;
                }
                let open = self.expect(TokenKind::LParen)?;
                let _ = self.push_open(TokenKind::LParen, open.span)?;
                let mut args = Vec::new();
                while !self.check(TokenKind::RParen) && !self.at_eof() {
                    args.push(self.parse_expr()?);
                    if !self.eat(TokenKind::Comma) { break; }
                }
                let close = self.expect(TokenKind::RParen)?;
                self.close_open(TokenKind::RParen);
                let span = cover(expr_span(&expr), close.span);
                expr = Expr::Call { callee, args, unsafe_call: false, span };
                continue;
            }
            if self.check(TokenKind::LBracket) {
                let open = self.expect(TokenKind::LBracket)?;
                let _ = self.push_open(TokenKind::LBracket, open.span)?;
                let index = self.parse_expr()?;
                let close = self.expect(TokenKind::RBracket)?;
                self.close_open(TokenKind::RBracket);
                let span = cover(expr_span(&expr), close.span);
                expr = Expr::Index { base: Box::new(expr), index: Box::new(index), span };
                continue;
            }
            break;
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        match self.peek() {
            TokenKind::I64 => {
                let tok = self.advance();
                let val = self.integer_literal(tok.span)?;
                Ok(Expr::Literal(Literal::I64(val), tok.span))
            }
            TokenKind::F64 => {
                let tok = self.advance();
                let val = self.float_literal(tok.span)?;
                Ok(Expr::Literal(Literal::F64(val), tok.span))
            }
            TokenKind::Str => {
                let tok = self.advance();
                let raw = self.text(tok.span)?;
                self.parse_string_literal(&raw, tok.span)
            }
            TokenKind::True => {
                let tok = self.advance();
                Ok(Expr::Literal(Literal::Bool(true), tok.span))
            }
            TokenKind::False => {
                let tok = self.advance();
                Ok(Expr::Literal(Literal::Bool(false), tok.span))
            }
            TokenKind::LBracket => {
                // `[]T { a, b }`. The brackets are empty and the element type
                // follows, so an empty literal is typed without inference.
                let open = self.advance();
                let _ = self.push_open(TokenKind::LBracket, open.span)?;
                let _ = self.expect(TokenKind::RBracket)?;
                self.close_open(TokenKind::RBracket);
                let elem = self.parse_type_expr()?;
                let brace = self.expect(TokenKind::LBrace)?;
                let _ = self.push_open(TokenKind::LBrace, brace.span)?;
                let mut elements = Vec::new();
                while !self.check(TokenKind::RBrace) && !self.at_eof() {
                    elements.push(self.parse_expr()?);
                    if !self.eat(TokenKind::Comma) {
                        break;
                    }
                }
                let close = self.expect(TokenKind::RBrace)?;
                self.close_open(TokenKind::RBrace);
                Ok(Expr::SliceLit {
                    elem: Box::new(elem),
                    elements,
                    span: cover(open.span, close.span),
                })
            }
            TokenKind::Switch => {
                // switch expression
                let kw = self.advance();
                let subject = self.parse_expr()?;
                let (arms, close_span) = self.parse_switch_arms_for_expr()?;
                let span = cover(kw.span, close_span);
                Ok(Expr::SwitchExpr { subject: Box::new(subject), arms, span })
            }
            TokenKind::LParen => {
                let open = self.expect(TokenKind::LParen)?;
                let _ = self.push_open(TokenKind::LParen, open.span)?;
                if self.eat(TokenKind::RParen) {
                    self.close_open(TokenKind::RParen);
                    // unit not an expr; return? error
                    return Err(self.error(open.span, "expected an expression".to_string()));
                }
                let e = self.parse_expr()?;
                self.expect(TokenKind::RParen)?;
                self.close_open(TokenKind::RParen);
                Ok(e)
            }
            TokenKind::Ident => {
                let (first, first_span) = self.identifier()?;
                let mut path = vec![first];
                let mut end = first_span;
                while self.check(TokenKind::Dot) {
                    self.advance();
                    if self.check(TokenKind::Ident) {
                        let (seg, sspan) = self.identifier()?;
                        path.push(seg);
                        end = sspan;
                    } else { break; }
                }
                // struct literal? Only treat `{ ... }` as a struct literal when it
                // actually looks like one (`{ ident: ... }`); otherwise a `{`
                // belongs to whatever surrounds this expression -- the body of an
                // `if`/`while`/loop, the arms of a `switch`, an arm body.
                //
                // The empty form `{}` is deliberately *not* a struct literal.
                // Every struct declares at least one field, so a zero-field
                // literal is never a program -- while `{}` after a bare name is
                // exactly how an empty body is spelled. It used to be accepted as
                // a literal, so `while running { }` parsed as `running {}` and
                // then failed with "expected `{`, found `}`" pointing at the end
                // of the enclosing function: the same bug for `if flag { }` and
                // `for i in 0..n { }`, and for an empty `switch` arm body.
                let looks_like_struct_lit = self.check(TokenKind::LBrace)
                    && self.peek_second() == TokenKind::Ident
                    && self.peek_third() == TokenKind::Colon;
                if looks_like_struct_lit {
                    let open = self.expect(TokenKind::LBrace)?;
                    let _ = self.push_open(TokenKind::LBrace, open.span)?;
                    let mut fields = Vec::new();
                    while !self.check(TokenKind::RBrace) && !self.at_eof() {
                        let (fname, fspan) = self.identifier()?;
                        self.expect(TokenKind::Colon)?;
                        let val = self.parse_expr()?;
                        fields.push((fname, val, fspan));
                        if !self.eat(TokenKind::Comma) { break; }
                    }
                    let close = self.expect(TokenKind::RBrace)?;
                    self.close_open(TokenKind::RBrace);
                    let span = cover(first_span, close.span);
                    return Ok(Expr::StructLit { path, fields, span });
                }
                let span = cover(first_span, end);
                Ok(Expr::Ident(path, span))
            }
            TokenKind::Unsafe => {
                let unsafe_tok = self.advance();
                let (first, start_span) = self.identifier()?;
                let mut callee = vec![first];
                let mut end = start_span;
                while self.check(TokenKind::Dot) {
                    self.advance();
                    if self.check(TokenKind::Ident) {
                        let (seg, sspan) = self.identifier()?;
                        callee.push(seg);
                        end = sspan;
                    } else { break; }
                }
                let _ = end;
                let open = self.expect(TokenKind::LParen)?;
                let _ = self.push_open(TokenKind::LParen, open.span)?;
                let mut args = Vec::new();
                while !self.check(TokenKind::RParen) && !self.at_eof() {
                    args.push(self.parse_expr()?);
                    if !self.eat(TokenKind::Comma) { break; }
                }
                let close = self.expect(TokenKind::RParen)?;
                self.close_open(TokenKind::RParen);
                let span = cover(unsafe_tok.span, close.span);
                let _ = start_span;
                Ok(Expr::Call { callee, args, unsafe_call: true, span })
            }
            _ => Err(self.error(self.peek_span(), "expected an expression".to_string())),
        }
    }

    fn parse_switch_arms_for_expr(&mut self) -> Result<(Vec<crate::ast::SwitchArm>, Span), ParseError> {
        // reuse stmts method? call parse_switch_arms? That's private in stmts impl but same trait. We'll duplicate minimal.
        let open = self.expect(TokenKind::LBrace)?;
        let _ = self.push_open(TokenKind::LBrace, open.span)?;
        let mut arms = Vec::new();
        while !self.check(TokenKind::RBrace) && !self.at_eof() {
            let is_default = if self.eat(TokenKind::Default) { true } else { self.expect(TokenKind::Case)?; false };
            let pattern = if is_default {
                crate::ast::Pattern::Wildcard(self.previous_span())
            } else {
                self.parse_pattern()?
            };
            let body = self.parse_switch_arm_body_expr()?;
            let arm_span = cover(pattern_span(&pattern), expr_span(&body));
            arms.push(crate::ast::SwitchArm { pattern, body, is_default, span: arm_span });
        }
        let close = self.expect(TokenKind::RBrace)?;
        self.close_open(TokenKind::RBrace);
        Ok((arms, close.span))
    }

    fn parse_switch_arm_body_expr(&mut self) -> Result<Expr, ParseError> {
        if self.check(TokenKind::LBrace) {
            let open = self.expect(TokenKind::LBrace)?;
            let _ = self.push_open(TokenKind::LBrace, open.span)?;
            let expr = self.parse_expr()?;
            let _ = self.eat(TokenKind::Semi);
            self.expect(TokenKind::RBrace)?;
            self.close_open(TokenKind::RBrace);
            Ok(expr)
        } else {
            self.parse_expr()
        }
    }

    /// Builds the expression for a `Str` token: a plain literal, or an
    /// interpolation when the text contains `${ ... }`.
    ///
    /// `raw` is the token's text, quotes included. The split is
    /// [`otter_lexer::split_interpolation`]'s job, so the escape rules and the
    /// `${ ... }` shape are defined once, in the lexer, and this reads the same
    /// pieces the lexer accepted the literal on.
    fn parse_string_literal(&mut self, raw: &str, span: Span) -> Result<Expr, ParseError> {
        let pieces = match otter_lexer::split_interpolation(raw) {
            Ok(pieces) => pieces,
            // The lexer accepts a literal only when the split agrees, so a
            // failure here means the token did not come from this file. An empty
            // literal is the honest fallback: wrong, but not a guess at text.
            Err(_) => return Ok(Expr::Literal(Literal::Str(empty_string()), span)),
        };

        let mut parts: Vec<InterpPart> = Vec::new();
        let mut has_expr = false;
        for piece in pieces {
            match piece {
                StringPiece::Text(lit) => parts.push(InterpPart::Text(lit)),
                StringPiece::Interpolation { start, end } => {
                    has_expr = true;
                    if start == end {
                        return Err(self.error(
                            sub_span(span, start.saturating_sub(2), end.saturating_add(1)),
                            "this `${}` is empty: put an expression between `${` and `}`",
                        ));
                    }
                    let expr = self.parse_interpolated_expr(span, start, end)?;
                    parts.push(InterpPart::Expr(expr));
                }
            }
        }

        if !has_expr {
            // A literal with no `${ ... }` is exactly one text piece (or none,
            // for `""`), so its value is the concatenation of what is there.
            let mut value = String::new();
            for part in &parts {
                if let InterpPart::Text(lit) = part {
                    value.push_str(&lit.value);
                }
            }
            return Ok(Expr::Literal(Literal::Str(str_lit(value)), span));
        }
        Ok(Expr::Interp { parts, span })
    }

    /// Parses the source of one `${ ... }` as a full expression.
    ///
    /// The expression's text is a byte range inside the literal, so it is lexed
    /// on its own and the resulting tokens are shifted back to their true file
    /// offsets before a sub-parser reads them. The sub-parser borrows the same
    /// source map and file id, so a name inside the interpolation is read out of
    /// the file exactly as one outside it is, and every span it reports points
    /// at the real text.
    fn parse_interpolated_expr(
        &mut self,
        token_span: Span,
        rel_start: usize,
        rel_end: usize,
    ) -> Result<Expr, ParseError> {
        let base = usize::try_from(token_span.start).unwrap_or(usize::MAX);
        let start = base.saturating_add(rel_start);
        let end = base.saturating_add(rel_end);
        let region = sub_span(token_span, rel_start, rel_end);

        // Interpolation nests the *parser* as well as the input: each `${ ... }`
        // is read by a fresh sub-parser, whose delimiter stack starts empty. So
        // the shared delimiter limit cannot bound this, and a literal like
        // `"${"${"${...}"}"}"` would recurse until the machine stack ran out --
        // an abort, which is the one failure a compiler for hostile input must
        // not have. Count interpolation levels explicitly instead.
        if self.interpolation_depth() >= MAX_NESTING_DEPTH {
            return Err(crate::error::nested_too_deep(region));
        }

        let text = self.source().file(self.file());
        let sub = match text.get(start..end) {
            Some(sub) => sub,
            None => {
                return Err(self.error(region, "this interpolation could not be read from the source file"));
            }
        };

        let shift = u32::try_from(start).unwrap_or(u32::MAX);
        let tokens = match Lexer::with_file(sub, self.file()).tokenize() {
            Ok(tokens) => tokens,
            Err(diags) => {
                let failure = match diags.first() {
                    Some(d) => ParseError::new(shift_span(d.span, shift), d.message.clone()),
                    None => self.error(region, "this interpolation could not be lexed"),
                };
                return Err(failure);
            }
        };
        let tokens: Vec<Token> = tokens
            .into_iter()
            .map(|t| Token { kind: t.kind, span: shift_span(t.span, shift) })
            .collect();

        let mut sub_parser = self.sub_parser(tokens);
        let expr = sub_parser.parse_expr()?;
        if !sub_parser.at_eof() {
            // The interpolation's `}` was found by the lexer, but there is more
            // than one expression before it (`${a b}`). Point at the token that
            // should have been the `}`.
            return Err(crate::error::expected(
                TokenKind::RBrace,
                sub_parser.peek(),
                sub_parser.peek_span(),
            ));
        }
        Ok(expr)
    }
}

// collect callee path from an expression
fn collect_callee(expr: &Expr, out: &mut Vec<String>) -> bool {
    match expr {
        Expr::Ident(path, _) => { out.extend(path.clone()); true }
        Expr::FieldAccess { base, field, .. } => collect_callee(base, out)
            && {
                out.push(field.clone());
                true
            },
        Expr::Index { base, .. } => collect_callee(base, out),
        _ => false,
    }
}

/// The span of `[rel_start, rel_end)` bytes into a token that begins at `token`.
///
/// Interpolation pieces carry offsets relative to the literal, which is the
/// only frame the lexer's scanner works in; a diagnostic needs file offsets, so
/// the conversion happens in one place rather than at every use.
fn sub_span(token: Span, rel_start: usize, rel_end: usize) -> Span {
    let to_u32 = |rel: usize| {
        token
            .start
            .saturating_add(u32::try_from(rel).unwrap_or(u32::MAX))
    };
    Span::new(to_u32(rel_start), to_u32(rel_end))
}

/// Moves a span by `by` bytes, for tokens lexed from a slice of the file.
fn shift_span(span: Span, by: u32) -> Span {
    Span::new(span.start.saturating_add(by), span.end.saturating_add(by))
}

/// The empty string literal, for a token whose text cannot be read back.
fn empty_string() -> StringLit {
    str_lit(String::new())
}

/// A [`StringLit`] from its decoded text, keeping `value` and `bytes` in step.
fn str_lit(value: String) -> StringLit {
    let bytes = value.as_bytes().to_vec();
    StringLit { value, bytes }
}

/// The span of `expr` as it was written.
///
/// Public because the span of an expression node is not recoverable from the
/// outside: every `Expr` variant carries its own span, in a different field
/// position, so a caller that has an `Expr` and needs to point a diagnostic at
/// it -- the type checker does, for an assignment target it refuses -- would
/// otherwise have to re-list every variant and drift out of step the next time
/// one is added.
pub fn expr_span(expr: &Expr) -> Span {
    match expr {
        Expr::Literal(_, s) => *s,
        Expr::Ident(_, s) => *s,
        Expr::Binary { span, .. } => *span,
        Expr::Unary { span, .. } => *span,
        Expr::Move(_, span) => *span,
        Expr::Call { span, .. } => *span,
        Expr::StructLit { span, .. } => *span,
        Expr::SliceLit { span, .. } => *span,
        Expr::FieldAccess { span, .. } => *span,
        Expr::Index { span, .. } => *span,
        Expr::Interp { span, .. } => *span,
        Expr::Cast { span, .. } => *span,
        Expr::Try { span, .. } => *span,
        Expr::SwitchExpr { span, .. } => *span,
    }
}

pub(crate) fn ty_span(ty: &TypeExpr) -> Span {
    match ty {
        TypeExpr::Name(_, span)
        | TypeExpr::Option(_, span)
        | TypeExpr::Ptr { span, .. }
        | TypeExpr::Slice(_, span)
        | TypeExpr::Unit(span) => *span,
        TypeExpr::Result(_, _, span) => *span,
        TypeExpr::Tuple(_, span) => *span,
    }
}

pub(crate) fn pattern_span(p: &crate::ast::Pattern) -> Span {
    use crate::ast::Pattern;
    match p {
        Pattern::Wildcard(s) => *s,
        Pattern::UnitVariant { span, .. } => *span,
        Pattern::TupleVariant { span, .. } => *span,
        Pattern::StructVariant { span, .. } => *span,
        Pattern::Binding { span, .. } => *span,
        Pattern::Literal(_, s) => *s,
    }
}

