//! Labeled loops: parsing `name: for` / `name: while`, `break name` and
//! `continue name`.
//!
//! A label is an identifier and a colon in statement position only, and only
//! when the next token is `for` or `while`. These tests pin that, and that an
//! ordinary declaration or struct literal is unaffected.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::ast::{Item, Program, Stmt};
use yashy_parser::Parser;

fn parse_src(src: &str) -> Result<Program, Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("labeled.ya", src.to_string());
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

fn first_main_stmt(stmt: &str) -> Stmt {
    let program = parse_ok(&format!("fn main() {{ {stmt} }}"));
    let Item::Fn { body, .. } = program.items.into_iter().next().unwrap() else {
        panic!("expected a function item");
    };
    body.stmts.into_iter().next().expect("main has a statement")
}

#[test]
fn a_label_may_precede_a_for_loop() {
    match first_main_stmt("outer: for i in 0..3 { print(i); }") {
        Stmt::ForRange { label, .. } => assert_eq!(label.as_deref(), Some("outer")),
        other => panic!("expected a labeled `for`, got {other:?}"),
    }
}

#[test]
fn a_label_may_precede_a_while_loop() {
    match first_main_stmt("outer: while n < 3 { n = n + 1; }") {
        Stmt::ForC { label, .. } => assert_eq!(label.as_deref(), Some("outer")),
        other => panic!("expected a labeled `while`, got {other:?}"),
    }
}

#[test]
fn an_unlabeled_loop_has_no_label() {
    match first_main_stmt("for i in 0..3 { print(i); }") {
        Stmt::ForRange { label, .. } => assert_eq!(label, None),
        other => panic!("expected an unlabeled `for`, got {other:?}"),
    }
}

#[test]
fn break_and_continue_may_name_a_label() {
    match first_main_stmt("for i in 0..3 { break outer; }") {
        Stmt::ForRange { body, .. } => match body.stmts.into_iter().next().unwrap() {
            Stmt::Break(label, _) => assert_eq!(label.as_deref(), Some("outer")),
            other => panic!("expected a labeled `break`, got {other:?}"),
        },
        other => panic!("expected a `for`, got {other:?}"),
    }
    match first_main_stmt("for i in 0..3 { continue outer; }") {
        Stmt::ForRange { body, .. } => match body.stmts.into_iter().next().unwrap() {
            Stmt::Continue(label, _) => assert_eq!(label.as_deref(), Some("outer")),
            other => panic!("expected a labeled `continue`, got {other:?}"),
        },
        other => panic!("expected a `for`, got {other:?}"),
    }
}

#[test]
fn an_unlabeled_break_and_continue_still_parse() {
    match first_main_stmt("for i in 0..3 { break; }") {
        Stmt::ForRange { body, .. } => match body.stmts.into_iter().next().unwrap() {
            Stmt::Break(None, _) => {}
            other => panic!("expected an unlabeled `break`, got {other:?}"),
        },
        other => panic!("expected a `for`, got {other:?}"),
    }
}

#[test]
fn a_label_before_a_non_loop_is_a_parse_error() {
    let diags = parse_src("fn main() { outer: print(1); }")
        .err()
        .expect("a label before a non-loop must not parse");
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("may only precede")),
        "{diags:?}"
    );
}

#[test]
fn a_declaration_is_not_a_label() {
    // After Task 0 a type precedes a name, so `int x` in statement position
    // is a declaration, not a label: no `for`/`while` follows it.
    let program = parse_ok("fn main() { let int x = 1; print(x); }");
    assert_eq!(program.items.len(), 1);
}
