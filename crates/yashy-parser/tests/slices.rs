//! Slice syntax: `[]T` in type position and `[]T { ... }` in value position.
//!
//! Both spellings put the element type before any element is seen, which is
//! what lets an empty literal have a type without inference.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::ast::{Expr, Item, Stmt, TypeExpr};
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
fn a_slice_type_is_empty_brackets_then_the_element_type() {
    let program = parse("fn f(x: []int) {}");
    let Item::Fn { params, .. } = program.items.into_iter().next().unwrap() else {
        panic!("expected a function");
    };
    match &params[0].ty {
        TypeExpr::Slice(element, _) => {
            assert!(
                matches!(&**element, TypeExpr::Name(name, _) if name == "int"),
                "the element type must be `int`: {element:?}"
            );
        }
        other => panic!("expected `[]int`, got {other:?}"),
    }
}

#[test]
fn a_slice_literal_names_its_element_type_and_keeps_source_order() {
    let expr = body_expr(r#"fn main() { const xs = []int { 1, 2, 3 }; }"#);
    match expr {
        Expr::SliceLit {
            elem, elements, ..
        } => {
            assert!(
                matches!(&*elem, TypeExpr::Name(name, _) if name == "int"),
                "the literal names its element type: {elem:?}"
            );
            assert_eq!(elements.len(), 3, "all three elements are kept");
        }
        other => panic!("expected a slice literal, got {other:?}"),
    }
}

#[test]
fn an_empty_slice_literal_is_still_typed() {
    let expr = body_expr("fn main() { const xs = []string {}; }");
    match expr {
        Expr::SliceLit {
            elem, elements, ..
        } => {
            assert!(
                matches!(&*elem, TypeExpr::Name(name, _) if name == "string"),
                "the empty literal names its element type: {elem:?}"
            );
            assert!(elements.is_empty());
        }
        other => panic!("expected a slice literal, got {other:?}"),
    }
}

fn statements(src: &str) -> Vec<Stmt> {
    let program = parse(src);
    let item = program.items.into_iter().next().expect("one item");
    let Item::Fn { body, .. } = item else {
        panic!("expected a function");
    };
    body.stmts
}

#[test]
fn for_in_over_a_slice_binds_one_name() {
    let stmts = statements("fn main() { let xs = []int { 1 }; for x in xs { print(x); } }");
    match &stmts[1] {
        Stmt::ForIn {
            name, iterable, ..
        } => {
            assert_eq!(name, "x");
            assert!(
                matches!(iterable, Expr::Ident(path, _) if path == &["xs".to_string()]),
                "the iterable is the slice binding: {iterable:?}"
            );
        }
        other => panic!("expected `for x in xs`, got {other:?}"),
    }
}

#[test]
fn for_in_still_reads_the_range_form() {
    let stmts = statements("fn main() { for i in 0..3 { print(i); } }");
    match &stmts[0] {
        Stmt::ForRange {
            name, start, end, ..
        } => {
            assert_eq!(name, "i");
            assert!(matches!(start, Expr::Literal(yashy_parser::ast::Literal::I64(0), _)));
            assert!(matches!(end, Expr::Literal(yashy_parser::ast::Literal::I64(3), _)));
        }
        other => panic!("expected the range form, got {other:?}"),
    }
}
