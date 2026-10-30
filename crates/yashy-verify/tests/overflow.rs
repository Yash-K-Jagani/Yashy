//! Provable integer overflow, end to end.
//!
//! Integer `+`, `-`, `*` and unary `-` are **checked**: an operation that
//! overflows stops the program with a message and a position instead of
//! wrapping to a wrong number. A provable overflow is therefore a certain
//! fault, and the verifier rejects it with `E1201` -- unlike a provably
//! out-of-range index, which is a defined `None` (see `tests/bounds.rs`).
//!
//! The rule is deliberately narrow. It reports only when *every* result the
//! operands can produce is outside `i64`. An operation that may or may not fit
//! is a runtime check that stays, recorded as an unestablished obligation, and
//! one that always fits is an obligation phase 3 may use to delete the check.
//! Division and remainder are not modelled here: the domain does not model
//! them, and their faults have their own rule.
//!
//! Every test is a whole program, like the contract and bounds tests, so the
//! verdicts are the ones a real checker produces.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::check_program;
use yashy_verify::{verify, ObligationKind, ProofObligation, VerifyOptions, VerifyReport};

/// Parse and check `source`, then verify it under `options`.
fn verify_source_with(source: &str, options: &VerifyOptions) -> VerifyReport {
    let mut sources = SourceMap::new();
    let file = sources.add_file("overflow.ya", source.to_string());
    let tokens = Lexer::new(source)
        .tokenize()
        .expect("the fixture's lexer must succeed");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser.parse_program().expect("the fixture must parse");
    match check_program(&program, &sources, file) {
        Ok(hir) => verify(&hir, options),
        Err(diagnostics) => VerifyReport {
            diagnostics,
            obligations: Vec::new(),
            facts: Default::default(),
            effects: Default::default(),
            analyses: 0,
        },
    }
}

fn verify_source(source: &str) -> VerifyReport {
    verify_source_with(source, &VerifyOptions::default())
}

/// The overflow obligations, in walk order.
fn overflows(report: &VerifyReport) -> Vec<&ProofObligation> {
    report
        .obligations
        .iter()
        .filter(|obligation| obligation.kind == ObligationKind::Overflow)
        .collect()
}

/// The codes of the diagnostics, in report order.
fn codes(report: &VerifyReport) -> Vec<&'static str> {
    report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.unwrap_or("<uncoded>"))
        .collect()
}

// -- certain faults are rejected -----------------------------------------

#[test]
fn adding_one_to_the_largest_int_is_rejected() {
    let source = "fn main() { let x: int = 9223372036854775807; let y = x + 1; }";
    let report = verify_source(source);
    assert_eq!(codes(&report), vec!["E1201"]);
    let obligation = overflows(&report);
    assert_eq!(obligation.len(), 1);
    assert!(!obligation[0].established, "an overflow is not proved safe");
}

#[test]
fn adding_two_to_the_largest_int_is_rejected() {
    let source = "fn main() { let x: int = 9223372036854775807; let y = x + 2; }";
    assert_eq!(codes(&verify_source(source)), vec!["E1201"]);
}

#[test]
fn subtracting_past_the_smallest_int_is_rejected() {
    // `-9223372036854775807 - 1` is `i64::MIN`, and one less has no value.
    let source = "fn main() { let x: int = -9223372036854775807 - 1; let y = x - 1; }";
    assert_eq!(codes(&verify_source(source)), vec!["E1201"]);
}

#[test]
fn multiplying_the_largest_int_is_rejected() {
    let source = "fn main() { let x: int = 9223372036854775807; let y = x * 2; }";
    assert_eq!(codes(&verify_source(source)), vec!["E1201"]);
}

#[test]
fn negating_the_smallest_int_is_rejected() {
    // `-i64::MIN` is `i64::MAX + 1`, which no `int` holds.
    let source = "fn main() { let x: int = -9223372036854775807 - 1; let y = -x; }";
    assert_eq!(codes(&verify_source(source)), vec!["E1201"]);
}

// -- arithmetic that fits is silent --------------------------------------

#[test]
fn adding_two_small_ints_is_not_rejected() {
    let report = verify_source("fn main() { let x = 1 + 2; }");
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    let obligation = overflows(&report);
    assert_eq!(obligation.len(), 1);
    assert!(obligation[0].established, "`1 + 2` always fits");
}

#[test]
fn a_loop_index_plus_one_fits() {
    // `i` is `0..3`, so `i + 1` is `1..4`.
    let source = "fn main() { for i in 0..3 { i + 1; } }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    let obligation = overflows(&report);
    assert_eq!(obligation.len(), 1);
    assert!(obligation[0].established);
}

// -- arithmetic the domain cannot decide is recorded, not rejected -------

#[test]
fn arithmetic_on_parameters_is_recorded_not_rejected() {
    let source = "fn f(a: int, b: int) { a + b; } fn main() { }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    let obligation = overflows(&report);
    assert_eq!(obligation.len(), 1);
    assert!(
        !obligation[0].established,
        "the sum of two unknown ints may not fit"
    );
}

#[test]
fn a_sum_that_straddles_the_boundary_is_recorded_not_rejected() {
    // `x` is `i64::MAX`, so `x + y` with unknown `y` may or may not fit.
    let source =
        "fn f(y: int) -> int { let x: int = 9223372036854775807; return x + y; } fn main() { }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert!(!overflows(&report)[0].established);
}

// -- operations that cannot fault are not overflow obligations -----------

#[test]
fn a_string_concatenation_is_not_an_overflow_obligation() {
    // `+` on strings has no runtime fault, so there is no overflow to prove.
    let report = verify_source("fn main() { let s = \"a\" + \"b\"; }");
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert!(overflows(&report).is_empty());
}

#[test]
fn a_float_product_is_not_an_overflow_obligation() {
    // Float arithmetic is not checked, so it has no `FaultSite`.
    let report = verify_source("fn main() { let x = 1.0 * 2.0; }");
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert!(overflows(&report).is_empty());
}

#[test]
fn a_comparison_is_not_an_overflow_obligation() {
    let report = verify_source("fn main() { let x = 1 < 2; }");
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert!(overflows(&report).is_empty());
}

// -- unreachable arithmetic is not recorded ------------------------------

#[test]
fn arithmetic_after_a_return_is_not_recorded() {
    // A fault that never runs is not worth rejecting, and phase 3 must not
    // elide a check in code that cannot execute.
    let source = "fn main() { return; let x: int = 9223372036854775807; let y = x + 1; }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert!(overflows(&report).is_empty());
}
