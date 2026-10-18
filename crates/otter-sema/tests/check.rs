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

#[test]
fn matches_some_and_binds_the_payload() {
    // Switch-expression arm bodies are bare expressions: `case None { 0 }`,
    // not `case None { 0; }`. The `;` belongs to the enclosing `return`.
    let (result, _, _) = check_source(
        "fn f(o: Option[i64]) -> i64 { return switch o { case None { 0 } case Some(x) { x } }; }",
    );
    result.expect("Some(x) should bind the i64 payload");
}

#[test]
fn matching_none_against_some_payload_is_an_error() {
    assert_error(
        "fn f(o: Option[i64]) -> i64 { return switch o { case Some(x) { x + true } default { 0 } }; }",
        "cannot apply",
    );
}

#[test]
fn none_is_typed_by_the_annotation_on_its_binding() {
    // `None` is the one constructor with no payload to infer from, so it can
    // only be typed from context. An annotated binding is such a context, and
    // the annotation has to be resolved *before* the initializer is checked for
    // that to work.
    let (result, _, _) = check_source("fn f() { const empty: Option[i64] = None; }");
    result.expect("the annotation should type `None`");
}

#[test]
fn none_is_typed_by_the_parameter_it_is_passed_to() {
    // Call arguments are checked against the parameter's declared type, which is
    // what gives `None` a payload type here. Checking the arguments first and
    // comparing afterwards works for every other expression but cannot type a
    // constructor that has nothing of its own to infer from.
    let (result, _, _) = check_source(
        "fn unwrap_or(o: Option[i64]) -> i64 { return switch o { case Some(x) { x } case None { 0 } }; } \
         fn f() -> i64 { return unwrap_or(None); }",
    );
    result.expect("the parameter type should type `None`");
}

#[test]
fn none_without_any_context_says_what_is_missing() {
    // With no annotation and no parameter there is nothing to infer from, and
    // the diagnostic should say that rather than reporting a spelling problem.
    // "cannot find value `None` in this scope" would send the reader looking for
    // a typo instead of for a type.
    assert_error(
        "fn f() { const x = None; }",
        "cannot infer the payload type of `None`",
    );
}

#[test]
fn a_bare_none_is_a_constructor_not_a_variable() {
    // With the wrong expected type the complaint is a type error naming the
    // expected type, which is only possible if `None` was resolved as the
    // constructor it is.
    assert_error("fn f() { const x: i64 = None; }", "expected `i64`, found `Option[?]`");
}

#[test]
fn result_ok_and_err_bind_different_payload_types() {
    let (result, _, _) = check_source(
        "fn f(r: Result[i64, string]) -> i64 { return switch r { case Ok(v) { v } case Err(e) { 0 } }; }",
    );
    result.expect("Ok(v) binds i64 and Err(e) binds string");
}

#[test]
fn try_on_a_result_returns_the_ok_value() {
    let (result, _, _) = check_source(
        "fn g() -> Result[i64, string] { return Ok(7); } fn f() -> Result[i64, string] { const v = g()?; return Ok(v); }",
    );
    result.expect("`?` on Result[i64, string] yields i64");
}

#[test]
fn try_on_a_non_result_is_an_error() {
    assert_error(
        "fn f() -> i64 { const v = 1?; return v; }",
        "`?` can only be used on a `Result`",
    );
}

#[test]
fn non_exhaustive_switch_on_enum_is_an_error_listing_missing_cases() {
    assert_error(
        "enum C { Red, Green, Blue } fn f(c: C) -> i64 { return switch c { case Red { 0 } }; }",
        "not exhaustive",
    );
}

#[test]
fn all_arms_must_yield_the_same_type() {
    assert_error(
        "enum C { Red, Green } fn f(c: C) -> i64 { return switch c { case Red { 0 } case Green { 1.0 } }; }",
        "expected `i64`, found `f64`",
    );
}

#[test]
fn switch_on_a_non_enum_without_default_is_an_error() {
    assert_error(
        "fn f(x: i64) -> i64 { return switch x { case 1 { 0 } }; }",
        "add a `default` arm",
    );
}

#[test]
fn return_inside_an_if_yields_the_function_return_type() {
    // Regression: nested blocks used to be checked against `()`, so any
    // `return <value>` inside an `if` or a loop body was rejected with a
    // nonsensical "expected `()`, found `i64`".
    check_source("fn f(x: bool) -> i64 { if x { return 1; } return 0; }")
        .0
        .expect("a `return` inside an `if` yields the function's return type");
}

#[test]
fn return_inside_an_else_yields_the_function_return_type() {
    check_source("fn f(x: bool) -> i64 { if x { return 1; } else { return 2; } }")
        .0
        .expect("a `return` inside an `else` yields the function's return type");
}

#[test]
fn return_inside_a_loop_body_yields_the_function_return_type() {
    check_source("fn f() -> i64 { for i in 0..3 { return i; } return 0; }")
        .0
        .expect("a `return` inside a loop body yields the function's return type");
    check_source("fn g() -> i64 { for i := 0; i < 3; print_i64(i) { return i; } return 0; }")
        .0
        .expect("and so does a `return` inside a C-style loop");
}

#[test]
fn a_nested_return_still_has_to_match_the_return_type() {
    assert_error(
        "fn f(x: bool) -> i64 { if x { return \"s\"; } return 0; }",
        "expected `i64`, found `string`",
    );
}

#[test]
fn an_unknown_type_name_in_a_signature_is_an_error() {
    // Regression: signature types used to be resolved with
    // `unwrap_or_else(|_| Never)`, so `double` (not a type in Otter) silently
    // became a `void` parameter instead of a diagnostic.
    assert_error(
        "fn f(b: double) -> i64 { return 0; }",
        "cannot find type `double`",
    );
    assert_error(
        "fn g() -> double { return 0; }",
        "cannot find type `double`",
    );
}

#[test]
fn an_unknown_type_name_in_a_struct_field_is_an_error() {
    assert_error(
        "struct S { w: double } fn f() -> i64 { return 0; }",
        "cannot find type `double`",
    );
}

// -- assignment -------------------------------------------------------------

#[test]
fn assigning_a_var_binding_is_legal() {
    check_source("fn main() { var x = 1; x = 2; }")
        .0
        .expect("a `var` binding exists to be assigned to");
}

#[test]
fn assigning_a_const_is_an_error() {
    // The whole point of having two binding forms is that one of them refuses
    // assignment. A `const` that accepts it is a `var` with extra steps.
    assert_error(
        "fn main() { const x = 1; x = 2; }",
        "cannot assign to `x`, which is immutable",
    );
}

#[test]
fn assigning_an_undeclared_name_is_an_error() {
    // Same wording as reading an undeclared name: one situation, one message.
    assert_error("fn main() { x = 2; }", "cannot find value `x` in this scope");
}

#[test]
fn the_assigned_value_must_match_the_binding_type() {
    assert_error(
        "fn main() { var x = 1; x = 2.0; }",
        "expected `i64`, found `f64`",
    );
}

#[test]
fn only_a_var_binding_can_be_assigned() {
    // The parser builds the target as an expression and lets the checker decide
    // what is assignable, so this is where the decision is made.
    assert_error("fn main() { 1 = 2; }", "only a `var` binding can be assigned");
}

#[test]
fn assignment_resolves_the_innermost_binding() {
    // `x` inside the `if` is a different binding from `x` outside it. If the
    // checker resolved the wrong one, one of these two assignments would be
    // judged against the other's type: the inner `x` is an `f64` and the outer
    // one an `i64`, so a mixed-up lookup cannot pass both.
    check_source(
        "fn main() { var x = 1; if true { var x = 2.0; x = 3.0; } x = 4; }",
    )
    .0
    .expect("each assignment must be judged against the binding it names");
}

#[test]
fn an_assignment_is_not_a_binding() {
    // Assignment adds no name to the scope, so assigning to a name that was
    // only ever assigned to is still an error -- otherwise `x = 1;` would
    // silently declare `x`.
    assert_error(
        "fn main() { x = 1; x = 2; }",
        "cannot find value `x` in this scope",
    );
}
