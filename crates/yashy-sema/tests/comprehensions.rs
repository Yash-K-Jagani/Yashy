//! Comprehensions lower to a builder loop in a synthesized function, so
//! the verifier sees an ordinary loop and a `push`/`insert` per value.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program_with, CheckOptions, HirItem, HirProgram};

/// Lex, parse and check `source` as a single file.
fn check_source(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sm = SourceMap::new();
    let file = sm.add_file("test.ya", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("lexer should never panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser
        .parse_program()
        .expect("parser should never panic on these inputs");
    check_program_with(&ast, &sm, file, CheckOptions::default())
}

/// The synthesized builder functions, by name.
fn builders(hir: &HirProgram) -> Vec<String> {
    hir.items
        .iter()
        .filter_map(|item| match item {
            HirItem::Fn(f) if f.name.starts_with("__comp") => Some(f.name.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_filtered_array_comprehension_checks() {
    let hir = check_source(
        "fn f() -> []int { let xs = []int { 1, 2, 3 }; return [x * 2 for x in xs if x > 1]; }",
    )
    .expect("the comprehension must check");
    assert_eq!(builders(&hir).len(), 1, "one builder: {hir:?}");
}

#[test]
fn a_map_comprehension_checks() {
    let hir = check_source(
        "fn f() -> map[string]int { let xs = []string { \"a\", \"b\" }; return {k: 1 for k in xs}; }",
    )
    .expect("the comprehension must check");
    assert_eq!(builders(&hir).len(), 1, "one builder: {hir:?}");
}

#[test]
fn nested_comprehensions_check() {
    let hir = check_source(
        "fn f() -> []int { let xs = []int { 1, 2 }; return [z for z in [y for y in xs]]; }",
    )
    .expect("nested comprehensions must check");
    assert_eq!(builders(&hir).len(), 2, "two builders: {hir:?}");
}
