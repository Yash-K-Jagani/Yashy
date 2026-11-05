//! Trailing blocks and inferred lambda parameters.
//!
//! A trailing block is a lambda in the last argument position, and its
//! parameters and return type come from the callee. These tests pin both the
//! inference and the refusal when there is nothing to infer from.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program_with, CheckOptions, HirProgram};

/// Lex, parse and check `source`.
fn check_source(src: &str) -> Result<HirProgram, Vec<yashy_common::Diagnostic>> {
    let mut sm = SourceMap::new();
    let file = sm.add_file("test.ya", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("lexer should never panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser
        .parse_program()
        .expect("parser should never panic on these inputs");
    check_program_with(&ast, &sm, file, CheckOptions { release: false })
}

/// Assert that `source` reports an error whose message contains `message`.
fn assert_error(source: &str, message: &str) {
    let diagnostics = check_source(source).expect_err("expected the program to be refused");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains(message)),
        "expected an error containing `{message}`, got {:?}",
        diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_trailing_block_takes_its_parameter_from_the_callee() {
    let result = check_source(
        "fn apply(int x, fn(int) -> int f) -> int { return f(x); } \
         fn main() { const r = apply(41) { |x| return x + 1; }; }",
    );
    result.expect("`|x|` should take `int` from the callee");
}

#[test]
fn a_trailing_block_takes_its_return_type_from_the_callee() {
    // No arrow: the body's `return x + 1` is checked against the callee's
    // declared `int` return, not against unit.
    let result = check_source(
        "fn apply(int x, fn(int) -> int f) -> int { return f(x); } \
         fn main() { const r = apply(41) { |x| return x + 1; }; }",
    );
    result.expect("the lambda's return type should come from the callee");
}

#[test]
fn a_trailing_block_may_write_its_types_too() {
    let result = check_source(
        "fn apply(int x, fn(int) -> int f) -> int { return f(x); } \
         fn main() { const r = apply(41) { |int x| -> int { return x + 1; } }; }",
    );
    result.expect("written types that agree with the callee are accepted");
}

#[test]
fn a_written_type_that_disagrees_is_reported() {
    assert_error(
        "fn apply(int x, fn(int) -> int f) -> int { return f(x); } \
         fn main() { const r = apply(41) { |string x| return 1; }; }",
        "expected `int`, found `string`",
    );
}

#[test]
fn a_trailing_block_with_the_wrong_return_type_is_e0100() {
    assert_error(
        "fn apply(int x, fn(int) -> int f) -> int { return f(x); } \
         fn main() { const r = apply(41) { |x| return \"text\"; }; }",
        "expected `int`, found `string`",
    );
}

#[test]
fn a_trailing_block_infers_from_map() {
    let result = check_source(
        "fn main() { \
             const xs = []int { 1, 2, 3 }; \
             const ys = xs.map { |x| return x * 2; }; \
         }",
    );
    result.expect("`map` fixes the element type");
}

#[test]
fn a_trailing_block_infers_from_filter() {
    let result = check_source(
        "fn main() { \
             const xs = []int { 1, 2, 3 }; \
             const ys = xs.filter { |x| return x > 1; }; \
         }",
    );
    result.expect("`filter` fixes the element and the bool return");
}

#[test]
fn a_trailing_block_infers_both_parameters_of_fold() {
    let result = check_source(
        "fn main() { \
             const xs = []int { 1, 2, 3 }; \
             const total = xs.fold(0) { |acc, x| return acc + x; }; \
         }",
    );
    result.expect("`fold` fixes the accumulator and the element");
}

#[test]
fn a_trailing_block_for_a_generic_needs_written_parameters() {
    // `T` is fixed by the first argument, but the generic argument pass has no
    // resolved signature to hand the lambda yet, so the parameters must be
    // written. The diagnostic names the type it could not resolve.
    assert_error(
        "fn apply[T](T x, fn(T) -> T f) -> T { return f(x); } \
         fn main() { const r = apply(1) { |x| return x; }; }",
        "fn(T) -> T",
    );
}
