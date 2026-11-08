//! `yashy context`: the token-budgeted context pack.

mod common;

use std::path::PathBuf;
use std::process::Command;

fn yashy() -> &'static str {
    common::yashy()
}

fn run(args: &[&str]) -> (bool, String, String) {
    let output = Command::new(yashy())
        .args(args)
        .output()
        .expect("the yashy CLI must be runnable");
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
fn a_pack_contains_the_function_and_the_covering_test() {
    let dir = fixture_dir("context-pack");
    let (ok, stdout, stderr) = run(&[
        "context",
        "--task",
        "fix the bug in add",
        "--json",
        dir.to_str().unwrap(),
    ]);
    assert!(ok, "context must succeed: {stderr}");
    assert!(
        stdout.contains("\"tokens\""),
        "the pack reports its own size:\n{stdout}"
    );
    let (ok, markdown, stderr) = run(&[
        "context",
        "--task",
        "fix the bug in add",
        dir.to_str().unwrap(),
    ]);
    assert!(ok, "context must succeed: {stderr}");
    assert!(
        markdown.contains("fn add"),
        "the enclosing function:\n{markdown}"
    );
    assert!(
        markdown.contains("smoke.ya"),
        "the covering test:\n{markdown}"
    );
}

#[test]
fn the_pack_never_exceeds_its_budget_and_names_what_it_dropped() {
    let dir = fixture_dir("context-budget");
    let (ok, stdout, stderr) = run(&[
        "context",
        "--task",
        "fix the bug in add",
        "--budget",
        "50",
        "--json",
        dir.to_str().unwrap(),
    ]);
    assert!(ok, "a partial pack still exits 0: {stderr}");
    assert!(
        stdout.contains("\"dropped\""),
        "dropped sections are named:\n{stdout}"
    );
    let tokens: usize = stdout
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("\"tokens\": ")
                .and_then(|rest| rest.trim_end_matches(',').parse().ok())
        })
        .expect("a tokens field");
    assert!(tokens <= 50, "tokens <= budget: {stdout}");
}

#[test]
fn the_same_task_produces_byte_identical_output() {
    let dir = fixture_dir("context-stable");
    let (_, first, _) = run(&["context", "--task", "fix the bug in add", dir.to_str().unwrap()]);
    let (_, second, _) = run(&["context", "--task", "fix the bug in add", dir.to_str().unwrap()]);
    assert_eq!(first, second, "no run-order dependence");
}

#[test]
fn a_task_that_names_nothing_is_a_usage_error() {
    let dir = common::work_dir("context-empty");
    let (ok, _, stderr) = run(&[
        "context",
        "--task",
        "fix the bug in zzz",
        dir.to_str().unwrap(),
    ]);
    assert!(!ok, "exit 2 on an unknown task: {stderr}");
}
