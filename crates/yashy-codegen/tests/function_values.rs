//! First-class function values: the thunk, the value, and the indirect call.
//!
//! A function value is a `{ i8* code, i8* env }` pair. The code pointer is
//! the address of a synthesized thunk that takes the environment as its first
//! argument; the environment is null for a top-level function, which has
//! nothing to capture. These tests pin the three pieces separately, because
//! a mismatch in any one of them produces an IR that still looks plausible
//! and fails at link time.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::check_program;

use yashy_codegen::{emit_program, CodegenOptions};

/// Lex, parse, check and emit `src` as one module.
fn ir_of(src: &str) -> String {
    let mut sm = SourceMap::new();
    let file = sm.add_file("main.ya", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("lexer should never panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser
        .parse_program()
        .expect("these test sources must parse");
    let hir = check_program(&ast, &sm, file).expect("these test sources must type-check");
    emit_program(&hir, "main.ya", CodegenOptions::default())
}

#[test]
fn a_top_level_function_becomes_a_thunk_that_takes_the_environment() {
    let ir_text = ir_of("fn square(x: i64) -> i64 { return x; } fn main() { const f = square; }");
    assert!(
        ir_text.contains("define i64 @__fnvalue_square(i8* %env, i64 %arg0)"),
        "the thunk takes the environment first, then the arguments:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call i64 @square(i64 %arg0)"),
        "the thunk calls the real function:\n{ir_text}"
    );
}

#[test]
fn a_function_value_is_a_code_pointer_and_a_null_environment() {
    let ir_text = ir_of("fn square(x: i64) -> i64 { return x; } fn main() { const f = square; }");
    assert!(
        ir_text.contains("insertvalue { i8*, i8* } undef, i8* @__fnvalue_square, 0"),
        "the code pointer is the thunk's address:\n{ir_text}"
    );
    assert!(
        ir_text
            .lines()
            .any(|line| line.contains("insertvalue { i8*, i8* }") && line.contains("i8* null, 1")),
        "a top-level function captures nothing, so the environment is null:\n{ir_text}"
    );
}

#[test]
fn calling_a_function_value_goes_through_the_stored_code_pointer() {
    let ir_text = ir_of(
        "fn square(x: i64) -> i64 { return x; } fn main() { const f = square; print_i64(f(4)); }",
    );
    assert!(
        ir_text.contains("load { i8*, i8* }, ptr"),
        "the value is loaded out of the binding's slot:\n{ir_text}"
    );
    assert!(
        ir_text.contains("extractvalue { i8*, i8* }"),
        "the code and the environment are read out of the pair:\n{ir_text}"
    );
    let indirect = ir_text
        .lines()
        .find(|line| line.contains("call i64 %") && line.contains("i8* %"))
        .expect("the call must go through the stored code pointer");
    assert!(
        indirect.contains("i8* %"),
        "the environment is passed as the first argument:\n{indirect}"
    );
}

#[test]
fn a_function_parameter_is_passed_as_the_same_pair() {
    let ir_text = ir_of(
        "fn square(x: i64) -> i64 { return x; } \
         fn apply(f: fn(i64) -> i64, x: i64) -> i64 { return f(x); } \
         fn main() { print_i64(apply(square, 7)); }",
    );
    assert!(
        ir_text.contains("define i64 @apply({ i8*, i8* } %arg0, i64 %arg1)"),
        "the parameter is the same pair the value is:\n{ir_text}"
    );
    assert!(
        ir_text.contains("call i64 @apply({ i8*, i8* }"),
        "the argument is passed as the pair:\n{ir_text}"
    );
}

#[test]
fn one_thunk_is_shared_by_two_uses_of_the_same_function() {
    let ir_text = ir_of(
        "fn square(x: i64) -> i64 { return x; } \
         fn main() { const f = square; print_i64(f(4)); print_i64(square(5)); }",
    );
    assert_eq!(
        ir_text.matches("define i64 @__fnvalue_square").count(),
        1,
        "the thunk is synthesized once per function:\n{ir_text}"
    );
}

#[test]
fn a_function_that_receives_a_value_is_not_inlined() {
    // Inlining the receiver would let the optimizer see the value it was
    // handed and fold the indirect call back into a direct one.
    let ir_text = ir_of(
        "fn square(x: i64) -> i64 { return x; } \
         fn apply(f: fn(i64) -> i64, x: i64) -> i64 { return f(x); } \
         fn main() { print_i64(apply(square, 7)); }",
    );
    assert!(
        ir_text.contains("@apply({ i8*, i8* } %arg0, i64 %arg1) noinline"),
        "the receiver of a function value is not inlined:\n{ir_text}"
    );
}

#[test]
fn a_lambda_is_emitted_as_a_private_function() {
    // The lambda has no name in the source, so the backend gives it one, and
    // the environment parameter is the convention every function value uses.
    let ir_text = ir_of(
        "fn main() { const f = |x: i64| -> i64 { return x + 1; }; print_i64(f(1)); }",
    );
    assert!(
        ir_text.contains("define i64 @__lambda_0(i8* %env, i64 %arg0)"),
        "the lambda is a function with an environment parameter:\n{ir_text}"
    );
    assert!(
        ir_text.contains("insertvalue { i8*, i8* } undef, i8* @__lambda_0, 0"),
        "the value is the lambda's address:\n{ir_text}"
    );
    assert!(
        ir_text.contains("i8* null, 1"),
        "a lambda that captures nothing has a null environment:\n{ir_text}"
    );
}

#[test]
fn two_lambdas_get_two_functions() {
    let ir_text = ir_of(
        "fn main() { const f = |x: i64| -> i64 { return x + 1; }; \
         const g = |x: i64| -> i64 { return x + 2; }; print_i64(f(1)); print_i64(g(1)); }",
    );
    assert!(
        ir_text.contains("define i64 @__lambda_0(i8* %env, i64 %arg0)"),
        "the first lambda:\n{ir_text}"
    );
    assert!(
        ir_text.contains("define i64 @__lambda_1(i8* %env, i64 %arg0)"),
        "the second lambda has its own function:\n{ir_text}"
    );
}

#[test]
fn a_lambda_may_contain_another_lambda() {
    // The inner lambda's body is written while the outer one's body is still
    // open, so the two bodies must not be written into the same buffer.
    let ir_text = ir_of(
        "fn main() { const f = |x: i64| -> i64 { \
         const g = |y: i64| -> i64 { return y; }; return g(x); }; print_i64(f(1)); }",
    );
    assert!(
        ir_text.contains("define i64 @__lambda_0(i8* %env, i64 %arg0)"),
        "the outer lambda:\n{ir_text}"
    );
    assert!(
        ir_text.contains("define i64 @__lambda_1(i8* %env, i64 %arg0)"),
        "the inner lambda has its own function:\n{ir_text}"
    );
    assert!(
        ir_text.contains("insertvalue { i8*, i8* } undef, i8* @__lambda_1, 0"),
        "the outer lambda's body builds the inner lambda's value:\n{ir_text}"
    );
}

#[test]
fn a_closure_carries_its_captures_in_an_environment() {
    // The environment is a block of its own: the captured value is copied into
    // it, and the closure reads it from there rather than from the scope it
    // was made in.
    let ir_text = ir_of(
        "fn adder(n: i64) -> fn(i64) -> i64 { \
         return |x: i64| -> i64 { return x + n; }; }",
    );
    assert!(
        ir_text.contains("call i8* @yashy_env_new(i64 1, i64 8, i64 16)"),
        "the environment is allocated with the captured value in it:\n{ir_text}"
    );
    assert!(
        ir_text.contains("getelementptr i8, ptr %env, i64 32"),
        "the closure reads its capture out of the environment:\n{ir_text}"
    );
}

#[test]
fn a_thrown_away_closure_releases_its_environment() {
    // A closure that is made and never used still owns an environment, and the
    // environment has to come back.
    let ir_text = ir_of(
        "fn main() { const n = 1; |x: i64| -> i64 { return x + n; }; }",
    );
    assert!(
        ir_text.contains("call void @yashy_env_release(i8* %"),
        "the discarded closure's environment is released:\n{ir_text}"
    );
}

#[test]
fn a_captured_string_is_retained_by_the_environment() {
    // The environment owns a reference to the string it captured, and the
    // table says so: the release at the end of the environment's life is the
    // runtime's, driven by the table the generated code wrote.
    let ir_text = ir_of(
        "fn greeter(greeting: string) -> fn() -> string { \
         return || -> string { return greeting; }; }",
    );
    assert!(
        ir_text.contains("call i8* @yashy_str_retain(i8* %"),
        "the environment retains the string it captured:\n{ir_text}"
    );
    assert!(
        ir_text.contains("store i64 1, ptr %"),
        "the table entry says the environment owns a reference:\n{ir_text}"
    );
}
