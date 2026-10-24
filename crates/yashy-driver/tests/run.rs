mod common;

use common::{exe, work_dir, write, yashy};
use std::process::Command;

#[test]
fn run_compiles_and_prints_program_output() {
    let dir = work_dir("run-output");
    write(
        &dir,
        "main.ya",
        "fn main() { println(\"hello from run\"); }\n",
    );

    let output = Command::new(yashy())
        .arg("run")
        .arg(dir.join("main.ya"))
        .current_dir(&dir)
        .output()
        .expect("the yashy CLI must be runnable");

    assert!(
        output.status.success(),
        "run must succeed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "hello from run\n");
}

#[test]
fn run_forwards_the_program_exit_code() {
    let dir = work_dir("run-exit");
    write(&dir, "main.ya", "fn main() -> int { return 7; }\n");

    let output = Command::new(yashy())
        .arg("run")
        .arg(dir.join("main.ya"))
        .current_dir(&dir)
        .output()
        .expect("the yashy CLI must be runnable");

    assert_eq!(output.status.code(), Some(7));
}

#[test]
fn run_rejects_output_paths() {
    let dir = work_dir("run-output-flag");
    write(&dir, "main.ya", "fn main() {}\n");

    let output = Command::new(yashy())
        .arg("run")
        .arg(dir.join("main.ya"))
        .arg("-o")
        .arg(exe(&dir, "kept"))
        .output()
        .expect("the yashy CLI must be runnable");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("run"),
        "the error should mention run:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
