//! Phase 10 concurrency harnesses.
//!
//! Each program is built through the CLI and must print its marker and exit 0.

mod common;

use std::path::Path;
use std::process::Command;

fn run(root: &Path, source: &str, name: &str, expected: &str) {
    let paths = root.join("crates/yashy-stdlib/yashy").display().to_string();
    let dir = common::work_dir(name);
    let binary = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    let build = Command::new(common::yashyc())
        .arg("build")
        .arg(root.join(source))
        .arg("-o")
        .arg(&binary)
        .env("YASHY_PATH", &paths)
        .current_dir(root)
        .output()
        .expect("the compiler must run");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let output = Command::new(&binary)
        .output()
        .expect("the program must run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), expected);
}

#[test]
fn parallel_iterator_program_is_checked() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    run(
        &root,
        "crates/yashy-driver/tests/programs/parallel_iter.ya",
        "p10-par-iter",
        "par-iter-ok",
    );
}

#[test]
fn scope_mpmc_and_cancel_program_is_checked() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    run(
        &root,
        "crates/yashy-driver/tests/programs/sync_scope.ya",
        "p10-scope",
        "scope-ok",
    );
}

#[test]
fn locks_program_is_checked() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    run(
        &root,
        "crates/yashy-driver/tests/programs/sync_locks.ya",
        "p10-locks",
        "locks-ok",
    );
}

#[test]
fn atomic_program_is_checked() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    run(
        &root,
        "crates/yashy-driver/tests/programs/atomic_test.ya",
        "p10-atomic",
        "atomic-ok",
    );
}

#[test]
fn non_blocking_select_program_is_checked() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    run(
        &root,
        "crates/yashy-driver/tests/programs/select_try.ya",
        "p10-select-try",
        "select-try-ok",
    );
}
