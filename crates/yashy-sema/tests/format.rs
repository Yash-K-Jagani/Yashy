//! The format engine's compile-time half: a literal specifier is parsed and
//! checked when the program is checked, not when it runs.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program_with, CheckOptions};

fn check(src: &str) -> Result<yashy_sema::HirProgram, Vec<Diagnostic>> {
    let mut sm = SourceMap::new();
    let file = sm.add_file("test.ya", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("lexes");
    let ast = Parser::new(tokens, &sm, file)
        .parse_program()
        .expect("parses");
    check_program_with(&ast, &sm, file, CheckOptions { release: false })
}

fn error(src: &str, needle: &str) {
    let diags = check(src).expect_err("expected a format diagnostic");
    assert!(
        diags.iter().any(|d| d.message.contains(needle)),
        "expected a diagnostic containing {needle:?}, got: {:?}",
        diags.iter().map(|d| d.message.clone()).collect::<Vec<_>>()
    );
}

#[test]
fn format_produces_a_string() {
    // Returning the result from a `-> string` function makes the checker prove
    // the conversion; an `int` result would be a type mismatch.
    check("fn f() -> string { return format(\"{:>8.2}\", 3.14159); } fn main() {}")
        .expect("a formatted value is a string");
}

#[test]
fn format_accepts_text_fields_and_escapes() {
    check("fn main() { const s = format(\"{{}} {:<4} {}\", \"a\", true); }")
        .expect("text, alignment and auto fields check");
}

#[test]
fn format_reports_a_malformed_specifier() {
    error(
        "fn main() { const s = format(\"{:z}\", 1); }",
        "invalid format specifier",
    );
}

#[test]
fn format_reports_an_argument_count_mismatch() {
    error(
        "fn main() { const s = format(\"{} {}\", 1); }",
        "expects 2 arguments",
    );
}

#[test]
fn format_reports_a_type_mismatch() {
    error(
        "fn main() { const s = format(\"{:d}\", \"text\"); }",
        "needs an int",
    );
}

#[test]
fn format_requires_a_literal_specifier() {
    error(
        "fn main() { let string spec = \"{}\"; const s = format(spec, 1); }",
        "must be a string literal",
    );
}
