//! The slice type at the checking edge: literals, linearity, and the two
//! operations that need the element type (`push`, `free_slice`).

use otter_common::{Diagnostic, SourceMap};
use otter_lexer::Lexer;
use otter_parser::Parser;
use otter_sema::{check_program, HirProgram};

fn check(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("test.ot", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .expect("the parser must accept the program");
    check_program(&program, &sources, file)
}

fn assert_rejected(src: &str, needle: &str) {
    match check(src) {
        Ok(_) => panic!("expected an error containing `{needle}`"),
        Err(diagnostics) => assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains(needle)),
            "expected an error containing `{needle}`, got {diagnostics:?}"
        ),
    }
}

#[test]
fn a_slice_literal_checks_every_element_against_its_written_type() {
    check("fn main() { var xs = []i64 { 1, 2, 3 }; push(xs, 4); free_slice(xs); }")
        .expect("a well-typed literal and its operations must check");
}

#[test]
fn a_wrong_element_type_is_reported() {
    assert_rejected(
        r#"fn main() { var xs = []i64 { 1, "two" }; free_slice(xs); }"#,
        "expected `i64`",
    );
}

#[test]
fn a_slice_is_linear_like_a_pointer() {
    // Exactly one binding owns the backing array: copying the header would give
    // two owners and a double free.
    assert_rejected(
        "fn main() { var a = []i64 { 1 }; var b = a; free_slice(a); free_slice(b); }",
        "cannot assign linear value `a` without `move`",
    );
}

#[test]
fn a_slice_of_slices_is_linear_at_both_levels() {
    // The outer slice owns its element array; the inner slices own theirs. A
    // copy of either header is rejected.
    assert_rejected(
        r#"fn main() {
               var inner = []i64 { 1 };
               var outer = [][]i64 { inner };
               var copy = outer;
           }"#,
        "cannot assign linear value `outer` without `move`",
    );
}

#[test]
fn pushing_keeps_the_slice() {
    // `push` may reallocate, but the binding stays the owner: no `move` is
    // needed and the slice is usable again.
    check("fn main() { var xs = []i64 { 1 }; push(xs, 2); push(xs, 3); free_slice(xs); }")
        .expect("a slice survives a push");
}

#[test]
fn pushing_a_wrong_element_type_is_reported() {
    assert_rejected(
        r#"fn main() { var xs = []i64 { 1 }; push(xs, "two"); free_slice(xs); }"#,
        "expected `i64`",
    );
}

#[test]
fn freeing_a_slice_moves_it() {
    assert_rejected(
        "fn main() { var xs = []i64 { 1 }; free_slice(xs); free_slice(xs); }",
        "value `xs` has already been moved",
    );
}

#[test]
fn push_needs_a_slice() {
    assert_rejected(
        "fn main() { var x = 1; push(x, 2); }",
        "`push` expects a slice",
    );
}

#[test]
fn a_slice_cannot_hold_a_value_with_no_size() {
    assert_rejected("fn main() { var xs = []() {}; }", "no size");
}
