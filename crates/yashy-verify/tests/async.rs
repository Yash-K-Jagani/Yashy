//! Structured concurrency and race freedom over async captures, end to end.
//!
//! Every test is a whole program: parse it, check it, verify it, and read the
//! diagnostics. A task that escapes its scope is `E1701`, a lock held across
//! an `await` is `E1702`, and a bare linear capture crossing `spawn` is the
//! phase 2 race rule, `E1402`.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::check_program;
use yashy_verify::{verify, VerifyOptions, VerifyReport};

/// Parse and check `source`, then verify it under the default options.
fn verify_source(source: &str) -> VerifyReport {
    let mut sources = SourceMap::new();
    let file = sources.add_file("async.ya", source.to_string());
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
fn returning_a_spawned_task_handle_is_rejected() {
    // `return t` cannot be spelled after the scope (the checker rejects the
    // name there first), so the observable escape is the handle pushed into
    // a collection that outlives the scope.
    let source = "\
async fn g() -> int { return 1; }
async fn f() -> int {
    let t = g();
    scope {
        t = spawn(g);
    }
    let n = t.await;
    return n;
}
fn main() { print(block_on(f())); }
";
    let report = verify_source(source);
    for d in &report.diagnostics {
        eprintln!("DIAG: code={:?} msg={}", d.code, d.message);
    }
    assert_eq!(codes(&report), vec!["E1701"]);
}

#[test]
fn locking_a_mutex_then_awaiting_is_rejected() {
    let source = "\
async fn g() -> int { return 1; }
async fn f() -> int {
    let m = mutex_new();
    m.lock();
    let n = g().await;
    m.unlock();
    return n;
}
fn main() { print(block_on(f())); }
";
    let report = verify_source(source);
    for d in &report.diagnostics {
        eprintln!("DIAG: code={:?} msg={}", d.code, d.message);
    }
    assert_eq!(codes(&report), vec!["E1702"]);
}

#[test]
fn a_deferred_release_makes_the_await_legal() {
    let source = "\
async fn g() -> int { return 1; }
async fn f() -> int {
    let m = mutex_new();
    m.lock();
    defer { m.unlock(); }
    let n = g().await;
    return n;
}
fn main() { print(block_on(f())); }
";
    let report = verify_source(source);
    for d in &report.diagnostics {
        eprintln!("DIAG: code={:?} msg={}", d.code, d.message);
    }
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
}

#[test]
fn awaiting_inside_the_scope_is_legal() {
    let source = "\
async fn g() -> int { return 1; }
async fn f() -> int {
    let n = 0;
    scope {
        let t = spawn(g);
        n = t.await;
    }
    return n;
}
fn main() { print(block_on(f())); }
";
    let report = verify_source(source);
    for d in &report.diagnostics {
        eprintln!("DIAG: code={:?} msg={}", d.code, d.message);
    }
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
}

#[test]
fn a_bare_pointer_capture_across_spawn_is_rejected() {
    let source = "\
fn bump(*mut int p) -> int { p[0] = p[0] + 1; return p[0]; }
async fn f(*mut int p) -> int {
    let n = 0;
    scope {
        let t = spawn(bump, p);
        n = t.await;
    }
    return n;
}
fn main() { print(0); }
";
    let report = verify_source(source);
    for d in &report.diagnostics {
        eprintln!("DIAG: code={:?} msg={}", d.code, d.message);
    }
    assert!(codes(&report).contains(&"E1402"), "{:?}", codes(&report));
}

#[test]
fn a_mutex_capture_across_spawn_is_accepted() {
    let source = "\
fn worker(Mutex m) -> int { m.lock(); m.unlock(); return 1; }
async fn f(Mutex m) -> int {
    let n = 0;
    scope {
        let t = spawn(worker, m);
        n = t.await;
    }
    return n;
}
fn main() { print(0); }
";
    let report = verify_source(source);
    for d in &report.diagnostics {
        eprintln!("DIAG: code={:?} msg={}", d.code, d.message);
    }
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
}
