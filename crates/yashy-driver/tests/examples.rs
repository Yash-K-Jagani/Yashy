//! The examples are projects, not snippets: each one must pass `yashy test`,
//! and the two filters must produce the output their README promises.

mod common;

use common::yashy;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn examples_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
}

fn example(name: &str) -> PathBuf {
    let dir = examples_root().join(name);
    assert!(
        dir.join("yashy.toml").is_file(),
        "{} must be a project",
        dir.display()
    );
    assert!(
        dir.join("main.ya").is_file(),
        "{} must have main.ya",
        dir.display()
    );
    assert!(
        dir.join("tests").join("smoke.ya").is_file(),
        "{} must have tests/smoke.ya",
        dir.display()
    );
    dir
}

fn run_test(dir: &Path) {
    let output = Command::new(yashy())
        .arg("test")
        .current_dir(dir)
        .output()
        .expect("the yashy CLI must be runnable");
    assert!(
        output.status.success(),
        "`yashy test` failed in {}:\n{}\n{}",
        dir.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_with_stdin(dir: &Path, input: &[u8]) -> Output {
    let mut child = Command::new(yashy())
        .arg("run")
        .arg("main.ya")
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the yashy CLI must be runnable");
    child
        .stdin
        .take()
        .expect("the child stdin must be piped")
        .write_all(input)
        .expect("the example input must be writable");
    child.wait_with_output().expect("the example must finish")
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

#[test]
fn hello_builds_and_tests() {
    let dir = example("hello");
    run_test(&dir);

    let output = Command::new(yashy())
        .arg("run")
        .arg("main.ya")
        .current_dir(&dir)
        .output()
        .expect("the yashy CLI must be runnable");
    assert!(
        output.status.success(),
        "the hello example failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(stdout_of(&output), "Hello from the Yashy examples.\n");
}

#[test]
fn wordcount_builds_and_tests() {
    let dir = example("wordcount");
    run_test(&dir);

    let output = run_with_stdin(&dir, b"hello world\nsecond line\n");
    assert!(
        output.status.success(),
        "the wordcount example failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(stdout_of(&output), "2 4 24\n");
}

#[test]
fn json_report_builds_and_tests() {
    let dir = example("json-report");
    run_test(&dir);

    let output = run_with_stdin(
        &dir,
        b"{\"name\": \"yashy\"}\n{\"age\": 3}\n{\"name\": \"again\"}\n",
    );
    assert!(
        output.status.success(),
        "the json-report example failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        stdout_of(&output),
        "{\"documents\": 3, \"distinct_keys\": 2, \"note\": \"counted by Yashy\"}\n"
    );
}
