//! Map syntax: `map[K]V` in type position.
//!
//! The key sits inside the brackets and the value after them, so the spelling
//! shares `Option[..]`'s opening bracket but not its shape. `map` without a
//! following `[` stays an ordinary type name; the checker is what reports an
//! unknown one.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::ast::{Item, TypeExpr};
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

fn first_param(src: &str) -> TypeExpr {
    let program = parse(src);
    let Item::Fn { params, .. } = program.items.into_iter().next().expect("one item") else {
        panic!("expected a function");
    };
    params.into_iter().next().expect("one parameter").ty
}

#[test]
fn a_map_type_is_a_bracketed_key_then_a_value() {
    match first_param("fn f(m: map[string]int) {}") {
        TypeExpr::Map(key, value, _) => {
            assert!(
                matches!(&*key, TypeExpr::Name(name, _) if name == "string"),
                "the key is the type inside the brackets: {key:?}"
            );
            assert!(
                matches!(&*value, TypeExpr::Name(name, _) if name == "int"),
                "the value is the type after the bracket: {value:?}"
            );
        }
        other => panic!("expected `map[string]int`, got {other:?}"),
    }
}

#[test]
fn a_map_value_may_be_another_map() {
    match first_param("fn f(m: map[int]map[bool]float) {}") {
        TypeExpr::Map(key, value, _) => {
            assert!(matches!(&*key, TypeExpr::Name(name, _) if name == "int"));
            assert!(
                matches!(&*value, TypeExpr::Map(_, _, _)),
                "the value is parsed as a whole type: {value:?}"
            );
        }
        other => panic!("expected a nested map, got {other:?}"),
    }
}

#[test]
fn a_map_key_may_be_an_option() {
    match first_param("fn f(m: map[Option[int]]int) {}") {
        TypeExpr::Map(key, _, _) => assert!(
            matches!(&*key, TypeExpr::Option(_, _)),
            "the bracketed type is parsed as a whole type: {key:?}"
        ),
        other => panic!("expected a map, got {other:?}"),
    }
}

#[test]
fn map_without_brackets_is_an_ordinary_type_name() {
    // The parser must not reserve the word: resolution is what says whether a
    // type named `map` exists.
    match first_param("fn f(m: map) {}") {
        TypeExpr::Name(name, _) => assert_eq!(name, "map"),
        other => panic!("expected the name `map`, got {other:?}"),
    }
}
