//! The function type the first-class-function milestone adds to the grammar.
//!
//! `fn(T, U) -> R` has to survive parsing as its own node: a checker that
//! flattened it into a tuple would lose the distinction between "a pair of
//! values" and "a function of two parameters", which is the whole reason the
//! type exists. These tests inspect the AST for that reason.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::ast::{Item, TypeExpr};
use yashy_parser::Parser;

/// Parse `fn f(x: <spelling>) {}` and return the parameter's type.
fn param_type(spelling: &str) -> TypeExpr {
    let src = format!("fn f(x: {spelling}) {{ }}");
    let mut sources = SourceMap::new();
    let file = sources.add_file("test.ya", src.clone());
    let tokens = Lexer::new(&src)
        .tokenize()
        .expect("the lexer must accept a type spelling");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .unwrap_or_else(|error| panic!("`{spelling}` must parse: {error:?}"));
    let item = program
        .items
        .into_iter()
        .next()
        .expect("the program has one item");
    match item {
        Item::Fn { params, .. } => params
            .into_iter()
            .next()
            .expect("the function has one parameter")
            .ty,
        other => panic!("expected a function item, got {other:?}"),
    }
}

/// The named type inside a function type, or the unit type.
fn return_type(spelling: &str) -> TypeExpr {
    match param_type(spelling) {
        TypeExpr::Fn { ret, .. } => *ret,
        other => panic!("`{spelling}` must parse as a function type, got {other:?}"),
    }
}

/// The named parameter types inside a function type.
fn parameter_types(spelling: &str) -> Vec<String> {
    match param_type(spelling) {
        TypeExpr::Fn { params, .. } => params
            .into_iter()
            .map(|param| match param {
                TypeExpr::Name(name, _) => name,
                other => panic!("expected a named parameter type, got {other:?}"),
            })
            .collect(),
        other => panic!("`{spelling}` must parse as a function type, got {other:?}"),
    }
}

#[test]
fn a_function_type_keeps_its_parameters_and_return_type() {
    assert_eq!(
        parameter_types("fn(int, string) -> bool"),
        vec!["int".to_string(), "string".to_string()]
    );
    assert!(matches!(
        return_type("fn(int, string) -> bool"),
        TypeExpr::Name(name, _) if name == "bool"
    ));
}

#[test]
fn a_function_type_without_an_arrow_returns_void() {
    // The missing arrow means the no-value type, and the parser synthesizes
    // its Yashy spelling rather than the legacy `()` node: nothing was
    // written, so there is no written `()` for the checker to reject.
    for source in ["fn(int)", "fn()"] {
        assert!(
            matches!(return_type(source), TypeExpr::Name(name, _) if name == "void"),
            "`{source}` should return `void`"
        );
    }
}

#[test]
fn a_written_unit_type_still_parses() {
    // `()` parses to its own node so the checker can name the replacement.
    // The parser's job here is only to produce that node; rejecting it
    // belongs with the other type errors.
    assert!(matches!(
        return_type("fn(int) -> ()"),
        TypeExpr::Unit(_)
    ));
}

#[test]
fn a_function_type_may_nest() {
    // A function that takes a function: the inner type is a function type in
    // its own right, not a name.
    let nested = param_type("fn(fn(int) -> int, int) -> int");
    let TypeExpr::Fn { params, .. } = nested else {
        panic!("expected a function type");
    };
    assert_eq!(params.len(), 2);
    assert!(matches!(&params[0], TypeExpr::Fn { params, .. } if params.len() == 1));
}
