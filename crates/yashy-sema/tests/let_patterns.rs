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

#[test]
fn a_fixed_array_pattern_binds_each_element() {
    check("fn main() { const a = [1, 2]; let [x, y] = a; print(x + y); }")
        .expect("a full fixed-array pattern binds every element");
}

#[test]
fn a_rest_binds_the_run_it_covers_as_a_view() {
    check("fn main() { const a = [1, 2, 3]; let [head, ..rest] = a; print(head + rest.len()); }")
        .expect("`..rest` binds a view over the unnamed run");
}

#[test]
fn a_rest_may_sit_between_written_elements() {
    check("fn main() { const a = [1, 2, 3]; let [first, .., last] = a; print(first + last); }")
        .expect("elements after the `..` count down from the end");
    check("fn main() { const a = [1, 2, 3]; let [.., last] = a; print(last); }")
        .expect("a leading `..` is the same rule with nothing before it");
}

#[test]
fn a_rest_may_cover_no_elements() {
    check("fn main() { const a = [1]; let [x, ..rest] = a; print(x + rest.len()); }")
        .expect("an empty run is a zero-length view, not a mistake");
}

#[test]
fn an_at_binding_wraps_a_fixed_array_pattern() {
    check("fn main() { const a = [1, 2]; let whole @ [x, y] = a; print(whole.len() + x + y); }")
        .expect("`whole @ [x, y]` binds the array and its elements");
}

#[test]
fn a_fixed_array_pattern_with_the_wrong_element_count_is_e0606() {
    assert_error_code(
        "fn main() { const a = [1, 2, 3]; let [x, y] = a; print(x + y); }",
        "E0606",
    );
    assert_error_code(
        "fn main() { const a = [1, 2]; let [x, y, z] = a; print(x + y + z); }",
        "E0606",
    );
}

#[test]
fn a_fixed_array_pattern_against_a_slice_is_e0606() {
    assert_error_code(
        "fn main() { let xs = []int { 1, 2 }; let [x, y] = xs; print(x + y); }",
        "E0606",
    );
}

#[test]
fn a_literal_element_makes_the_pattern_refutable() {
    // The array has one shape, so a literal element is the only thing that can
    // fail; binding position has no branch to take when it does.
    assert_error_code(
        "fn main() { const a = [1, 2]; let [1, y] = a; print(y); }",
        "E0100",
    );
}
