//! `derive(..)` is a clause on a type, in the same family as `repr(..)`:
//! the parser records the names in declaration order and refuses duplicates.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::{Item, Parser};

fn parse(src: &str) -> Result<Item, Vec<yashy_common::Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("derives.ya".to_string(), src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the lexer must not panic");
    Parser::new(tokens, &sources, file)
        .parse_program()
        .map(|program| program.items.into_iter().next().expect("one item"))
}

#[test]
fn a_struct_carries_its_derive_names() {
    let item = parse("struct Point derive(Debug, Eq, Hash) { x: int, y: int }")
        .expect("the clause is part of the grammar");
    let Item::Struct { derives, .. } = item else {
        panic!("expected a struct, got {item:?}");
    };
    assert_eq!(derives, ["Debug", "Eq", "Hash"]);
}

#[test]
fn an_enum_carries_its_derive_names() {
    let item = parse("enum Color derive(Debug, Eq) { Red, Green(int) }")
        .expect("the clause is part of the grammar");
    let Item::Enum { derives, .. } = item else {
        panic!("expected an enum, got {item:?}");
    };
    assert_eq!(derives, ["Debug", "Eq"]);
}

#[test]
fn a_type_without_a_clause_has_no_derives() {
    let item = parse("struct Point { x: int }").expect("an ordinary struct");
    let Item::Struct { derives, .. } = item else {
        panic!("expected a struct");
    };
    assert!(derives.is_empty());
}

#[test]
fn a_duplicate_derive_name_is_refused() {
    assert!(
        parse("struct Point derive(Debug, Debug) { x: int }").is_err(),
        "the same expansion must not be named twice"
    );
}

#[test]
fn derive_and_repr_can_be_combined() {
    let item =
        parse("repr(C) struct Point derive(Debug) { x: i32 }").expect("the two clauses compose");
    let Item::Struct { derives, repr, .. } = item else {
        panic!("expected a struct");
    };
    assert_eq!(derives, ["Debug"]);
    assert!(repr.is_some());
}
