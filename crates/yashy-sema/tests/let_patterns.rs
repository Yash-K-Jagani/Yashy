//! A pattern in binding position: `let`, `const` and `for`.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::check_program;

fn check(src: &str) -> Result<(), Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("test.ya", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .expect("the parser must accept the program");
    check_program(&program, &sources, file).map(|_| ())
}

fn assert_error_code(src: &str, code: &str) {
    match check(src) {
        Ok(()) => panic!("expected an error with code {code}"),
        Err(diagnostics) => assert!(
            diagnostics.iter().any(|d| d.code == Some(code)),
            "expected code {code}, got {diagnostics:?}"
        ),
    }
}

const POINT: &str = "struct Point { x: int, y: int } ";

#[test]
fn struct_destructuring_in_let() {
    check(&format!(
        "{POINT}fn main() {{ const p = Point {{ x: 1, y: 2 }}; let Point {{ x: x, y: y }} = p; print(x + y); }}"
    ))
    .expect("a struct pattern binds its fields");
}

#[test]
fn nested_struct_patterns() {
    check(&format!(
        "struct Inner {{ v: int }} struct Outer {{ inner: Inner, z: int }} fn main() {{ const o = Outer {{ inner: Inner {{ v: 3 }}, z: 4 }}; let Outer {{ inner: Inner {{ v: v }}, .. }} = o; print(v); }}"
    ))
    .expect("nested struct patterns bind through the layers");
}

#[test]
fn a_wildcard_skips_a_tuple_element() {
    check("fn main() { const pair = (1, 2); let (a, _) = pair; print(a); }")
        .expect("`_` skips one element");
}

#[test]
fn for_binds_through_a_pattern() {
    // Slices hold only numbers, bool and string today, so a destructuring
    // `for` pattern over a struct slice cannot be written yet; a wildcard
    // and a name pattern still bind. The pattern path itself is pinned by
    // `let` above.
    check("fn main() { let xs = []int { 1, 2 }; for x in xs { print(x); } }")
        .expect("a for name pattern binds each element");
    check("fn main() { let xs = []int { 1, 2 }; for _ in xs { } }")
        .expect("a for wildcard binds nothing");
}

#[test]
fn a_refutable_pattern_in_let_is_e0100() {
    assert_error_code(
        "fn main() { let opt = Some(1); let Some(x) = opt; print(x); }",
        "E0100",
    );
}

#[test]
fn an_or_pattern_binds_the_same_names_on_both_sides() {
    check(&format!(
        "{POINT}enum Shape {{ A(Point), B(Point) }} fn main() {{ const s = Shape.A(Point {{ x: 1, y: 2 }}); print(switch s {{ case A(Point {{ x: x, .. }}) | B(Point {{ x: x, .. }}) {{ x }} default {{ 0 }} }}); }}"
    ))
    .expect("an or-pattern with matching bindings on both sides checks");
}

#[test]
fn an_or_pattern_that_binds_a_name_on_one_side_is_e1002() {
    assert_error_code(
        "enum Opt { A(int), B } fn main() { const o = Opt.B; print(switch o { case A(n) | B { 0 } default { 1 } }); }",
        "E1002",
    );
}
