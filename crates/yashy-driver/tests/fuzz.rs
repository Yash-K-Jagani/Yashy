//! Bounded, deterministic wire-input corpus for the HTTP parsers.
//!
//! The corpus lives in `packages/http/tests/fuzz.ya`: every row is a byte
//! string a peer could send, and the program must finish with a zero leak
//! report. A panic, a hang or an unbounded allocation fails this test.

mod common;

use std::process::Command;

#[test]
fn http_parser_corpus_is_fault_free() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let paths = format!(
        "{}:{}:{}",
        root.join("crates/yashy-stdlib/yashy").display(),
        root.join("packages/net/src").display(),
        root.join("packages/http/src").display()
    );
    let dir = common::work_dir("http-fuzz");
    let binary = dir.join(format!("httpfuzz{}", std::env::consts::EXE_SUFFIX));
    let build = Command::new(common::yashyc())
        .arg("build")
        .arg(root.join("packages/http/tests/fuzz.ya"))
        .arg("-o")
        .arg(&binary)
        .env("YASHY_PATH", &paths)
        .current_dir(&root)
        .output()
        .expect("the compiler must run");
    assert!(
        build.status.success(),
        "building the fuzz corpus failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&binary)
        .env("YASHY_LEAK_REPORT", "1")
        .output()
        .expect("the fuzz corpus must run");
    assert!(
        run.status.success(),
        "the fuzz corpus exited with {:?}:\n{}",
        run.status.code(),
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert_eq!(stdout, "fuzz-ok\n", "unexpected output: {stdout}");
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("yashy: leak report: 0 live allocations"),
        "the fuzz corpus leaked:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
}
