//! Type checker tests: every test builds a small program, runs it through
//! the full pipeline (lex, parse, check), and asserts either the inferred
//! type of an expression or the diagnostic that was produced.
//!
//! The pipeline helper is deliberately local to this file: it is the same
//! sequence `otter-driver` will eventually run, written once here so the
//! tests never depend on the driver existing.

use otter_common::{Diagnostic, FileId, Severity, SourceMap};
use otter_lexer::Lexer;
use otter_parser::ast::Literal;
use otter_parser::Parser;
use otter_sema::{check_program, expr_ty, HirExpr, HirItem, HirProgram, HirStmt, TypeKind};

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
fn an_unknown_function_does_not_hide_its_arguments_errors() {
    // Resolving the callee before its arguments is what lets `f(None)` infer
    // what `None` is an `Option` of. It must not also mean that an unknown
    // callee swallows the arguments: `nope(bad_var)` has two independent
    // problems, and reporting one of them costs the reader a second build to
    // discover the other.
    let (result, _, _) = check_source("fn main() { nope(bad_var); }");
    let diags = match result {
        Ok(_) => panic!("expected the program to fail to check"),
        Err(diags) => diags,
    };
    let messages: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
    assert!(
        messages
            .iter()
            .any(|m| m.contains("cannot find function `nope`")),
        "the unknown callee should be reported: {messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|m| m.contains("cannot find value `bad_var`")),
        "the bad argument should be reported too: {messages:?}"
    );
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

/// The `n`th statement of the function called `name`, as a checked expression.
fn expr_of_statement(src: &str, name: &str, n: usize) -> HirExpr {
    let (result, _, _) = check_source(src);
    let hir = result.expect("expected program to type-check");
    let f = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(f) if f.name == name => Some(f),
            _ => None,
        })
        .expect("function should exist");
    match f.body.stmts.get(n) {
        Some(HirStmt::Const { init, .. }) => init.clone(),
        other => panic!("expected a const binding at {n}, got {:?}", other),
    }
}

#[test]
fn field_access_has_the_fields_declared_type() {
    // `p` is a parameter, so the field access is the first statement and
    // `assert_type` can read it.
    assert_type(
        "struct P { x: i64, y: f64 } fn f(p: P) { const a = p.y; }",
        "f",
        TypeKind::F64,
    );
}

#[test]
fn field_access_carries_the_declaration_order_index() {
    // The index is what the backend extracts with, so it has to be the position
    // in the declaration -- not the position in any literal.
    let expr = expr_of_statement(
        "struct P { x: i64, y: f64 } fn f(p: P) { const a = p.y; }",
        "f",
        0,
    );
    match expr {
        HirExpr::FieldAccess { field, index, ty, .. } => {
            assert_eq!(field, "y");
            assert_eq!(index, 1, "`y` is the second declared field");
            assert_eq!(ty.kind, TypeKind::F64);
        }
        other => panic!("expected a field access, got {:?}", other),
    }
}

#[test]
fn a_dotted_name_reads_fields_of_nested_structs() {
    let expr = expr_of_statement(
        "struct P { x: i64, y: i64 }\n\
         struct L { from: P, to: P }\n\
         fn f(l: L) { const a = l.from.y; }",
        "f",
        0,
    );
    let outer = match expr {
        HirExpr::FieldAccess { field, index, ty, base, .. } => {
            assert_eq!(field, "y", "the tail of the chain is the last field written");
            assert_eq!(index, 1);
            assert_eq!(ty.kind, TypeKind::I64);
            base
        }
        other => panic!("expected a field access chain, got {:?}", other),
    };
    match *outer {
        HirExpr::FieldAccess { ref field, index, .. } => {
            assert_eq!(field, "from");
            assert_eq!(index, 0, "`from` is the first declared field of `L`");
        }
        other => panic!("expected `l.from` as the base, got {:?}", other),
    }
}

#[test]
fn a_field_that_the_struct_does_not_declare_is_an_error() {
    assert_error(
        "struct P { x: i64 } fn f(p: P) { const a = p.z; }",
        "struct `P` has no field `z`",
    );
}

#[test]
fn a_field_access_on_a_non_struct_is_an_error() {
    assert_error(
        "fn f(n: i64) { const a = n.x; }",
        "cannot access a field on `i64`",
    );
}

#[test]
fn a_struct_literal_is_laid_out_in_declaration_order() {
    // Written `y, x`; the HIR must be `x, y`, because that is the order the IR
    // aggregate is built in and the order a field's index refers to.
    let expr = expr_of_statement(
        "struct P { x: i64, y: i64 } fn f() { const p = P { y: 2, x: 1 }; }",
        "f",
        0,
    );
    match expr {
        HirExpr::StructLit { fields, .. } => {
            let names: Vec<&str> = fields.iter().map(|(n, _, _)| n.as_str()).collect();
            assert_eq!(names, vec!["x", "y"], "declaration order, not written order");
        }
        other => panic!("expected a struct literal, got {:?}", other),
    }
}

#[test]
fn a_while_condition_must_be_bool() {
    // `while` parses to the condition-only `ForC`, so it gets the loop's
    // condition rule for free: the checker requires `bool` exactly as it does
    // for `if`.
    let (result, _, _) = check_source("fn f(flag: bool) { while flag { } }");
    result.expect("a bool condition should type-check");
    assert_error("fn f(n: i64) { while n { } }", "expected `bool`, found `i64`");
}

#[test]
fn an_empty_body_after_a_bare_name_still_type_checks() {
    // The parser change that made `while flag { }` a loop with an empty body
    // (rather than `flag {}` plus a missing body) is only useful if the checker
    // then accepts it. A busy-wait is `while running { }` and nothing else.
    let (result, _, _) = check_source("fn f(flag: bool) { if flag { } while flag { } }");
    result.expect("empty bodies should type-check");
    let (range, _, _) = check_source("fn f(n: i64) { for i in 0..n { } }");
    range.expect("an empty range loop should type-check");
}

#[test]
fn compound_assignment_is_an_assignment_of_the_expanded_value() {
    // The parser desugars, so the checker sees `n = n + 1` and applies its
    // existing rules -- including the rule about *what* can be assigned to,
    // which is why an immutable binding is rejected exactly as it would be.
    let (result, _, _) = check_source("fn f() { var n = 0; n += 1; n -= 1; n *= 2; n /= 2; n %= 2; }");
    result.expect("every compound operator should type-check on a `var`");
    assert_error(
        "fn f() { const n = 0; n += 1; }",
        "cannot assign to `n`, which is immutable",
    );
    // And the right operand meets the operator's rule, because the expanded
    // form is what the checker saw: `n += 1.5` is `n = n + 1.5`.
    assert_error(
        "fn f() { var n = 0; n += 1.5; }",
        "cannot apply `+` to `i64` and `f64`",
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

/// The three conversions a program uses to put a value inside a string.
fn assert_conversion(param: &str, arg: &str, expected: TypeKind) {
    let src = format!("fn f(x: {param}) {{ const s = str_from_{param}(x); }}");
    assert_type(&src, "f", expected.clone());
    let wrong = format!("fn f() {{ const s = str_from_{param}({arg}); }}");
    assert_error(&wrong, "found");
}

#[test]
fn the_value_to_string_conversions_resolve_and_check_their_argument() {
    // Each conversion takes exactly one type, so a wrong argument is a type
    // error rather than a call to a runtime function that would read the
    // register as something else.
    assert_conversion("i64", "\"text\"", TypeKind::Str);
    assert_conversion("f64", "1", TypeKind::Str);
    assert_conversion("bool", "1", TypeKind::Str);
}

#[test]
fn a_conversion_no_longer_leaves_the_runtime_unreachable() {
    // `otter_str_from_i64` and friends have existed since the M3 string work
    // and nothing could call them; `print_i64` was the only way to show a
    // number. This is the pin that they are reachable from the language.
    let (result, _, _) = check_source(
        "fn f(n: i64) { const line = \"n = \" + str_from_i64(n); }",
    );
    result.expect("a built line should type-check");
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

// ---- M3 Task 2: pointer types and linear ownership ----------------------

/// The printed type of the first binding in function `needle`.
fn binding_type_in_fn(src: &str, needle: &str) -> String {
    let (result, _, _) = check_source(src);
    let hir = result.expect("expected the program to type-check");
    let item = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(f) if f.name == needle => Some(f),
            _ => None,
        })
        .expect("function should exist");
    match item.body.stmts.first() {
        Some(HirStmt::Const { init, .. }) | Some(HirStmt::Var { init, .. }) => {
            expr_ty(init).to_string()
        }
        other => panic!("expected a binding first, got {other:?}"),
    }
}

#[test]
fn a_pointer_type_prints_as_a_mutable_pointer() {
    let (result, _, _) = check_source("fn f(p: *mut i64) -> *mut i64 { return p; }");
    let hir = result.expect("a pointer parameter and return type should check");
    let item = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(f) if f.name == "f" => Some(f),
            _ => None,
        })
        .expect("f should exist");
    let param = item.params.first().expect("f has one parameter");
    assert_eq!(param.ty.to_string(), "*mut i64");
}

#[test]
fn alloc_returns_a_pointer_to_i64() {
    assert_eq!(
        binding_type_in_fn("fn f() { const p = alloc(16); }", "f"),
        "*mut i64"
    );
}

#[test]
fn a_pointer_annotation_and_alloc_agree() {
    // `*mut i64` is a real type, not a spelling: an annotated binding accepts
    // `alloc`'s result, which it would not if the two disagreed.
    let (result, _, _) = check_source("fn f() { const p: *mut i64 = alloc(16); }");
    assert!(result.is_ok(), "expected `*mut i64` to accept alloc: {result:?}");
}

#[test]
fn a_pointer_cannot_be_initialised_from_an_integer() {
    assert_error(
        "fn f() { const p: *mut i64 = 0; }",
        "expected `*mut i64`, found `i64`",
    );
}

#[test]
fn assigning_a_linear_binding_without_move_is_an_error() {
    assert_error(
        "fn f() { var buf = alloc(16); var other = buf; }",
        "cannot assign linear value `buf` without `move`",
    );
}

#[test]
fn moving_a_linear_binding_is_legal_and_releases_it() {
    let (result, _, _) = check_source(
        "fn f() { var buf = alloc(16); var other = move buf; dealloc(other); }",
    );
    assert!(result.is_ok(), "move should transfer ownership: {result:?}");
}

#[test]
fn reading_a_moved_binding_is_an_error() {
    assert_error(
        "fn f() { var buf = alloc(16); var other = move buf; dealloc(buf); }",
        "value `buf` has already been moved",
    );
}

#[test]
fn a_second_dealloc_of_the_same_pointer_is_an_error() {
    assert_error(
        "fn f() { var buf = alloc(16); dealloc(buf); dealloc(buf); }",
        "value `buf` has already been moved",
    );
}

#[test]
fn rebinding_a_var_releases_the_previous_linear_value() {
    let (result, _, _) = check_source(
        "fn f() { var buf = alloc(16); buf = alloc(32); dealloc(buf); }",
    );
    assert!(result.is_ok(), "a rebind installs a fresh owner: {result:?}");
}

#[test]
fn rebinding_a_var_from_another_linear_binding_needs_move() {
    assert_error(
        "fn f() { var buf = alloc(16); var other = alloc(32); buf = other; }",
        "cannot assign linear value `other` without `move`",
    );
}

// -- string interpolation ---------------------------------------------------

#[test]
fn an_interpolation_checks_as_a_string() {
    assert_type(
        "fn f() { const line = \"n = ${1}\"; }",
        "f",
        TypeKind::Str,
    );
    assert_type("fn f() { const line = \"${1} text\"; }", "f", TypeKind::Str);
}

#[test]
fn an_interpolation_lowers_to_the_matching_conversion_and_a_concatenation() {
    // The parser cannot pick a conversion -- it does not know the type -- so the
    // checker desugars `${n}` into `str_from_i64(n)` and joins it to the text
    // with `+`. That is the pin that interpolation is *sugar*: code generation
    // sees only a call and a string concatenation, the two things it already
    // knew how to lower.
    let src = "fn f(n: i64) { const line = \"n = ${n}\"; }";
    let expr = expr_of_statement(src, "f", 0);
    let HirExpr::Binary { op, lhs, rhs, .. } = &expr else {
        panic!("expected a concatenation, got {expr:?}");
    };
    assert_eq!(*op, otter_sema::BinOp::Add);
    assert!(
        matches!(
            lhs.as_ref(),
            HirExpr::Literal(otter_parser::ast::Literal::Str(lit), _, _) if lit.value == "n = "
        ),
        "the text piece, got {lhs:?}"
    );
    let HirExpr::Call { callee, args, .. } = rhs.as_ref() else {
        panic!("expected the conversion call, got {rhs:?}");
    };
    assert_eq!(callee, &vec!["str_from_i64".to_string()]);
    assert_eq!(args.len(), 1, "one argument: {args:?}");
}

#[test]
fn each_interpolated_type_gets_its_own_conversion() {
    // A wrong pick -- say every number going through `str_from_i64` -- would
    // compile and print nonsense, so each type must reach its own builtin.
    for (param, callee) in [
        ("i64", "str_from_i64"),
        ("f64", "str_from_f64"),
        ("bool", "str_from_bool"),
    ] {
        let src = format!("fn f(x: {param}) {{ const line = \"${{x}}\"; }}");
        let expr = expr_of_statement(&src, "f", 0);
        let HirExpr::Call { callee: got, .. } = &expr else {
            panic!("`{param}` must lower to a conversion call, got {expr:?}");
        };
        assert_eq!(got, &vec![callee.to_string()], "`{param}` converts with {callee}");
    }
}

#[test]
fn a_string_piece_is_used_as_it_is_without_a_conversion() {
    // A `string` already *is* text: wrapping it in anything would be a second
    // allocation and a place for the two to disagree.
    let src = "fn f(s: string) { const line = \"${s}\"; }";
    let expr = expr_of_statement(src, "f", 0);
    assert!(
        matches!(&expr, HirExpr::Ident(path, _, _) if path == &vec!["s".to_string()]),
        "a lone interpolation of a string is that string itself, got {expr:?}"
    );
}

#[test]
fn interpolating_a_type_with_no_text_form_is_an_error() {
    assert_error(
        "struct P { x: i64 } fn f(p: P) { const line = \"${p}\"; }",
        "cannot interpolate",
    );
    // A pointer is the dangerous one: printing its address would succeed and
    // mean nothing, which is the wrong answer the checker exists to refuse.
    assert_error(
        "fn f() { var p = alloc(1); const line = \"${p}\"; }",
        "cannot interpolate",
    );
}

#[test]
fn dealloc_wants_a_pointer_not_an_integer() {
    assert_error("fn f() { dealloc(1); }", "expected `*mut i64`, found `i64`");
}

#[test]
fn freeing_in_both_branches_of_an_if_is_legal() {
    // The branch check restores the entry ownership state before checking the
    // `else`, so a `then`-branch move does not poison the `else` branch. Freeing
    // in both branches is the shape of "free exactly once" without a `defer`.
    let (result, _, _) = check_source(
        "fn f(c: bool) { var buf = alloc(16); if c { dealloc(buf); } else { dealloc(buf); } }",
    );
    assert!(result.is_ok(), "both branches free: {result:?}");
}

#[test]
fn a_move_in_only_one_branch_is_conservatively_moved() {
    // The checker does not prove a path, so after `if c { dealloc(buf); }` a
    // later `dealloc(buf)` is rejected rather than risked: `buf` may have been
    // freed. Conservative by design -- see `ownership.rs`.
    assert_error(
        "fn f(c: bool) { var buf = alloc(16); if c { dealloc(buf); } dealloc(buf); }",
        "value `buf` has already been moved",
    );
}

#[test]
fn moving_a_copy_value_is_a_no_op() {
    // `i64` is not linear, so `move` asks for nothing: the value stays live and
    // readable. Being explicit about intent is not an error.
    let (result, _, _) = check_source("fn f() { var x = 1; var y = move x; print_i64(x + y); }");
    assert!(result.is_ok(), "move of a copy value is a no-op: {result:?}");
}

#[test]
fn a_linear_argument_to_a_user_function_is_moved_out() {
    // A parameter of linear type takes ownership, so `consume(buf)` moves `buf`
    // without a `move` keyword, and a later read is an error -- the same rule
    // `dealloc` follows, applied to user functions.
    assert_error(
        "fn consume(p: *mut i64) { dealloc(p); } \
         fn f() { var buf = alloc(16); consume(buf); dealloc(buf); }",
        "value `buf` has already been moved",
    );
}

#[test]
fn a_linear_argument_does_not_have_to_be_a_name() {
    // A fresh `alloc(...)` owns nothing yet, so passing it straight through a
    // call leaves no moved binding behind.
    let (result, _, _) = check_source(
        "fn consume(p: *mut i64) { dealloc(p); } fn f() { consume(alloc(16)); }",
    );
    assert!(result.is_ok(), "a temporary argument should check: {result:?}");
}

// -- panic --------------------------------------------------------------------

/// The `n`th statement of the function called `name`.
fn statement_of(src: &str, name: &str, n: usize) -> HirStmt {
    let (result, _, _) = check_source(src);
    let hir = result.expect("expected program to type-check");
    let f = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(f) if f.name == name => Some(f),
            _ => None,
        })
        .expect("function should exist");
    f.body
        .stmts
        .get(n)
        .cloned()
        .unwrap_or_else(|| panic!("expected a statement at {n}"))
}

#[test]
fn panic_is_a_builtin_whose_result_is_never() {
    // `!` and not `()`: a panic produces no value at all, and that is what lets
    // it stand in for a value of any type -- a `return`, a switch arm -- without
    // also letting a program *use* a value that does not exist.
    let HirStmt::Expr(expr, _, ty) = statement_of("fn f() { panic(\"stop\"); }", "f", 0) else {
        panic!("expected an expression statement");
    };
    assert_eq!(ty.kind, TypeKind::Never);
    assert_eq!(expr_ty(&expr).kind, TypeKind::Never);
}

#[test]
fn panic_carries_the_position_it_was_written_at() {
    // The file, line and column a panic reports are facts only the checker has:
    // the backend is handed IR text and a path, not a source map. So the checker
    // resolves the position and hands it down as ordinary literal arguments --
    // the message the program wrote, then the file, line and column.
    let HirStmt::Expr(HirExpr::Call { callee, args, ty, .. }, _, _) =
        statement_of("fn f() {\n    panic(\"boom\");\n}\n", "f", 0)
    else {
        panic!("expected the lowered panic call");
    };
    assert_eq!(callee, vec!["panic".to_string()]);
    assert_eq!(ty.kind, TypeKind::Never);
    let kinds: Vec<TypeKind> = args.iter().map(|a| expr_ty(a).kind.clone()).collect();
    assert_eq!(
        kinds,
        vec![TypeKind::Str, TypeKind::Str, TypeKind::I64, TypeKind::I64],
        "the message, the file, the line and the column"
    );
    let text = |e: &HirExpr| match e {
        HirExpr::Literal(Literal::Str(lit), _, _) => lit.value.clone(),
        other => panic!("expected a string literal, got {other:?}"),
    };
    let integer = |e: &HirExpr| match e {
        HirExpr::Literal(Literal::I64(value), _, _) => *value,
        other => panic!("expected an integer literal, got {other:?}"),
    };
    assert_eq!(text(&args[0]), "boom", "the message the program wrote");
    assert_eq!(text(&args[1]), "test.ot", "the file the compiler was given");
    assert_eq!(integer(&args[2]), 2, "the panic is on line 2");
    assert_eq!(integer(&args[3]), 5, "and starts at column 5");
}

#[test]
fn panic_takes_exactly_one_string() {
    assert_error("fn f() { panic(1); }", "expected `string`, found `i64`");
    assert_error("fn f() { panic(); }", "expected 1 arguments, found 0");
    assert_error(
        "fn f() { panic(\"a\", \"b\"); }",
        "expected 1 arguments, found 2",
    );
}

// -- checked arithmetic -------------------------------------------------------

/// Whether the tree marks this operation as one that can fault at run time.
///
/// The checker decides *which* operations those are -- integer arithmetic -- and
/// resolves the position for each; the backend decides how to check. So this
/// flag is the whole of the language-level contract, and a comparison carrying it
/// would be a position nothing could ever report from.
fn is_checked(expr: &HirExpr) -> bool {
    match expr {
        HirExpr::Binary { fault, .. } | HirExpr::Unary { fault, .. } => fault.is_some(),
        other => panic!("expected an arithmetic expression, got {other:?}"),
    }
}

#[test]
fn integer_arithmetic_is_checkable_and_carries_its_position() {
    // All five operations that can fault, each with the resolved position of its
    // own source text. The columns come from the fixture rather than from a
    // second copy of it, so a test that disagrees with the fixture says so.
    for (symbol, source_expr) in [
        ("add", "a + b"),
        ("subtract", "a - b"),
        ("multiply", "a * b"),
        ("divide", "a / b"),
        ("remainder", "a % b"),
    ] {
        let src = format!("fn f(a: i64, b: i64) {{\n    const x = {source_expr};\n}}\n");
        let checked = expr_of_statement(&src, "f", 0);
        let site = match &checked {
            HirExpr::Binary { fault: Some(site), .. } => site.clone(),
            other => panic!("`{symbol}` should be checkable, got {other:?}"),
        };
        let at = src
            .find(source_expr)
            .unwrap_or_else(|| panic!("the fixture must contain `{source_expr}`"));
        let before = &src[..at];
        let line = u32::try_from(before.matches('\n').count() + 1)
            .expect("the fixture is a few lines long");
        let line_start = before.rfind('\n').map_or(0, |index| index + 1);
        let column =
            u32::try_from(at - line_start + 1).expect("the fixture is a few columns wide");
        assert_eq!(site.file, "test.ot", "the file the compiler was given");
        assert_eq!(site.line, line, "the line of `{source_expr}`");
        assert_eq!(site.column, column, "its column");
    }
}

#[test]
fn comparisons_and_boolean_logic_are_not_checkable() {
    // None of these can produce a wrong answer: a comparison's result is either
    // the answer or the program is wrong about the types, which the checker has
    // already rejected.
    for source_expr in [
        "a < b",
        "a <= b",
        "a > b",
        "a >= b",
        "a == b",
        "a != b",
        "(a < b) && (b < a)",
        "(a < b) || (b < a)",
    ] {
        let src = format!("fn f(a: i64, b: i64) {{\n    const x = {source_expr};\n}}\n");
        let checked = expr_of_statement(&src, "f", 0);
        assert!(
            !is_checked(&checked),
            "`{source_expr}` cannot fault, so it must carry no position"
        );
    }
}

#[test]
fn float_and_string_arithmetic_are_not_checkable() {
    // An `f64` that overflows becomes an infinity, which is a float answer and
    // not a wrong one, and neither `fadd` nor `fdiv` traps. A `+` on strings is a
    // concatenation. Neither has a fault to report or a position to report it
    // from, which is a decision about the language and is made here.
    for (what, src) in [
        (
            "float add",
            "fn f(x: f64, y: f64) {\n    const s = x + y;\n}\n",
        ),
        (
            "float subtract",
            "fn f(x: f64, y: f64) {\n    const s = x - y;\n}\n",
        ),
        (
            "float multiply",
            "fn f(x: f64, y: f64) {\n    const s = x * y;\n}\n",
        ),
        (
            "float divide",
            "fn f(x: f64, y: f64) {\n    const s = x / y;\n}\n",
        ),
        (
            "float remainder",
            "fn f(x: f64, y: f64) {\n    const s = x % y;\n}\n",
        ),
        (
            "string concatenation",
            "fn f(s: string, t: string) {\n    const u = s + t;\n}\n",
        ),
    ] {
        let checked = expr_of_statement(src, "f", 0);
        assert!(
            !is_checked(&checked),
            "`{what}` cannot fault, so it must carry no position"
        );
    }
}

#[test]
fn negating_an_integer_is_checkable_and_everything_else_is_not() {
    // `-i64::MIN` wraps to itself, so a silent wraparound turns a mistake into a
    // plausible number. Negating any `f64`, infinity included, is exact, and `!`
    // is not arithmetic at all.
    let checked = expr_of_statement("fn f(a: i64) {\n    const x = -a;\n}\n", "f", 0);
    match &checked {
        HirExpr::Unary {
            fault: Some(site), ..
        } => {
            assert_eq!(site.file, "test.ot");
            assert_eq!(site.line, 2);
            assert_eq!(site.column, 15, "the negation starts at the `-`");
        }
        other => panic!("integer negation should be checkable, got {other:?}"),
    }

    for (what, src) in [
        ("float negation", "fn f(x: f64) {\n    const y = -x;\n}\n"),
        ("boolean not", "fn f(b: bool) {\n    const c = !b;\n}\n"),
    ] {
        let checked = expr_of_statement(src, "f", 0);
        assert!(
            !is_checked(&checked),
            "`{what}` cannot fault, so it must carry no position"
        );
    }
}

// -- the arithmetic opt-outs --------------------------------------------------

#[test]
fn the_arithmetic_builtins_have_the_types_they_claim() {
    // `wrapping_*` and `saturating_*` answer with an `i64`: two's complement
    // always has an answer, wrapped or clamped. `checked_*` answers with an
    // `Option`, because "there is no answer" is something the caller has to be
    // able to see.
    for name in [
        "wrapping_add",
        "wrapping_sub",
        "wrapping_mul",
        "saturating_add",
        "saturating_sub",
        "saturating_mul",
    ] {
        let src = format!("fn f(a: i64, b: i64) {{ const x = {name}(a, b); }}");
        assert_type(&src, "f", TypeKind::I64);
    }
    // The payload comes out through a match, which the checker could only do if
    // the result really were an `Option[i64]`: the `switch` has to cover both
    // variants, and its own type is the payload's.
    assert_type(
        "fn f(a: i64, b: i64) -> i64 {\n\
         const n = switch checked_add(a, b) {\n\
         case Some(v) { v }\n\
         case None { 0 }\n\
         };\n\
         return n;\n\
         }",
        "f",
        TypeKind::I64,
    );
}

#[test]
fn a_checked_result_cannot_be_used_without_opening_it() {
    // The point of the `Option` is that the number is not there when it does not
    // exist, so arithmetic on the result itself is a type error rather than a
    // wrapped answer -- `checked_add(a, b) + 1` has to be spelled as a match,
    // which is where the program decides what a missing answer means.
    assert_error(
        "fn f(a: i64, b: i64) { const x = checked_add(a, b) + 1; }",
        "cannot apply `+` to `Option[i64]` and `i64`",
    );
}

#[test]
fn the_arithmetic_builtins_take_exactly_two_arguments() {
    // They are ordinary functions with an ordinary arity, so a wrong call gets
    // the diagnostic every other call gets. A `wrapping_add` that quietly took
    // one argument would be a different operation with the same name.
    assert_error(
        "fn f(a: i64) { const x = wrapping_add(a); }",
        "expected 2 arguments, found 1",
    );
    assert_error(
        "fn f(a: i64) { const x = saturating_mul(a, 2, 3); }",
        "expected 2 arguments, found 3",
    );
}

#[test]
fn a_diverging_arm_does_not_decide_the_switch_type() {
    // `!` fits wherever a value is wanted, which is what makes
    // `case None { panic("...") }` a legal arm beside `case Some(v) { v }` -- in
    // either order. Without the rule the first arm would fix the switch's type at
    // `!` and the `return` would reject it, so a diverging arm could never be
    // anything but the last one.
    for arms in [
        "case None { panic(\"none\") } case Some(v) { v }",
        "case Some(v) { v } case None { panic(\"none\") }",
    ] {
        let src = format!("fn f(o: Option[i64]) -> i64 {{ return switch o {{ {arms} }}; }}");
        let HirStmt::Return(Some(expr), _) = statement_of(&src, "f", 0) else {
            panic!("expected a return statement");
        };
        assert_eq!(expr_ty(&expr).kind, TypeKind::I64, "arms: {arms}");
    }
}

#[test]
fn a_diverging_return_fits_any_return_type() {
    for ret in ["i64", "f64", "bool", "string"] {
        let src = format!("fn f() -> {ret} {{ return panic(\"nope\"); }}");
        let (result, _, _) = check_source(&src);
        assert!(
            result.is_ok(),
            "`return panic(..)` must satisfy a `{ret}` return: {result:?}"
        );
    }
}

#[test]
fn a_function_whose_only_statement_panics_checks() {
    // The body falls off the end, which Otter already allows -- code generation
    // returns a zero there. The panic makes that path unreachable, so the program
    // is honest even though nothing ever produces a value.
    let (result, _, _) = check_source(
        "fn f() -> i64 { panic(\"always\"); } fn main() { print_i64(f()); }",
    );
    assert!(result.is_ok(), "a panicking function must check: {result:?}");
}

#[test]
fn only_a_diverging_expression_stands_in_for_a_value() {
    // `fits` accepts `!` where a value is wanted. It must not accept anything
    // else, or the sites it touched would stop catching mistakes.
    assert_error(
        "fn f() -> i64 { return true; }",
        "expected `i64`, found `bool`",
    );
    assert_error("fn f() -> i64 { return; }", "expected `i64`, found `()`");
    assert_error(
        "fn f() -> i64 { return switch 1 { default { true } }; }",
        "expected `i64`, found `bool`",
    );
}

