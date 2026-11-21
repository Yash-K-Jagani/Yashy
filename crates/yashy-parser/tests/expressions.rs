//! Range values and the spellings that must stay distinct from them.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::ast::{Expr, Item, Stmt};
use yashy_parser::Parser;

fn program(src: &str) -> yashy_parser::ast::Program {
    let mut sources = SourceMap::new();
    let file = sources.add_file("expressions.ya".to_string(), src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("must lex");
    Parser::new(tokens, &sources, file)
        .parse_program()
        .expect("must parse")
}

fn expression(src: &str) -> Expr {
    let parsed = program(src);
    let Item::Fn { body, .. } = parsed.items.into_iter().next().unwrap() else {
        panic!("expected a function");
    };
    match &body.stmts[0] {
        Stmt::Const { init, .. } => init.clone(),
        other => panic!("expected a const, got {other:?}"),
    }
}

#[test]
fn a_range_is_a_value_expression() {
    let value = expression("fn main() { const r = 0..10; }");
    match value {
        Expr::Range {
            start,
            end,
            inclusive,
            ..
        } => {
            assert!(matches!(
                *start,
                Expr::Literal(yashy_parser::ast::Literal::I64(0), _)
            ));
            assert!(matches!(
                *end,
                Expr::Literal(yashy_parser::ast::Literal::I64(10), _)
            ));
            assert!(!inclusive);
        }
        other => panic!("expected a range, got {other:?}"),
    }
}

#[test]
fn an_inclusive_range_keeps_its_equals() {
    let value = expression("fn main() { const r = 'a'..='z'; }");
    assert!(matches!(
        value,
        Expr::Range {
            inclusive: true,
            ..
        }
    ));
}

#[test]
fn a_for_head_and_an_index_keep_their_own_dots() {
    let parsed = program("fn main() { for n in 0..10 { } const xs = a[0..2]; }");
    let Item::Fn { body, .. } = &parsed.items[0] else {
        panic!("expected a function");
    };
    assert!(
        matches!(body.stmts[0], Stmt::ForRange { .. }),
        "the for head is a counted loop, got {:?}",
        body.stmts[0]
    );
    let Stmt::Const { init, .. } = &body.stmts[1] else {
        panic!("expected a const");
    };
    assert!(
        matches!(init, Expr::IndexRange { .. }),
        "the bracket range is a view, got {init:?}"
    );
}

#[test]
fn membership_accepts_a_range_as_its_container() {
    let value = expression("fn main() { const yes = 3 in 0..10; }");
    match value {
        Expr::MethodCall {
            receiver, method, ..
        } => {
            assert_eq!(method, "contains");
            assert!(matches!(*receiver, Expr::Range { .. }));
        }
        other => panic!("expected a method call, got {other:?}"),
    }
}

#[test]
fn a_parenthesised_range_in_a_for_head_is_a_value() {
    let parsed = program("fn main() { for n in (0..10) { } }");
    let Item::Fn { body, .. } = &parsed.items[0] else {
        panic!("expected a function");
    };
    assert!(
        matches!(body.stmts[0], Stmt::ForIn { .. }),
        "a parenthesised range is a value, got {:?}",
        body.stmts[0]
    );
}

#[test]
fn step_by_is_a_method_on_the_range_value() {
    let parsed = program("fn main() { for n in (0..10).step_by(2) { } }");
    let Item::Fn { body, .. } = &parsed.items[0] else {
        panic!("expected a function");
    };
    let Stmt::ForIn { iterable, .. } = &body.stmts[0] else {
        panic!("expected a for-in, got {:?}", body.stmts[0]);
    };
    assert!(matches!(iterable, Expr::MethodCall { method, .. } if method == "step_by"));
}
