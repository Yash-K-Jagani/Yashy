//! Contract checking: the clauses reach the HIR, they are `bool`, and `result`
//! exists exactly where it should.
//!
//! Discharging a contract -- proving a call site satisfies it -- is the
//! verifier's job and is tested in `yashy-verify`. Here the question is only
//! whether the checker understood the clause.

use yashy_common::{codes, Diagnostic, FileId, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program_with, CheckOptions, HirExpr, HirFn, HirItem, HirStmt};

fn check_source(src: &str) -> (Result<yashy_sema::HirProgram, Vec<Diagnostic>>, FileId) {
    let mut sources = SourceMap::new();
    let file = sources.add_file("test.ya", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the lexer must not panic on these inputs");
    let ast = Parser::new(tokens, &sources, file)
        .parse_program()
        .expect("the parser must not panic on these inputs");
    let result = check_program_with(&ast, &sources, file, CheckOptions { release: false });
    (result, file)
}

fn check_ok(src: &str) -> yashy_sema::HirProgram {
    match check_source(src).0 {
        Ok(hir) => hir,
        Err(diags) => panic!("{src:?} must check, got {diags:?}"),
    }
}

fn diagnostics(src: &str) -> Vec<Diagnostic> {
    match check_source(src).0 {
        Ok(hir) => panic!("{src:?} must not check, got {hir:?}"),
        Err(diags) => diags,
    }
}

fn function<'a>(hir: &'a yashy_sema::HirProgram, name: &str) -> &'a HirFn {
    hir.items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(f) if f.name == name => Some(f),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no function named `{name}`"))
}

fn codes_of(diags: &[Diagnostic]) -> Vec<&str> {
    diags.iter().filter_map(|d| d.code.as_deref()).collect()
}

fn all_messages(diags: &[Diagnostic]) -> String {
    diags
        .iter()
        .map(|d| d.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

/// True when `expr` is a bare name.
fn names(expr: &HirExpr, needle: &str) -> bool {
    matches!(expr, HirExpr::Ident(path, _, _) if path.len() == 1 && path[0] == needle)
}

// -- the clauses reach the HIR --------------------------------------------

#[test]
fn a_precondition_and_a_postcondition_reach_the_hir() {
    let hir = check_ok("fn f(n: int) -> int requires n > 0 ensures result > 0 { return n; }");
    let f = function(&hir, "f");
    assert_eq!(f.requires.len(), 1);
    assert_eq!(f.ensures.len(), 1);
}

#[test]
fn a_postcondition_sees_result_and_a_precondition_does_not() {
    let hir = check_ok("fn f(n: int) -> int requires n > 0 ensures result > 0 { return n; }");
    let f = function(&hir, "f");
    // `ensures result > 0` is a comparison whose left side is `result`.
    let HirExpr::Binary { lhs, .. } = &f.ensures[0].expr else {
        panic!("expected a comparison, got {:?}", f.ensures[0].expr);
    };
    assert!(names(lhs, "result"));
    // `requires n > 0` is a comparison whose left side is `n`.
    let HirExpr::Binary { lhs, .. } = &f.requires[0].expr else {
        panic!("expected a comparison, got {:?}", f.requires[0].expr);
    };
    assert!(names(lhs, "n"));
}

#[test]
fn a_loop_invariant_reaches_the_hir_as_the_first_statement() {
    // A parameter is immutable, so the loop needs its own counter; the loop is
    // then the second statement of the body, after the binding.
    let hir = check_ok("fn f(n: int) { let i = n; while i > 0 invariant i >= 0 { i = i - 1; } }");
    let f = function(&hir, "f");
    let HirStmt::ForC { body, .. } = &f.body.stmts[1] else {
        panic!("expected a while loop, got {:?}", f.body.stmts[1]);
    };
    assert!(
        matches!(body.stmts.first(), Some(HirStmt::LoopInvariant(_))),
        "the invariant must be the loop body's first statement: {:?}",
        body.stmts
    );
}

// -- a clause must be a bool ----------------------------------------------

#[test]
fn a_non_bool_precondition_is_a_type_mismatch() {
    let diags = diagnostics("fn f(n: int) requires 1 { }");
    assert_eq!(codes_of(&diags), vec![codes::TYPE_MISMATCH]);
}

#[test]
fn a_non_bool_postcondition_is_a_type_mismatch() {
    let diags = diagnostics("fn f(n: int) -> int ensures result { return n; }");
    assert_eq!(codes_of(&diags), vec![codes::TYPE_MISMATCH]);
}

#[test]
fn a_non_bool_loop_invariant_is_a_type_mismatch() {
    let diags = diagnostics("fn f(n: int) { while n > 0 invariant 1 { } }");
    assert_eq!(codes_of(&diags), vec![codes::TYPE_MISMATCH]);
}

// -- `result` is scoped to `ensures` --------------------------------------

#[test]
fn result_is_not_visible_in_the_body() {
    let diags = diagnostics("fn f(n: int) -> int ensures result > 0 { return result; }");
    assert!(
        codes_of(&diags).contains(&codes::UNKNOWN_NAME),
        "the body must not see `result`: {diags:?}"
    );
}

#[test]
fn result_is_not_visible_in_a_precondition() {
    let diags = diagnostics("fn f(n: int) -> int requires result > 0 { return n; }");
    assert!(
        codes_of(&diags).contains(&codes::UNKNOWN_NAME),
        "`requires` must not see `result`: {diags:?}"
    );
}

#[test]
fn result_has_the_return_type() {
    // `result` is an `int` here, so a clause that uses it as the condition --
    // where a `bool` is required -- is a mismatch even though `result` itself
    // exists. (An unknown `result` would be `E0001` instead.)
    let diags = diagnostics("fn f() -> int ensures result { return 1; }");
    assert!(
        codes_of(&diags).contains(&codes::TYPE_MISMATCH),
        "`result` must have the function's return type: {diags:?}"
    );
}

// -- what a contract cannot be written on --------------------------------

#[test]
fn a_method_cannot_declare_a_contract() {
    let diags = diagnostics(
        "struct P { x: int }
         impl P { fn get(self) -> int requires self.x > 0 { return self.x; } }",
    );
    assert!(
        all_messages(&diags).contains("method cannot declare a contract"),
        "a method contract must be refused, not dropped: {diags:?}"
    );
}

// -- generics -------------------------------------------------------------

#[test]
fn a_generic_contract_is_checked_at_its_instantiation() {
    let hir = check_ok(
        "fn id[T](x: T) -> T requires true { return x; }
         fn main() { let a = id(1); }",
    );
    let instance = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Fn(f) if f.name.starts_with("id") && f.name != "id" => Some(f),
            _ => None,
        })
        .expect("the generic must be instantiated");
    assert_eq!(instance.requires.len(), 1);
}
