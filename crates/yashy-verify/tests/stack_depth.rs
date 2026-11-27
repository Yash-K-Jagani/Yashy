//! Recursion-depth accounting, end to end.
//!
//! A cycle of calls that reserves `stack_alloc` bytes is refused with E1202;
//! a recursion with no reservation is not. Every test is a whole program:
//! parse, check, verify, and read the diagnostics from the same pipeline a
//! build uses.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::check_program;
use yashy_verify::{verify, VerifyOptions, VerifyReport};

fn verify_source(source: &str) -> VerifyReport {
    let mut sources = SourceMap::new();
    let file = sources.add_file("stack_depth.ya", source.to_string());
    let tokens = Lexer::new(source)
        .tokenize()
        .expect("the fixture's lexer must succeed");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser.parse_program().expect("the fixture must parse");
    match check_program(&program, &sources, file) {
        Ok(hir) => verify(&hir, &VerifyOptions::default()),
        Err(diagnostics) => VerifyReport {
            diagnostics,
            obligations: Vec::new(),
            facts: Default::default(),
            effects: Default::default(),
            analyses: 0,
        },
    }
}

fn codes(report: &VerifyReport) -> Vec<&'static str> {
    report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.unwrap_or("<uncoded>"))
        .collect()
}

#[test]
fn direct_recursion_with_a_stack_reservation_is_rejected() {
    let source = "
fn descend(int n) {
    let *mut int scratch = stack_alloc(16);
    if n > 0 {
        descend(n - 1);
    }
}
fn main() { descend(3); }
";
    let report = verify_source(source);
    assert!(
        codes(&report).contains(&"E1202"),
        "expected E1202, got {:?}: {}",
        codes(&report),
        report
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
            .join("; ")
    );
}

#[test]
fn mutual_recursion_with_a_reservation_is_rejected() {
    let source = "
fn ping(int n) {
    let *mut int scratch = stack_alloc(8);
    if n > 0 { pong(n - 1); }
}
fn pong(int n) {
    if n > 0 { ping(n - 1); }
}
fn main() { ping(3); }
";
    let report = verify_source(source);
    assert!(
        codes(&report).contains(&"E1202"),
        "got {:?}",
        codes(&report)
    );
}

#[test]
fn recursion_without_a_reservation_is_allowed() {
    let source = "
fn count(int n) -> int {
    if n > 0 { return count(n - 1); }
    return 0;
}
fn main() { print(count(3)); }
";
    let report = verify_source(source);
    assert!(
        !codes(&report).contains(&"E1202"),
        "got {:?}",
        codes(&report)
    );
}

#[test]
fn a_non_recursive_reservation_is_allowed() {
    let source = "
fn work() {
    let *mut int scratch = stack_alloc(32);
}
fn main() { work(); }
";
    let report = verify_source(source);
    assert!(
        !codes(&report).contains(&"E1202"),
        "got {:?}",
        codes(&report)
    );
}
