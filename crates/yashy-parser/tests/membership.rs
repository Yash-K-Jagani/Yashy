//! `needle in container` is sugar for `container.contains(needle)`.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::ast::{Expr, Item, Stmt};
use yashy_parser::Parser;

fn expression(src: &str) -> Expr {
    let mut sources = SourceMap::new();
    let file = sources.add_file("membership.ya".to_string(), src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("must lex");
    let program = Parser::new(tokens, &sources, file)
        .parse_program()
        .expect("must parse");
    let Item::Fn { body, .. } = program.items.into_iter().next().unwrap() else {
        panic!("expected a function");
    };
    match &body.stmts[0] {
        Stmt::Const { init, .. } => init.clone(),
        other => panic!("expected a const, got {other:?}"),
    }
}

#[test]
fn membership_becomes_a_contains_method_call() {
    let word = expression("fn main() { const ok = 3 in xs; }");
    let method = expression("fn main() { const ok = xs.contains(3); }");
    assert_eq!(word, method, "the tree must be the method call");
}

#[test]
fn membership_rhs_binds_tighter_than_the_operator() {
    // `1 + 2 in xs` is `(1 + 2) in xs`, not `1 + (2 in xs)`.
    let word = expression("fn main() { const ok = 1 + 2 in xs; }");
    let method = expression("fn main() { const ok = xs.contains(1 + 2); }");
    assert_eq!(word, method);
}

#[test]
fn membership_does_not_chain() {
    // A second `in` after the container is a parse error, like `==` chaining
    // through a different operator family: `a in b in c` is not one operator.
    let mut sources = SourceMap::new();
    let src = "fn main() { const ok = a in b in c; }";
    let file = sources.add_file("membership.ya".to_string(), src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("must lex");
    assert!(
        Parser::new(tokens, &sources, file).parse_program().is_err(),
        "membership must not chain"
    );
}
