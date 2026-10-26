//! The diagnostic corpus: every common mistake has a stable code and a span.
//!
//! The code is assigned where the error is built, so this test pins the pair
//! together. It also checks that an uncoded diagnostic is allowed -- the count
//! is what keeps the remaining catalog work visible.

use yashy_common::{codes, Diagnostic, DiagnosticBag, SourceMap, Span};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program_with, CheckOptions};

/// Lex, parse and check `source`, returning the map and any diagnostics.
fn check(source: &str) -> (SourceMap, Vec<Diagnostic>) {
    let mut sm = SourceMap::new();
    let file = sm.add_file("test.ya", source.to_string());
    let tokens = Lexer::new(source)
        .tokenize()
        .expect("lexer should never panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser
        .parse_program()
        .expect("parser should never panic on these inputs");
    let result = check_program_with(&ast, &sm, file, CheckOptions { release: false });
    (sm, result.err().unwrap_or_default())
}

/// Assert that `source` reports `code`, and that the primary span covers
/// `snippet`.
fn assert_code(source: &str, code: &str, snippet: &str) {
    let (sm, diags) = check(source);
    let found = diags.iter().find(|d| d.code == Some(code));
    let diagnostic = found.unwrap_or_else(|| {
        panic!(
            "expected {code}, got {:?}",
            diags
                .iter()
                .map(|d| (d.code, d.message.as_str()))
                .collect::<Vec<_>>()
        )
    });
    assert_ne!(
        diagnostic.span,
        Span::SYNTHETIC,
        "a coded diagnostic needs a real span: {}",
        diagnostic.message
    );
    assert_eq!(
        sm.snippet(diagnostic.file, diagnostic.span),
        snippet,
        "code {code} points at the wrong source"
    );
}

#[test]
fn an_unknown_value_is_e0001() {
    assert_code(
        "fn main() {\n    const total = 3;\n    print(totl);\n}\n",
        codes::UNKNOWN_NAME,
        "totl",
    );
}

#[test]
fn an_unknown_type_is_e0002() {
    assert_code(
        "fn main() {\n    const x: Nope = 1;\n}\n",
        codes::UNKNOWN_TYPE,
        "Nope",
    );
}

#[test]
fn a_scalar_mismatch_is_e0100() {
    assert_code(
        "fn main() {\n    const x: int = \"three\";\n}\n",
        codes::TYPE_MISMATCH,
        "\"three\"",
    );
}

#[test]
fn an_unknown_field_is_e0200() {
    let (sm, diags) = check(
        "struct Point { x: int, y: int }\n\
         fn main() {\n\
             const p = Point { x: 1, y: 2 };\n\
             print(p.z);\n\
         }\n",
    );
    let diagnostic = diags
        .iter()
        .find(|d| d.code == Some(codes::UNKNOWN_FIELD))
        .expect("an unknown field needs E0200");
    assert!(
        sm.snippet(diagnostic.file, diagnostic.span).contains('z'),
        "the span should cover the unknown field: {:?}",
        sm.snippet(diagnostic.file, diagnostic.span)
    );
}

#[test]
fn a_missing_field_is_e0201() {
    let (sm, diags) = check(
        "struct Point { x: int, y: int }\n\
         fn main() {\n\
             const p = Point { x: 1 };\n\
         }\n",
    );
    let diagnostic = diags
        .iter()
        .find(|d| d.code == Some(codes::MISSING_FIELD))
        .expect("a missing field needs E0201");
    assert!(
        sm.snippet(diagnostic.file, diagnostic.span)
            .contains("Point"),
        "the span should cover the struct literal"
    );
}

#[test]
fn a_wrong_argument_count_is_e0700() {
    assert_code(
        "fn add(a: int, b: int) -> int { return a + b; }\n\
         fn main() {\n\
             print(add(1));\n\
         }\n",
        codes::WRONG_ARGUMENT_COUNT,
        "add(1)",
    );
}

#[test]
fn a_second_use_of_a_linear_value_is_e0500() {
    assert_code(
        "fn main() {\n    let buf = alloc(16);\n    buf.free();\n    buf.free();\n}\n",
        codes::MOVED_VALUE,
        "buf.free()",
    );
}

#[test]
fn assigning_a_const_is_e0400() {
    assert_code(
        "fn main() {\n    const x = 1;\n    x = 2;\n}\n",
        codes::CONST_REASSIGNMENT,
        "x",
    );
}

#[test]
fn a_missing_switch_case_is_e0600() {
    let (sm, diags) = check(
        "enum Color { Red, Green }\n\
         fn code(c: Color) -> int {\n\
             return switch c {\n\
                 case Red { 1 }\n\
             };\n\
         }\n\
         fn main() { }\n",
    );
    let diagnostic = diags
        .iter()
        .find(|d| d.code == Some(codes::NON_EXHAUSTIVE_MATCH))
        .expect("a missing case needs E0600");
    assert!(
        sm.snippet(diagnostic.file, diagnostic.span)
            .contains("switch"),
        "the span should cover the switch"
    );
}

#[test]
fn a_result_used_as_its_payload_is_e0300() {
    assert_code(
        "fn parse() -> Result[int, string] { return Ok(7); }\n\
         fn main() {\n\
             const n: int = parse();\n\
         }\n",
        codes::MISSING_QUESTION,
        "parse()",
    );
}

#[test]
fn a_diagnostic_without_a_code_is_allowed_and_counted() {
    let (_, diags) = check("fn main() {\n    const x = 1;\n    const x = 2;\n}\n");
    assert!(
        !diags.is_empty(),
        "a duplicate definition is still an error"
    );
    let mut bag = DiagnosticBag::new();
    for diagnostic in diags {
        bag.push(diagnostic);
    }
    assert!(
        bag.uncoded_count() >= 1,
        "the duplicate definition has no R1 code, so it must count as uncoded"
    );
}
