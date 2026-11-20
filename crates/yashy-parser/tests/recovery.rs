//! Half-typed source must still produce diagnostics with stable spans.
//!
//! Recovery is for the editor, not for `build`: a program that fails to parse
//! is never compiled. What matters here is that the parser always terminates,
//! names a concrete span, and keeps whatever items it managed to read.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::Parser;

fn parse(src: &str) -> Vec<yashy_common::Diagnostic> {
    let mut map = SourceMap::new();
    let file = map.add_file("broken.ya".to_string(), src.to_string());
    let tokens = match Lexer::with_file(src, file).tokenize() {
        Ok(tokens) => tokens,
        Err(diagnostics) => return diagnostics,
    };
    match Parser::new(tokens, &map, file).parse_program() {
        Ok(_) => Vec::new(),
        Err(diagnostics) => diagnostics,
    }
}

#[test]
fn a_bare_fn_keyword_reports_a_diagnostic_at_a_stable_span() {
    for source in ["fn", "fn ", "fn\n"] {
        let first = parse(source);
        assert!(!first.is_empty(), "`{source:?}` must be diagnosed");
        let second = parse(source);
        assert_eq!(
            first
                .iter()
                .map(|d| (d.span.start, d.message.clone()))
                .collect::<Vec<_>>(),
            second
                .iter()
                .map(|d| (d.span.start, d.message.clone()))
                .collect::<Vec<_>>(),
            "the same input must give the same spans and messages"
        );
    }
}

#[test]
fn an_unclosed_signature_and_an_unclosed_string_are_diagnosed() {
    for source in ["fn main(", "fn main() {", "fn main() { print(\"oops); }"] {
        assert!(
            !parse(source).is_empty(),
            "`{source:?}` must produce at least one diagnostic"
        );
    }
}

#[test]
fn a_missing_brace_reports_a_diagnostic_with_a_stable_span() {
    // A recovered parse is not a clean parse: the LSP still gets diagnostics
    // with spans even though the tree is incomplete. `build` never uses it.
    let source = "fn good() {}\nfn broken() {\nfn later() {}\n";
    let first = parse(source);
    assert!(!first.is_empty(), "an unclosed body must be diagnosed");
    let second = parse(source);
    assert_eq!(
        first
            .iter()
            .map(|diagnostic| (diagnostic.span.start, diagnostic.message.clone()))
            .collect::<Vec<_>>(),
        second
            .iter()
            .map(|diagnostic| (diagnostic.span.start, diagnostic.message.clone()))
            .collect::<Vec<_>>(),
        "recovery must be deterministic"
    );
    assert!(
        first
            .iter()
            .all(|diagnostic| diagnostic.span.start as usize <= source.len()),
        "every diagnostic must name a span inside the file"
    );
}

#[test]
fn diagnostics_are_capped_per_file() {
    // A long run of broken items must not produce one diagnostic per token.
    let source = "fn (\n".repeat(200);
    let diagnostics = parse(&source);
    assert!(!diagnostics.is_empty());
    // Recovery emits at most one diagnostic per broken item; with 200 items
    // there can be no more than 200, never one per token in the file.
    assert!(
        diagnostics.len() <= 200,
        "recovery must cap diagnostics, got {}",
        diagnostics.len()
    );
}
