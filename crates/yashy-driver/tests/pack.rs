//! Structured context packs: the excerpts are compiler facts with paths,
//! spans and hashes, and a cut pack names everything that did not fit.

mod common;

use std::path::PathBuf;
use std::process::Command;

fn yashyc() -> &'static str {
    common::yashyc()
}

fn run(args: &[&str]) -> (bool, String, String) {
    let output = Command::new(yashyc())
        .args(args)
        .output()
        .expect("the yashyc CLI must be runnable");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn fixture_dir(name: &str) -> PathBuf {
    let dir = common::work_dir(name);
    common::write(
        &dir,
        "tests/lib.ya",
        "/// Adds two numbers.\npub pure fn add(int a, int b) -> int {\n    return a + b;\n}\n",
    );
    common::write(
        &dir,
        "tests/smoke.ya",
        "import lib;\nfn main() {\n    print(lib.add(1, 2));\n}\n",
    );
    dir
}

#[test]
fn a_focus_pack_returns_structured_excerpts_and_a_revision() {
    let dir = fixture_dir("pack-focus");
    let root = dir.to_str().unwrap();
    let (ok, stdout, stderr) = run(&[
        "context",
        "--json",
        "--task",
        "fix the bug in add",
        "--mode",
        "focus",
        root,
    ]);
    assert!(ok, "a focus pack must succeed: {stderr}");
    for field in [
        "\"excerpts\"",
        "\"file\"",
        "\"hash\"",
        "\"score\"",
        "\"revision\"",
        "\"signatures\"",
    ] {
        assert!(stdout.contains(field), "{field} is missing:\n{stdout}");
    }
    assert!(
        stdout.contains("add"),
        "the focus signature is in the pack: {stdout}"
    );
    assert!(
        stdout.contains("smoke.ya"),
        "the covering test is in the pack: {stdout}"
    );
}

#[test]
fn the_same_request_has_the_same_revision() {
    let dir = fixture_dir("pack-stable");
    let root = dir.to_str().unwrap();
    let (_, first, _) = run(&[
        "context",
        "--json",
        "--task",
        "fix the bug in add",
        "--mode",
        "focus",
        root,
    ]);
    let (_, second, _) = run(&[
        "context",
        "--json",
        "--task",
        "fix the bug in add",
        "--mode",
        "focus",
        root,
    ]);
    assert_eq!(first, second, "packs are byte-stable");
}

#[test]
fn a_small_budget_drops_excerpts_with_a_reason() {
    let dir = fixture_dir("pack-budget");
    let (ok, stdout, stderr) = run(&[
        "context",
        "--json",
        "--task",
        "fix the bug in add",
        "--mode",
        "focus",
        "--budget",
        "10",
        dir.to_str().unwrap(),
    ]);
    assert!(ok, "a cut pack still exits 0: {stderr}");
    assert!(stdout.contains("\"dropped\""), "drops are named: {stdout}");
    assert!(stdout.contains("over the 10 token budget"), "{stdout}");
}

#[test]
fn an_error_pack_carries_the_diagnostic_and_an_edited_file_pack_carries_the_file() {
    let dir = fixture_dir("pack-modes");
    let root = dir.to_str().unwrap();
    let (ok, stdout, stderr) = run(&[
        "context",
        "--json",
        "--task",
        "fix the bug in add",
        "--mode",
        "error",
        "--diagnostic",
        "E0100 at line 2",
        root,
    ]);
    assert!(ok, "an error pack must succeed: {stderr}");
    assert!(stdout.contains("E0100 at line 2"), "{stdout}");

    let (ok, stdout, stderr) = run(&[
        "context",
        "--json",
        "--task",
        "fix the bug in add",
        "--mode",
        "diff",
        "--changed",
        "tests/lib.ya",
        root,
    ]);
    assert!(ok, "a diff pack must succeed: {stderr}");
    assert!(stdout.contains("tests/lib.ya"), "{stdout}");
}

#[test]
fn an_unknown_mode_is_a_usage_error() {
    let dir = fixture_dir("pack-bad-mode");
    let (ok, _, stderr) = run(&[
        "context",
        "--task",
        "fix the bug in add",
        "--mode",
        "prompt",
        dir.to_str().unwrap(),
    ]);
    assert!(!ok, "an unknown mode exits 2: {stderr}");
    assert!(stderr.contains("unknown pack mode"), "{stderr}");
}
