//! Tests for macro declarations, the first seam of expansion.
//!
//! A macro is declared in the language and stored as tokens; it is not yet
//! expanded. These tests pin the declaration shapes the expander will consume:
//! that a balanced group is one tree, that two arms parse as two arms, and
//! that the formatter leaves a declaration exactly as it was written.

use yashy_common::SourceMap;
use yashy_expand::tt::{token_trees, Delim, TokenTree};
use yashy_lexer::Lexer;
use yashy_parser::{
    ast::{Item, MacroArm},
    Parser,
};

fn tokens(src: &str) -> Vec<yashy_lexer::Token> {
    let mut map = SourceMap::new();
    let file = map.add_file("main.ya", src.to_string());
    Lexer::with_file(src, file)
        .tokenize()
        .expect("the fixture must lex")
}

fn program(src: &str) -> yashy_parser::ast::Program {
    let mut map = SourceMap::new();
    let file = map.add_file("main.ya", src.to_string());
    let toks = Lexer::with_file(src, file)
        .tokenize()
        .expect("the fixture must lex");
    Parser::new(toks, &map, file)
        .parse_program()
        .expect("the fixture must parse")
}

fn macro_item(src: &str) -> (String, Vec<MacroArm>) {
    for item in program(src).items {
        if let Item::Macro { name, arms, .. } = item {
            return (name, arms);
        }
    }
    panic!("the fixture must declare a macro");
}

#[test]
fn a_balanced_group_is_one_delimited_tree() {
    let trees = token_trees(&tokens("f(1, 2) [a] { b; }")).expect("the fixture must nest");

    assert_eq!(trees.len(), 4);
    match &trees[1] {
        TokenTree::Delimited { open, trees, .. } => {
            assert_eq!(*open, Delim::Paren);
            assert_eq!(trees.len(), 3);
        }
        other => panic!("expected a group, got {other:?}"),
    }
    match &trees[2] {
        TokenTree::Delimited { open, .. } => assert_eq!(*open, Delim::Bracket),
        other => panic!("expected a group, got {other:?}"),
    }
    match &trees[3] {
        TokenTree::Delimited { open, .. } => assert_eq!(*open, Delim::Brace),
        other => panic!("expected a group, got {other:?}"),
    }
}

#[test]
fn an_unbalanced_group_is_a_tree_error() {
    let err = token_trees(&tokens("(1, 2")).expect_err("unbalanced must fail");
    assert_eq!(
        err,
        yashy_expand::TreeError::Unbalanced {
            open: yashy_lexer::Token {
                kind: yashy_lexer::TokenKind::LParen,
                span: err_open_span(&err),
            }
        }
    );
}

#[test]
fn two_arms_parse_as_two_arms() {
    let (name, arms) =
        macro_item("macro unless { $cond:expr => { if $cond { } } $cond:ident => { true } }");

    assert_eq!(name, "unless");
    assert_eq!(arms.len(), 2);
}

#[test]
fn a_macro_declaration_formats_as_written() {
    let src = "macro unless {\n    $ cond: expr => {\n        if $ cond {}\n    }\n}\n";
    assert_eq!(
        yashy_fmt::format(src).expect("the fixture must format"),
        src
    );
}

fn err_open_span(err: &yashy_expand::TreeError) -> yashy_common::Span {
    match err {
        yashy_expand::TreeError::Unbalanced { open } => open.span,
        yashy_expand::TreeError::Stray { close } => close.span,
        yashy_expand::TreeError::TooDeep { at } => at.span,
    }
}
