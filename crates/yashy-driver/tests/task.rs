//! A task is a named list of compiler verbs, not a shell command.

mod common;

use common::{work_dir, write, yashyc};
use std::process::Command;

fn run(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(yashyc())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("the CLI must run")
}

fn project(dir: &std::path::Path, tasks: &str) {
    write(
        dir,
        "yashy.toml",
        &format!("[package]\nname = \"tasks\"\nversion = \"0.1.0\"\n\n[tasks]\n{tasks}\n"),
    );
    write(dir, "main.ya", "fn main() {}\n");
    write(dir, "tests/smoke.ya", "fn main() { assert(true); }\n");
}

#[test]
fn a_task_runs_named_verbs() {
    let dir = work_dir("task-run");
    project(
        &dir,
        "check = [\"check\", \"main.ya\"]\nfirst = [\"--list-not-a-verb\"]\n",
    );
    let listed = run(&dir, &["task", "--list"]);
    assert!(listed.status.success());
    let stdout = String::from_utf8_lossy(&listed.stdout);
    assert!(stdout.contains("check"), "{stdout}");

    let output = run(&dir, &["task", "check"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_task_cannot_name_an_unknown_verb() {
    let dir = work_dir("task-unknown");
    project(&dir, "bad = [\"rm\", \"-rf\", \"/\"]\n");
    let output = run(&dir, &["task", "bad"]);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not a compiler verb"), "{stderr}");
}

#[test]
fn an_unknown_task_is_a_diagnostic() {
    let dir = work_dir("task-missing");
    project(&dir, "check = [\"check\", \"main.ya\"]\n");
    let output = run(&dir, &["task", "nonesuch"]);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("no task named"));
}
