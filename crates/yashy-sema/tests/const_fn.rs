//! `const fn` checks: the full pipeline folds a call and reports mistakes at
//! the call site. This file is the named gate from the Phase 4 plan.

use yashy_common::{Diagnostic, FileId, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program_with, CheckOptions, HirItem, HirProgram};

fn check_source(src: &str) -> (Result<HirProgram, Vec<Diagnostic>>, SourceMap, FileId) {
    let mut sm = SourceMap::new();
    let file = sm.add_file("test.ya".to_string(), src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser.parse_program().expect("the source must parse");
    let result = check_program_with(&ast, &sm, file, CheckOptions::default());
    (result, sm, file)
}

fn assert_error(src: &str, needle: &str) {
    let (result, _, _) = check_source(src);
    let diags = result.expect_err("the program must fail the check");
    assert!(
        diags.iter().any(|d| d.message.contains(needle)),
        "expected `{needle}`, got {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn a_const_fn_call_is_folded_into_the_value_it_computed() {
    let (result, _, _) = check_source(
        "const fn square(int n) -> int { return n * n; }\n\
         fn f() { const t = square(9); }",
    );
    let hir = result.expect("a const fn call should fold");
    let f = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(function) if function.name == "f" => Some(function),
            _ => None,
        })
        .expect("the function should exist");
    let text = format!("{:?}", f.body);
    assert!(
        text.contains("I64(81)"),
        "the folded value is in the tree: {text}"
    );
    assert!(
        !text.contains("square"),
        "the call is not in the tree: {text}"
    );
}

#[test]
fn a_const_fn_is_not_emitted_as_a_function() {
    let (result, _, _) = check_source(
        "const fn square(int n) -> int { return n * n; }\n\
         fn f() { const t = square(9); }",
    );
    let hir = result.expect("the program should check");
    assert!(
        !hir.items
            .iter()
            .any(|item| matches!(item, HirItem::Fn(f) if f.name == "square")),
        "a const fn is not a runtime function"
    );
}

#[test]
fn a_const_fn_body_may_only_use_what_the_compiler_can_compute() {
    assert_error(
        "const fn f() -> int { return read_line(); }\n\
         fn g() { const t = f(); }",
        "`read_line` is not allowed in a const fn",
    );
    assert_error(
        "const fn f(int n) -> int { let i = 0; while i < n { i = i + 1; } return i; }\n\
         fn g() { const t = f(3); }",
        "is not allowed in a const fn",
    );
}

#[test]
fn a_const_fn_cannot_be_called_at_run_time() {
    assert_error(
        "const fn square(int n) -> int { return n * n; }\n\
         fn f() { let t = square(9); }",
        "is a `const fn`",
    );
}

#[test]
fn a_const_fn_may_call_another_const_fn() {
    let (result, _, _) = check_source(
        "const fn square(int n) -> int { return n * n; }\n\
         const fn fourth(int n) -> int { return square(square(n)); }\n\
         fn f() { const t = fourth(3); }",
    );
    let hir = result.expect("a const fn may call another const fn");
    let f = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(function) if function.name == "f" => Some(function),
            _ => None,
        })
        .expect("the function");
    assert!(format!("{:?}", f.body).contains("I64(81)"));
}

#[test]
fn a_const_fn_that_recurses_forever_is_reported() {
    assert_error(
        "const fn loop(int n) -> int { return loop(n + 1); }\n\
         fn f() { const t = loop(0); }",
        "did not terminate",
    );
}

#[test]
fn evaluation_is_bounded_by_a_step_fuel() {
    use yashy_common::Span;
    use yashy_sema::const_eval::{evaluate, ConstFns, Missing, Work};
    use yashy_sema::ConstValue;

    let mut work = Work::default();
    work.steps = 1_000_000;
    let expression =
        yashy_parser::ast::Expr::Literal(yashy_parser::ast::Literal::I64(1), Span::new(0, 1));
    let result = evaluate(
        &expression,
        &[],
        &ConstFns::new(),
        &mut work,
        Missing::NotFound,
    );
    let error = result.expect_err("the fuel must be exhausted");
    assert!(error.message.contains("steps"), "{error:?}");
    let _ = ConstValue::I64(1);
}
