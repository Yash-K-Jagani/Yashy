//! Struct and array spread.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program, HirProgram};

fn check(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("test.ya", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .expect("the parser must accept the program");
    check_program(&program, &sources, file)
}

fn assert_error(src: &str, needle: &str) {
    match check(src) {
        Ok(_) => panic!("expected an error containing `{needle}`"),
        Err(diagnostics) => assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains(needle) || d.code.as_deref() == Some(needle)),
            "expected an error containing `{needle}`, got {diagnostics:?}"
        ),
    }
}

const POINT: &str = "struct Point { x: int, y: int } ";

#[test]
fn a_spread_fills_the_unlisted_fields() {
    check(&format!(
        "{POINT}fn main() {{ const base = Point {{ x: 1, y: 2 }}; const p = Point {{ x: 9, ..base }}; }}"
    ))
    .expect("the spread fills `y` from `base`");
}

#[test]
fn an_explicit_field_wins_over_the_spread() {
    check(&format!(
        "{POINT}fn main() {{ const base = Point {{ x: 1, y: 2 }}; const p = Point {{ ..base }}; }}"
    ))
    .expect("a spread alone copies every field");
}

#[test]
fn a_field_given_twice_is_e0202() {
    assert_error(
        &format!("{POINT}fn main() {{ const p = Point {{ x: 1, x: 2, y: 3 }}; }}"),
        "E0202",
    );
}

#[test]
fn using_a_spread_operand_afterwards_is_e1002() {
    assert_error(
        &format!(
            "{POINT}fn main() {{ const base = Point {{ x: 1, y: 2 }}; const p = Point {{ ..base }}; const int z = base.x; }}"
        ),
        "E1002",
    );
}

#[test]
fn a_spread_of_the_wrong_type_is_e0100() {
    assert_error(
        &format!(
            "{POINT}struct Other {{ x: int, y: int }} fn main() {{ const o = Other {{ x: 1, y: 2 }}; const p = Point {{ ..o }}; }}"
        ),
        "E0100",
    );
}

#[test]
fn an_array_spread_concatenates_in_order() {
    check("fn main() { let rest = []int { 1, 2 }; let xs = []int { 0, ..rest, 4 }; xs.free(); }")
        .expect("prefix, spread and suffix concatenate");
}

#[test]
fn an_array_spread_alone_is_legal() {
    check("fn main() { let rest = []int { 1, 2 }; let xs = [..rest]; xs.free(); }")
        .expect("a spread alone is a slice of the same elements");
}

#[test]
fn using_a_spread_slice_afterwards_is_e1002() {
    assert_error(
        "fn main() { let rest = []int { 1 }; let xs = [..rest]; let int n = rest.len(); }",
        "E1002",
    );
}
