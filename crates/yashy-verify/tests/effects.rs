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
    let source = "pure fn f() { print(\"x\"); } fn main() { }";
    assert_eq!(codes(&verify_source(source)), vec!["E1300"]);
}

#[test]
fn an_unannotated_caller_of_an_io_function_is_inferred_io() {
    let source = "io fn g() { print(\"x\"); } fn h() { g(); } fn main() { h(); }";
    assert!(verify_source(source).diagnostics.is_empty());
}

#[test]
fn a_pure_function_calling_an_io_function_is_rejected() {
    let source = "io fn g() { print(\"x\"); } pure fn h() { g(); } fn main() { }";
    assert_eq!(codes(&verify_source(source)), vec!["E1300"]);
}

#[test]
fn a_pure_caller_of_a_trait_with_an_io_impl_is_rejected() {
    // The trait call resolves to the one concrete impl, so the effect
    // pass sees that impl's `print` and the `pure` caller breaks E1300.
    let source = "\
trait Show { fn show(self) -> string; }\n\
struct P { n: int }\n\
impl Show for P { fn show(self) -> string { print(self.n); return \"x\"; } }\n\
pure fn f() { let p = P { n: 1 }; p.show(); }\n\
fn main() { }";
    assert_eq!(codes(&verify_source(source)), vec!["E1300"]);
}

#[test]
fn a_pure_trait_with_an_io_impl_is_rejected_at_the_impl() {
    let source = "\
pure trait Show { fn show(self) -> string; }\n\
struct P { n: int }\n\
impl Show for P { fn show(self) -> string { print(self.n); return \"x\"; } }\n\
fn main() { }";
    assert_eq!(codes(&verify_source(source)), vec!["E1300"]);
}

#[test]
fn a_pure_trait_with_a_pure_impl_is_accepted() {
    let source = "\
pure trait Show { fn show(self) -> string; }\n\
struct P { n: int }\n\
impl Show for P { fn show(self) -> string { return \"x\"; } }\n\
fn main() { }";
    assert!(verify_source(source).diagnostics.is_empty());
}

#[test]
fn mutual_recursion_terminates_and_propagates() {
    let source =
        "fn a(int n) { if n > 0 { b(n - 1); } } fn b(int n) { print(\"x\"); a(n); } fn main() { }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty());
    assert!(report.effects["a"].io);
}

// -- the pass's own guarantees -------------------------------------------

#[test]
fn an_unannotated_program_stays_silent() {
    let source = "fn add(int a, int b) -> int { return a + b; } fn main() { let x = add(1, 2); }";
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
    let source = "fn a() { b(); } fn b() { print(\"x\"); } fn main() { a(); }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty());
    assert!(report.effects["a"].io);
    assert!(report.effects["b"].io);
}

#[test]
fn a_loop_carrying_an_io_call_is_io() {
    let source = "fn f(int n) { for i in 0..n { print(\"x\"); } } fn main() { f(3); }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty());
    assert!(report.effects["f"].io);
}

#[test]
fn an_async_function_is_async_effect() {
    let source = "async fn f() -> int { return 1; } fn main() { }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty());
    assert!(report.effects["f"].async_, "the async flag is inferred");
}

#[test]
fn an_async_caller_of_an_async_function_is_accepted() {
    let source =
        "async fn f() -> int { return 1; } async fn g() -> int { return f().await; } fn main() { }";
    let report = verify_source(source);
    assert!(
        report.diagnostics.is_empty(),
        "diagnostics: {:?}",
        report.diagnostics
    );
    assert!(report.effects["g"].async_);
}

#[test]
fn a_sync_caller_of_an_async_function_is_e1300_naming_the_callee() {
    let source = "async fn f() -> int { return 1; } fn g() { f(); } fn main() { }";
    let report = verify_source(source);
    assert_eq!(codes(&report), vec!["E1300"]);
    assert!(
        report.diagnostics.iter().any(|d| d.message.contains("`f`")),
        "the diagnostic names the async callee: {:?}",
        report.diagnostics
    );
}
