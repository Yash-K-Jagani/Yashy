//! The one-harness test runner: every test is linked once, and each test still
//! runs in its own process.
//!
//! The contract these tests hold the runner to:
//!
//! * every discovered test is named, in the human report and in `--json`;
//! * a test that returns a non-zero status keeps that status in the report, so
//!   a failure is attributable instead of just "red";
//! * a test that prints owns its output, so a passing test's stdout is not
//!   mixed into another test's report;
//! * `passed`, `failed` and `compile_error` stay three different facts.

mod common;

use common::{work_dir, write, yashyc};
use std::fs;
use std::process::Command;

fn run_test(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(yashyc())
        .arg("test")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("the yashyc CLI must be runnable")
}

/// Three tests that must not be able to interfere with each other:
/// one passes, one exits 3, one prints and passes.
fn fixture(name: &str) -> std::path::PathBuf {
    let dir = work_dir(name);
    write(&dir, "tests/a_pass.ya", "fn main() {}\n");
    write(&dir, "tests/b_exit.ya", "fn main() { exit(3); }\n");
    write(
        &dir,
        "tests/c_prints.ya",
        "fn main() { print(\"stdout from c_prints\"); }\n",
    );
    dir
}

#[test]
fn json_reports_one_start_and_one_end_event_per_test() {
    let dir = fixture("test-runner-v2-events");
    let output = run_test(&dir, &["--json"]);
    let json = String::from_utf8_lossy(&output.stdout);
    assert_eq!(json.matches("\"event\": \"start\"").count(), 3, "{json}");
    assert_eq!(json.matches("\"event\": \"end\"").count(), 3, "{json}");
    assert!(
        json.contains("\"failure\": \"exit 3: "),
        "a failed event names why it failed: {json}"
    );
    assert!(
        json.contains("\"stdout\": \"stdout from c_prints\""),
        "the end event owns the test's stdout: {json}"
    );
}

#[test]
fn builds_one_harness_binary_for_the_whole_suite() {
    // The whole point of v2: one link for the suite, not one link per test.
    // This checks the build path directly so it can run where launching fresh
    // binaries is restricted by the host.
    let dir = fixture("test-runner-v2-build");
    let tests: Vec<std::path::PathBuf> =
        ["tests/a_pass.ya", "tests/b_exit.ya", "tests/c_prints.ya"]
            .iter()
            .map(|name| dir.join(name))
            .collect();
    let toolchain = yashy_driver::detect().expect("a toolchain must be installed");
    let tmp = dir.join("harness-tmp");
    std::fs::create_dir_all(&tmp).expect("the build directory must be creatable");

    let built = yashy_driver::test_harness::build_harness(
        &tmp,
        &tests,
        &toolchain,
        yashy_codegen::host_target_triple(),
        0,
        &mut Vec::new(),
    )
    .expect("the harness must build");

    assert!(built.binary.is_file(), "one harness binary must be linked");
    assert_eq!(
        built.tests.len(),
        3,
        "every test is in the harness; compile errors: {:?}",
        built.compile_errors
    );
    assert!(built.compile_errors.is_empty());
    for (index, entry) in built.tests.iter().enumerate() {
        assert_eq!(entry.symbol, format!("__yashy_test_{index}"));
    }
}

#[test]
fn one_harness_names_every_test_and_keeps_each_status() {
    let dir = fixture("test-runner-v2-report");
    let output = run_test(&dir, &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success(), "a failing test must fail the run");
    assert!(stdout.contains("tests/a_pass.ya"), "stdout:\n{stdout}");
    assert!(stdout.contains("tests/b_exit.ya"), "stdout:\n{stdout}");
    assert!(stdout.contains("tests/c_prints.ya"), "stdout:\n{stdout}");
    // A run failure names the status the process returned. "FAILED" alone
    // makes a reader rerun the test by hand to learn why it failed.
    assert!(
        stdout.contains("FAILED (exit 3)"),
        "the failing test must report its exit status:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("2 passed; 1 failed; 0 compile errors"),
        "the summary must count the two passing tests and the one failure:\n{stdout}"
    );
}

#[test]
fn one_hundred_tests_finish_inside_the_two_second_budget() {
    // The phase's acceptance gate: 100 ordinary programs must run in under two
    // seconds. The 1.0 runner took 1235 s for 101 tests because it linked once
    // per test; the harness links once for the whole suite.
    let dir = work_dir("test-runner-v2-100");
    fs::create_dir_all(dir.join("tests")).expect("the test directory must be creatable");
    for index in 0..100 {
        write(
            &dir,
            &format!("tests/t{index:03}.ya"),
            &format!("fn main() {{ assert({index} >= 0); }}\n"),
        );
    }

    let started = std::time::Instant::now();
    let output = run_test(&dir, &[]);
    let elapsed = started.elapsed();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "all 100 tests must pass:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("100 passed"),
        "the report must name all 100 tests:\n{stdout}"
    );
    assert!(
        elapsed.as_secs_f64() < 2.0,
        "100 tests must finish in < 2 s, took {elapsed:?}"
    );
}

#[test]
fn a_printing_test_owns_its_stdout() {
    let dir = fixture("test-runner-v2-stdout");
    let output = run_test(&dir, &["--json"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(1),
        "one failing test fails the run:\n{stdout}{stderr}"
    );
    // The printing test is the one that wrote these bytes. The report carries
    // them per test, not as one blob for the suite.
    let c = stdout
        .split("\"name\":")
        .find(|part| part.contains("\"c_prints\""))
        .unwrap_or_else(|| panic!("c_prints must be in the report:\n{stdout}"));
    assert!(
        c.contains("\"status\": \"passed\""),
        "c_prints must pass:\n{c}"
    );
    assert!(
        c.contains("stdout from c_prints"),
        "c_prints must own the bytes it printed:\n{c}"
    );
    assert!(
        !c.contains("a_pass") && !c.contains("b_exit"),
        "a test's stdout must not leak into another test's object:\n{c}"
    );
}

#[test]
fn a_run_failure_reports_its_exit_status_in_json() {
    let dir = fixture("test-runner-v2-exit");
    let output = run_test(&dir, &["--json"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    let b = stdout
        .split("\"name\":")
        .find(|part| part.contains("\"b_exit\""))
        .unwrap_or_else(|| panic!("b_exit must be in the report:\n{stdout}"));
    assert!(
        b.contains("\"status\": \"failed\""),
        "b_exit must be a failed test, not a compile error:\n{b}"
    );
    assert!(
        b.contains("\"exit\": 3"),
        "b_exit must report the status it returned:\n{b}"
    );
    assert!(
        stdout.contains("\"passed\": 2") && stdout.contains("\"failed\": 1"),
        "the summary must count the two passing tests and the one failure:\n{stdout}"
    );
    assert!(
        stdout.contains("\"compile_errors\": 0"),
        "a test that ran is not a compile error:\n{stdout}"
    );
}
