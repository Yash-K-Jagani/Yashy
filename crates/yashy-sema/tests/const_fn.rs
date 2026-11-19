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
    // An expression whose value only exists when the program runs is refused
    // by name: a switch is parsed as a value here, not as a statement with
    // `return` in its arms.
    assert_error(
        "const fn f(int n) -> int { return switch n { default { 0 } }; }\n\
         fn g() { const t = f(3); }",
        "is not allowed in a const fn",
    );
}

#[test]
fn a_const_fn_may_accumulate_in_a_loop() {
    let (result, _, _) = check_source(
        "const fn sum_to(int n) -> int {\n\
             let int total = 0;\n\
             let i = 1;\n\
             while i <= n {\n\
                 total += i;\n\
                 i += 1;\n\
             }\n\
             return total;\n\
         }\n\
         fn f() { const t = sum_to(10); }",
    );
    let hir = result.expect("the compiler folds a loop it can run itself");
    let f = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(function) if function.name == "f" => Some(function),
            _ => None,
        })
        .expect("the function");
    assert!(
        format!("{:?}", f.body).contains("I64(55)"),
        "the sum 1..=10 is a literal in the tree: {:?}",
        f.body
    );
}

#[test]
fn a_break_leaves_a_const_fn_loop() {
    let (result, _, _) = check_source(
        "const fn first_above(int n) -> int {\n\
             let i = 0;\n\
             while i < 100 {\n\
                 if i > n {\n\
                     break;\n\
                 }\n\
                 i += 1;\n\
             }\n\
             return i;\n\
         }\n\
         fn f() { const t = first_above(7); }",
    );
    let hir = result.expect("a `break` ends a loop the compiler is running");
    let f = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(function) if function.name == "f" => Some(function),
            _ => None,
        })
        .expect("the function");
    assert!(
        format!("{:?}", f.body).contains("I64(8)"),
        "the loop stops at the first value above 7: {:?}",
        f.body
    );
}

#[test]
fn a_runaway_const_loop_is_stopped_by_the_step_budget() {
    assert_error(
        "const fn spin() -> int { let i = 0; while true { } return i; }\n\
         fn f() { const t = spin(); }",
        "exceeded",
    );
}

#[test]
fn a_const_fn_cannot_assign_to_its_parameter() {
    assert_error(
        "const fn f(int n) -> int { n = 2; return n; }\n\
         fn g() { const t = f(1); }",
        "cannot assign to `n`, which is immutable",
    );
}

#[test]
fn a_const_fn_local_must_match_its_written_type() {
    assert_error(
        "const fn f() -> int { let int x = true; return 0; }\n\
         fn g() { const t = f(); }",
        "expected `int`, found `bool`",
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
fn const_eval_failures_carry_stable_codes() {
    let cases = [
        (
            "const fn f() -> int { return 9223372036854775807 + 1; }\nfn main() { const x = f(); }",
            "E1520",
        ),
        (
            "const fn f() -> int { return 8 / 0; }\nfn main() { const x = f(); }",
            "E1521",
        ),
        (
            "const fn loop(int n) -> int { return loop(n + 1); }\nfn main() { const x = loop(0); }",
            "E1523",
        ),
        (
            "fn f() { let n = 9; const x = square(n); }\nconst fn square(int n) -> int { return n * n; }",
            "E1524",
        ),
    ];
    for (source, expected) in cases {
        let (result, _, _) = check_source(source);
        let diagnostics = result.expect_err("the const expression must fail");
        assert!(
            diagnostics.iter().any(|d| d.code == Some(expected)),
            "expected {expected}, got {diagnostics:?}"
        );
    }
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
    assert_eq!(error.code, Some("E1522"));
    let _ = ConstValue::I64(1);
}

#[test]
fn a_const_fn_can_build_and_index_a_lookup_table() {
    let (result, _, _) = check_source(
        "const fn table(int n) -> [int; 4] { return [n, n + 1, n + 2, n + 3]; }\n\
         fn f() { const t = table(10); const x = table(10)[2]; }",
    );
    let hir = result.expect("a table is a fixed array the compiler can build");
    let f = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(function) if function.name == "f" => Some(function),
            _ => None,
        })
        .expect("the function");
    let text = format!("{:?}", f.body);
    assert!(
        text.contains("FixedArrayLit") && text.contains("I64(12)"),
        "the table and its indexed entry are literals in the tree: {text}"
    );
    assert!(
        !text.contains("table"),
        "no const fn call survives into the tree: {text}"
    );
}

#[test]
fn an_out_of_bounds_index_into_a_const_table_is_reported() {
    assert_error(
        "const fn table() -> [int; 2] { return [1, 2]; }\n\
         fn f() { const x = table()[9]; }",
        "index 9 is outside a fixed array of length 2",
    );
}

#[test]
fn a_const_fn_cannot_build_a_table_of_strings() {
    assert_error(
        "const fn bad() -> [string; 2] { return [\"a\", \"b\"]; }\n\
         fn f() { const t = bad(); }",
        "a fixed array cannot hold `string`",
    );
}
