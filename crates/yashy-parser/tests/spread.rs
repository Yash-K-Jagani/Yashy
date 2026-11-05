//! Spread: `P { x: 1, ..base }` and `[0, ..xs, 4]` / `[]int { 0, ..xs, 4 }`.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::ast::{Expr, Item, SliceElement, Stmt};
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

fn parse_err(src: &str) -> String {
    let mut sources = SourceMap::new();
    let file = sources.add_file("test.ya", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer must accept it");
    let mut parser = Parser::new(tokens, &sources, file);
    match parser.parse_program() {
        Ok(_) => panic!("expected a parse error"),
        Err(diagnostics) => diagnostics
            .first()
            .map(|d| d.message.clone())
            .unwrap_or_default(),
    }
}

#[test]
fn a_struct_spread_is_kept() {
    let expr = body_expr("fn main() { const p = P { x: 1, ..base }; }");
    match expr {
        Expr::StructLit { fields, spread, .. } => {
            assert_eq!(fields.len(), 1);
            assert_eq!(fields[0].0, "x");
            assert!(spread.is_some(), "the spread operand is kept");
        }
        other => panic!("expected a struct literal, got {other:?}"),
    }
}

#[test]
fn a_struct_spread_alone_is_legal() {
    let expr = body_expr("fn main() { const p = P { ..base }; }");
    match expr {
        Expr::StructLit { fields, spread, .. } => {
            assert!(fields.is_empty());
            assert!(spread.is_some());
        }
        other => panic!("expected a struct literal, got {other:?}"),
    }
}

#[test]
fn a_spread_must_be_written_last() {
    let message = parse_err("fn main() { const p = P { ..base, x: 1 }; }");
    assert!(
        message.contains("the spread is written last"),
        "got: {message}"
    );
}

#[test]
fn an_array_spread_keeps_its_position() {
    let expr = body_expr("fn main() { const xs = []int { 0, ..rest, 4 }; }");
    match expr {
        Expr::SliceLit { elements, .. } => {
            assert_eq!(elements.len(), 3);
            assert!(matches!(elements[0], SliceElement::Value(_)));
            assert!(matches!(elements[1], SliceElement::Spread(_)));
            assert!(matches!(elements[2], SliceElement::Value(_)));
        }
        other => panic!("expected a slice literal, got {other:?}"),
    }
}

#[test]
fn a_bare_array_spread_is_a_slice_literal() {
    let expr = body_expr("fn main() { const xs = [0, ..rest, 4]; }");
    match expr {
        Expr::SliceLit { elements, .. } => {
            assert_eq!(elements.len(), 3);
            assert!(matches!(elements[1], SliceElement::Spread(_)));
        }
        other => panic!("expected a slice literal, got {other:?}"),
    }
}
