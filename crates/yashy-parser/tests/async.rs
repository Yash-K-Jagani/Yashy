//! Async syntax: `async fn`, `await`, `spawn` and `scope`.
//!
//! The parser only has to say what was written. Whether an `await` may appear
//! here is the checker's `E1700`, and whether the call of an async function
//! needs one is the checker's `E1300`, so these tests stop at the tree.

use yashy_common::{DeclaredEffect, Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::ast::{Expr, Item, Program, Stmt};
use yashy_parser::Parser;

fn parse_src(src: &str) -> Result<Program, Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("async.ya", src.to_string());
    let tokens = Lexer::new(src).tokenize()?;
    let mut parser = Parser::new(tokens, &sources, file);
    parser.parse_program()
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
        Err(diags) => diags,
    }
}

fn first_main_stmt(stmt: &str) -> Stmt {
    let program = parse_ok(&format!("fn main() {{ {stmt} }}"));
    let Item::Fn { body, .. } = program.items.into_iter().next().unwrap() else {
        panic!("expected a function item");
    };
    body.stmts.into_iter().next().expect("main has a statement")
}

fn stmt_expr(stmt: &str) -> Expr {
    match first_main_stmt(stmt) {
        Stmt::Expr(expr, _) => expr,
        other => panic!("expected an expression statement, got {other:?}"),
    }
}

#[test]
fn an_async_function_is_an_effect_wrapper_over_a_function() {
    let program = parse_ok("async fn fetch(string url) -> string { return url; }");
    match program.items.into_iter().next().unwrap() {
        Item::Effect { effect, item, .. } => {
            assert_eq!(effect, DeclaredEffect::Async);
            assert!(matches!(*item, Item::Fn { .. }));
        }
        other => panic!("expected an async effect item, got {other:?}"),
    }
}

#[test]
fn await_binds_tighter_than_a_binary_operator() {
    match stmt_expr("x.await + 1;") {
        Expr::Binary { lhs, .. } => assert!(matches!(*lhs, Expr::Await { .. })),
        other => panic!("expected (x.await) + 1, got {other:?}"),
    }
}

#[test]
fn await_parses_outside_an_async_function_and_the_checker_rejects_it() {
    // The parser accepts the form wherever an expression can appear; E1700
    // is a checker diagnostic, so parsing must succeed.
    let _ = stmt_expr("fetch(url).await;");
}

#[test]
fn spawn_parses_as_a_spawn_expression() {
    match stmt_expr("spawn(worker, 3);") {
        Expr::Spawn { callee, args, .. } => {
            assert!(matches!(*callee, Expr::Ident(_, _)));
            assert_eq!(args.len(), 1);
        }
        other => panic!("expected a spawn node, got {other:?}"),
    }
}

#[test]
fn a_scope_statement_holds_a_block() {
    match first_main_stmt("scope { spawn(worker); }") {
        Stmt::Scope { body, .. } => assert_eq!(body.stmts.len(), 1),
        other => panic!("expected a scope statement, got {other:?}"),
    }
}

#[test]
fn await_and_async_are_keywords_not_identifiers() {
    assert!(!parse_err("fn main() { let await = 1; }").is_empty());
    assert!(!parse_err("fn main() { let async = 1; }").is_empty());
}
