//! The checks that make a program safe to read: an error is propagated only
//! where it can be, a value-returning function returns on every path, and `&&`
//! and `||` do not evaluate their right operand when the left one decides.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program_with, CheckOptions};

/// Lex, parse and check `source`.
fn check_source(source: &str) -> Result<(), Vec<Diagnostic>> {
    let mut sm = SourceMap::new();
    let file = sm.add_file("test.ya", source.to_string());
    let tokens = Lexer::new(source)
        .tokenize()
        .expect("lexer should never panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser
        .parse_program()
        .expect("parser should never panic on these inputs");
    check_program_with(&ast, &sm, file, CheckOptions { release: false }).map(|_| ())
}

/// Assert that `source` fails to type-check with a message containing `needle`.
fn assert_error(source: &str, needle: &str) {
    let diags = match check_source(source) {
        Ok(()) => panic!("expected a type error containing `{needle}`"),
        Err(diags) => diags,
    };
    assert!(
        diags.iter().any(|d| d.message.contains(needle)),
        "expected an error containing `{needle}`, got {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn question_requires_a_result_function() {
    assert_error(
        "fn f() -> int { const v = g()?; return v; } \
         fn g() -> Result[int, string] { return Ok(1); }",
        "`?` needs a function returning `Result`",
    );
}

#[test]
fn question_needs_the_same_error_type() {
    assert_error(
        "fn f() -> Result[int, string] { const v = g()?; return Ok(v); } \
         fn g() -> Result[int, bool] { return Ok(1); }",
        "error type mismatch: `bool` vs `string`",
    );
}

#[test]
fn question_in_a_result_function_is_fine() {
    check_source(
        "fn f() -> Result[int, string] { const v = g()?; return Ok(v); } \
         fn g() -> Result[int, string] { return Ok(1); }",
    )
    .expect("a `?` that propagates the same error should type-check");
}

#[test]
fn a_value_returning_function_has_to_return() {
    assert_error(
        "fn f(n: int) -> int { if n > 0 { return 1; } }",
        "missing return",
    );
}

#[test]
fn a_function_that_returns_on_every_path_is_fine() {
    check_source(
        "fn f(n: int) -> int { if n > 0 { return 1; } else { return 0; } }",
    )
    .expect("a function with an else that returns should type-check");
}

#[test]
fn a_unit_function_needs_no_return() {
    check_source("fn f(n: int) { if n > 0 { print(1); } }")
        .expect("a function that returns nothing needs no return");
}

#[test]
fn two_structs_are_compared_field_by_field() {
    check_source(
        "struct Point { x: int, y: int } \
         fn main() { const a = Point { x: 1, y: 2 }; const b = Point { x: 1, y: 2 }; \
         const same = a == b; }",
    )
    .expect("comparing two structs should type-check");
}

#[test]
fn a_struct_with_a_field_that_has_no_equality_cannot_be_compared() {
    assert_error(
        "struct Holder { xs: []int } \
         fn main() { const a = Holder { xs: []int {} }; const b = Holder { xs: []int {} }; \
         const same = a == b; }",
        "`==` needs fields with equality",
    );
}

#[test]
fn a_generic_call_has_to_infer_every_type_parameter() {
    // `T` appears nowhere in the parameters, so there is nothing at the call
    // to replace it with. The error belongs at the call, not in the body.
    assert_error(
        "fn make[T]() -> int { return 0; } fn main() { print(make()); }",
        "cannot infer type parameter `T`",
    );
}

#[test]
fn a_generic_function_cannot_shadow_a_type() {
    assert_error(
        "struct T { x: int } fn f[T](x: int) -> int { return x; }",
        "type parameter `T` shadows a type of the same name",
    );
}

#[test]
fn polymorphic_recursion_is_rejected() {
    // The instantiation for `(int, string)` asks for `(string, int)`, which
    // would ask for the first again: there is no finite family to emit.
    assert_error(
        "fn swap[T, U](x: T, y: U) -> int { return swap(y, x); } \
         fn main() { print(swap(1, \"x\")); }",
        "polymorphic recursion has no finite instantiation",
    );
}

#[test]
fn a_view_borrows_its_source_until_the_function_ends() {
    // Moving the array would hand its storage to a new owner while `view`
    // still points into it. Rebinding is refused for the same reason.
    assert_error(
        "fn take(xs: []int) -> int { return xs.len(); } \
         fn main() { let xs = []int { 1, 2, 3 }; const view = xs[0..2]; \
         print(take(move xs)); }",
        "cannot move `xs` while a view of it is alive",
    );
    assert_error(
        "fn main() { let xs = []int { 1, 2, 3 }; const view = xs[0..2]; \
         xs = []int { 4 }; }",
        "cannot change `xs` while a view of it is alive",
    );
}

#[test]
fn changing_a_borrowed_array_is_rejected() {
    // `push` grows the backing array, possibly to a new block; a view taken
    // before it would be left pointing at the old one.
    assert_error(
        "fn main() { let xs = []int { 1, 2 }; const view = xs[0..1]; xs.push(3); }",
        "cannot change `xs` while a view of it is alive",
    );
    assert_error(
        "fn main() { let xs = []int { 1, 2 }; const view = xs[0..1]; xs.free(); }",
        "cannot change `xs` while a view of it is alive",
    );
}

#[test]
fn a_view_parameter_cannot_be_named() {
    // A view borrows storage in the frame that made it. Yashy has no lifetime
    // syntax, so letting a return type or a field name `[T]` would let the
    // view outlive the array; the type is inferred at the range instead.
    assert_error(
        "fn f(x: [int]) -> int { return 0; } fn main() { }",
        "a view cannot be named",
    );
}

#[test]
fn a_fixed_array_cannot_hold_a_reference() {
    // Copying a fixed array copies bytes; a `string` element would duplicate a
    // handle without taking a reference on it.
    assert_error(
        "fn main() { const xs = [\"a\", \"b\"]; }",
        "a fixed array cannot hold `string`",
    );
}

#[test]
fn a_fixed_array_needs_a_length() {
    assert_error(
        "fn main() { const xs: [int; 0] = [1]; }",
        "a fixed array needs at least one element",
    );
}
