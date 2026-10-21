//! Function values: types, passing, and calling through a binding.
//!
//! A top-level function name is a value of its function type, so it can be
//! passed to a parameter, bound to a local, and called through either. These
//! tests pin the *type* of that value and the diagnostics a misuse produces:
//! the point of a function type is that the checker can tell a function from
//! a number, and can tell one function from another.

use otter_common::{Diagnostic, SourceMap, Span};
use otter_lexer::Lexer;
use otter_parser::Parser;
use otter_sema::{check_program_with, expr_ty, CheckOptions, HirExpr, HirProgram, Type, TypeKind};

/// A type with a placeholder span: the span is never compared.
fn ty(kind: TypeKind) -> Type {
    Type {
        kind,
        span: Span::new(0, 0),
    }
}

/// Lex, parse and check `source`.
fn check_source(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sm = SourceMap::new();
    let file = sm.add_file("test.ot", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("lexer should never panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser
        .parse_program()
        .expect("parser should never panic on these inputs");
    check_program_with(&ast, &sm, file, CheckOptions { release: false })
}

/// Assert that `src` type-checks, and return the type of the expression in
/// the first `const` binding named `needle`.
fn assert_type(src: &str, needle: &str, expected: TypeKind) {
    let hir = check_source(src).expect("expected program to type-check");
    let expr = first_const(&hir, needle);
    let ty = expr_ty(expr);
    assert_eq!(
        ty.kind, expected,
        "type of `{needle}` should be {expected:?}, got {ty}"
    );
}

/// The initializer of the first `const` binding named `name`.
fn first_const<'a>(hir: &'a HirProgram, name: &str) -> &'a HirExpr {
    for item in &hir.items {
        let otter_sema::HirItem::Fn(f) = item else {
            continue;
        };
        for stmt in &f.body.stmts {
            let otter_sema::HirStmt::Const { name: bound, init, .. } = stmt else {
                continue;
            };
            if bound == name {
                return init;
            }
        }
    }
    panic!("no const binding named `{name}`");
}

/// Assert that `src` fails to type-check with a message containing `needle`.
fn assert_error(src: &str, needle: &str) {
    let diags = match check_source(src) {
        Ok(_) => panic!("expected a type error containing `{needle}`"),
        Err(diags) => diags,
    };
    assert!(
        diags.iter().any(|d| d.message.contains(needle)),
        "expected an error containing `{needle}`, got {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn a_function_type_annotation_resolves_to_a_function_type() {
    assert_type(
        "fn f() { const g: fn(i64, string) -> bool = g0; } \
         fn g0(a: i64, b: string) -> bool { return true; }",
        "g",
        TypeKind::Func(
            vec![ty(TypeKind::I64), ty(TypeKind::Str)],
            Box::new(ty(TypeKind::Bool)),
        ),
    );
}

#[test]
fn a_top_level_function_name_has_its_function_type() {
    assert_type(
        "fn f() { const g = square; } fn square(x: i64) -> i64 { return x; }",
        "g",
        TypeKind::Func(
            vec![ty(TypeKind::I64)],
            Box::new(ty(TypeKind::I64)),
        ),
    );
}

#[test]
fn a_function_can_be_passed_to_a_function_parameter() {
    let result = check_source(
        "fn square(x: i64) -> i64 { return x; } \
         fn apply(f: fn(i64) -> i64, x: i64) -> i64 { return f(x); } \
         fn main() { const r = apply(square, 7); }",
    );
    result.expect("passing a function as an argument should type-check");
}

#[test]
fn a_bound_function_can_be_called_through_the_binding() {
    let result = check_source(
        "fn square(x: i64) -> i64 { return x; } \
         fn main() { const f = square; const r = f(4); }",
    );
    result.expect("calling a function through a binding should type-check");
}

#[test]
fn a_function_of_the_wrong_type_is_rejected() {
    assert_error(
        "fn twice(s: string) -> i64 { return 1; } \
         fn apply(f: fn(i64) -> i64, x: i64) -> i64 { return f(x); } \
         fn main() { const r = apply(twice, 7); }",
        "expected `fn(i64) -> i64`",
    );
}

#[test]
fn calling_a_value_that_is_not_a_function_is_rejected() {
    assert_error(
        "fn main() { const f = 1; const r = f(2); }",
        "`f` is not a function",
    );
}

#[test]
fn calling_a_function_value_with_the_wrong_arity_is_rejected() {
    assert_error(
        "fn square(x: i64) -> i64 { return x; } \
         fn main() { const f = square; const r = f(1, 2); }",
        "expected 1 arguments, found 2",
    );
}

#[test]
fn the_entry_point_cannot_be_used_as_a_value() {
    assert_error(
        "fn main() { const f = main; }",
        "`main` is the entry point and cannot be used as a value",
    );
}

#[test]
fn a_lambda_has_its_function_type() {
    assert_type(
        "fn main() { const f = |x: i64| -> i64 { return x + 1; }; }",
        "f",
        TypeKind::Func(vec![ty(TypeKind::I64)], Box::new(ty(TypeKind::I64))),
    );
}

#[test]
fn a_lambda_can_be_passed_to_a_function_parameter() {
    let result = check_source(
        "fn apply(f: fn(i64) -> i64, x: i64) -> i64 { return f(x); } \
         fn main() { const r = apply(|x: i64| -> i64 { return x + 1; }, 41); }",
    );
    result.expect("passing a lambda as an argument should type-check");
}

#[test]
fn a_lambda_that_captures_an_enclosing_local_is_an_error() {
    assert_error(
        "fn main() { const n = 1; const f = |x: i64| -> i64 { return x + n; }; }",
        "closures are not captured yet; this lambda needs `n`",
    );
}

#[test]
fn a_lambda_may_call_a_top_level_function() {
    // A top-level function is a global, not a capture: nothing has to be
    // copied into the lambda for it to be there.
    let result = check_source(
        "fn square(x: i64) -> i64 { return x * x; } \
         fn main() { const f = |x: i64| -> i64 { return square(x); }; }",
    );
    result.expect("a lambda may call a top-level function");
}

#[test]
fn a_lambda_body_is_checked_against_its_return_type() {
    assert_error(
        "fn main() { const f = |x: i64| -> i64 { return \"not an i64\"; }; }",
        "expected `i64`",
    );
}
