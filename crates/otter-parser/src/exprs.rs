use otter_common::Span;
use otter_lexer::TokenKind;

use crate::ast::{Expr, Literal, TypeExpr, UnOp, BinOp};
use crate::error::ParseError;
use crate::parser::{cover, Parser};

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
            break;
        }
        Ok(expr)
    }


    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        match self.peek() {
            TokenKind::I64 => {
                let tok = self.advance();
                let text = self.text(tok.span).unwrap_or_default();
                let val = text.parse::<i64>().unwrap_or(0);
                Ok(Expr::Literal(Literal::I64(val), tok.span))
            }
            TokenKind::F64 => {
                let tok = self.advance();
                let text = self.text(tok.span).unwrap_or_default();
                let val = text.parse::<f64>().unwrap_or(0.0);
                Ok(Expr::Literal(Literal::F64(val), tok.span))
            }
            TokenKind::Str => {
                let tok = self.advance();
                let raw = self.text(tok.span).unwrap_or_default();
                let lit = otter_lexer::StringLit::decode(&raw).unwrap_or(otter_lexer::StringLit { value: String::new(), bytes: Vec::new() });
                Ok(Expr::Literal(Literal::Str(lit), tok.span))
            }
            TokenKind::True => {
                let tok = self.advance();
                Ok(Expr::Literal(Literal::Bool(true), tok.span))
            }
            TokenKind::False => {
                let tok = self.advance();
                Ok(Expr::Literal(Literal::Bool(false), tok.span))
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
                // actually looks like one (`{}` or `{ ident: ... }`); otherwise a
                // `{` belongs to whatever surrounds this expression (e.g. the arms
                // of a `switch` whose subject is a bare identifier).
                let looks_like_struct_lit = self.check(TokenKind::LBrace)
                    && (self.peek_second() == TokenKind::RBrace
                        || (self.peek_second() == TokenKind::Ident
                            && self.peek_third() == TokenKind::Colon));
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
            self.expect(TokenKind::RBrace)?;
            self.close_open(TokenKind::RBrace);
            Ok(expr)
        } else {
            self.parse_expr()
        }
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
        _ => false,
    }
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
        Expr::FieldAccess { span, .. } => *span,
        Expr::Cast { span, .. } => *span,
        Expr::Try { span, .. } => *span,
        Expr::SwitchExpr { span, .. } => *span,
    }
}

pub(crate) fn ty_span(ty: &TypeExpr) -> Span {
    match ty {
        TypeExpr::Name(_, span)
        | TypeExpr::Option(_, span)
        | TypeExpr::Ptr(_, span)
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

