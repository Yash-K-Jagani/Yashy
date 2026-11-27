//! Benchmark dashboard: history keyed by compiler, target and toolchain, a
//! Markdown rendering with baseline and delta, and `--check` that fails a row
//! which regressed past its budget. A first row is "new", never "green".

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn history(dir: &Path, rows: &str) {
    fs::write(
        dir.join("history.json"),
        format!("{{\"schema_version\":1,\"rows\":{rows}}}"),
    )
    .unwrap();
}

fn run(dir: &Path, args: &[&str]) -> (bool, String, String) {
    let output = Command::new(common::yashyc())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("the CLI must run");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn workspace(name: &str) -> PathBuf {
    common::work_dir(name)
}

#[test]
fn a_new_row_is_reported_but_not_a_failure() {
    let dir = workspace("dashboard-new");
    history(
        &dir,
        "[{\"compiler\":\"c1\",\"target\":\"x86_64\",\"toolchain\":\"clang\",\"key\":\"fib\",\"value\":100,\"budget_pct\":5}]",
    );
    let (ok, stdout, stderr) = run(
        &dir,
        &["bench", "dashboard", "--history", "history.json", "--check"],
    );
    assert!(ok, "{stdout} {stderr}");
    assert!(stdout.contains("| fib |"), "{stdout}");
    assert!(stdout.contains("new"), "{stdout}");
}

#[test]
fn a_regression_fails_the_check() {
    let dir = workspace("dashboard-regression");
    history(
        &dir,
        "[{\"compiler\":\"c1\",\"target\":\"x86_64\",\"toolchain\":\"clang\",\"key\":\"fib\",\"value\":100,\"budget_pct\":5},{\"compiler\":\"c1\",\"target\":\"x86_64\",\"toolchain\":\"clang\",\"key\":\"fib\",\"value\":110,\"budget_pct\":5}]",
    );
    let (ok, stdout, stderr) = run(
        &dir,
        &["bench", "dashboard", "--history", "history.json", "--check"],
    );
    assert!(!ok, "a 10% regression past 5% must fail: {stdout} {stderr}");
    assert!(stdout.contains("regressed"), "{stdout}");
}

#[test]
fn json_carries_the_same_rows() {
    let dir = workspace("dashboard-json");
    history(
        &dir,
        "[{\"compiler\":\"c1\",\"target\":\"x86_64\",\"toolchain\":\"clang\",\"key\":\"fib\",\"value\":100,\"budget_pct\":5}]",
    );
    let (ok, stdout, stderr) = run(
        &dir,
        &["bench", "dashboard", "--history", "history.json", "--json"],
    );
    assert!(ok, "{stderr}");
    assert!(stdout.contains("\"rows\""), "{stdout}");
    assert!(stdout.contains("\"fib\""), "{stdout}");
}
