//! Contract clauses: `requires`, `ensures`, and a loop's `invariant`.
//!
//! The clauses are parsed into the tree rather than checked here: the parser's
//! job is to put the expression where the verifier will look for it and to keep
//! the span the diagnostic needs. A clause that is not a `bool` is the
//! checker's business and is tested in `yashy-sema`.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::ast::{BinOp, Contract, Expr, Item, Program, Stmt};
use yashy_parser::Parser;

// -- helpers ---------------------------------------------------------------

fn parse_src(src: &str) -> Result<Program, Vec<Diagnostic>> {
    let mut map = SourceMap::new();
    let file = map.add_file("main.ya", src.to_string());
    let tokens = match Lexer::with_file(src, file).tokenize() {
        Ok(tokens) => tokens,
        Err(diags) => panic!("test source must lex, got {diags:?}"),
    };
    Parser::new(tokens, &map, file).parse_program()
}

fn parse_ok(src: &str) -> Program {
    match parse_src(src) {
        Ok(program) => program,
        Err(diags) => panic!("{src:?} must parse, got {diags:?}"),
    }
}

fn parse_err(src: &str) -> Vec<Diagnostic> {
    match parse_src(src) {
        Ok(program) => panic!("{src:?} must not parse, got {program:?}"),
        Err(diags) => {
            assert!(
                !diags.is_empty(),
                "{src:?} must report at least one diagnostic"
            );
            diags
        }
    }
}

fn all_messages(diags: &[Diagnostic]) -> String {
    diags
        .iter()
        .map(|d| d.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The one function item in `program`, looking through a `pub` wrapper.
fn only_fn(program: &Program) -> (&Vec<Contract>, &Vec<Contract>) {
    match &program.items[0] {
        Item::Public { item, .. } => match item.as_ref() {
            Item::Fn {
                requires, ensures, ..
            } => (requires, ensures),
            other => panic!("expected a function, got {other:?}"),
        },
        Item::Fn {
            requires, ensures, ..
        } => (requires, ensures),
        other => panic!("expected a function, got {other:?}"),
    }
}

/// The operator of a binary expression, for a clause assertion that does not
/// want to pin the whole tree.
fn op_of(contract: &Contract) -> BinOp {
    match &contract.expr {
        Expr::Binary { op, .. } => *op,
        other => panic!("expected a binary clause, got {other:?}"),
    }
}

// -- function clauses ------------------------------------------------------

#[test]
fn a_function_can_declare_a_precondition_and_a_postcondition() {
    let program = parse_ok("fn f(n: int) -> int requires n > 0 ensures result > 0 { return n; }");
    let (requires, ensures) = only_fn(&program);
    assert_eq!(requires.len(), 1);
    assert_eq!(ensures.len(), 1);
    assert_eq!(op_of(&requires[0]), BinOp::Gt);
    assert_eq!(op_of(&ensures[0]), BinOp::Gt);
}

#[test]
fn a_clause_keeps_the_span_of_its_expression() {
    let src = "fn f(n: int) requires n > 0 { }";
    let program = parse_ok(src);
    let (requires, _) = only_fn(&program);
    let start = src.find("n > 0").expect("the fixture contains the clause");
    assert_eq!(
        requires[0].span,
        yashy_common::Span::new(start as u32, (start + 5) as u32),
        "the clause is reported at its expression, not at the keyword"
    );
}

#[test]
fn clauses_repeat_and_either_order_is_kept() {
    let program = parse_ok(
        "fn f(a: bool, b: bool, c: bool, d: bool) ensures a requires b ensures c requires d { }",
    );
    let (requires, ensures) = only_fn(&program);
    assert_eq!(requires.len(), 2, "both `requires` clauses survive");
    assert_eq!(ensures.len(), 2, "both `ensures` clauses survive");
}

#[test]
fn a_function_without_clauses_has_none() {
    let program = parse_ok("fn f(n: int) -> int { return n; }");
    let (requires, ensures) = only_fn(&program);
    assert!(requires.is_empty());
    assert!(ensures.is_empty());
}

#[test]
fn a_public_function_carries_its_clauses() {
    let program = parse_ok("pub fn f(n: int) requires n > 0 { }");
    let (requires, _) = only_fn(&program);
    assert_eq!(requires.len(), 1);
}

#[test]
fn a_clause_may_call_a_function() {
    // Nothing in this slice restricts what a clause may contain; the purity
    // rule arrives with effects in phase 2.
    let program = parse_ok("fn f(n: int) requires is_positive(n) { }");
    let (requires, _) = only_fn(&program);
    assert_eq!(requires.len(), 1);
}

#[test]
fn a_const_fn_cannot_declare_a_contract() {
    let diags = parse_err("const fn f(n: int) requires n > 0 { return n; }");
    assert!(
        all_messages(&diags).contains("const fn"),
        "the diagnostic must name the const fn: {diags:?}"
    );
}

// -- loop invariants -------------------------------------------------------

#[test]
fn a_loop_can_declare_an_invariant() {
    let program = parse_ok("fn f(n: int) { while n > 0 invariant n >= 0 { n = n - 1; } }");
    let body = match &program.items[0] {
        Item::Fn { body, .. } => body,
        other => panic!("expected a function, got {other:?}"),
    };
    let Stmt::ForC { body, .. } = &body.stmts[0] else {
        panic!("expected the while loop, got {:?}", body.stmts[0]);
    };
    match &body.stmts[0] {
        Stmt::Invariant(contract) => assert_eq!(op_of(contract), BinOp::Ge),
        other => panic!("the invariant must come first, got {other:?}"),
    }
}

#[test]
fn several_invariants_keep_the_order_they_were_written_in() {
    let program =
        parse_ok("fn f(n: int) { while n > 0 invariant n >= 0 invariant n < 100 { n = n - 1; } }");
    let body = match &program.items[0] {
        Item::Fn { body, .. } => body,
        other => panic!("expected a function, got {other:?}"),
    };
    let Stmt::ForC { body, .. } = &body.stmts[0] else {
        panic!("expected the while loop");
    };
    assert!(matches!(&body.stmts[0], Stmt::Invariant(c) if op_of(c) == BinOp::Ge));
    assert!(matches!(&body.stmts[1], Stmt::Invariant(c) if op_of(c) == BinOp::Lt));
    assert!(matches!(&body.stmts[2], Stmt::Assign { .. }));
}

#[test]
fn every_loop_form_takes_an_invariant() {
    let program = parse_ok(
        "fn f(n: int, xs: []int) {
            for i in 0..n invariant i >= 0 { }
            for x in xs invariant x > 0 { }
            for ; n > 0; n - 1 invariant n >= 0 { }
        }",
    );
    let body = match &program.items[0] {
        Item::Fn { body, .. } => body,
        other => panic!("expected a function, got {other:?}"),
    };
    for stmt in &body.stmts {
        match stmt {
            Stmt::ForRange { body, .. } | Stmt::ForIn { body, .. } | Stmt::ForC { body, .. } => {
                assert!(
                    matches!(body.stmts.first(), Some(Stmt::Invariant(_))),
                    "every loop form must carry its invariant: {stmt:?}"
                );
            }
            other => panic!("expected a loop, got {other:?}"),
        }
    }
}

#[test]
fn an_invariant_is_not_a_statement_of_an_ordinary_block() {
    // The clause belongs to the header. Writing it in the body is a parse
    // error, not a statement the checker has to reject later.
    let diags = parse_err("fn f(n: int) { invariant n > 0; }");
    assert!(!diags.is_empty());
}
