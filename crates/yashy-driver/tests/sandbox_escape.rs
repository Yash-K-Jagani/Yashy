//! The sandbox escape corpus: ten attempts that must fail to escape.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

fn yashy() -> &'static str {
    common::yashy()
}

/// Build `name.ya` from the corpus and run it sandboxed. Returns
/// (exit code, stdout+stderr).
fn run_sandboxed(name: &str, read_roots: &[&str], write_roots: &[&str]) -> (i32, String) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("sandbox_corpus")
        .join(format!("{name}.ya"));
    let mut command = Command::new(yashy());
    command.arg("run").arg("--sandbox").arg(&source);
    for root in read_roots {
        command.arg("--allow-read").arg(root);
    }
    for root in write_roots {
        command.arg("--allow-write").arg(root);
    }
    let output = command.output().expect("the yashy CLI must be runnable");
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.code().unwrap_or(-1), text)
}

fn workspace_dir() -> PathBuf {
    common::work_dir("sandbox-workspace")
}

fn result_is_denied(code: i32, output: &str) -> bool {
    // A denied read/write is a value the program matches on (io.Error), so
    // the program itself prints the denial; the sandbox's job is that the
    // file outside the allowlist was never touched.
    code == 0 && output.contains("denied")
}

#[test]
fn an_absolute_path_outside_the_root_is_denied() {
    let dir = workspace_dir();
    let (code, output) = run_sandboxed("abs_path", &[dir.to_str().unwrap()], &[]);
    assert!(
        result_is_denied(code, output.as_str()),
        "a read outside the allowlist must be denied:\n{output}"
    );
}

#[test]
fn a_traversal_outside_the_root_is_denied() {
    let dir = workspace_dir();
    let (code, output) = run_sandboxed("traversal", &[dir.to_str().unwrap()], &[]);
    assert!(
        result_is_denied(code, output.as_str()),
        "a `..` traversal must be denied:\n{output}"
    );
}

#[test]
fn a_symlink_outside_the_root_is_denied() {
    let dir = workspace_dir();
    let (code, output) = run_sandboxed("symlink", &[dir.to_str().unwrap()], &[]);
    assert!(
        result_is_denied(code, output.as_str()),
        "a symlink out must be denied:\n{output}"
    );
}

#[test]
fn an_environment_named_path_is_denied() {
    let dir = workspace_dir();
    let (code, output) = run_sandboxed("env_path", &[dir.to_str().unwrap()], &[]);
    assert!(
        result_is_denied(code, output.as_str()),
        "a path from the environment must be denied:\n{output}"
    );
}

#[test]
fn proc_self_mem_is_denied() {
    let dir = workspace_dir();
    let (code, output) = run_sandboxed("proc_mem", &[dir.to_str().unwrap()], &[]);
    assert!(
        result_is_denied(code, output.as_str()),
        "/proc/self/mem must be denied:\n{output}"
    );
}

#[test]
fn a_write_outside_the_root_is_denied() {
    let dir = workspace_dir();
    let (code, output) = run_sandboxed("write_out", &[], &[dir.to_str().unwrap()]);
    assert!(
        result_is_denied(code, output.as_str()),
        "a write outside the allowlist must be denied:\n{output}"
    );
}

#[test]
fn a_huge_allocation_is_denied_by_the_memory_cap() {
    let dir = workspace_dir();
    // 200M ints is ~800 MB against a 256 MB cap: the runtime's limit check
    // turns the read/write into a denial, and the push loop must not get
    // to hand back a full slice. The program ends without printing
    // "escaped" and without a full run.
    let (code, output) = run_sandboxed("big_alloc", &[dir.to_str().unwrap()], &[]);
    assert!(
        !output.contains("escaped"),
        "1 GiB under a 256 MB cap must be denied:\n{output} (exit {code})"
    );
}

#[test]
fn a_socket_connect_is_denied_by_default() {
    let dir = workspace_dir();
    let (code, output) = run_sandboxed("socket", &[dir.to_str().unwrap()], &[]);
    assert!(
        result_is_denied(code, output.as_str()),
        "network is off by default:\n{output}"
    );
}

#[test]
fn a_child_process_spawn_is_denied_by_default() {
    let dir = workspace_dir();
    // A denied spawn exits loudly (yashy_exit(1)); an allowed one would
    // print 1. Either way the worker's answer never appears.
    let (code, output) = run_sandboxed("spawn_child", &[dir.to_str().unwrap()], &[]);
    assert!(
        code != 0 || !output.contains("\n1\n"),
        "a task must not start when spawn is off:\n{output}"
    );
}

#[test]
fn an_infinite_loop_is_killed_by_the_time_limit() {
    let dir = workspace_dir();
    let (code, output) = run_sandboxed("infinite_loop", &[dir.to_str().unwrap()], &[]);
    // The time limit is enforced by the OS (Job Object / rlimits), so the
    // process is killed: a non-zero exit the program did not choose.
    assert!(
        code != 0,
        "the time limit must end an infinite loop:\n{output}"
    );
}
