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
fn closing_a_channel_twice_is_rejected() {
    let source = "fn main() { let Channel[int] c = chan_new(4); c.close(); c.close(); }";
    let report = verify_source(source);
    for d in &report.diagnostics {
        eprintln!("DIAG: code={:?} msg={}", d.code, d.message);
    }
    assert_eq!(codes(&report), vec!["E1400"]);
}

#[test]
fn using_a_channel_after_close_is_rejected() {
    let source = "fn main() { let Channel[int] c = chan_new(4); c.close(); c.send(1); }";
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

const SOCKET_HANDLE: &str = "\
struct Listener { fd: int }\n\
struct Socket { fd: int }\n\
enum Error { Failure }\n\
unsafe fn net_listen(string host, int port) -> Result[Listener, Error] { const fd = 0; return Ok(Listener { fd: fd }); }\n\
unsafe fn net_connect(string host, int port) -> Result[Socket, Error] { const fd = 0; return Ok(Socket { fd: fd }); }\n\
unsafe fn net_read(Socket peer) -> Result[Option[string], Error] { const Option[string] nothing = None; return Ok(nothing); }\n\
unsafe fn net_close(Socket peer) {}\n\
unsafe fn net_close_listener(Listener listener) {}\n";

#[test]
fn closing_a_listener_twice_is_e1400() {
    let source = format!(
        "{SOCKET_HANDLE}\
fn fail() -> Result[int, Error] {{ const listener = net_listen(\"127.0.0.1\", 19179)?; net_close_listener(listener); net_close_listener(listener); return Ok(0); }}\n\
fn main() {{ }}"
    );
    assert!(codes(&verify_source(&source)).contains(&"E1400"), "{:?}", codes(&verify_source(&source)));
}

#[test]
fn using_a_socket_after_close_is_e1401() {
    let source = format!(
        "{SOCKET_HANDLE}\
fn fail() -> Result[int, Error] {{ const socket = net_connect(\"127.0.0.1\", 19180)?; net_close(socket); const data = net_read(socket)?; print(data); return Ok(0); }}\n\
fn main() {{ }}"
    );
    assert!(codes(&verify_source(&source)).contains(&"E1401"), "{:?}", codes(&verify_source(&source)));
}

const ITERATOR: &str = "\
trait Iterator { fn next(self: *mut Self) -> Option[int]; }\n\
struct Counter { n: int }\n\
impl Iterator for Counter { fn next(self: *mut Self) -> Option[int] { if self[0].n <= 0 { return None; } self[0].n = self[0].n - 1; return Some(self[0].n); } }\n";

#[test]
fn next_after_none_on_the_same_path_is_rejected() {
    let source = format!(
        "{ITERATOR}\
fn main() {{ let c = Counter {{ n: 1 }}; if let None = c.next() {{ if let Some(z) = c.next() {{ print(z); }} }} }}"
    );
    assert!(codes(&verify_source(&source)).contains(&"E1401"), "{:?}", codes(&verify_source(&source)));
}

#[test]
fn next_after_none_on_a_different_branch_is_not_rejected() {
    let source = format!(
        "{ITERATOR}\
fn main() {{ let c = Counter {{ n: 1 }}; if let Some(x) = c.next() {{ if let Some(z) = c.next() {{ print(z); }} }} }}"
    );
    let report = verify_source(&source);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
}

#[test]
fn next_after_a_none_only_path_is_rejected_after_the_if_let() {
    // The `Some` path returns, so only the `None` (else) path falls
    // through: the iterator is definitely closed at the join.
    let source = format!(
        "{ITERATOR}\
fn use_it(Counter c) -> int {{ let d = c; if let Some(x) = d.next() {{ return x; }} else {{ }} if let Some(z) = d.next() {{ return z; }} return 0; }}\n\
fn main() {{ let c = Counter {{ n: 1 }}; use_it(c); }}"
    );
    assert!(codes(&verify_source(&source)).contains(&"E1401"), "{:?}", verify_source(&source).diagnostics);
}
