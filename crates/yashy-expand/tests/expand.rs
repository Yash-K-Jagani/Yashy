//! Tests for the expander: caps, determinism, and duplicate names.

use yashy_common::{codes, FileId, SourceMap};
use yashy_expand::expand_tokens;
use yashy_lexer::{Lexer, Token};
use yashy_parser::ast::Item;
use yashy_parser::Parser;

fn files(src: &str) -> (SourceMap, FileId, Vec<Item>, Vec<Token>) {
    let mut map = SourceMap::new();
    let file = map.add_file("main.ya", src.to_string());
    let tokens = Lexer::with_file(src, file)
        .tokenize()
        .expect("the fixture must lex");
    let program = Parser::new(tokens.clone(), &map, file)
        .parse_program()
        .expect("the fixture must parse");
    (map, file, program.items, tokens)
}

fn kind_spans(tokens: &[Token]) -> Vec<(yashy_lexer::TokenKind, yashy_common::Span)> {
    tokens
        .iter()
        .map(|token| (token.kind, token.span))
        .collect()
}

#[test]
fn a_self_recursive_macro_is_a_depth_error() {
    let src = "macro again { $x:expr => { again($x) } } fn main() { again(1); }";
    let (map, file, items, tokens) = files(src);

    let (result, _) = expand_tokens(&items, &map, file, &tokens);
    let diagnostic = result.expect_err("a self-recursive macro must be refused");
    assert_eq!(diagnostic.code, Some(codes::MACRO_DEPTH));
}

#[test]
fn sixty_four_nested_macros_pass_and_sixty_five_fail() {
    let chain = |count: usize| {
        let mut src = String::new();
        for index in 0..count {
            let next = if index + 1 < count {
                format!("m{}($x)", index + 1)
            } else {
                "done($x)".to_string()
            };
            src.push_str(&format!("macro m{index} {{ $x:expr => {{ {next} }} }} "));
        }
        src.push_str("fn main() { m0(1); }");
        src
    };

    let (map, file, items, tokens) = files(&chain(64));
    let (result, _) = expand_tokens(&items, &map, file, &tokens);
    assert!(result.is_ok(), "64 distinct macros must expand");

    let (map, file, items, tokens) = files(&chain(65));
    let (result, _) = expand_tokens(&items, &map, file, &tokens);
    let diagnostic = result.expect_err("65 distinct macros must exceed the depth cap");
    assert_eq!(diagnostic.code, Some(codes::MACRO_DEPTH));
}

#[test]
fn a_duplicate_macro_name_is_rejected() {
    let src =
        "macro unless { $cond:expr => { if $cond { } } } macro unless { $cond:ident => { true } }";
    let (map, file, items, tokens) = files(src);

    let (result, _) = expand_tokens(&items, &map, file, &tokens);
    let diagnostic = result.expect_err("a duplicate macro name must be refused");
    assert_eq!(diagnostic.code, Some(codes::MACRO_DUPLICATE));
}

#[test]
fn a_failed_match_names_the_invocation_and_the_arm_pattern() {
    let src = "macro unless { $cond:expr => { if $cond { } } } fn main() { unless(ready, count); }";
    let (map, file, items, tokens) = files(src);

    let (result, _) = expand_tokens(&items, &map, file, &tokens);
    let diagnostic = result.expect_err("an unmatched invocation must be refused");
    assert_eq!(diagnostic.code, Some(codes::MACRO_NO_MATCH));
    assert_eq!(
        diagnostic.label.as_deref(),
        Some("unless(ready, count)"),
        "the caret label must be the invocation the caller wrote"
    );
    assert_eq!(
        usize::try_from(diagnostic.span.end - diagnostic.span.start).unwrap_or(0),
        "unless(ready, count)".len(),
        "the underlined span must cover the whole invocation"
    );
    assert!(
        diagnostic.message.contains("unless(ready, count)"),
        "the message must quote the invocation the caller wrote: {}",
        diagnostic.message
    );
    let notes = diagnostic.notes.join("\n");
    assert!(
        notes.contains("$cond:expr"),
        "the notes must show the pattern that was tried: {notes}"
    );
    assert!(
        !notes.contains("if $cond"),
        "a pattern note must not quote the replacement: {notes}"
    );
}

#[test]
fn expanding_the_same_program_twice_is_deterministic() {
    let src = "macro unless { $cond:expr => { if $cond { } } } fn main() { unless(ready); }";
    let (map, file, items, tokens) = files(src);

    let (first, first_map) = expand_tokens(&items, &map, file, &tokens);
    let (second, second_map) = expand_tokens(&items, &map, file, &tokens);
    let first = first.expect("the first expansion must succeed");
    let second = second.expect("the second expansion must succeed");

    assert_eq!(kind_spans(&first), kind_spans(&second));
    assert_eq!(first_map.records.len(), 1);
    assert_eq!(second_map.records.len(), 1);
}
