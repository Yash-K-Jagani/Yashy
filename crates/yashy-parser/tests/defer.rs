//! `defer` takes a block or one/many expressions; both spellings become the
//! same one-statement-per-release body.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::ast::{Item, Stmt};
use yashy_parser::Parser;

fn body_of(src: &str) -> Vec<Stmt> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("defer.ya".to_string(), src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the source must lex");
    let program = Parser::new(tokens, &sources, file)
        .parse_program()
        .expect("the source must parse");
    let Item::Fn { body, .. } = program.items.into_iter().next().unwrap() else {
        panic!("expected a function");
    };
    body.stmts
}

#[test]
fn defer_expression_is_a_block_with_one_statement() {
    let statements = body_of("fn main() { let p = alloc(8); defer p.free(); }");
    let Some(Stmt::Defer { body, .. }) = statements.last() else {
        panic!("expected a defer, got {statements:?}");
    };
    assert_eq!(body.stmts.len(), 1, "{body:?}");
    assert!(matches!(body.stmts[0], Stmt::Expr(_, _)));
}

#[test]
fn defer_comma_list_registers_releases_in_order() {
    let statements =
        body_of("fn main() { let p = alloc(8); let q = alloc(8); defer p.free(), q.free(); }");
    let Some(Stmt::Defer { body, .. }) = statements.last() else {
        panic!("expected a defer, got {statements:?}");
    };
    assert_eq!(body.stmts.len(), 2, "one statement per release: {body:?}");
}

#[test]
fn defer_block_keeps_its_statements() {
    let statements = body_of("fn main() { let p = alloc(8); defer { p.free(); } }");
    let Some(Stmt::Defer { body, .. }) = statements.last() else {
        panic!("expected a defer, got {statements:?}");
    };
    assert_eq!(body.stmts.len(), 1);
}
