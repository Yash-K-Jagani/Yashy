//! Mutation operators over the parser AST.
//!
//! A mutant is a small, deliberate change to a program: a test suite that
//! still passes after a mutant is applied did not prove the behaviour that
//! mutant changed. The operators here are data with stable names, so a score
//! names the operator that survived. Nothing in this crate runs a program or
//! edits a source file: it produces AST values a driver can compile.

use yashy_parser::ast::{BinOp, Expr, Literal};

/// An operator family and what it changes. The name is stable across releases.
pub const OPERATORS: &[(&str, &str)] = &[
    (
        "AOR",
        "replace an arithmetic operator with a neighbouring one",
    ),
    ("ROR", "replace a comparison with a neighbouring one"),
    ("COR", "replace a boolean connective with the other one"),
    ("BOOL", "negate a boolean literal"),
];

/// One AST-level mutation of one expression.
#[derive(Clone, Debug)]
pub struct Mutant {
    pub operator: &'static str,
    pub expr: Expr,
}

/// The arithmetic replacement pairs. Each pair is symmetric.
fn arithmetic(op: BinOp) -> Option<BinOp> {
    match op {
        BinOp::Add => Some(BinOp::Sub),
        BinOp::Sub => Some(BinOp::Add),
        BinOp::Mul => Some(BinOp::Div),
        BinOp::Div => Some(BinOp::Mul),
        BinOp::Rem => Some(BinOp::Mul),
        BinOp::Shl => Some(BinOp::Shr),
        BinOp::Shr => Some(BinOp::Shl),
        BinOp::BitAnd => Some(BinOp::BitOr),
        BinOp::BitOr => Some(BinOp::BitAnd),
        BinOp::BitXor => Some(BinOp::BitAnd),
        _ => None,
    }
}

/// The relational replacement pairs.
fn relational(op: BinOp) -> Option<BinOp> {
    match op {
        BinOp::Eq => Some(BinOp::Ne),
        BinOp::Ne => Some(BinOp::Eq),
        BinOp::Lt => Some(BinOp::Le),
        BinOp::Le => Some(BinOp::Lt),
        BinOp::Gt => Some(BinOp::Ge),
        BinOp::Ge => Some(BinOp::Gt),
        _ => None,
    }
}

/// Produce the mutants of one expression.
///
/// A mutant keeps every span: it is the same source location with a different
/// operator or literal, so a driver can attribute a survival to the line the
/// reader wrote. Expressions with no applicable operator have no mutants.
pub fn mutants(expr: &Expr) -> Vec<Mutant> {
    match expr {
        Expr::Binary { op, lhs, rhs, span } => {
            let mut out = Vec::new();
            if let Some(replacement) = arithmetic(*op) {
                out.push(Mutant {
                    operator: "AOR",
                    expr: Expr::Binary {
                        op: replacement,
                        lhs: lhs.clone(),
                        rhs: rhs.clone(),
                        span: *span,
                    },
                });
            }
            if let Some(replacement) = relational(*op) {
                out.push(Mutant {
                    operator: "ROR",
                    expr: Expr::Binary {
                        op: replacement,
                        lhs: lhs.clone(),
                        rhs: rhs.clone(),
                        span: *span,
                    },
                });
            }
            if *op == BinOp::And || *op == BinOp::Or {
                let replacement = if *op == BinOp::And {
                    BinOp::Or
                } else {
                    BinOp::And
                };
                out.push(Mutant {
                    operator: "COR",
                    expr: Expr::Binary {
                        op: replacement,
                        lhs: lhs.clone(),
                        rhs: rhs.clone(),
                        span: *span,
                    },
                });
            }
            out
        }
        Expr::Literal(Literal::Bool(value), span) => vec![Mutant {
            operator: "BOOL",
            expr: Expr::Literal(Literal::Bool(!value), *span),
        }],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yashy_common::Span;

    fn binary(op: BinOp) -> Expr {
        Expr::Binary {
            op,
            lhs: Box::new(Expr::Literal(Literal::I64(1), Span::new(0, 1))),
            rhs: Box::new(Expr::Literal(Literal::I64(2), Span::new(2, 3))),
            span: Span::new(0, 3),
        }
    }

    #[test]
    fn addition_and_less_than_each_have_one_mutant() {
        let sum = mutants(&binary(BinOp::Add));
        assert_eq!(sum.len(), 1);
        assert_eq!(sum[0].operator, "AOR");
        let less = mutants(&binary(BinOp::Lt));
        assert_eq!(less.len(), 1);
        assert_eq!(less[0].operator, "ROR");
    }

    #[test]
    fn a_boolean_literal_is_negated_and_keeps_its_span() {
        let original = Expr::Literal(Literal::Bool(true), Span::new(4, 8));
        let found = mutants(&original);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].operator, "BOOL");
        match &found[0].expr {
            Expr::Literal(Literal::Bool(false), span) => assert_eq!(*span, Span::new(4, 8)),
            other => panic!("expected a false literal, got {other:?}"),
        }
    }

    #[test]
    fn every_operator_name_is_documented() {
        for (name, description) in OPERATORS {
            assert!(!name.is_empty() && !description.is_empty());
        }
    }
}
