//! Typestate, end to end.
//!
//! Every test is a whole program: parse it, check it, verify it, and read the
//! diagnostics. State is opt-in: a program with no resource operations stays
//! silent.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::check_program;
use yashy_verify::{verify, VerifyOptions, VerifyReport};

/// Parse and check `source`, then verify it under the default options.
fn verify_source(source: &str) -> VerifyReport {
    let mut sources = SourceMap::new();
    let file = sources.add_file("typestate.ya", source.to_string());
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
fn closing_a_channel_twice_is_rejected() {
    let source = "fn main() { let c: Channel[int] = chan_new(4); c.close(); c.close(); }";
    let report = verify_source(source);
    for d in &report.diagnostics {
        eprintln!("DIAG: code={:?} msg={}", d.code, d.message);
    }
    assert_eq!(codes(&report), vec!["E1400"]);
}

#[test]
fn using_a_channel_after_close_is_rejected() {
    let source = "fn main() { let c: Channel[int] = chan_new(4); c.close(); c.send(1); }";
    assert!(codes(&verify_source(source)).contains(&"E1401"));
}

#[test]
fn unlocking_a_mutex_that_is_not_locked_is_rejected() {
    let source = "fn main() { let m = mutex_new(); m.unlock(); }";
    let report = verify_source(source);
    for d in &report.diagnostics {
        eprintln!("DIAG: code={:?} msg={}", d.code, d.message);
    }
    assert_eq!(codes(&report), vec!["E1400"]);
}

#[test]
fn lock_then_unlock_is_accepted() {
    assert!(verify_source("fn main() { let m = mutex_new(); m.lock(); m.unlock(); }").diagnostics.is_empty());
}
