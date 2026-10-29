//! Contract discharge, end to end.
//!
//! Every test is a whole program: parse it, check it, verify it, and read the
//! diagnostics. A contract is only worth what a program can prove, so the tests
//! state programs rather than build HIR by hand.

use yashy_common::{Severity, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::check_program;
use yashy_verify::{verify, ContractMode, VerifyOptions, VerifyReport};

/// Parse and check `source`, then verify it under `options`.
fn verify_source_with(source: &str, options: &VerifyOptions) -> VerifyReport {
    let mut sources = SourceMap::new();
    let file = sources.add_file("contracts.ya", source.to_string());
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
        },
    }
}

fn verify_source(source: &str) -> VerifyReport {
    verify_source_with(source, &VerifyOptions::default())
}

fn codes(report: &VerifyReport) -> Vec<&'static str> {
    report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.unwrap_or("<uncoded>"))
        .collect()
}

// -- the plan's three tests ----------------------------------------------

#[test]
fn a_provable_precondition_is_silent() {
    let source = "fn f(n: int) -> int requires n > 0 ensures result > 0 { return n; } fn main() { let x = f(1); }";
    assert!(verify_source(source).diagnostics.is_empty());
}

#[test]
fn an_unprovable_precondition_warns_and_names_the_fact() {
    let source = "fn f(n: int) -> int requires n > 0 { return n; } fn main() { let x = f(-1); }";
    let report = verify_source(source);
    assert_eq!(codes(&report), vec!["E1100"]);
    assert_eq!(report.diagnostics[0].severity, Severity::Warning);
    assert!(report.diagnostics[0].message.contains("n > 0"));
}

#[test]
fn a_contract_naming_an_unknown_identifier_is_a_type_error_not_a_proof_failure() {
    let source = "fn f(n: int) -> int requires m > 0 { return n; } fn main() { }";
    assert_eq!(codes(&verify_source(source)), vec!["E0001"]);
}

// -- the proof object ----------------------------------------------------

#[test]
fn a_failed_clause_says_which_obligation_and_which_fact() {
    let source = "fn f(n: int) -> int requires n > 0 { return n; } fn main() { let x = f(-1); }";
    let report = verify_source(source);
    let proof = report.diagnostics[0]
        .proof
        .as_ref()
        .expect("an undischarged contract carries a proof trace");
    assert_eq!(proof.obligation, "requires");
    assert_eq!(proof.missing_fact, "n > 0");
    assert!(!proof.remedy.is_empty());
}

#[test]
fn enforce_mode_makes_the_same_clause_an_error() {
    let source = "fn f(n: int) -> int requires n > 0 { return n; } fn main() { let x = f(-1); }";
    let report = verify_source_with(
        source,
        &VerifyOptions {
            contracts: ContractMode::Enforce,
        },
    );
    assert_eq!(codes(&report), vec!["E1100"]);
    assert_eq!(report.diagnostics[0].severity, Severity::Error);
    assert_eq!(report.error_count(), 1);
}

// -- postconditions ------------------------------------------------------

#[test]
fn a_postcondition_the_return_does_not_establish_is_reported() {
    let source = "fn f(n: int) -> int requires n > 0 ensures result > 10 { return n; } \
                  fn main() { let x = f(100); }";
    let report = verify_source(source);
    assert_eq!(codes(&report), vec!["E1100"]);
    assert_eq!(
        report.diagnostics[0].proof.as_ref().unwrap().obligation,
        "ensures"
    );
    assert!(report.diagnostics[0].message.contains("result > 10"));
}

#[test]
fn a_postcondition_following_from_the_precondition_is_silent() {
    let source = "fn f(n: int) -> int requires n >= 0 ensures result >= 0 { return n; } \
                  fn main() { let x = f(0); }";
    assert!(verify_source(source).diagnostics.is_empty());
}

#[test]
fn every_return_is_checked_against_the_postcondition() {
    // The first return is fine; the second is not, and one bad path is enough
    // to break the promise.
    let source = "fn f(n: int) -> int requires n >= 0 ensures result >= 0 { \
                      if n > 0 { return n; } \
                      return 0 - 1; \
                  } \
                  fn main() { let x = f(1); }";
    let report = verify_source(source);
    assert_eq!(codes(&report), vec!["E1100"]);
    assert_eq!(
        report.diagnostics[0].proof.as_ref().unwrap().obligation,
        "ensures"
    );
}

// -- loop invariants -----------------------------------------------------

#[test]
fn an_invariant_established_before_the_loop_and_kept_by_the_body_is_silent() {
    let source = "fn main() { let n = 3; for i in 0..n invariant i >= 0 { let y = i; } }";
    assert!(verify_source(source).diagnostics.is_empty());
}

#[test]
fn an_invariant_that_does_not_hold_at_the_first_iteration_is_reported() {
    let source = "fn main() { let n = 3; for i in 0..n invariant i > 0 { let y = i; } }";
    let report = verify_source(source);
    assert_eq!(codes(&report), vec!["E1100"]);
    let proof = report.diagnostics[0].proof.as_ref().unwrap();
    assert_eq!(proof.obligation, "invariant");
    assert_eq!(proof.missing_fact, "i > 0");
}

#[test]
fn an_invariant_the_body_breaks_is_reported_at_the_back_edge() {
    // `i` is in `[0, 2]` at the head and the body assigns `i + 10`, so the
    // invariant holds on entry and fails when the next iteration is reached.
    let source = "fn main() { let i = 0; while i < 3 invariant i < 5 { i = i + 10; } }";
    let report = verify_source(source);
    assert_eq!(codes(&report), vec!["E1100"]);
    assert!(report.diagnostics[0]
        .message
        .contains("at the end of an iteration"));
}

// -- calls inside expressions --------------------------------------------

#[test]
fn a_precondition_is_checked_when_the_call_is_an_argument() {
    let source = "fn positive(n: int) -> int requires n > 0 { return n; } \
                  fn twice(n: int) -> int { return n + n; } \
                  fn main() { let x = twice(positive(-1)); }";
    assert_eq!(codes(&verify_source(source)), vec!["E1100"]);
}

// -- lengths -------------------------------------------------------------

#[test]
fn a_precondition_about_a_literal_length_is_proved() {
    let source = "fn first(xs: []int) -> int requires xs.len() > 0 { return 0; } \
                  fn main() { let y = first([]int { 1, 2 }); }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty(), "{:#?}", report.diagnostics);
}

#[test]
fn a_precondition_about_a_length_the_literal_does_not_satisfy_is_reported() {
    let source = "fn long(xs: []int) -> int requires xs.len() > 5 { return 0; } \
                  fn main() { let y = long([]int { 1, 2 }); }";
    let report = verify_source(source);
    assert_eq!(codes(&report), vec!["E1100"], "{:#?}", report.diagnostics);
    assert!(report.diagnostics[0].message.contains("xs.len() > 5"));
}

// -- what the domain does not know ---------------------------------------

#[test]
fn a_fact_about_a_call_result_is_not_assumed() {
    // `id(1)` is a call, and the domain does not model what a call returns, so
    // the argument is unknown and the clause is unproved.
    let source = "fn id(n: int) -> int { return n; } \
                  fn positive(n: int) -> int requires n > 0 { return n; } \
                  fn main() { let x = positive(id(1)); }";
    assert_eq!(codes(&verify_source(source)), vec!["E1100"]);
}

#[test]
fn a_guard_before_the_call_narrows_the_argument() {
    let source = "fn positive(n: int) -> int requires n > 0 { return n; } \
                  fn main() { let n = -1; if n > 0 { let x = positive(n); } }";
    // The binding is a literal `-1`, so the branch is not taken and the call is
    // unreachable; either way no clause is left unproved.
    assert!(verify_source(source).diagnostics.is_empty());
}

#[test]
fn a_parameter_narrowed_by_a_branch_is_provable() {
    let source = "fn positive(n: int) -> int requires n > 0 { return n; } \
                  fn g(n: int) { if n > 0 { let x = positive(n); } } \
                  fn main() { let y = g(1); }";
    assert!(verify_source(source).diagnostics.is_empty());
}

// -- regression: a program with no contract has nothing to prove ----------

#[test]
fn a_program_without_contracts_has_no_contract_diagnostics() {
    assert!(verify_source("fn main() { let x = 1 + 2; }")
        .diagnostics
        .is_empty());
}
