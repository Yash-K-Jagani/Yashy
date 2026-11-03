//! The `--no-opt=escape` flag and the promotion timing rows.
//!
//! Promotion is on for a build, and the flag only exists so a suspected
//! miscompile can be bisected without taking an optimisation away from anyone
//! else. These tests drive the CLI because the promises are about what a user
//! sees: the IR the compiler prints, the rows `--timings` adds, and the refusal
//! of a pass that does not exist.

mod common;

use common::{work_dir, write, yashy};
use std::path::Path;
use std::process::{Command, Output};

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(yashy())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("the yashy CLI must be runnable")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

const SOURCE: &str = "\
fn f() -> int {
    let p = alloc(8);
    p[0] = 1;
    let v = p[0];
    p.free();
    return v;
}
fn main() { }
";

#[test]
fn a_build_promotes_by_default_and_the_flag_keeps_the_heap() {
    let dir = work_dir("escape-flags");
    write(&dir, "main.ya", SOURCE);

    let promoted = run(&dir, &["build", "main.ya", "--emit-ir"]);
    assert!(
        promoted.status.success(),
        "the build must succeed:\n{}",
        stderr(&promoted)
    );
    let ir = stdout(&promoted);
    assert!(
        ir.contains("alloca [8 x i64]"),
        "the default must promote:\n{ir}"
    );
    assert!(
        !ir.contains("call void @yashy_free("),
        "the promoted allocation has no free:\n{ir}"
    );

    let disabled = run(&dir, &["build", "main.ya", "--emit-ir", "--no-opt=escape"]);
    assert!(
        disabled.status.success(),
        "the build must succeed:\n{}",
        stderr(&disabled)
    );
    let ir = stdout(&disabled);
    assert!(
        ir.contains("call i8* @yashy_alloc("),
        "`--no-opt=escape` keeps the allocator:\n{ir}"
    );
    assert!(
        ir.contains("call void @yashy_free("),
        "`--no-opt=escape` keeps the free:\n{ir}"
    );
}

#[test]
fn timings_report_what_was_promoted() {
    let dir = work_dir("escape-timings");
    write(&dir, "main.ya", SOURCE);

    let output = run(&dir, &["build", "main.ya", "--emit-ir", "--timings"]);
    assert!(
        output.status.success(),
        "the build must succeed:\n{}",
        stderr(&output)
    );
    let report = stderr(&output);
    assert!(
        report.contains("promoted_stack=1"),
        "one site should be promoted:\n{report}"
    );
    assert!(
        report.contains("promoted_arena=0"),
        "nothing needs an arena:\n{report}"
    );

    let disabled = run(
        &dir,
        &[
            "build",
            "main.ya",
            "--emit-ir",
            "--timings",
            "--no-opt=escape",
        ],
    );
    assert!(
        disabled.status.success(),
        "the build must succeed:\n{}",
        stderr(&disabled)
    );
    assert!(
        !stderr(&disabled).contains("promoted_stack="),
        "a disabled pass reports no promotion:\n{}",
        stderr(&disabled)
    );
}

#[test]
fn an_unknown_disabled_pass_is_a_usage_error() {
    let dir = work_dir("escape-unknown-pass");
    write(&dir, "main.ya", SOURCE);

    let output = run(&dir, &["build", "main.ya", "--no-opt=inlining"]);
    assert_eq!(output.status.code(), Some(2), "a usage error exits 2");
    assert!(
        stderr(&output).contains("escape"),
        "the message should name the pass that exists:\n{}",
        stderr(&output)
    );
}
