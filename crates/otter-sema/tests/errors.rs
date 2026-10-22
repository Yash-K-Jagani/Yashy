//! The checks that make a program safe to read: an error is propagated only
//! where it can be, a value-returning function returns on every path, and `&&`
//! and `||` do not evaluate their right operand when the left one decides.

use otter_common::{Diagnostic, SourceMap};
use otter_lexer::Lexer;
use otter_parser::Parser;
use otter_sema::{check_program_with, CheckOptions};

/// Lex, parse and check `source`.
fn check_source(source: &str) -> Result<(), Vec<Diagnostic>> {
    let mut sm = SourceMap::new();
    let file = sm.add_file("test.ot", source.to_string());
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
        "fn f() -> i64 { const v = g()?; return v; } \
         fn g() -> Result[i64, string] { return Ok(1); }",
        "`?` needs a function returning `Result`",
    );
}

#[test]
fn question_needs_the_same_error_type() {
    assert_error(
        "fn f() -> Result[i64, string] { const v = g()?; return Ok(v); } \
         fn g() -> Result[i64, bool] { return Ok(1); }",
        "error type mismatch: `bool` vs `string`",
    );
}

#[test]
fn question_in_a_result_function_is_fine() {
    check_source(
        "fn f() -> Result[i64, string] { const v = g()?; return Ok(v); } \
         fn g() -> Result[i64, string] { return Ok(1); }",
    )
    .expect("a `?` that propagates the same error should type-check");
}

#[test]
fn a_value_returning_function_has_to_return() {
    assert_error(
        "fn f(n: i64) -> i64 { if n > 0 { return 1; } }",
        "missing return",
    );
}

#[test]
fn a_function_that_returns_on_every_path_is_fine() {
    check_source(
        "fn f(n: i64) -> i64 { if n > 0 { return 1; } else { return 0; } }",
    )
    .expect("a function with an else that returns should type-check");
}

#[test]
fn a_unit_function_needs_no_return() {
    check_source("fn f(n: i64) { if n > 0 { print_i64(1); } }")
        .expect("a function that returns nothing needs no return");
}

#[test]
fn two_structs_are_compared_field_by_field() {
    check_source(
        "struct Point { x: i64, y: i64 } \
         fn main() { const a = Point { x: 1, y: 2 }; const b = Point { x: 1, y: 2 }; \
         const same = a == b; }",
    )
    .expect("comparing two structs should type-check");
}

#[test]
fn a_struct_with_a_field_that_has_no_equality_cannot_be_compared() {
    assert_error(
        "struct Holder { xs: []i64 } \
         fn main() { const a = Holder { xs: []i64 {} }; const b = Holder { xs: []i64 {} }; \
         const same = a == b; }",
        "`==` needs fields with equality",
    );
}

#[test]
fn a_generic_call_has_to_infer_every_type_parameter() {
    // `T` appears nowhere in the parameters, so there is nothing at the call
    // to replace it with. The error belongs at the call, not in the body.
    assert_error(
        "fn make[T]() -> i64 { return 0; } fn main() { print_i64(make()); }",
        "cannot infer type parameter `T`",
    );
}

#[test]
fn a_generic_function_cannot_shadow_a_type() {
    assert_error(
        "struct T { x: i64 } fn f[T](x: i64) -> i64 { return x; }",
        "type parameter `T` shadows a type of the same name",
    );
}

#[test]
fn polymorphic_recursion_is_rejected() {
    // The instantiation for `(i64, string)` asks for `(string, i64)`, which
    // would ask for the first again: there is no finite family to emit.
    assert_error(
        "fn swap[T, U](x: T, y: U) -> i64 { return swap(y, x); } \
         fn main() { print_i64(swap(1, \"x\")); }",
        "polymorphic recursion has no finite instantiation",
    );
}
