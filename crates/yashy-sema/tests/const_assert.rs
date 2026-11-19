//! `const_assert` is a check, not a runtime operation: the compiler evaluates
//! the condition with the same interpreter that folds `const` initializers.

use yashy_common::{codes, Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program_with, CheckOptions, HirProgram};

fn check(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sm = SourceMap::new();
    let file = sm.add_file("test.ya".to_string(), src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser.parse_program().expect("the source must parse");
    check_program_with(&ast, &sm, file, CheckOptions::default())
}

#[test]
fn a_true_compile_time_assertion_is_accepted() {
    check("fn main() { const_assert(2 + 2 == 4); }").expect("the compiler can prove this");
}

#[test]
fn a_false_compile_time_assertion_is_e1525() {
    let diagnostics =
        check("fn main() { const_assert(2 + 2 == 5); }").expect_err("the fact is false");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == Some(codes::CONST_ASSERT)),
        "expected E1525, got {diagnostics:?}"
    );
}

#[test]
fn a_const_fn_result_can_be_asserted() {
    check(
        "const fn square(int n) -> int { return n * n; }\n\
         fn main() { const_assert(square(3) == 9); }",
    )
    .expect("const fn calls fold, so the condition is a compile-time value");
}

#[test]
fn a_runtime_name_is_not_a_compile_time_condition() {
    let diagnostics = check("fn main() { const n = 1; const_assert(n == 1); }")
        .expect_err("the interpreter does not see local bindings");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == Some(codes::CONST_NOT_CONSTANT)),
        "expected E1524, got {diagnostics:?}"
    );
}

#[test]
fn runtime_assert_is_unchanged() {
    check("fn main() { assert(1 == 1); }").expect("the runtime spelling still checks");
}
