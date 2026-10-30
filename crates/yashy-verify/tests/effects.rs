//! Effect inference, end to end.
//!
//! Every test is a whole program: parse it, check it, verify it, and read the
//! diagnostics plus the inferred effect map. Effects are opt-in: a program with
//! no annotations must compile exactly as it does today.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::check_program;
use yashy_verify::{verify, VerifyOptions, VerifyReport};

/// Parse and check `source`, then verify it under the default options.
fn verify_source(source: &str) -> VerifyReport {
    let mut sources = SourceMap::new();
    let file = sources.add_file("effects.ya", source.to_string());
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

// -- the plan's four tests -----------------------------------------------

#[test]
fn a_pure_function_that_prints_is_rejected() {
    let source = "pure fn f() { println(\"x\"); } fn main() { }";
    assert_eq!(codes(&verify_source(source)), vec!["E1300"]);
}

#[test]
fn an_unannotated_caller_of_an_io_function_is_inferred_io() {
    let source = "io fn g() { println(\"x\"); } fn h() { g(); } fn main() { h(); }";
    assert!(verify_source(source).diagnostics.is_empty());
}

#[test]
fn a_pure_function_calling_an_io_function_is_rejected() {
    let source = "io fn g() { println(\"x\"); } pure fn h() { g(); } fn main() { }";
    assert_eq!(codes(&verify_source(source)), vec!["E1300"]);
}

#[test]
fn mutual_recursion_terminates_and_propagates() {
    let source = "fn a(n: int) { if n > 0 { b(n - 1); } } fn b(n: int) { println(\"x\"); a(n); } fn main() { }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty());
    assert!(report.effects["a"].io);
}

// -- the pass's own guarantees -------------------------------------------

#[test]
fn an_unannotated_program_stays_silent() {
    let source = "fn add(a: int, b: int) -> int { return a + b; } fn main() { let x = add(1, 2); }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty());
    assert!(!report.effects["add"].io);
}

#[test]
fn an_unannotated_function_may_make_an_unsafe_call() {
    // `unsafe free(p)` is the language's ordinary spelling for a release. The
    // marker at the call site is the acknowledgement, so an unannotated
    // function is not a violation: it simply becomes unsafe-effect.
    let source = "extern \"C\" { unsafe fn c_fn(); } fn f() { unsafe c_fn(); } fn main() { }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty());
    assert!(report.effects["f"].unsafe_);
}

#[test]
fn a_declared_function_may_not_make_an_unsafe_call() {
    let source = "extern \"C\" { unsafe fn c_fn(); } io fn f() { unsafe c_fn(); } fn main() { }";
    assert_eq!(codes(&verify_source(source)), vec!["E1301"]);
}

#[test]
fn a_pure_function_that_calls_unsafe_reports_only_e1301() {
    // The call site is the mistake; `E1300` would state it twice.
    let source = "extern \"C\" { unsafe fn c_fn(); } pure fn f() { unsafe c_fn(); } fn main() { }";
    assert_eq!(codes(&verify_source(source)), vec!["E1301"]);
}

#[test]
fn an_unsafe_fn_may_call_an_extern_function() {
    let source = "extern \"C\" { unsafe fn c_fn(); } unsafe fn f() { c_fn(); } fn main() { }";
    assert!(verify_source(source).diagnostics.is_empty());
}

#[test]
fn io_propagates_through_two_hops() {
    let source = "fn a() { b(); } fn b() { println(\"x\"); } fn main() { a(); }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty());
    assert!(report.effects["a"].io);
    assert!(report.effects["b"].io);
}

#[test]
fn a_loop_carrying_an_io_call_is_io() {
    let source = "fn f(n: int) { for i in 0..n { println(\"x\"); } } fn main() { f(3); }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty());
    assert!(report.effects["f"].io);
}
