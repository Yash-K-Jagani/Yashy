//! Chained comparisons: `a < b <= c`.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::ast::{BinOp, Expr, Item, Stmt};
use yashy_parser::Parser;

fn parse(src: &str) -> yashy_parser::ast::Program {
    let mut sources = SourceMap::new();
    let file = sources.add_file("test.ya", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer must accept it");
    let mut parser = Parser::new(tokens, &sources, file);
    parser
        .parse_program()
        .unwrap_or_else(|error| panic!("`{src}` must parse: {error:?}"))
}

fn body_expr(src: &str) -> Expr {
    let program = parse(src);
    let item = program.items.into_iter().next().expect("one item");
    let Item::Fn { body, .. } = item else {
        panic!("expected a function");
    };
    let stmt = body.stmts.into_iter().next().expect("one statement");
    match stmt {
        Stmt::Expr(expr, _) | Stmt::Const { init: expr, .. } => expr,
        other => panic!("unexpected statement: {other:?}"),
    }
}

#[test]
fn a_chain_is_one_node_with_all_operands() {
    let expr = body_expr("fn main() { const b = 0 <= x < 10; }");
    match expr {
        Expr::CompareChain { operands, ops, .. } => {
            assert_eq!(operands.len(), 3);
            assert_eq!(ops, vec![BinOp::Le, BinOp::Lt]);
        }
        other => panic!("expected a chain, got {other:?}"),
    }
}

#[test]
fn a_single_comparison_stays_a_binary() {
    let expr = body_expr("fn main() { const b = x < 10; }");
    assert!(matches!(expr, Expr::Binary { .. }), "got {expr:?}");
    let expr = body_expr("fn main() { const b = x == 10; }");
    assert!(matches!(expr, Expr::Binary { .. }), "got {expr:?}");
}

#[test]
fn a_mixed_chain_parses_as_a_chain() {
    let expr = body_expr("fn main() { const b = a < b > c; }");
    match expr {
        Expr::CompareChain { ops, .. } => assert_eq!(ops, vec![BinOp::Lt, BinOp::Gt]),
        other => panic!("expected a chain, got {other:?}"),
    }
}
