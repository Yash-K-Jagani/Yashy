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
    let source = "fn clamp(int n) -> int requires n >= 0 ensures result >= 0 {\n    while n > 10 invariant n >= 0 {\n        n = n - 1;\n    }\n    return n;\n}\n";
    assert_eq!(
        format(source),
        "fn clamp(int n) -> int\n    requires n >= 0\n    ensures result >= 0\n{\n    while n > 10\n        invariant n >= 0\n    {\n        n = n - 1;\n    }\n    return n;\n}\n"
    );
}

#[test]
fn a_formatted_contract_file_is_a_fixed_point() {
    let source = "fn f(int n) -> int\n    requires n > 0\n    ensures result > 0\n{\n    while n > 1\n        invariant n > 0\n    {\n        n = n - 1;\n    }\n    return n;\n}\n";
    let once = format(source);
    assert_eq!(once, source, "the clause layout must be the canonical one");
    assert_eq!(format(&once), once, "formatting must be idempotent");
}

#[test]
fn a_contract_with_a_struct_literal_keeps_its_literal_nested() {
    // The clause list ends at the body brace. A `{` that looks like a struct
    // literal is part of the clause's expression, not the body's, so the
    // formatter must not treat it as the end of the list.
    let source = "struct P {\n    x: int\n}\n\nfn f(P p) -> bool\n    requires p == P {\n        x: 1\n    }\n{\n    return true;\n}\n";
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
fn the_try_operator_binds_to_the_expression_it_applies_to() {
    // `?` is postfix: it is part of the expression, not a word after it, so
    // `read_line()?` keeps its shape. A space would read as two things.
    let source = "fn main() {\n    const line = read_line() ?;\n}\n";
    assert_eq!(
        format(source),
        "fn main() {\n    const line = read_line()?;\n}\n"
    );
}

#[test]
fn a_negative_literal_keeps_its_sign_attached() {
    // `-1` is one thing, not a subtraction of nothing: the sign negates, and
    // it binds to the number it negates. A binary minus keeps its air.
    let source = "fn main() {\n    const a = -1;\n    const b = 2 - 1;\n    print(\"{a}\");\n}\n";
    assert_eq!(
        format(source),
        "fn main() {\n    const a = -1;\n    const b = 2 - 1;\n    print(\"{a}\");\n}\n"
    );
}

#[test]
fn a_cfg_clause_keeps_its_parenthesis_attached() {
    // `cfg` is one keyword and adds no syntax beyond the parenthesis, so the
    // clause binds like `repr(C)` does.
    let source =
        "cfg(windows) fn platform() -> string {\n    return \"windows\";\n}\n\nfn main() {\n    cfg(unix) {\n        print(\"unix\");\n    }\n}\n";
    assert_eq!(format(source), source);
}

#[test]
fn a_lambda_bars_bind_to_their_parameter_list() {
    // `|x: i64|` reads as one thing: the bar opens the list, the bar closes
    // it, and neither takes air. The arrow after the closing bar does.
    let source = "fn main() {\n    const f = |int x| -> int { return x; };\n}\n";
    assert_eq!(
        format(source),
        "fn main() {\n    const f = |int x| -> int {\n        return x;\n    };\n}\n"
    );
}

#[test]
fn a_trait_prints_each_signature_on_its_own_line() {
    // A trait is a brace block of signatures: each `fn .. ;` ends its line and
    // the closing brace returns to the declaration's indent. The formatter
    // must not need a second pass to reach this shape.
    let source = "trait Shape {\n    fn area(self) -> float;\n    fn name(self) -> string;\n    fn sides(self) -> int;\n}\n";
    assert_eq!(format(source), source);
    assert_eq!(format(&format(source)), format(source));
}

#[test]
fn generic_declaration_and_use_round_trip() {
    // Type parameters, bounds and uses are tokens: the formatter keeps their
    // spacing stable and reaches a fixed point on the second pass.
    let source = "struct Pair[T] {\n    first: T,\n    second: T\n}\n\nfn f(Pair[int] p) -> int {\n    return 0;\n}\n";
    assert_eq!(format(source), source);
    assert_eq!(format(&format(source)), format(source));
}

#[test]
fn a_type_path_call_round_trips() {
    // `::` binds both halves with no air around it, and the trailing block
    // still attaches the way a dotted call's does.
    let source = "fn f() -> Point {\n    return Point::new(1, 2);\n}\n";
    assert_eq!(format(source), source);
    assert_eq!(format(&format(source)), format(source));
}

#[test]
fn comprehensions_round_trip() {
    // The comprehension's brackets bind like a literal's, and `for`/`in`
    // take ordinary word spacing.
    let source = "fn f() -> []int {\n    let xs = []int {\n        1, 2, 3\n    };\n    return [x * 2 for x in xs if x > 1];\n}\n";
    assert_eq!(format(source), source);
    assert_eq!(format(&format(source)), format(source));
}

#[test]
fn a_pub_method_round_trips_inside_a_type() {
    // `pub` is a modifier word in front of a member, the same shape as in
    // front of an item.
    let source = "object Counter {\n    pub value: int,\n    pub fn get(self) -> int {\n        return self.value;\n    }\n    ,\n}\n";
    assert_eq!(format(source), source);
    assert_eq!(format(&format(source)), format(source));
}

#[test]
fn a_trailing_block_argument_formats_as_a_block() {
    // The trailing block is a lambda in the last argument position: its brace
    // stays with the call, its body indents beneath it, and the closing brace
    // returns to the call's line. Formatting is a fixed point.
    let source = "fn main() {\n    const r = apply(41) { |x| return x + 1; };\n}\n";
    let formatted = format(source);
    assert_eq!(
        formatted,
        "fn main() {\n    const r = apply(41) {\n        |x| return x + 1;\n    };\n}\n"
    );
    assert_eq!(format(&formatted), formatted, "formatting must be idempotent");
}

#[test]
fn a_spread_round_trips() {
    let source = "fn main() {\n    const p = Point { x: 1, ..base };\n    const xs = []int { 0, ..rest, 4 };\n}\n";
    let formatted = format(source);
    assert_eq!(
        formatted,
        "fn main() {\n    const p = Point {\n        x: 1, ..base\n    };\n    const xs = []int {\n        0, ..rest, 4\n    };\n}\n"
    );
    assert_eq!(format(&formatted), formatted, "formatting must be idempotent");
}

#[test]
fn a_comparison_chain_round_trips() {
    let source = "fn main() {\n    const b = 0 <= x < 10;\n}\n";
    let formatted = format(source);
    assert_eq!(formatted, source);
    assert_eq!(format(&formatted), formatted, "formatting must be idempotent");
}

#[test]
fn an_indexed_loop_round_trips() {
    let source = "fn main() {\n    for i, x in xs {\n        print(x);\n    }\n}\n";
    let formatted = format(source);
    assert_eq!(formatted, source);
    assert_eq!(format(&formatted), formatted, "formatting must be idempotent");
}

#[test]
fn an_async_form_round_trips() {
    let source = "async fn fetch(string url) -> string {\n    return url;\n}\n\nfn main() {\n    const body = fetch(url).await;\n    scope {\n        spawn(worker);\n    };\n}\n";
    let formatted = format(source);
    assert_eq!(formatted, source);
    assert_eq!(format(&formatted), formatted, "formatting must be idempotent");
}

#[test]
fn a_doc_comment_round_trips_and_normalizes_its_space() {
    let source = "/// Adds two numbers.\nfn add(int a, int b) -> int {\n    return a + b;\n}\n";
    let formatted = format(source);
    assert_eq!(formatted, source);
    assert_eq!(format(&formatted), formatted, "formatting must be idempotent");
    // `///no space` normalizes to `/// no space`.
    assert_eq!(
        format("///Docs.\nfn f() {}\n"),
        "/// Docs.\nfn f() {}\n"
    );
    // `////` is an ordinary comment and stays as written.
    assert_eq!(
        format("//// Not docs.\nfn f() {}\n"),
        "//// Not docs.\nfn f() {}\n"
    );
}

#[test]
fn a_labeled_loop_round_trips() {
    let source = "fn main() {\n    outer: for i in 0..3 {\n        for j in 0..3 {\n            break outer;\n        }\n    }\n}\n";
    let formatted = format(source);
    assert_eq!(formatted, source);
    assert_eq!(format(&formatted), formatted, "formatting must be idempotent");
}

#[test]
fn a_field_default_round_trips() {
    let source = "struct Config {\n    retries: int = 3,\n    name: string,\n}\n";
    let formatted = format(source);
    assert_eq!(formatted, source);
    assert_eq!(format(&formatted), formatted, "formatting must be idempotent");
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
fn repr_c_binds_to_the_type_name_like_a_call() {
    // `repr(C)` is one clause: `repr` binds to its parenthesis the way a call
    // does, so the printed form has no space before `(`. The type behind it
    // keeps its own space.
    let source = "repr(C) struct P {\n    x: int\n}\n\nrepr(C) enum Kind {\n    A, B\n}\n";
    assert_eq!(format(source), source);
    assert_eq!(format(&format(source)), source, "formatting must be idempotent");
    let spaced = format("repr(C)  struct  P  {\n  x: int\n}\n");
    assert!(
        spaced.contains("repr(C) struct P {"),
        "the clause and its type keep one space between them:\n{spaced}"
    );
}

#[test]
fn if_let_and_while_let_keep_their_keyword_spacing() {
    // `if let` and `while let` are two keywords, not a name: one space between
    // them, one around the `=`, and the block opens on the next line like any
    // other statement block.
    let source = "fn main() {\n    if let Some(x) = opt {\n        print(x);\n    } else {\n        print(0);\n    }\n    while let Some(x) = next() {\n        print(x);\n    }\n}\n";
    assert_eq!(format(source), source);
    assert_eq!(format(&format(source)), source, "formatting must be idempotent");
}

#[test]
fn or_patterns_ranges_and_rest_spacing_round_trip() {
    // `|` in pattern position is the alternative separator and keeps air on
    // both sides; `..` binds tightly and `..=` is one operator.
    let source = "fn main() {\n    switch n {\n        case 1 | 2 | 3 {\n            0\n        }\n        case 0..10 {\n            1\n        }\n        case 'a'..='z' {\n            2\n        }\n        default {\n            4\n        }\n    }\n}\n";
    assert_eq!(format(source), source);
    assert_eq!(format(&format(source)), source, "formatting must be idempotent");
    let struct_pattern = "fn main() { switch n { case Point { x: 1, .. } { 3 } default { 4 } } }";
    let formatted = format(struct_pattern);
    assert_eq!(format(&formatted), formatted, "formatting must be idempotent");
    assert_eq!(tokens_of(struct_pattern), tokens_of(&formatted));
    assert!(
        formatted.contains("x: 1, .."),
        "the rest marker lost its spacing:\n{formatted}"
    );
    let formatted = format(
        "fn main() { switch n { case 1|2 { 0 } case 0 .. = 9 { 1 } default { 2 } } }",
    );
    assert!(
        formatted.contains("1 | 2") && formatted.contains("0..=9"),
        "pattern operators were spaced incorrectly:\n{formatted}"
    );
}

#[test]
fn an_arm_guard_round_trips() {
    // The guard belongs to the arm, after the pattern and before the body
    // brace. The formatter prints tokens, so the one thing to pin is that the
    // `if` keeps the arm's spacing and the body still opens on its own line.
    let source = "fn main() {\n    switch n {\n        case x if x > 0 {\n            1\n        }\n        default {\n            0\n        }\n    }\n}\n";
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
