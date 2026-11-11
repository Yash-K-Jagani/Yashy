//! `yashy fmt --check`: the format gate CI can enforce.
//!
//! A gate is only useful if it is honest on a clean checkout, so the tests here
//! cover the two failure modes that make people disable one: a file that is
//! merely CRLF (not a formatting difference) and a flag combination that
//! silently writes when the caller asked only to look.

mod common;

use common::{work_dir, yashy};
use std::fs;
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(yashy())
        .args(args)
        .output()
        .expect("the yashy CLI must be runnable")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A file the formatter would rewrite: two spaces of indentation where the
/// canonical form has four.
const UNFORMATTED: &str = "fn main() {\n  print(\"a\");\n  print(\"b\");\n}\n";

#[test]
fn check_exits_one_for_a_file_that_would_change_and_names_it() {
    let dir = work_dir("fmt-check");
    let file = dir.join("main.ya");
    fs::write(&file, UNFORMATTED).expect("the test source must be writable");

    let output = run(&["fmt", "--check", file.to_str().unwrap()]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "a file the formatter would change must fail the gate:\n{}",
        stderr_of(&output)
    );
    assert!(
        stdout_of(&output).contains("main.ya"),
        "the gate must name the file it would change:\n{}",
        stdout_of(&output)
    );
    // Nothing was written: the gate is a question, not an edit.
    assert_eq!(
        fs::read_to_string(&file).expect("the source must be readable"),
        UNFORMATTED
    );
}

#[test]
fn check_exits_zero_after_write() {
    let dir = work_dir("fmt-write-then-check");
    let file = dir.join("main.ya");
    fs::write(&file, UNFORMATTED).expect("the test source must be writable");

    let written = run(&["fmt", "-w", file.to_str().unwrap()]);
    assert_eq!(
        written.status.code(),
        Some(0),
        "fmt -w must succeed:\n{}",
        stderr_of(&written)
    );

    let checked = run(&["fmt", "--check", file.to_str().unwrap()]);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "a canonical file must pass the gate:\n{}",
        stderr_of(&checked)
    );
    assert!(
        stdout_of(&checked).is_empty(),
        "a passing gate says nothing:\n{}",
        stdout_of(&checked)
    );
}

#[test]
fn check_and_write_together_is_a_usage_error() {
    let dir = work_dir("fmt-check-write");
    let file = dir.join("main.ya");
    fs::write(&file, UNFORMATTED).expect("the test source must be writable");

    let output = run(&["fmt", "--check", "--write", file.to_str().unwrap()]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "--check --write is a usage error, never a guess:\n{}",
        stderr_of(&output)
    );
    assert_eq!(
        fs::read_to_string(&file).expect("the source must be readable"),
        UNFORMATTED,
        "a refused command must not have written anything"
    );
}

#[test]
fn check_normalises_crlf_so_a_windows_checkout_passes() {
    let dir = work_dir("fmt-crlf");
    let file = dir.join("main.ya");
    // The canonical text, checked out with `core.autocrlf=true`.
    let canonical = "fn main() {\n    print(\"a\");\n}\n".replace('\n', "\r\n");
    fs::write(&file, &canonical).expect("the test source must be writable");

    let output = run(&["fmt", "--check", file.to_str().unwrap()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "CRLF alone is not a formatting difference:\n{}",
        stderr_of(&output)
    );
}

#[test]
fn check_json_lists_exactly_the_changed_files() {
    let dir = work_dir("fmt-json");
    let messy = dir.join("messy.ya");
    let clean = dir.join("clean.ya");
    fs::write(&messy, UNFORMATTED).expect("the test source must be writable");

    // Ask the formatter what canonical text is, so the "clean" fixture cannot
    // drift from the formatter's own idea of canonical.
    let formatted = run(&["fmt", messy.to_str().unwrap()]);
    assert_eq!(
        formatted.status.code(),
        Some(0),
        "fmt must print the canonical text:\n{}",
        stderr_of(&formatted)
    );
    fs::write(&clean, stdout_of(&formatted)).expect("the canonical source must be writable");

    let output = run(&[
        "fmt",
        "--check",
        "--json",
        messy.to_str().unwrap(),
        clean.to_str().unwrap(),
    ]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "one changed file must fail the gate:\n{}",
        stderr_of(&output)
    );
    let json = stdout_of(&output);
    assert!(
        json.contains("\"schema_version\": 1"),
        "the document is schema v1:\n{json}"
    );
    assert!(
        json.contains("\"tool\": \"fmt\""),
        "the document names its tool:\n{json}"
    );
    assert!(
        json.contains("\"changed\": true"),
        "the document lists the changed file:\n{json}"
    );
    assert!(
        !json.contains("clean.ya"),
        "the document lists only the changed files:\n{json}"
    );
}

#[test]
fn json_without_check_is_a_usage_error() {
    let dir = work_dir("fmt-json-without-check");
    let file = dir.join("main.ya");
    fs::write(&file, UNFORMATTED).expect("the test source must be writable");

    let output = run(&["fmt", "--json", file.to_str().unwrap()]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "--json lists the gate's answer, so it needs the gate:\n{}",
        stderr_of(&output)
    );
}
