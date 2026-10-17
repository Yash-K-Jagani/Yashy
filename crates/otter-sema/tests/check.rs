//! Type checker tests: every test builds a small program, runs it through
//! the full pipeline (lex, parse, check), and asserts either the inferred
//! type of an expression or the diagnostic that was produced.
//!
//! The pipeline helper is deliberately local to this file: it is the same
//! sequence `otter-driver` will eventually run, written once here so the
//! tests never depend on the driver existing.

use otter_common::{Diagnostic, FileId, Severity, SourceMap};
use otter_lexer::Lexer;
use otter_parser::Parser;
use otter_sema::{check_program, expr_ty, HirItem, HirProgram, HirStmt, TypeKind};

/// Lex, parse and check `source`.
fn check_source(src: &str) -> (Result<HirProgram, Vec<Diagnostic>>, SourceMap, FileId) {
    let mut sm = SourceMap::new();
    let file = sm.add_file("test.ot", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("lexer should never panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser
        .parse_program()
        .expect("parser should never panic on these inputs");
    let result = check_program(&ast, &sm, file);
    (result, sm, file)
}

/// Assert that `src` type-checks, and return the type of the expression in
/// the first `const` binding named `needle`.
fn assert_type(src: &str, needle: &str, expected: TypeKind) {
    let (result, _, _) = check_source(src);
    let hir = result.expect("expected program to type-check");
    let item = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(f) if f.name == needle => Some(f),
            _ => None,
        })
        .expect("function should exist");
    let init = match item.body.stmts.first() {
        Some(HirStmt::Const { init, .. }) => init,
        other => panic!("expected a const binding, got {:?}", other),
    };
    let ty = expr_ty(init);
    assert_eq!(
        ty.kind, expected,
        "type of `{needle}` should be {expected:?}, got {ty}"
    );
}

/// Assert that `src` fails to type-check with a message containing `needle`.
fn assert_error(src: &str, needle: &str) {
    let (result, _, _) = check_source(src);
    match result {
        Ok(_) => panic!("expected a type error containing `{needle}`"),
        Err(diags) => {
            assert!(
                diags.iter().any(|d| d.message.contains(needle)),
                "expected an error containing `{needle}`, got {:?}",
                diags.iter().map(|d| &d.message).collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn infers_i64_from_integer_literal() {
    assert_type("fn f() { const x = 1; }", "f", TypeKind::I64);
}

#[test]
fn infers_f64_from_float_literal() {
    assert_type("fn f() { const x = 1.5; }", "f", TypeKind::F64);
}

#[test]
fn function_parameters_and_returns_are_checked() {
    let (result, _, _) = check_source("fn f(a: i64) -> i64 { return a; }");
    result.expect("fn with matching signature should type-check");
}

#[test]
fn wrong_return_type_is_a_diagnostic_with_a_span() {
    let (result, sm, file) = check_source("fn f() -> i64 { return 1.0; }");
    let diags = result.expect_err("returning f64 where i64 is expected must fail");
    assert_eq!(diags.len(), 1);
    let d = &diags[0];
    assert_eq!(d.severity, Severity::Error);
    assert!(
        d.message.contains("i64"),
        "message should name the expected type: {}",
        d.message
    );
    let loc = sm.locate(file, d.span);
    // The literal `1.0` is at line 1, column 24.
    assert_eq!((loc.line, loc.col), (1, 24));
}

#[test]
fn duplicate_binding_in_one_scope_is_an_error_not_last_wins() {
    assert_error(
        "fn f() { const x = 1; const x = 2; }",
        "duplicate definition of `x`",
    );
}

#[test]
fn two_functions_named_main_is_an_error() {
    assert_error(
        "fn main() {} fn main() {}",
        "duplicate definition of `main`",
    );
}

#[test]
fn no_implicit_narrowing_conversion() {
    assert_error(
        "fn f(a: i64) -> f64 { return a; }",
        "expected `f64`, found `i64`",
    );
}

#[test]
fn explicit_as_cast_permits_an_intentional_conversion() {
    let (result, _, _) = check_source("fn f(a: i64) -> f64 { return a as f64; }");
    result.expect("i64 as f64 is a permitted cast");
}

#[test]
fn as_cast_to_an_unrelated_type_is_an_error() {
    assert_error(
        "fn f() -> bool { return 1 as bool; }",
        "cannot cast `i64` to `bool`",
    );
}

#[test]
fn as_cast_rejects_conversion_to_struct() {
    assert_error(
        "struct P { x: i64 } fn f(p: P) -> P { return p as P; }",
        "cannot cast",
    );
}

#[test]
fn struct_literal_fields_are_checked_by_name() {
    assert_error(
        "struct P { x: f64, y: f64 } fn f() { const p = P { x: 1, y: 2 }; }",
        "expected `f64`, found `i64`",
    );
}

#[test]
fn unknown_field_in_struct_literal_is_an_error() {
    assert_error(
        "struct P { x: f64 } fn f() { const p = P { z: 1.0 }; }",
        "unknown field `z` in struct `P`",
    );
}

#[test]
fn if_condition_must_be_bool() {
    assert_error("fn f() { if 1 { } }", "expected `bool`, found `i64`");
}

#[test]
fn builtin_calls_resolve() {
    let (result, _, _) = check_source("fn f() { print_i64(1); }");
    result.expect("print_i64(1) should type-check");
    assert_error("fn f() { print_str(1); }", "expected `string`, found `i64`");
}
