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

#[test]
fn a_test_program_that_fails_verification_is_a_compile_error() {
    // A leak is a verifier error, so the test never runs. It is a compile
    // failure of that test, not a test that ran and failed: a tool that counts
    // red tests must not count this one, and a reader must not go looking for
    // an assertion that fired.
    let dir = work_dir("test-runner-verify-error");
    write(
        &dir,
        "tests/leak.ya",
        "fn main() {\n    let buf = alloc(16);\n}\n",
    );

    let output = run_test(&dir, &["--json"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(1),
        "a test that fails verification must fail the run:\n{stdout}{stderr}"
    );
    assert!(
        stdout.contains("\"status\": \"compile_error\""),
        "the test's status must be `compile_error`:\n{stdout}"
    );
    assert!(
        stdout.contains("\"compile_errors\": 1"),
        "the summary must count the compile error:\n{stdout}"
    );
    assert!(
        stdout.contains("\"failed\": 0"),
        "a compile error is not a failing test:\n{stdout}"
    );
    assert!(
        stdout.contains("\"code\": \"E1000\""),
        "the diagnostic must carry its code:\n{stdout}"
    );
}

#[test]
fn a_failing_test_still_reports_as_failed_not_compile_error() {
    // The program compiles and verifies; it exits 1 because its assertion is
    // false. That is a failing test, and the distinction is the point.
    let dir = work_dir("test-runner-run-error");
    write(&dir, "tests/fail.ya", "fn main() { assert(false); }\n");

    let output = run_test(&dir, &["--json"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(1),
        "a failing test must fail the run:\n{stdout}{stderr}"
    );
    assert!(
        stdout.contains("\"status\": \"failed\""),
        "the test's status must be `failed`:\n{stdout}"
    );
    assert!(
        !stdout.contains("\"status\": \"compile_error\""),
        "a test that ran is not a compile error:\n{stdout}"
    );
    assert!(
        stdout.contains("\"compile_errors\": 0"),
        "the summary must not count a run failure as a compile error:\n{stdout}"
    );
    assert!(
        stdout.contains("\"failed\": 1"),
        "the summary must count the failing test:\n{stdout}"
    );
}
