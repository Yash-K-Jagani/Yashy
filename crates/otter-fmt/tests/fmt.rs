//! The formatter's contract: idempotence, comment preservation, literal
//! preservation, and unchanged meaning.

use std::fs;
use std::path::{Path, PathBuf};

use otter_common::SourceMap;
use otter_lexer::{Lexer, TokenKind};
use otter_parser::{Parser, Program};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn read(name: &str) -> String {
    fs::read_to_string(fixtures().join(name))
        .unwrap_or_else(|error| panic!("cannot read fixture {name}: {error}"))
        .replace("\r\n", "\n")
}

fn format(src: &str) -> String {
    otter_fmt::format(src).unwrap_or_else(|diagnostics| {
        panic!(
            "expected the source to format; diagnostics: {:?}",
            diagnostics
                .iter()
                .map(|d| &d.message)
                .collect::<Vec<_>>()
        )
    })
}

/// Parse a source file into its AST, which is what "the same program" means.
fn check(src: &str) -> Program {
    let mut sources = SourceMap::new();
    let file = sources.add_file("fixture.ot", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer must not panic");
    Parser::new(tokens, &sources, file)
        .parse_program()
        .expect("the fixture must parse")
}

/// The token sequence with each token's own text, which is what "the same
/// program" means for a printer: the AST's spans move when whitespace moves,
/// so comparing ASTs would fail on every reformat, while comparing (kind, text)
/// pairs catches a dropped, added, reordered, or rewritten token.
fn tokens_of(src: &str) -> Vec<(TokenKind, String)> {
    Lexer::new(src)
        .tokenize()
        .expect("the lexer must not panic")
        .into_iter()
        .filter(|token| token.kind != TokenKind::Eof)
        .map(|token| {
            let text = &src[token.span.start as usize..token.span.end as usize];
            (token.kind, text.to_string())
        })
        .collect()
}

/// Every comment's text, read from the gaps between tokens (the only places a
/// comment can be).
fn comments_of(src: &str) -> Vec<String> {
    let tokens = Lexer::new(src).tokenize().expect("the lexer must not panic");
    let mut comments = Vec::new();
    let mut end = 0;
    for token in &tokens {
        let start = token.span.start as usize;
        let gap = &src[end..start];
        let bytes = gap.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/') {
                let start = index;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
                comments.push(gap[start..index].trim_end().to_string());
            } else if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
                let start = index;
                index += 2;
                let mut depth = 1usize;
                while index < bytes.len() && depth > 0 {
                    if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
                        depth += 1;
                        index += 2;
                    } else if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
                        depth -= 1;
                        index += 2;
                    } else {
                        index += 1;
                    }
                }
                comments.push(gap[start..index].to_string());
            } else {
                index += 1;
            }
        }
        end = token.span.end as usize;
    }
    comments
}

#[test]
fn formatting_an_already_formatted_file_changes_nothing() {
    let clean = read("clean.ot");
    assert_eq!(
        format(&clean),
        clean,
        "the clean fixture must already be the formatter's fixed point"
    );
}

#[test]
fn formatting_twice_equals_formatting_once() {
    let once = format(&read("messy.ot"));
    let twice = format(&once);
    assert_eq!(once, twice, "formatting must be idempotent");
}

#[test]
fn every_comment_in_a_file_survives_formatting() {
    for name in ["comments.ot", "clean.ot", "strings.ot"] {
        let source = read(name);
        let formatted = format(&source);
        for comment in comments_of(&source) {
            assert!(
                formatted.contains(&comment),
                "{name}: comment `{comment}` did not survive:\n{formatted}"
            );
        }
    }
}

#[test]
fn string_interpolation_content_is_never_reformatted() {
    let formatted = format(&read("strings.ot"));
    // The interior spacing is the author's, down to the two spaces after
    // `x +`, and the `//` inside a literal stays literal text.
    assert!(
        formatted.contains("\"value: ${x +  1} and a tab:\\t\""),
        "the interpolation was rewritten:\n{formatted}"
    );
    assert!(
        formatted.contains("\"a // not a comment, and a /* not one either */\""),
        "a literal was read as a comment:\n{formatted}"
    );
}

#[test]
fn formatting_preserves_meaning() {
    let source = read("messy.ot");
    let formatted = format(&source);
    // Both must still parse, and the token sequence must be byte-identical
    // token for token: same kinds, same identifier and literal text.
    let _ = check(&source);
    let _ = check(&formatted);
    assert_eq!(
        tokens_of(&source),
        tokens_of(&formatted),
        "formatting changed the token sequence"
    );
}

#[test]
fn empty_braces_collapse_to_one_pair() {
    // A brace pair with nothing in it is one token pair even when the source
    // split it across lines, and the `;` still attaches to it.
    let source = "fn main() {\n    var parts = []string {\n    };\n}\n";
    assert_eq!(
        format(source),
        "fn main() {\n    var parts = []string {};\n}\n"
    );
}

#[test]
fn an_unparseable_file_is_reported_and_left_unmodified() {
    let source = "fn main( {\n";
    let diagnostics = otter_fmt::format(source).expect_err("bad input must be reported");
    assert!(
        !diagnostics.is_empty(),
        "a parse failure must carry at least one diagnostic"
    );
}
