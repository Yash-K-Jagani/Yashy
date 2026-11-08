//! Task-aware debugging: `yashy tasks` and `yashy run --debug` print the
//! live tasks (id, async function, resume index, state) on a panic.
//!
//! The fixture `async_debug_tasks.ya` spawns one task and panics in the
//! main task, so at the panic point exactly two tasks are live:
//! `main_task` running and `blocked_on_channel` still queued (pending).
//! That is the task-7 test the plan names: both reported, the blocked
//! one marked `pending`, the correct function name, in a stable order.

mod common;

use common::yashy;
use std::path::{Path, PathBuf};
use std::process::Command;

fn program() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("programs")
        .join("async_debug_tasks.ya")
}

fn run_tasks(args: &[&str], extra: &[&str]) -> std::process::Output {
    let mut command = Command::new(yashy());
    command.args(extra);
    command.args(args);
    command
        .output()
        .expect("the yashy CLI must be runnable")
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The two task lines the fixture must produce, in order: ids are assigned
/// at block-on and at spawn, so the order is stable run to run. Lines are
/// matched without their line ending: the CRT's text mode writes CRLF on
/// stderr on Windows.
fn assert_task_table(output: &std::process::Output) {
    let stderr = stderr_of(output);
    for line in ["tasks: 2", "task 1 main_task resume=0 state=running"] {
        assert!(
            stderr.contains(line),
            "the dump must contain `{line}`:\n{stderr}"
        );
    }
    let running = stderr.find("task 1 main_task resume=0 state=running");
    let pending = stderr.find("task 2 blocked_on_channel resume=0 state=pending");
    assert!(
        running.is_some(),
        "the running main task must be named with its resume index:\n{stderr}"
    );
    assert!(
        pending.is_some(),
        "the blocked task must be marked pending with its function name:\n{stderr}"
    );
    assert!(
        running.unwrap() < pending.unwrap(),
        "the table must list tasks in id order:\n{stderr}"
    );
}

#[test]
fn tasks_reports_both_tasks_at_the_panic() {
    let source = program().to_string_lossy().into_owned();
    let output = run_tasks(&[source.as_str()], &["tasks"]);
    assert!(
        !output.status.success(),
        "the fixture panics, so the run must fail:\n{}",
        stderr_of(&output)
    );
    assert_task_table(&output);
}

#[test]
fn run_debug_reports_on_panic_too() {
    let source = program().to_string_lossy().into_owned();
    let output = run_tasks(&[source.as_str()], &["run", "--debug"]);
    assert!(
        !output.status.success(),
        "the fixture panics, so the run must fail:\n{}",
        stderr_of(&output)
    );
    assert_task_table(&output);
}

#[test]
fn tasks_refuses_a_release_build() {
    let source = program().to_string_lossy().into_owned();
    let output = run_tasks(&[source.as_str()], &["tasks", "--release"]);
    assert!(
        !output.status.success(),
        "`tasks` must refuse `--release`:\n{}",
        stderr_of(&output)
    );
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("debug build"),
        "the refusal must name the debug-build rule:\n{stderr}"
    );
}
