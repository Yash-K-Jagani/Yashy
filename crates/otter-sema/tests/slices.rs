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
fn len_reads_a_slice_and_a_string() {
    check(
        "fn main() { var xs = []i64 { 1, 2 }; var n: i64 = len(xs); \
         var s = \"ab\"; var m: i64 = len(s); }",
    )
    .expect("`len` is the same spelling for both inputs");
}

#[test]
fn len_rejects_a_number() {
    assert_rejected("fn main() { var n = len(1); }", "expects a string or a slice");
}

#[test]
fn indexing_a_slice_yields_an_option() {
    check(
        "fn main() { var xs = []i64 { 1 }; var first: Option[i64] = xs[0]; \
         var missing: Option[i64] = xs[9]; }",
    )
    .expect("an index is always an Option, in and out of range");
}

#[test]
fn a_slice_element_cannot_be_assigned() {
    assert_rejected(
        "fn main() { var xs = []i64 { 1 }; xs[0] = 2; }",
        "cannot assign to a slice element",
    );
}

#[test]
fn pop_yields_the_element_in_an_option() {
    check("fn main() { var xs = []i64 { 1 }; var last: Option[i64] = pop(xs); }")
        .expect("`pop` hands the last element out or `None`");
}

#[test]
fn pop_needs_a_slice_binding() {
    assert_rejected(
        "fn main() { var last = pop([]i64 { 1 }); }",
        "needs a slice binding",
    );
}

#[test]
fn for_in_binds_the_element_type() {
    check("fn main() { var xs = []i64 { 1 }; for x in xs { var y: i64 = x; } }")
        .expect("the loop variable has the element type");
}

#[test]
fn for_in_over_a_non_slice_is_rejected() {
    assert_rejected("fn main() { for x in 1 { } }", "must iterate a slice");
}

#[test]
fn for_in_over_a_temporary_is_rejected() {
    assert_rejected(
        "fn main() { for x in []i64 { 1 } { } }",
        "slice binding",
    );
}

#[test]
fn moving_the_slice_while_iterating_is_rejected() {
    assert_rejected(
        "fn main() { var xs = []i64 { 1 }; for x in xs { free_slice(xs); } }",
        "while it is being iterated",
    );
}

#[test]
fn a_slice_of_strings_can_be_built_and_iterated() {
    check(
        "fn main() { var xs = []string { \"a\", \"b\" }; push(xs, \"c\"); \
         for x in xs { print_str(x); } free_slice(xs); }",
    )
    .expect("a `[]string` owns its elements: a literal and push retain, free releases");
}

#[test]
fn indexing_a_string_slice_yields_an_option_of_string() {
    check("fn main() { var xs = []string { \"a\" }; var first: Option[string] = xs[0]; }")
        .expect("an index borrows the element and a miss is `None`");
}

#[test]
fn popping_a_string_slice_is_rejected() {
    assert_rejected(
        "fn main() { var xs = []string { \"a\" }; var last = pop(xs); }",
        "popping a `string` is not supported yet",
    );
}

#[test]
fn a_field_slice_cannot_be_used_where_a_binding_is_needed() {
    // A slice field has no binding slot: `push` needs one to update the header
    // in place. Lowering looks names up in the local map, so accepting a
    // projection would make the call silently do nothing.
    assert_rejected(
        "struct S { items: []i64 }\n\
         fn main() { var s = S { items: []i64 {} }; push(s.items, 1); }",
        "not a field access",
    );
    assert_rejected(
        "struct S { items: []i64 }\n\
         fn main() { var s = S { items: []i64 {} }; for x in s.items { } }",
        "not a temporary",
    );
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
