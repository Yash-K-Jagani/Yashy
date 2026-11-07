//! Tests for metavariable matching over token trees.

use yashy_common::{codes, FileId, SourceMap};
use yashy_expand::fragment::FragmentKind;
use yashy_expand::r#match::{matches, no_match_diagnostic, MatchFailure};
use yashy_expand::tt::{token_trees, TokenTree};
use yashy_lexer::Lexer;
use yashy_parser::ast::Item;
use yashy_parser::Parser;

fn files(src: &str) -> (SourceMap, FileId) {
    let mut map = SourceMap::new();
    let file = map.add_file("main.ya", src.to_string());
    (map, file)
}

fn trees(src: &str, map: &SourceMap, file: FileId) -> Vec<TokenTree> {
    let tokens = Lexer::with_file(src, file)
        .tokenize()
        .expect("the fixture must lex");
    token_trees(&tokens).expect("the fixture must nest")
}

fn pattern_for(src: &str) -> (Vec<TokenTree>, Vec<TokenTree>, SourceMap, FileId, String) {
    let (map, file) = files(src);
    let tokens = Lexer::with_file(src, file)
        .tokenize()
        .expect("the fixture must lex");
    let program = Parser::new(tokens, &map, file)
        .parse_program()
        .expect("the fixture must parse");
    for item in program.items {
        if let Item::Macro { name, arms, .. } = item {
            let arm = arms
                .into_iter()
                .next()
                .expect("the fixture must have an arm");
            let pattern = token_trees(&arm.pattern).expect("the arm pattern must nest");
            let replacement = token_trees(&arm.replacement).expect("the arm replacement must nest");
            return (pattern, replacement, map, file, name);
        }
    }
    panic!("the fixture must declare a macro");
}

fn input_trees(src: &str) -> (Vec<TokenTree>, SourceMap, FileId) {
    let (map, file) = files(src);
    (trees(src, &map, file), map, file)
}

#[test]
fn an_expr_metavariable_consumes_one_expression() {
    let src = "macro unless { $cond:expr, $body:block => { if $cond { } } }";
    let (pattern, _replacement, definition_source, definition_file, _name) = pattern_for(src);
    let input_src = "x > 0, { a(); }";
    let (input, invocation_source, invocation_file) = input_trees(input_src);

    let bindings = matches(
        &pattern,
        &definition_source,
        definition_file,
        &input,
        &invocation_source,
        invocation_file,
    )
    .expect("a malformed pattern must be reported, not silently unmatched")
    .expect("the invocation must match");

    assert_eq!(bindings["cond"].len(), 3);
    assert_eq!(bindings["body"].len(), 1);
}

#[test]
fn a_mismatched_fragment_kind_is_rejected() {
    let src = "macro get { $value:ty => { $value } }";
    let (pattern, _replacement, definition_source, definition_file, _name) = pattern_for(src);
    let (input, invocation_source, invocation_file) = input_trees("1 + 2");

    let failure = matches(
        &pattern,
        &definition_source,
        definition_file,
        &input,
        &invocation_source,
        invocation_file,
    )
    .expect_err("a type metavariable cannot take an expression");
    match failure {
        MatchFailure::BadFragment { kind, .. } => assert_eq!(kind, FragmentKind::Ty),
        other => panic!("expected a fragment failure, got {other:?}"),
    }
    let diagnostic = failure
        .diagnostic(invocation_file, &invocation_source)
        .expect("a fragment failure is a diagnostic");
    assert_eq!(diagnostic.code, Some(codes::MACRO_FRAGMENT));
}

#[test]
fn no_matching_arm_prints_the_invocation() {
    let src = "macro unless { $cond:expr => { if $cond { } } }";
    let (pattern, _replacement, definition_source, definition_file, name) = pattern_for(src);
    let input_src = "ready, count";
    let (input, _invocation_source, invocation_file) = input_trees(input_src);

    assert_eq!(
        matches(
            &pattern,
            &definition_source,
            definition_file,
            &input,
            &_invocation_source,
            invocation_file,
        )
        .expect("a well-formed pattern must match or not"),
        None
    );

    let span = input
        .first()
        .map(TokenTree::span)
        .unwrap_or(yashy_common::Span::SYNTHETIC);
    let diagnostic = no_match_diagnostic(
        &name,
        invocation_file,
        span,
        input_src,
        &[input_src.to_string()],
    );
    assert_eq!(diagnostic.code, Some(codes::MACRO_NO_MATCH));
    assert!(diagnostic
        .label
        .as_deref()
        .unwrap_or("")
        .contains(input_src));
}

#[test]
fn an_identifier_metavariable_matches_only_an_identifier() {
    let src = "macro use_it { $name:ident => { $name } }";
    let (pattern, _replacement, definition_source, definition_file, _name) = pattern_for(src);
    let (input, invocation_source, invocation_file) = input_trees("1");

    assert_eq!(
        matches(
            &pattern,
            &definition_source,
            definition_file,
            &input,
            &invocation_source,
            invocation_file,
        )
        .expect("a well-formed pattern must match or not"),
        None
    );
}
