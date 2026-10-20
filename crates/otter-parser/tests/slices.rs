//! Slice syntax: `[]T` in type position and `[]T { ... }` in value position.
//!
//! Both spellings put the element type before any element is seen, which is
//! what lets an empty literal have a type without inference.

use otter_common::SourceMap;
use otter_lexer::Lexer;
use otter_parser::ast::{Expr, Item, Stmt, TypeExpr};
use otter_parser::Parser;

fn parse(src: &str) -> otter_parser::ast::Program {
    let mut sources = SourceMap::new();
    let file = sources.add_file("test.ot", src.to_string());
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
    let program = parse("fn f(x: []i64) {}");
    let Item::Fn { params, .. } = program.items.into_iter().next().unwrap() else {
        panic!("expected a function");
    };
    match &params[0].ty {
        TypeExpr::Slice(element, _) => {
            assert!(
                matches!(&**element, TypeExpr::Name(name, _) if name == "i64"),
                "the element type must be `i64`: {element:?}"
            );
        }
        other => panic!("expected `[]i64`, got {other:?}"),
    }
}

#[test]
fn a_slice_literal_names_its_element_type_and_keeps_source_order() {
    let expr = body_expr(r#"fn main() { const xs = []i64 { 1, 2, 3 }; }"#);
    match expr {
        Expr::SliceLit {
            elem, elements, ..
        } => {
            assert!(
                matches!(&*elem, TypeExpr::Name(name, _) if name == "i64"),
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
