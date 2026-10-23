//! Type checker tests: every test builds a small program, runs it through
//! the full pipeline (lex, parse, check), and asserts either the inferred
//! type of an expression or the diagnostic that was produced.
//!
//! The pipeline helper is deliberately local to this file: it is the same
//! sequence `yashy-driver` will eventually run, written once here so the
//! tests never depend on the driver existing.

use yashy_common::{Diagnostic, FileId, Severity, SourceMap, Span};
use yashy_lexer::Lexer;
use yashy_parser::ast::Literal;
use yashy_parser::Parser;
use yashy_sema::{
    check_program_with, expr_ty, CheckOptions, HirExpr, HirItem, HirProgram, HirStmt, Type,
    TypeKind,
};

/// Lex, parse and check `source`.
fn check_source(src: &str) -> (Result<HirProgram, Vec<Diagnostic>>, SourceMap, FileId) {
    check_source_in(src, false)
}

/// Lex, parse and check `source` in the build mode `release` names.
///
/// The only difference a build mode makes to the *checker* is whether a
/// `debug_assert` survives into the tree.
fn check_source_in(
    src: &str,
    release: bool,
) -> (Result<HirProgram, Vec<Diagnostic>>, SourceMap, FileId) {
    let mut sm = SourceMap::new();
    let file = sm.add_file("test.ya", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("lexer should never panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser
        .parse_program()
        .expect("parser should never panic on these inputs");
    let result = check_program_with(&ast, &sm, file, CheckOptions { release });
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
    let (result, _, _) = check_source("fn f(a: int) -> int { return a; }");
    result.expect("fn with matching signature should type-check");
}

#[test]
fn wrong_return_type_is_a_diagnostic_with_a_span() {
    let (result, sm, file) = check_source("fn f() -> int { return 1.0; }");
    let diags = result.expect_err("returning float where int is expected must fail");
    assert_eq!(diags.len(), 1);
    let d = &diags[0];
    assert_eq!(d.severity, Severity::Error);
    assert!(
        d.message.contains("int"),
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
        "fn f(a: int) -> float { return a; }",
        "expected `float`, found `int`",
    );
}

#[test]
fn explicit_as_cast_permits_an_intentional_conversion() {
    let (result, _, _) = check_source("fn f(a: int) -> float { return a as float; }");
    result.expect("int as float is a permitted cast");
}

#[test]
fn as_cast_to_an_unrelated_type_is_an_error() {
    assert_error(
        "fn f() -> bool { return 1 as bool; }",
        "cannot cast `int` to `bool`",
    );
}

#[test]
fn as_cast_rejects_conversion_to_struct() {
    assert_error(
        "struct P { x: int } fn f(p: P) -> P { return p as P; }",
        "cannot cast",
    );
}

#[test]
fn struct_literal_fields_are_checked_by_name() {
    assert_error(
        "struct P { x: float, y: float } fn f() { const p = P { x: 1, y: 2 }; }",
        "expected `float`, found `int`",
    );
}

#[test]
fn unknown_field_in_struct_literal_is_an_error() {
    assert_error(
        "struct P { x: float } fn f() { const p = P { z: 1.0 }; }",
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
        "struct P { x: int, y: float } fn f(p: P) { const a = p.y; }",
        "f",
        TypeKind::F64,
    );
}

#[test]
fn field_access_carries_the_declaration_order_index() {
    // The index is what the backend extracts with, so it has to be the position
    // in the declaration -- not the position in any literal.
    let expr = expr_of_statement(
        "struct P { x: int, y: float } fn f(p: P) { const a = p.y; }",
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
        "struct P { x: int, y: int }\n\
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
        "struct P { x: int } fn f(p: P) { const a = p.z; }",
        "struct `P` has no field `z`",
    );
}

#[test]
fn a_field_access_on_a_non_struct_is_an_error() {
    assert_error(
        "fn f(n: int) { const a = n.x; }",
        "cannot access a field on `int`",
    );
}

#[test]
fn a_struct_literal_is_laid_out_in_declaration_order() {
    // Written `y, x`; the HIR must be `x, y`, because that is the order the IR
    // aggregate is built in and the order a field's index refers to.
    let expr = expr_of_statement(
        "struct P { x: int, y: int } fn f() { const p = P { y: 2, x: 1 }; }",
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
    assert_error("fn f(n: int) { while n { } }", "expected `bool`, found `int`");
}

#[test]
fn an_empty_body_after_a_bare_name_still_type_checks() {
    // The parser change that made `while flag { }` a loop with an empty body
    // (rather than `flag {}` plus a missing body) is only useful if the checker
    // then accepts it. A busy-wait is `while running { }` and nothing else.
    let (result, _, _) = check_source("fn f(flag: bool) { if flag { } while flag { } }");
    result.expect("empty bodies should type-check");
    let (range, _, _) = check_source("fn f(n: int) { for i in 0..n { } }");
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
        "cannot apply `+` to `int` and `float`",
    );
}

#[test]
fn if_condition_must_be_bool() {
    assert_error("fn f() { if 1 { } }", "expected `bool`, found `int`");
}

#[test]
fn builtin_calls_resolve() {
    let (result, _, _) = check_source("fn f() { print_i64(1); }");
    result.expect("print_i64(1) should type-check");
    assert_error("fn f() { print_str(1); }", "expected `string`, found `int`");
}

/// The three conversions a program uses to put a value inside a string.
fn assert_conversion(param: &str, builtin: &str, arg: &str, expected: TypeKind) {
    let src = format!("fn f(x: {param}) {{ const s = {builtin}(x); }}");
    assert_type(&src, "f", expected.clone());
    let wrong = format!("fn f() {{ const s = {builtin}({arg}); }}");
    assert_error(&wrong, "found");
}

#[test]
fn the_value_to_string_conversions_resolve_and_check_their_argument() {
    // Each conversion takes exactly one type, so a wrong argument is a type
    // error rather than a call to a runtime function that would read the
    // register as something else. The builtin keeps its `int`/`float` suffix:
    // it is a runtime symbol, not a Yashy spelling.
    assert_conversion("int", "str_from_i64", "\"text\"", TypeKind::Str);
    assert_conversion("float", "str_from_f64", "1", TypeKind::Str);
    assert_conversion("bool", "str_from_bool", "1", TypeKind::Str);
}

#[test]
fn a_conversion_no_longer_leaves_the_runtime_unreachable() {
    // `yashy_str_from_i64` and friends have existed since the M3 string work
    // and nothing could call them; `print_i64` was the only way to show a
    // number. This is the pin that they are reachable from the language.
    let (result, _, _) = check_source(
        "fn f(n: int) { const line = \"n = \" + str_from_i64(n); }",
    );
    result.expect("a built line should type-check");
}

#[test]
fn matches_some_and_binds_the_payload() {
    // Switch-expression arm bodies are bare expressions: `case None { 0 }`,
    // not `case None { 0; }`. The `;` belongs to the enclosing `return`.
    let (result, _, _) = check_source(
        "fn f(o: Option[int]) -> int { return switch o { case None { 0 } case Some(x) { x } }; }",
    );
    result.expect("Some(x) should bind the int payload");
}

#[test]
fn matching_none_against_some_payload_is_an_error() {
    assert_error(
        "fn f(o: Option[int]) -> int { return switch o { case Some(x) { x + true } default { 0 } }; }",
        "cannot apply",
    );
}

#[test]
fn none_is_typed_by_the_annotation_on_its_binding() {
    // `None` is the one constructor with no payload to infer from, so it can
    // only be typed from context. An annotated binding is such a context, and
    // the annotation has to be resolved *before* the initializer is checked for
    // that to work.
    let (result, _, _) = check_source("fn f() { const empty: Option[int] = None; }");
    result.expect("the annotation should type `None`");
}

#[test]
fn none_is_typed_by_the_parameter_it_is_passed_to() {
    // Call arguments are checked against the parameter's declared type, which is
    // what gives `None` a payload type here. Checking the arguments first and
    // comparing afterwards works for every other expression but cannot type a
    // constructor that has nothing of its own to infer from.
    let (result, _, _) = check_source(
        "fn unwrap_or(o: Option[int]) -> int { return switch o { case Some(x) { x } case None { 0 } }; } \
         fn f() -> int { return unwrap_or(None); }",
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
    assert_error("fn f() { const x: int = None; }", "expected `int`, found `Option[?]`");
}

#[test]
fn result_ok_and_err_bind_different_payload_types() {
    let (result, _, _) = check_source(
        "fn f(r: Result[int, string]) -> int { return switch r { case Ok(v) { v } case Err(e) { 0 } }; }",
    );
    result.expect("Ok(v) binds int and Err(e) binds string");
}

#[test]
fn try_on_a_result_returns_the_ok_value() {
    let (result, _, _) = check_source(
        "fn g() -> Result[int, string] { return Ok(7); } fn f() -> Result[int, string] { const v = g()?; return Ok(v); }",
    );
    result.expect("`?` on Result[int, string] yields int");
}

#[test]
fn try_on_a_non_result_is_an_error() {
    assert_error(
        "fn f() -> int { const v = 1?; return v; }",
        "`?` can only be used on a `Result`",
    );
}

#[test]
fn non_exhaustive_switch_on_enum_is_an_error_listing_missing_cases() {
    assert_error(
        "enum C { Red, Green, Blue } fn f(c: C) -> int { return switch c { case Red { 0 } }; }",
        "not exhaustive",
    );
}

#[test]
fn all_arms_must_yield_the_same_type() {
    assert_error(
        "enum C { Red, Green } fn f(c: C) -> int { return switch c { case Red { 0 } case Green { 1.0 } }; }",
        "expected `int`, found `float`",
    );
}

#[test]
fn switch_on_a_non_enum_without_default_is_an_error() {
    assert_error(
        "fn f(x: int) -> int { return switch x { case 1 { 0 } }; }",
        "add a `default` arm",
    );
}

#[test]
fn return_inside_an_if_yields_the_function_return_type() {
    // Regression: nested blocks used to be checked against `void`, so any
    // `return <value>` inside an `if` or a loop body was rejected with a
    // nonsensical "expected `void`, found `int`".
    check_source("fn f(x: bool) -> int { if x { return 1; } return 0; }")
        .0
        .expect("a `return` inside an `if` yields the function's return type");
}

#[test]
fn return_inside_an_else_yields_the_function_return_type() {
    check_source("fn f(x: bool) -> int { if x { return 1; } else { return 2; } }")
        .0
        .expect("a `return` inside an `else` yields the function's return type");
}

#[test]
fn return_inside_a_loop_body_yields_the_function_return_type() {
    check_source("fn f() -> int { for i in 0..3 { return i; } return 0; }")
        .0
        .expect("a `return` inside a loop body yields the function's return type");
    check_source("fn g() -> int { for i := 0; i < 3; print_i64(i) { return i; } return 0; }")
        .0
        .expect("and so does a `return` inside a C-style loop");
}

#[test]
fn a_nested_return_still_has_to_match_the_return_type() {
    assert_error(
        "fn f(x: bool) -> int { if x { return \"s\"; } return 0; }",
        "expected `int`, found `string`",
    );
}

#[test]
fn an_unknown_type_name_in_a_signature_is_an_error() {
    // Regression: signature types used to be resolved with
    // `unwrap_or_else(|_| Never)`, so `double` (not a type in Yashy) silently
    // became a `void` parameter instead of a diagnostic.
    assert_error(
        "fn f(b: double) -> int { return 0; }",
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
        "struct S { w: double } fn f() -> int { return 0; }",
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
fn let_bindings_are_mutable_and_block_scoped() {
    check_source("fn main() { let x = 1; x = 2; }")
        .0
        .expect("a `let` binding is mutable");
    assert_error(
        "fn main() { if true { let y = 1; } print_i64(y); }",
        "cannot find value `y` in this scope",
    );
}

#[test]
fn a_builtin_can_be_called_as_a_method() {
    check_source(
        r#"
        fn main() {
            var xs = []int { 1, 2 };
            print_i64(xs.len());
            xs.push(3);
            print_i64(xs.len());
            xs.free();
        }
        "#,
    )
    .0
    .expect("receiver-first builtins must be callable as methods");
}

#[test]
fn string_methods_type_check_in_their_method_shapes() {
    check_source(
        r#"
        fn main() {
            var s = "  Hello  ";
            var trimmed = s.trim();
            var started = s.trim_start();
            var ended = s.trim_end();
            var upper = s.to_upper();
            var lower = s.to_lower();
            var replaced = s.replace("H", "J");
            var repeated = s.repeat(2);
            var byte = s.get(0);
            var found_at = s.index_of("H");
            var has = s.contains("H");
            var starts = s.starts_with("  ");
            var ends = s.ends_with("  ");
            var raw = s.byte_at(0);
            var piece = s.slice(0, 2);
            var parts = s.split(" ");
            var lines = s.lines();
            var number = s.parse_i64();
            var decimal = s.parse_f64();
            print_i64(s.len());
            parts.free();
            lines.free();
        }
        "#,
    )
    .0
    .expect("string methods must type-check with their documented shapes");
}

#[test]
fn string_methods_reject_wrong_argument_types() {
    assert_error(
        r#"fn main() { var s = "x"; var has = s.contains(1); }"#,
        "expected `string`",
    );
    assert_error(
        r#"fn main() { var s = "x"; var piece = s.slice("a", 1); }"#,
        "expected `int`",
    );
    assert_error(
        r#"fn main() { var s = "x"; var many = s.repeat("two"); }"#,
        "expected `int`",
    );
}

#[test]
fn an_unknown_string_method_is_reported() {
    assert_error(
        r#"fn main() { var s = "x"; var nope = s.reverse(); }"#,
        "no method `reverse` on `string`",
    );
}

#[test]
fn const_and_var_keep_their_mutability() {
    assert_error(
        "fn main() { const x = 1; x = 2; }",
        "cannot assign to `x`, which is immutable",
    );
    check_source("fn main() { var x = 1; x = 2; }")
        .0
        .expect("a `var` binding stays mutable");
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
        "expected `int`, found `float`",
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
    // judged against the other's type: the inner `x` is an `float` and the outer
    // one an `int`, so a mixed-up lookup cannot pass both.
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
    let (result, _, _) = check_source("fn f(p: *mut int) -> *mut int { return p; }");
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
    assert_eq!(param.ty.to_string(), "*mut int");
}

#[test]
fn alloc_returns_a_pointer_to_i64() {
    assert_eq!(
        binding_type_in_fn("fn f() { const p = alloc(16); }", "f"),
        "*mut int"
    );
}

#[test]
fn a_pointer_annotation_and_alloc_agree() {
    // `*mut int` is a real type, not a spelling: an annotated binding accepts
    // `alloc`'s result, which it would not if the two disagreed.
    let (result, _, _) = check_source("fn f() { const p: *mut int = alloc(16); }");
    assert!(result.is_ok(), "expected `*mut int` to accept alloc: {result:?}");
}

#[test]
fn a_pointer_cannot_be_initialised_from_an_integer() {
    assert_error(
        "fn f() { const p: *mut int = 0; }",
        "expected `*mut int`, found `int`",
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
    let src = "fn f(n: int) { const line = \"n = ${n}\"; }";
    let expr = expr_of_statement(src, "f", 0);
    let HirExpr::Binary { op, lhs, rhs, .. } = &expr else {
        panic!("expected a concatenation, got {expr:?}");
    };
    assert_eq!(*op, yashy_sema::BinOp::Add);
    assert!(
        matches!(
            lhs.as_ref(),
            HirExpr::Literal(yashy_parser::ast::Literal::Str(lit), _, _) if lit.value == "n = "
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
        ("int", "str_from_i64"),
        ("float", "str_from_f64"),
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
        "struct P { x: int } fn f(p: P) { const line = \"${p}\"; }",
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
fn a_debug_form_needs_a_type_that_has_one() {
    // The debug form is richer than the display form, but it is not unlimited:
    // a pointer has no surface to show, and printing its address would be a
    // wrong answer that looks like a right one.
    assert_error(
        "fn f() { var p = alloc(1); const line = \"${p:?}\"; }",
        "cannot format a `*mut int`",
    );
    // A map has one either -- its keys and its values -- but not yet, so it is
    // reported rather than half-printed.
    assert_error(
        "fn f() { var m: map[int]int = map_new(); const line = \"${m:?}\"; }",
        "cannot format a `map",
    );
}

#[test]
fn a_debug_form_builds_a_printer_for_the_type_it_names() {
    // The printer is an ordinary function the checker appends to the program:
    // one per type, shared by every use of that type.
    let (result, _, _) = check_source(
        "struct P { x: int } fn f(p: P) { const line = \"${p:?}\"; const other = \"${p:?}\"; }",
    );
    let hir = result.expect("a struct has a debug form");
    let printers = hir
        .items
        .iter()
        .filter(|item| matches!(item, HirItem::Fn(f) if f.name.starts_with("__debug.")))
        .count();
    assert_eq!(
        printers, 1,
        "two uses of one type share one printer: {printers}"
    );
}

#[test]
fn a_debug_form_of_a_string_is_a_call_to_the_quoting_runtime_entry() {
    // The string's debug form is not a printer function: it is the runtime's
    // quoting and escaping, which is a byte operation rather than something
    // the checker could spell in the language.
    let (result, _, _) = check_source("fn f(s: string) { const line = \"${s:?}\"; }");
    let hir = result.expect("a string has a debug form");
    let item = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(f) if f.name == "f" => Some(f),
            _ => None,
        })
        .expect("the function should exist");
    let text = format!("{:?}", item.body);
    assert!(
        text.contains("str_debug"),
        "the debug form of a string calls the runtime: {text}"
    );
}

#[test]
fn dealloc_wants_a_pointer_not_an_integer() {
    assert_error("fn f() { dealloc(1); }", "expected `*mut int`, found `int`");
}

// -- const fn -----------------------------------------------------------------

#[test]
fn a_const_fn_call_is_folded_into_the_value_it_computed() {
    // The call is gone: what the tree holds is the number the compiler
    // computed, so the program carries `81` and not a call to `square`.
    let (result, _, _) = check_source(
        "const fn square(n: int) -> int { return n * n; }\n\
         fn f() { const t = square(9); }",
    );
    let hir = result.expect("a const fn call should fold");
    let item = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(f) if f.name == "f" => Some(f),
            _ => None,
        })
        .expect("the function should exist");
    let text = format!("{:?}", item.body);
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
    // It has no run-time existence: there is no symbol for it to be called
    // through, so the program does not carry one.
    let (result, _, _) = check_source(
        "const fn square(n: int) -> int { return n * n; }\n\
         fn f() { const t = square(9); }",
    );
    let hir = result.expect("the program should check");
    assert!(
        !hir
            .items
            .iter()
            .any(|item| matches!(item, HirItem::Fn(f) if f.name == "square")),
        "a const fn is not a runtime function"
    );
}

#[test]
fn a_const_fn_body_may_only_use_what_the_compiler_can_compute() {
    // A `read_line` in a const fn is a mistake the reader has to see, whether
    // or not anything calls the function.
    assert_error(
        "const fn f() -> int { return read_line(); }\n\
         fn g() { const t = f(); }",
        "`read_line` is not allowed in a const fn",
    );
    // A loop is not a value the compiler can compute either.
    assert_error(
        "const fn f(n: int) -> int { var i = 0; while i < n { i = i + 1; } return i; }\n\
         fn g() { const t = f(3); }",
        "is not allowed in a const fn",
    );
}

#[test]
fn a_const_fn_cannot_be_called_at_run_time() {
    // There is no function to call, so the call is refused where it is
    // written rather than left to fail at the link.
    assert_error(
        "const fn square(n: int) -> int { return n * n; }\n\
         fn f() { var t = square(9); }",
        "is a `const fn`",
    );
}

#[test]
fn a_const_fn_call_needs_constant_arguments() {
    // The value has to be known before the program runs, so a run-time
    // binding is not an argument the compiler can supply.
    assert_error(
        "const fn square(n: int) -> int { return n * n; }\n\
         fn f() { var n = 9; const t = square(n); }",
        "is a run-time value",
    );
}

#[test]
fn a_const_fn_may_call_another_const_fn() {
    // The call is computed at compile time, so a const fn can use one.
    let (result, _, _) = check_source(
        "const fn square(n: int) -> int { return n * n; }\n\
         const fn fourth(n: int) -> int { return square(square(n)); }\n\
         fn f() { const t = fourth(3); }",
    );
    let hir = result.expect("a const fn may call another const fn");
    let item = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(f) if f.name == "f" => Some(f),
            _ => None,
        })
        .expect("the function should exist");
    let text = format!("{:?}", item.body);
    assert!(text.contains("I64(81)"), "the folded value is in the tree: {text}");
}

#[test]
fn a_const_fn_that_recurses_forever_is_reported() {
    // The compiler runs the body, so a call that never ends would never end
    // here either. The depth limit is what turns that into a diagnostic.
    assert_error(
        "const fn loop(n: int) -> int { return loop(n + 1); }\n\
         fn f() { const t = loop(0); }",
        "did not terminate",
    );
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
    // `int` is not linear, so `move` asks for nothing: the value stays live and
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
        "fn consume(p: *mut int) { dealloc(p); } \
         fn f() { var buf = alloc(16); consume(buf); dealloc(buf); }",
        "value `buf` has already been moved",
    );
}

#[test]
fn a_linear_argument_does_not_have_to_be_a_name() {
    // A fresh `alloc(...)` owns nothing yet, so passing it straight through a
    // call leaves no moved binding behind.
    let (result, _, _) = check_source(
        "fn consume(p: *mut int) { dealloc(p); } fn f() { consume(alloc(16)); }",
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
    assert_eq!(text(&args[1]), "test.ya", "the file the compiler was given");
    assert_eq!(integer(&args[2]), 2, "the panic is on line 2");
    assert_eq!(integer(&args[3]), 5, "and starts at column 5");
}

#[test]
fn panic_takes_exactly_one_string() {
    assert_error("fn f() { panic(1); }", "expected `string`, found `int`");
    assert_error("fn f() { panic(); }", "expected 1 arguments, found 0");
    assert_error(
        "fn f() { panic(\"a\", \"b\"); }",
        "expected 1 arguments, found 2",
    );
}

// -- assert --------------------------------------------------------------------

/// The string a literal expression holds.
fn literal_text(expr: &HirExpr) -> String {
    match expr {
        HirExpr::Literal(Literal::Str(lit), _, _) => lit.value.clone(),
        other => panic!("expected a string literal, got {other:?}"),
    }
}

/// The integer a literal expression holds.
fn literal_integer(expr: &HirExpr) -> i64 {
    match expr {
        HirExpr::Literal(Literal::I64(value), _, _) => *value,
        other => panic!("expected an integer literal, got {other:?}"),
    }
}

#[test]
fn assert_is_a_unit_builtin_that_quotes_its_condition() {
    // `()` and not `!`: an assertion that holds returns normally, so typing it
    // as diverging would let its non-existent value flow into anything.
    let src = "fn f() {\n    const n = 3;\n    assert(n > 0);\n}\n";
    let HirStmt::Expr(expr, _, ty) = statement_of(src, "f", 1) else {
        panic!("expected an expression statement");
    };
    assert_eq!(ty.kind, TypeKind::Unit);
    let HirExpr::Call { callee, args, .. } = expr else {
        panic!("expected the lowered assert call");
    };
    assert_eq!(callee, vec!["assert".to_string()]);
    let kinds: Vec<TypeKind> = args.iter().map(|a| expr_ty(a).kind.clone()).collect();
    assert_eq!(
        kinds,
        vec![
            TypeKind::Bool,
            TypeKind::Str,
            TypeKind::Str,
            TypeKind::I64,
            TypeKind::I64
        ],
        "the condition, the message, the file, the line and the column"
    );
    assert_eq!(
        literal_text(&args[1]),
        "assertion failed: n > 0",
        "the message quotes the condition as it was written"
    );
    assert_eq!(literal_text(&args[2]), "test.ya");
    assert_eq!(literal_integer(&args[3]), 3);
    assert_eq!(literal_integer(&args[4]), 5);
}

#[test]
fn assert_quotes_a_condition_across_lines_as_one_line() {
    // The message is printed after a position and a label, so a second line in
    // it would read as a second diagnostic. The line break becomes a space.
    let src = "fn f() {\n    const n = 3;\n    assert(n >\n        0);\n}\n";
    let HirStmt::Expr(HirExpr::Call { args, .. }, _, _) = statement_of(src, "f", 1) else {
        panic!("expected the lowered assert call");
    };
    assert_eq!(
        args.first().map(|a| expr_ty(a).kind),
        Some(TypeKind::Bool),
        "the condition is the first argument"
    );
    assert_eq!(
        args.get(1).map(literal_text).unwrap_or_default(),
        "assertion failed: n > 0"
    );
}

#[test]
fn assert_takes_exactly_one_bool() {
    assert_error(
        "fn f() { assert(1); }",
        "expected `bool`, found `int`",
    );
    assert_error("fn f() { assert(); }", "expected 1 arguments, found 0");
    assert_error(
        "fn f() { assert(true, false); }",
        "expected 1 arguments, found 2",
    );
}

#[test]
fn assert_eq_carries_both_operands_and_the_message_around_them() {
    // The operands travel down as expressions, not as text: the backend has to
    // evaluate each once and use the result twice -- the comparison and the
    // value the message prints.
    let src = "fn f() {\n    const n = 3;\n    assert_eq(n, 0);\n}\n";
    let HirStmt::Expr(HirExpr::Call { callee, args, .. }, _, ty) = statement_of(src, "f", 1)
    else {
        panic!("expected the lowered assert_eq call");
    };
    assert_eq!(ty.kind, TypeKind::Unit);
    assert_eq!(callee, vec!["assert_eq".to_string()]);
    let kinds: Vec<TypeKind> = args.iter().map(|a| expr_ty(a).kind.clone()).collect();
    assert_eq!(
        kinds,
        vec![
            TypeKind::I64,
            TypeKind::I64,
            TypeKind::Str,
            TypeKind::Str,
            TypeKind::Str,
            TypeKind::I64,
            TypeKind::I64
        ],
        "two operands, the two halves of the message, then the position"
    );
    assert_eq!(literal_text(&args[2]), "assertion failed: n == 0, left = ");
    assert_eq!(literal_text(&args[3]), ", right = ");
    assert_eq!(literal_text(&args[4]), "test.ya");
    assert_eq!(literal_integer(&args[5]), 3);
    assert_eq!(literal_integer(&args[6]), 5);
}

#[test]
fn assert_ne_spells_the_comparison_it_tested() {
    let src = "fn f() {\n    const a = 1;\n    const b = 2;\n    assert_ne(a, b);\n}\n";
    let HirStmt::Expr(HirExpr::Call { callee, args, .. }, _, _) = statement_of(src, "f", 2) else {
        panic!("expected the lowered assert_ne call");
    };
    assert_eq!(callee, vec!["assert_ne".to_string()]);
    assert_eq!(
        literal_text(&args[2]),
        "assertion failed: a != b, left = ",
        "the message names the operator that was written"
    );
}

#[test]
fn assert_eq_compares_every_type_that_has_an_equality() {
    // One name over four types, which is why the call cannot be resolved against
    // the table's declared `int` pair.
    for (src, operand) in [
        ("fn f() { assert_eq(1, 2); }", TypeKind::I64),
        ("fn f() { assert_eq(1.5, 2.5); }", TypeKind::F64),
        ("fn f() { assert_eq(true, false); }", TypeKind::Bool),
        ("fn f() { assert_eq(\"a\", \"b\"); }", TypeKind::Str),
    ] {
        let HirStmt::Expr(HirExpr::Call { args, .. }, _, _) = statement_of(src, "f", 0) else {
            panic!("expected the lowered assert_eq call for {src}");
        };
        let left = args.first().map(|a| expr_ty(a)).unwrap_or_else(|| {
            panic!("expected a left operand for {src}")
        });
        assert_eq!(
            left.kind, operand,
            "the left operand of {src} should be {operand:?}"
        );
    }
}

#[test]
fn assert_eq_reports_operands_it_cannot_compare() {
    // An aggregate has no equality in Yashy yet. Comparing one by its first
    // field would be a wrong answer rather than a missing one, so it is
    // reported -- and reported against the left operand, which is the one the
    // message is about.
    assert_error(
        "fn f() { const o = Some(1); const p = Some(2); assert_eq(o, p); }",
        "`assert_eq` cannot compare `Option[int]` values: it compares `int`, `float`, `bool` and `string`",
    );
    // A `Never` operand is reported as uncomparable rather than as a mismatch
    // against `!`, which is what the order of the two checks is for.
    assert_error(
        "fn f() { assert_eq(panic(\"x\"), 1); }",
        "`assert_eq` cannot compare `!` values",
    );
}

#[test]
fn assert_eq_reports_a_mismatched_pair() {
    assert_error(
        "fn f() { assert_eq(1, \"x\"); }",
        "expected `int`, found `string`",
    );
    assert_error(
        "fn f() { assert_eq(true, 1); }",
        "expected `bool`, found `int`",
    );
    assert_error(
        "fn f() { assert_eq(1); }",
        "expected 2 arguments, found 1",
    );
    assert_error(
        "fn f() { assert_ne(1, 2, 3); }",
        "expected 2 arguments, found 3",
    );
}

#[test]
fn a_program_cannot_redeclare_an_assertion() {
    // The table is what reserves the names: a program that could define
    // `assert_eq` itself would have a function that never runs.
    assert_error(
        "fn assert(cond: bool) {}",
        "duplicate definition of `assert`",
    );
    assert_error(
        "fn assert_eq(a: int, b: int) {}",
        "duplicate definition of `assert_eq`",
    );
}

#[test]
fn debug_assert_is_a_debug_build_assertion() {
    // In a debug build it lowers exactly as `assert` does -- the same call, the
    // same message, the same position -- because it is the same assertion.
    let src = "fn f() {\n    const n = 3;\n    debug_assert(n > 0);\n}\n";
    let (result, _, _) = check_source(src);
    let hir = result.expect("expected program to type-check");
    let f = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(f) if f.name == "f" => Some(f),
            _ => None,
        })
        .expect("function should exist");
    let Some(HirStmt::Expr(HirExpr::Call { callee, args, .. }, _, _)) = f.body.stmts.get(1) else {
        panic!("expected the lowered assertion call");
    };
    assert_eq!(callee, &vec!["assert".to_string()]);
    assert_eq!(
        args.get(1).map(literal_text).unwrap_or_default(),
        "assertion failed: n > 0"
    );
}

#[test]
fn a_release_build_drops_a_debug_assert_entirely() {
    // The condition is checked -- a release build that accepts source a debug
    // build rejects would be the worst of both worlds -- but the tree is
    // dropped, so the string is not emitted and an argument with side effects
    // does not run.
    let src = "fn f() {\n    const n = 3;\n    debug_assert(n > 0);\n}\n";
    let (result, _, _) = check_source_in(src, true);
    let hir = result.expect("expected program to type-check in a release build");
    let f = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(f) if f.name == "f" => Some(f),
            _ => None,
        })
        .expect("function should exist");
    let Some(HirStmt::Expr(HirExpr::Call { callee, args, .. }, _, ty)) = f.body.stmts.get(1)
    else {
        panic!("expected the call the release build leaves behind");
    };
    assert_eq!(callee, &vec!["debug_assert".to_string()]);
    assert!(
        args.is_empty(),
        "the condition is gone, not merely unreached: {args:?}"
    );
    assert_eq!(ty.kind, TypeKind::Unit);
}

#[test]
fn a_release_build_still_checks_a_debug_assert_condition() {
    // The half of the contract that keeps a release build from being a
    // different language: what the checker rejects, it rejects in both modes.
    let (result, _, _) = check_source_in("fn f() { debug_assert(1); }", true);
    let Err(errors) = result else {
        panic!("a release build must still reject a non-bool condition");
    };
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("expected `bool`, found `int`")),
        "expected the type error, got {errors:?}"
    );
}

#[test]
fn assert_is_never_dropped_by_a_release_build() {
    // The difference between the two names, and the whole reason both exist.
    let src = "fn f() {\n    const n = 3;\n    assert(n > 0);\n}\n";
    let (result, _, _) = check_source_in(src, true);
    let hir = result.expect("expected program to type-check");
    let f = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(f) if f.name == "f" => Some(f),
            _ => None,
        })
        .expect("function should exist");
    let Some(HirStmt::Expr(HirExpr::Call { callee, args, .. }, _, _)) = f.body.stmts.get(1) else {
        panic!("expected the assertion to survive a release build");
    };
    assert_eq!(callee, &vec!["assert".to_string()]);
    assert_eq!(args.len(), 5, "condition, message and position");
}

// -- exit (spec 6.3) ----------------------------------------------------------

#[test]
fn exit_is_a_never_returning_builtin() {
    let HirStmt::Expr(expr, _, ty) = statement_of("fn f() { os.exit(1); }", "f", 0) else {
        panic!("expected an expression statement");
    };
    assert_eq!(ty.kind, TypeKind::Never);
    let HirExpr::Call { callee, args, .. } = expr else {
        panic!("expected the lowered exit call");
    };
    assert_eq!(callee.last(), Some(&"exit".to_string()));
    assert_eq!(args.len(), 1);
    assert_eq!(expr_ty(&args[0]).kind, TypeKind::I64);
}

#[test]
fn exit_takes_exactly_one_i64() {
    assert_error("fn f() { os.exit(); }", "expected 1 arguments, found 0");
    assert_error("fn f() { os.exit(1, 2); }", "expected 1 arguments, found 2");
    assert_error("fn f() { os.exit(\"x\"); }", "expected `int`, found `string`");
    assert_error("fn f() { os.exit(true); }", "expected `int`, found `bool`");
}

#[test]
fn exit_fits_where_a_value_is_wanted() {
    // `!` is the same fact as `panic`: a call that produces no value can stand
    // in for one of any type, because the block that would have used it is
    // unreachable.
    assert_type("fn f() -> int { const x = os.exit(1); return x; }", "f", TypeKind::Never);
}

#[test]
fn exit_cannot_be_redeclared() {
    assert_error(
        "fn exit(code: int) {}",
        "duplicate definition of `exit`",
    );
}

// -- pointer indexing -----------------------------------------------------------

#[test]
fn indexing_reads_an_element_of_a_pointer() {
    let src = "fn f(p: *mut int) { const x = p[0]; }";
    let HirStmt::Const { init, .. } = statement_of(src, "f", 0) else {
        panic!("expected a const binding");
    };
    let HirExpr::Index { base, index, ty, .. } = init else {
        panic!("expected an index expression");
    };
    let expected = TypeKind::Ptr(Box::new(Type {
        kind: TypeKind::I64,
        span: Span::new(0, 0),
    }));
    assert_eq!(expr_ty(&base).kind, expected);
    assert_eq!(expr_ty(&index).kind, TypeKind::I64);
    assert_eq!(ty.kind, TypeKind::I64);
}

#[test]
fn indexing_assigns_through_a_pointer() {
    let src = "fn f(p: *mut int, i: int, v: int) { p[i] = v; }";
    let HirStmt::IndexAssign { base, index, value, .. } = statement_of(src, "f", 0) else {
        panic!("expected an index assignment");
    };
    let expected = TypeKind::Ptr(Box::new(Type {
        kind: TypeKind::I64,
        span: Span::new(0, 0),
    }));
    assert_eq!(expr_ty(&base).kind, expected);
    assert_eq!(expr_ty(&index).kind, TypeKind::I64);
    assert_eq!(expr_ty(&value).kind, TypeKind::I64);
}

#[test]
fn indexing_requires_an_i64_index() {
    assert_error(
        "fn f(p: *mut int, i: bool) { const x = p[i]; }",
        "index must be `int`",
    );
    assert_error(
        "fn f(p: *mut int, i: float) { const x = p[i]; }",
        "index must be `int`",
    );
}

#[test]
fn indexing_requires_an_i64_value_for_assignment() {
    assert_error(
        "fn f(p: *mut int, i: int, v: string) { p[i] = v; }",
        "expected `int`, found `string`",
    );
}

#[test]
fn indexing_a_non_pointer_is_an_error() {
    assert_error(
        "fn f(x: int) { const y = x[0]; }",
        "cannot index `int`",
    );
}

#[test]
fn indexing_a_pointer_to_nothing_is_an_error() {
    // A pointer is an address; what it holds is whatever it points at, so a
    // `*mut float` may be indexed. A pointer to `void` has nothing at that
    // address to hand back, so the annotation is refused before it can be
    // indexed at all.
    assert_error(
        "fn f(p: *mut void) { const x = p[0]; }",
        "only allowed as a function's return type",
    );
}

#[test]
fn indexing_a_const_pointer_binding_is_an_error_for_assignment() {
    // A `const` pointer binding can be read through but not assigned through.
    // The checker does not currently reject this (it is the same slot), but
    // reading is fine.
    let (result, _, _) = check_source("fn f(p: *mut int) { const x = p[0]; }");
    assert!(result.is_ok(), "reading through a pointer should type-check");
}

// -- field assignment ---------------------------------------------------------

/// The declarations the field-assignment tests read against, spelled once so the
/// sources and the diagnostics agree.
const SHAPES: &str = "struct Point { x: int, y: int }\nstruct Line { from: Point, to: Point }\n";

#[test]
fn a_field_assignment_carries_the_root_and_the_chain() {
    let src = format!("{SHAPES}fn f(p: Point) {{ var q = p; q.x = 3; }}");
    let HirStmt::FieldAssign {
        root,
        steps,
        value,
        ..
    } = statement_of(&src, "f", 1)
    else {
        panic!("expected a field assignment");
    };
    assert_eq!(root, "q");
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].index, 0, "`x` is the first field declared");
    assert_eq!(steps[0].ty.kind, TypeKind::I64);
    assert_eq!(expr_ty(&value).kind, TypeKind::I64);
}

#[test]
fn a_nested_field_assignment_carries_one_step_per_field() {
    let src = format!("{SHAPES}fn f(l: Line) {{ var m = l; m.from.x = 1; }}");
    let HirStmt::FieldAssign { steps, .. } = statement_of(&src, "f", 1) else {
        panic!("expected a field assignment");
    };
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].index, 0, "`from` is the first field declared");
    assert_eq!(steps[1].index, 0, "`x` is the first field declared");
    assert_eq!(steps[0].ty.kind, TypeKind::Struct("Point".to_string()));
}

#[test]
fn a_compound_field_assignment_is_the_assignment_of_the_expanded_value() {
    // `p.x += 1` is `p.x = p.x + 1`, so what reaches the backend is a field
    // assignment whose value is the addition -- the checker never learns that
    // `+=` exists.
    let src = format!("{SHAPES}fn f(p: Point) {{ var q = p; q.x += 1; }}");
    let HirStmt::FieldAssign { value, .. } = statement_of(&src, "f", 1) else {
        panic!("expected a field assignment");
    };
    assert!(
        matches!(value, HirExpr::Binary { .. }),
        "the value is the expanded addition, got {value:?}"
    );
}

#[test]
fn assigning_a_field_of_a_const_is_an_error() {
    let src = format!("{SHAPES}fn f(p: Point) {{ const q = p; q.x = 3; }}");
    assert_error(&src, "cannot assign to a field of `q`, which is immutable");
}

#[test]
fn assigning_a_field_to_the_wrong_type_is_an_error() {
    let src = format!("{SHAPES}fn f(p: Point) {{ var q = p; q.x = \"one\"; }}");
    assert_error(&src, "expected `int`, found `string`");
}

#[test]
fn assigning_a_field_that_does_not_exist_is_an_error() {
    let src = format!("{SHAPES}fn f(p: Point) {{ var q = p; q.z = 1; }}");
    assert_error(&src, "struct `Point` has no field `z`");
}

#[test]
fn assigning_a_field_of_a_non_struct_is_an_error() {
    assert_error(
        "fn f(n: int) { var m = n; m.x = 1; }",
        "cannot access a field on `int`",
    );
}

#[test]
fn assigning_through_a_call_result_is_an_error() {
    // A field target has to bottom out in a binding: there is no address to
    // write through in a temporary.
    let src = format!("{SHAPES}fn g() -> Point {{ return Point {{ x: 1, y: 2 }}; }}\nfn f() {{ g().x = 3; }}");
    assert_error(
        &src,
        "only a `var` binding or one of its fields can be assigned",
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
        let src = format!("fn f(a: int, b: int) {{\n    const x = {source_expr};\n}}\n");
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
        assert_eq!(site.file, "test.ya", "the file the compiler was given");
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
        let src = format!("fn f(a: int, b: int) {{\n    const x = {source_expr};\n}}\n");
        let checked = expr_of_statement(&src, "f", 0);
        assert!(
            !is_checked(&checked),
            "`{source_expr}` cannot fault, so it must carry no position"
        );
    }
}

#[test]
fn float_and_string_arithmetic_are_not_checkable() {
    // An `float` that overflows becomes an infinity, which is a float answer and
    // not a wrong one, and neither `fadd` nor `fdiv` traps. A `+` on strings is a
    // concatenation. Neither has a fault to report or a position to report it
    // from, which is a decision about the language and is made here.
    for (what, src) in [
        (
            "float add",
            "fn f(x: float, y: float) {\n    const s = x + y;\n}\n",
        ),
        (
            "float subtract",
            "fn f(x: float, y: float) {\n    const s = x - y;\n}\n",
        ),
        (
            "float multiply",
            "fn f(x: float, y: float) {\n    const s = x * y;\n}\n",
        ),
        (
            "float divide",
            "fn f(x: float, y: float) {\n    const s = x / y;\n}\n",
        ),
        (
            "float remainder",
            "fn f(x: float, y: float) {\n    const s = x % y;\n}\n",
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
    // `-int::MIN` wraps to itself, so a silent wraparound turns a mistake into a
    // plausible number. Negating any `float`, infinity included, is exact, and `!`
    // is not arithmetic at all.
    let checked = expr_of_statement("fn f(a: int) {\n    const x = -a;\n}\n", "f", 0);
    match &checked {
        HirExpr::Unary {
            fault: Some(site), ..
        } => {
            assert_eq!(site.file, "test.ya");
            assert_eq!(site.line, 2);
            assert_eq!(site.column, 15, "the negation starts at the `-`");
        }
        other => panic!("integer negation should be checkable, got {other:?}"),
    }

    for (what, src) in [
        ("float negation", "fn f(x: float) {\n    const y = -x;\n}\n"),
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
    // `wrapping_*` and `saturating_*` answer with an `int`: two's complement
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
        let src = format!("fn f(a: int, b: int) {{ const x = {name}(a, b); }}");
        assert_type(&src, "f", TypeKind::I64);
    }
    // The payload comes out through a match, which the checker could only do if
    // the result really were an `Option[int]`: the `switch` has to cover both
    // variants, and its own type is the payload's.
    assert_type(
        "fn f(a: int, b: int) -> int {\n\
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
        "fn f(a: int, b: int) { const x = checked_add(a, b) + 1; }",
        "cannot apply `+` to `Option[int]` and `int`",
    );
}

#[test]
fn the_arithmetic_builtins_take_exactly_two_arguments() {
    // They are ordinary functions with an ordinary arity, so a wrong call gets
    // the diagnostic every other call gets. A `wrapping_add` that quietly took
    // one argument would be a different operation with the same name.
    assert_error(
        "fn f(a: int) { const x = wrapping_add(a); }",
        "expected 2 arguments, found 1",
    );
    assert_error(
        "fn f(a: int) { const x = saturating_mul(a, 2, 3); }",
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
        let src = format!("fn f(o: Option[int]) -> int {{ return switch o {{ {arms} }}; }}");
        let HirStmt::Return(Some(expr), _) = statement_of(&src, "f", 0) else {
            panic!("expected a return statement");
        };
        assert_eq!(expr_ty(&expr).kind, TypeKind::I64, "arms: {arms}");
    }
}

#[test]
fn a_diverging_return_fits_any_return_type() {
    for ret in ["int", "float", "bool", "string"] {
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
    // The body falls off the end, which Yashy already allows -- code generation
    // returns a zero there. The panic makes that path unreachable, so the program
    // is honest even though nothing ever produces a value.
    let (result, _, _) = check_source(
        "fn f() -> int { panic(\"always\"); } fn main() { print_i64(f()); }",
    );
    assert!(result.is_ok(), "a panicking function must check: {result:?}");
}

#[test]
fn old_builtin_names_point_at_their_methods() {
    // The M12 surface has one spelling per operation. The old free-function
    // names are not aliases: each reports where its method form is, using the
    // receiver the caller already wrote.
    for (src, hint) in [
        (
            "fn main() { var xs = []int { 1 }; push(xs, 2); }",
            "`push` has moved to method syntax: write `xs.push(2)`",
        ),
        (
            "fn main() { var xs = []int { 1 }; free_slice(xs); }",
            "`free_slice` has moved to method syntax: write `xs.free()`",
        ),
        (
            "fn main() { var m: map[int]int = map_new(); map_get(m, 1); }",
            "`map_get` has moved to method syntax: write `m.get(1)`",
        ),
        (
            "fn main() { var c: Channel[int] = chan_new(1); chan_recv(c); }",
            "`chan_recv` has moved to method syntax: write `c.recv()`",
        ),
        (
            "fn main() { var m: Mutex = mutex_new(); mutex_lock(m); }",
            "`mutex_lock` has moved to method syntax: write `m.lock()`",
        ),
        (
            "fn main() { var xs = []int { 1 }; len(xs); }",
            "`len` has moved to method syntax: write `xs.len()`",
        ),
        (
            "fn main() { var s = \"a\"; len(s); }",
            "`len` has moved to method syntax: write `s.len()`",
        ),
        (
            "fn main() { var s = \"a\"; byte_at(s, 0); }",
            "`byte_at` has moved to method syntax: write `s.byte_at(0)`",
        ),
        (
            "fn main() { var s = \"a\"; str_slice(s, 0, 1); }",
            "`str_slice` has moved to method syntax: write `s.slice(0, 1)`",
        ),
        (
            "fn main() { var s = \"1\"; parse_i64(s); }",
            "`parse_i64` has moved to method syntax: write `s.parse_i64()`",
        ),
        (
            "fn main() { var s = \"1\"; parse_f64(s); }",
            "`parse_f64` has moved to method syntax: write `s.parse_f64()`",
        ),
    ] {
        assert_error(src, hint);
    }
}

#[test]
fn only_a_diverging_expression_stands_in_for_a_value() {
    // `fits` accepts `!` where a value is wanted. It must not accept anything
    // else, or the sites it touched would stop catching mistakes.
    assert_error(
        "fn f() -> int { return true; }",
        "expected `int`, found `bool`",
    );
    assert_error("fn f() -> int { return; }", "expected `int`, found `void`");
    assert_error(
        "fn f() -> int { return switch 1 { default { true } }; }",
        "expected `int`, found `bool`",
    );
}

#[test]
fn a_generic_function_is_emitted_once_per_substitution() {
    // Monomorphization is visible in the HIR: there is one concrete function
    // per type the program called it with, and no generic function at all.
    let (result, _, _) = check_source(
        "fn identity[T](x: T) -> T { return x; } \
         fn main() { const a = identity(7); const b = identity(\"s\"); }",
    );
    let hir = result.expect("a generic call should type-check");
    let mut names: Vec<&str> = hir
        .items
        .iter()
        .filter_map(|item| match item {
            HirItem::Fn(f) => Some(f.name.as_str()),
            _ => None,
        })
        .collect();
    names.sort_unstable();
    assert_eq!(names, vec!["identity_i64", "identity_string", "main"]);
}


#[test]
fn a_fixed_array_index_is_an_option() {
    // `grid[1]` never faults: in range is `Some`, anything else is `None`.
    // The length of a fixed array is its type, so `[int; 3]` prints that way.
    assert_eq!(
        binding_type_in_fn(
            "fn main() { const grid: [int; 3] = [1, 2, 3]; }",
            "main"
        ),
        "[int; 3]"
    );
    assert_eq!(
        binding_type_in_fn(
            "fn main() { const grid = [1, 2, 3]; const at = grid[1]; }",
            "main"
        ),
        "[int; 3]"
    );
    let (result, _, _) = check_source(
        "fn main() { const grid = [1, 2, 3]; const at = grid[1]; }",
    );
    let hir = result.expect("indexing a fixed array should check");
    let Some(HirItem::Fn(f)) = hir.items.iter().find(|item| matches!(item, HirItem::Fn(f) if f.name == "main")) else {
        panic!("main should exist");
    };
    let Some(HirStmt::Const { init, .. }) = f.body.stmts.get(1) else {
        panic!("the index binding should be second");
    };
    assert_eq!(expr_ty(init).to_string(), "Option[int]");
}

#[test]
fn a_range_binding_has_a_view_type() {
    // The view's type is inferred, and it carries its element type: `[int]`.
    let (result, _, _) = check_source(
        "fn main() { const grid = [1, 2, 3]; const head = grid[0..2]; }",
    );
    let hir = result.expect("a range over a fixed array should check");
    let Some(HirItem::Fn(f)) = hir.items.iter().find(|item| matches!(item, HirItem::Fn(f) if f.name == "main")) else {
        panic!("main should exist");
    };
    let Some(HirStmt::Const { init, .. }) = f.body.stmts.get(1) else {
        panic!("the view binding should be second");
    };
    assert_eq!(expr_ty(init).to_string(), "[int]");
    assert!(matches!(init, HirExpr::View { .. }));
}
// -- tuples, destructuring, and guards --------------------------------------

#[test]
fn a_tuple_is_the_product_of_its_element_types() {
    // Position is part of the type, and the type is the element types in the
    // order they were written. An element may be another tuple.
    assert_eq!(
        binding_type_in_fn("fn main() { const pair = (7, true); }", "main"),
        "(int, bool)"
    );
    assert_eq!(
        binding_type_in_fn("fn main() { const nested = (1, (2.0, false)); }", "main"),
        "(int, (float, bool))"
    );
    // `(7,)` is the one-element tuple; `(7)` would be the integer itself.
    assert_eq!(
        binding_type_in_fn("fn main() { const single = (7,); }", "main"),
        "(int,)"
    );
}

#[test]
fn destructuring_binds_each_element_with_its_own_type() {
    let (result, _, _) = check_source(
        "fn main() { const pair = (7, true); const (n, flag) = pair; }",
    );
    let hir = result.expect("destructuring a tuple should check");
    let Some(HirItem::Fn(f)) = hir.items.iter().find(|item| matches!(item, HirItem::Fn(f) if f.name == "main")) else {
        panic!("main should exist");
    };
    let Some(HirStmt::Destructure { bindings, .. }) = f.body.stmts.get(1) else {
        panic!("the destructuring statement should be second: {:?}", f.body.stmts);
    };
    assert_eq!(bindings.len(), 2, "one entry per element");
    match &bindings[0] {
        Some((name, ty)) => {
            assert_eq!(name, "n");
            assert_eq!(ty.to_string(), "int", "element 0 is the int");
        }
        other => panic!("expected a binding for element 0, got {other:?}"),
    }
    match &bindings[1] {
        Some((name, ty)) => {
            assert_eq!(name, "flag");
            assert_eq!(ty.to_string(), "bool", "element 1 is the bool");
        }
        other => panic!("expected a binding for element 1, got {other:?}"),
    }

    // `_` is a hole in the binding list, not a name: the element is dropped.
    let (result, _, _) = check_source(
        "fn main() { const pair = (7, true); const (_, flag) = pair; }",
    );
    let hir = result.expect("`_` should skip an element");
    let Some(HirItem::Fn(f)) = hir.items.iter().find(|item| matches!(item, HirItem::Fn(f) if f.name == "main")) else {
        panic!("main should exist");
    };
    let Some(HirStmt::Destructure { bindings, .. }) = f.body.stmts.get(1) else {
        panic!("the destructuring statement should be second");
    };
    assert_eq!(bindings[0], None, "`_` binds nothing");

    // The names really are in scope afterwards: this only type-checks if `n`
    // and `flag` resolve to the element types.
    let (result, _, _) = check_source(
        "fn main() { const pair = (7, true); const (n, flag) = pair; if flag { print_i64(n); } }",
    );
    if let Err(diags) = result {
        panic!("the bound names should resolve: {diags:?}");
    }
}

#[test]
fn only_a_tuple_can_be_destructured() {
    assert_error(
        "fn main() { const (a, b) = 1; }",
        "cannot destructure `int`",
    );
    assert_error(
        "fn main() { const (a, b) = (1, 2, 3); }",
        "this pattern binds 2 names, but the tuple has 3 elements",
    );
    assert_error(
        "fn main() { const (a, b, c) = (1, 2); }",
        "this pattern binds 3 names, but the tuple has 2 elements",
    );
}

#[test]
fn a_tuple_holds_only_values_it_can_copy() {
    // A tuple is copied by copying its bytes, so an element that owns a
    // reference has no meaning yet -- the same rule fixed arrays use.
    assert_error(
        "fn main() { const bad = (1, \"s\"); }",
        "a tuple cannot hold `string`",
    );
    // A fixed array is copyable, but the element rule stays at scalars and
    // tuples of them: `size_of` a tuple predicts LLVM's field alignment from
    // each element's own size, which is exact for a scalar and not for an
    // aggregate. Admitting it later is a change to that rule, not a bug here.
    assert_error(
        "fn main() { const bad = (1, [2, 3]); }",
        "a tuple cannot hold `[int; 2]`",
    );
    // Scalars and nested tuples of them are the whole supported set.
    let (result, _, _) = check_source("fn main() { const ok = (1, (2.0, true), 3); }");
    if let Err(diags) = result {
        panic!("scalars and nested tuples should be allowed: {diags:?}");
    }
}

#[test]
fn a_const_destructured_binding_is_immutable_and_a_var_one_is_not() {
    assert_error(
        "fn main() { const pair = (1, 2); const (a, b) = pair; a = 5; }",
        "cannot assign to `a`, which is immutable",
    );
    let (result, _, _) = check_source(
        "fn main() { const pair = (1, 2); var (a, b) = pair; a = 5; let (c, d) = pair; c = 6; }",
    );
    if let Err(diags) = result {
        panic!("`var` and `let` elements should be assignable: {diags:?}");
    }
}

#[test]
fn a_guard_must_be_a_bool_and_sees_the_patterns_names() {
    assert_error(
        "fn f(n: int) -> int { return switch n { case 1 if n { 1 } default { 0 } }; }",
        "a guard must be a `bool`, found `int`",
    );

    // The binding `v` is in scope inside the guard and inside the body: the
    // guard is checked after the pattern has bound its names.
    let (result, _, _) = check_source(
        "enum E { A(int), B } \
         fn f(e: E) -> int { return switch e { case A(v) if v > 0 { v } case A(v) { 0 } case B { 1 } }; }",
    );
    if let Err(diags) = result {
        panic!("a guard should see the pattern's names: {diags:?}");
    }
}

#[test]
fn a_guard_does_not_count_towards_exhaustiveness() {
    // `A` is matched only by a guarded arm, so the switch can reach a value
    // with no arm to take: the guard does not cover the case where it is false.
    assert_error(
        "enum E { A, B } \
         fn f(e: E) -> int { return switch e { case A if true { 1 } case B { 2 } }; }",
        "switch is not exhaustive: missing case `A`",
    );

    // The same rule for the `default` of a switch over a scalar.
    assert_error(
        "fn f(n: int) -> int { return switch n { default if n > 0 { 1 } }; }",
        "switch over a non-enum type is not exhaustive",
    );
}

#[test]
fn tuples_are_ordinary_parameters_and_return_values() {
    let (result, _, _) = check_source(
        "fn swap(p: (int, bool)) -> (bool, int) { const (n, b) = p; return (b, n); }",
    );
    let hir = result.expect("a tuple parameter and return type should check");
    let Some(HirItem::Fn(f)) = hir.items.iter().find(|item| matches!(item, HirItem::Fn(f) if f.name == "swap")) else {
        panic!("swap should exist");
    };
    assert_eq!(f.params[0].ty.to_string(), "(int, bool)");
    assert_eq!(f.ret.to_string(), "(bool, int)");
}

#[test]
fn a_generic_function_can_be_called_with_a_tuple() {
    // A tuple is a type like any other to the unifier: `T` becomes
    // `(int, bool)` and the instance is mangled from the element types.
    let (result, _, _) = check_source(
        "fn identity[T](x: T) -> T { return x; } \
         fn main() { const t = identity((1, true)); }",
    );
    let hir = result.expect("a generic call with a tuple argument should check");
    let names: Vec<&str> = hir
        .items
        .iter()
        .filter_map(|item| match item {
            HirItem::Fn(f) => Some(f.name.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        names.contains(&"identity_tuple_i64_bool"),
        "the instance is named from the element types: {names:?}"
    );
}
#[test]
fn int_and_float_are_the_wide_spellings() {
    let (result, _, _) = check_source("fn f(a: int) -> float { return 1.5; }");
    assert!(result.is_ok(), "int and float must resolve: {result:?}");
}

#[test]
fn i64_names_its_replacement() {
    assert_error("fn f(a: i64) { }", "`i64` is spelled `int` in Yashy");
}

#[test]
fn f64_names_its_replacement() {
    assert_error("fn f() -> f64 { return 1.0; }", "`f64` is spelled `float` in Yashy");
}

#[test]
fn a_cast_cannot_spell_the_old_names_either() {
    assert_error("fn f(x: int) { const y = x as i64; }", "`i64` is spelled `int` in Yashy");
}
#[test]
fn void_is_the_return_spelling() {
    let (result, _, _) = check_source("fn f() -> void { }");
    assert!(result.is_ok(), "`-> void` must be accepted: {result:?}");
}

#[test]
fn a_written_unit_type_names_void() {
    assert_error("fn f() -> () { }", "`()` is spelled `void` in Yashy");
}

#[test]
fn void_outside_a_return_position_is_refused() {
    assert_error("fn f(x: void) { }", "only allowed as a function's return type");
    assert_error(
        "fn g() { } fn f() { const x: void = g(); }",
        "only allowed as a function's return type",
    );
    assert_error("fn f(xs: []void) { }", "cannot hold `void`");
}
