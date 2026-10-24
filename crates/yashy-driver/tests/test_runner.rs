mod common;

use common::{work_dir, write, yashy};
use std::process::Command;

fn run_test(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(yashy())
        .arg("test")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("the yashy CLI must be runnable")
}

#[test]
fn test_runs_passing_and_failing_files() {
    let dir = work_dir("test-runner");
    write(&dir, "tests/pass.ya", "fn main() { assert(1 + 1 == 2); }\n");
    write(
        &dir,
        "tests/fail.ya",
        "fn main() { panic(\"intentional failure\"); }\n",
    );

    let output = run_test(&dir, &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success(), "a failing test must fail the run");
    assert!(stdout.contains("pass.ya"), "stdout:\n{stdout}");
    assert!(stdout.contains("fail.ya"), "stdout:\n{stdout}");
    assert!(
        stdout.contains("1 passed") && stdout.contains("1 failed"),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stderr.contains("intentional failure"), "stderr:\n{stderr}");
}

#[test]
fn test_filter_runs_one_file() {
    let dir = work_dir("test-filter");
    write(&dir, "tests/pass.ya", "fn main() { assert(1 + 1 == 2); }\n");
    write(
        &dir,
        "tests/fail.ya",
        "fn main() { panic(\"intentional failure\"); }\n",
    );

    let output = run_test(&dir, &["--filter", "pass"]);
    assert!(
        output.status.success(),
        "filtered run must pass:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_with_no_files_is_a_clean_success() {
    let dir = work_dir("test-empty");
    let output = run_test(&dir, &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("no tests found"), "stdout:\n{stdout}");
}

#[test]
fn test_missing_explicit_path_is_an_error() {
    let dir = work_dir("test-missing");
    let output = run_test(&dir, &["tests/missing"]);

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("no tests at"),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_list_prints_discovered_files() {
    let dir = work_dir("test-list");
    write(&dir, "tests/smoke.ya", "fn main() { assert(true); }\n");

    let output = run_test(&dir, &["--list"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("tests/smoke.ya"), "stdout:\n{stdout}");
}
