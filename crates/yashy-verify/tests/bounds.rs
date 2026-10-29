//! Bounds proof obligations, end to end.
//!
//! Indexing in Yashy never traps: `xs[i]` is an `Option`, and a miss is `None`.
//! The tutorial says so, and `tests/programs/fixed_arrays.ya` demonstrates it
//! with `grid[3]` on a three-element array. A bounds rule therefore cannot be a
//! soundness rule -- there is nothing to be unsound about -- and this pass
//! reports no diagnostic. What it produces is a *proof*: for each index
//! expression, whether `0 <= i < len` holds on every path that reaches it.
//! Phase 3 deletes the runtime check exactly when that proof exists, and keeps
//! it otherwise. An unproved index is not an error; it is a check that stays.
//!
//! Every test is a whole program, like the contract tests, so the obligations
//! are the ones a real checker produces.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::check_program;
use yashy_verify::{verify, ObligationKind, ProofObligation, VerifyOptions, VerifyReport};

/// Parse and check `source`, then verify it under `options`.
fn verify_source_with(source: &str, options: &VerifyOptions) -> VerifyReport {
    let mut sources = SourceMap::new();
    let file = sources.add_file("bounds.ya", source.to_string());
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
            analyses: 0,
        },
    }
}

fn verify_source(source: &str) -> VerifyReport {
    verify_source_with(source, &VerifyOptions::default())
}

/// The bounds obligations, in walk order.
fn bounds(report: &VerifyReport) -> Vec<&ProofObligation> {
    report
        .obligations
        .iter()
        .filter(|obligation| obligation.kind == ObligationKind::Bounds)
        .collect()
}

#[test]
fn an_index_into_a_known_length_array_is_proved() {
    let source = "fn main() { let xs = [1, 2, 3]; xs[1]; }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    let obligations = bounds(&report);
    assert_eq!(obligations.len(), 1, "one index, one obligation");
    assert!(obligations[0].established);
}

#[test]
fn a_constant_index_past_the_end_is_unproved_not_rejected() {
    // `xs[5]` is always `None`, and the language defines that: it is a miss,
    // not a fault. The verifier records that it cannot prove the access is in
    // range; it does not reject the program.
    let source = "fn main() { let xs = [1, 2, 3]; xs[5]; }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    let obligations = bounds(&report);
    assert_eq!(obligations.len(), 1);
    assert!(!obligations[0].established);
}

#[test]
fn a_negative_index_is_unproved_not_rejected() {
    // A negative index is "also a miss, not an error" (tutorial, arrays).
    let source = "fn main() { let xs = [1, 2, 3]; xs[0 - 1]; }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    let obligations = bounds(&report);
    assert_eq!(obligations.len(), 1);
    assert!(!obligations[0].established);
}

#[test]
fn an_index_bounded_by_a_for_loop_is_proved() {
    // The range sets the loop variable to `0..3`, and the array's length is in
    // the domain, so every iteration is in range.
    let source = "fn main() { let xs = [1, 2, 3]; for i in 0..3 { xs[i]; } }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    let obligations = bounds(&report);
    assert_eq!(obligations.len(), 1);
    assert!(obligations[0].established);
}

#[test]
fn a_branch_that_bounds_the_index_proves_it() {
    let source = "fn f(i: int) { let xs = [1, 2, 3]; if i >= 0 && i < 3 { xs[i]; } } fn main() { }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    let obligations = bounds(&report);
    assert_eq!(obligations.len(), 1);
    assert!(obligations[0].established);
}

#[test]
fn a_while_loop_index_the_domain_loses_is_recorded_unproved() {
    // The plan's loop: accepted, because a miss is `None`. The widening in the
    // fixpoint drops the index's range, so the proof is not made -- and the
    // obligation says so instead of pretending.
    let source = "fn main() { let xs = [1, 2, 3]; let i = 0; \
         while i < xs.len() { xs[i]; i = i + 1; } }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    let obligations = bounds(&report);
    assert_eq!(obligations.len(), 1);
    assert!(!obligations[0].established);
}

#[test]
fn an_index_into_a_slice_of_unknown_length_is_recorded_unproved() {
    let source = "fn f(i: int, xs: []int) { xs[i]; } fn main() { }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    let obligations = bounds(&report);
    assert_eq!(obligations.len(), 1);
    assert!(!obligations[0].established);
}

#[test]
fn a_view_is_not_a_bounds_obligation() {
    // A view clamps by definition: `0..99` is the whole array and `2..1` is
    // empty. There is no check to prove, so there is no obligation.
    let source = "fn main() { let xs = [1, 2, 3]; xs[0..99]; }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert!(bounds(&report).is_empty());
}

#[test]
fn a_pointer_index_is_not_a_bounds_obligation() {
    // A pointer carries no length, so there is nothing to compare the index
    // against. The unsafe rules own pointer access, not this pass.
    let source = "fn main() { let buf = alloc(16); buf[0]; buf.free(); }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert!(bounds(&report).is_empty());
}

#[test]
fn an_index_after_a_return_is_not_recorded() {
    // A fact about unreachable code is not worth recording: phase 3 would
    // elide a check that never runs.
    let source = "fn main() { return; let xs = [1, 2, 3]; xs[1]; }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert!(bounds(&report).is_empty());
}
