//! The formatter's contract: idempotence, comment preservation, literal
//! preservation, and unchanged meaning.

use std::fs;
use std::path::{Path, PathBuf};

use yashy_common::SourceMap;
use yashy_lexer::{Lexer, TokenKind};
use yashy_parser::{Parser, Program};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn read(name: &str) -> String {
    fs::read_to_string(fixtures().join(name))
        .unwrap_or_else(|error| panic!("cannot read fixture {name}: {error}"))
        .replace("\r\n", "\n")
}

fn format(src: &str) -> String {
    yashy_fmt::format(src).unwrap_or_else(|diagnostics| {
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
    let file = sources.add_file("fixture.ya", src.to_string());
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
    let clean = read("clean.ya");
    assert_eq!(
        format(&clean),
        clean,
        "the clean fixture must already be the formatter's fixed point"
    );
}

#[test]
fn formatting_twice_equals_formatting_once() {
    let once = format(&read("messy.ya"));
    let twice = format(&once);
    assert_eq!(once, twice, "formatting must be idempotent");
}

#[test]
fn every_comment_in_a_file_survives_formatting() {
    for name in ["comments.ya", "clean.ya", "strings.ya"] {
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
    let formatted = format(&read("strings.ya"));
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
    let source = read("messy.ya");
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
fn contracts_print_one_per_line_before_the_brace() {
    // A clause sits between the signature and its body, one per line, indented
    // one level past the signature. The body brace keeps the signature's own
    // indent, so the clauses read as a list that belongs to the signature.
    let source = "fn clamp(n: int) -> int requires n >= 0 ensures result >= 0 {\n    while n > 10 invariant n >= 0 {\n        n = n - 1;\n    }\n    return n;\n}\n";
    assert_eq!(
        format(source),
        "fn clamp(n: int) -> int\n    requires n >= 0\n    ensures result >= 0\n{\n    while n > 10\n        invariant n >= 0\n    {\n        n = n - 1;\n    }\n    return n;\n}\n"
    );
}

#[test]
fn a_formatted_contract_file_is_a_fixed_point() {
    let source = "fn f(n: int) -> int\n    requires n > 0\n    ensures result > 0\n{\n    while n > 1\n        invariant n > 0\n    {\n        n = n - 1;\n    }\n    return n;\n}\n";
    let once = format(source);
    assert_eq!(once, source, "the clause layout must be the canonical one");
    assert_eq!(format(&once), once, "formatting must be idempotent");
}

#[test]
fn a_contract_with_a_struct_literal_keeps_its_literal_nested() {
    // The clause list ends at the body brace. A `{` that looks like a struct
    // literal is part of the clause's expression, not the body's, so the
    // formatter must not treat it as the end of the list.
    let source = "struct P {\n    x: int\n}\n\nfn f(p: P) -> bool\n    requires p == P {\n        x: 1\n    }\n{\n    return true;\n}\n";
    assert_eq!(format(source), source);
}

#[test]
fn empty_braces_collapse_to_one_pair() {
    // A brace pair with nothing in it is one token pair even when the source
    // split it across lines, and the `;` still attaches to it.
    let source = "fn main() {\n    let parts = []string {\n    };\n}\n";
    assert_eq!(
        format(source),
        "fn main() {\n    let parts = []string {};\n}\n"
    );
}

#[test]
fn a_lambda_bars_bind_to_their_parameter_list() {
    // `|x: i64|` reads as one thing: the bar opens the list, the bar closes
    // it, and neither takes air. The arrow after the closing bar does.
    let source = "fn main() {\n    const f = |x: int| -> int { return x; };\n}\n";
    assert_eq!(
        format(source),
        "fn main() {\n    const f = |x: int| -> int {\n        return x;\n    };\n}\n"
    );
}

#[test]
fn a_trailing_comma_keeps_its_closer_attached() {
    // `(5,)` is the one-element tuple and `[1,]` is an array with a trailing
    // separator; in both cases the comma is followed by the closer rather
    // than by another element, so the separator's space would be stray. The
    // same source is the fixed point: formatting it again changes nothing.
    let source = "fn main() {\n    const pair = (5,);\n    const (only,) = pair;\n    const xs = [1, 2, 3,];\n}\n";
    assert_eq!(format(source), source);
}

#[test]
fn bitwise_operators_keep_their_parentheses() {
    // `a & b | c ^ ~d` parses as `((a & b) | (c ^ (~d)))`; the printer keeps
    // that shape with no parentheses added and none removed.
    let source = "fn main() {\n    const r = a & b | c ^ ~d;\n}\n";
    assert_eq!(format(source), source);
    assert_eq!(format(&format(source)), source, "formatting must be idempotent");
}

#[test]
fn an_unparseable_file_is_reported_and_left_unmodified() {
    let source = "fn main( {\n";
    let diagnostics = yashy_fmt::format(source).expect_err("bad input must be reported");
    assert!(
        !diagnostics.is_empty(),
        "a parse failure must carry at least one diagnostic"
    );
}
