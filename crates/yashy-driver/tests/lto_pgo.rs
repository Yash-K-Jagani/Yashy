//! ThinLTO is the release link, and `--pgo` is the two-pass profile build.
//!
//! These tests drive the CLI because the promises are about what a user sees:
//! the `lto` and `pgo` timing rows, the `.yashy/pgo/` profile, and the refusal
//! to combine a training build with `--emit-ir`.

mod common;

use common::{launch_output, work_dir, write, yashy};
use std::path::Path;
use std::process::{Command, Output};

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(yashy())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("the yashy CLI must be runnable")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

const SOURCE: &str = "fn main() { print(1 + 1); print(\"\"); }";

#[test]
fn a_release_build_reports_thin_lto() {
    let dir = work_dir("lto-release");
    write(&dir, "main.ya", SOURCE);

    let output = run(&dir, &["build", "main.ya", "--release", "--timings"]);
    assert!(
        output.status.success(),
        "the release build must succeed:\n{}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("lto=thin"),
        "a release build links as one ThinLTO unit:\n{}",
        stderr(&output)
    );
}

#[test]
fn no_lto_takes_the_release_link_apart() {
    let dir = work_dir("lto-off");
    write(&dir, "main.ya", SOURCE);

    let output = run(
        &dir,
        &["build", "main.ya", "--release", "--no-lto", "--timings"],
    );
    assert!(
        output.status.success(),
        "the build must succeed:\n{}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("lto=off"),
        "`--no-lto` must turn the LTO row off:\n{}",
        stderr(&output)
    );
}

#[test]
fn a_debug_build_does_not_link_with_lto() {
    let dir = work_dir("lto-debug");
    write(&dir, "main.ya", SOURCE);

    let output = run(&dir, &["build", "main.ya", "--timings"]);
    assert!(
        output.status.success(),
        "the build must succeed:\n{}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("lto=off"),
        "LTO belongs to the release link, not to every build:\n{}",
        stderr(&output)
    );
}

#[test]
fn pgo_is_refused_with_emit_ir() {
    let dir = work_dir("pgo-emit-ir");
    write(&dir, "main.ya", SOURCE);

    let output = run(&dir, &["build", "main.ya", "--pgo", "--emit-ir"]);
    assert_eq!(output.status.code(), Some(2), "a usage error exits 2");
    let message = stderr(&output);
    assert!(
        message.contains("--pgo") && message.contains("--emit-ir"),
        "the refusal must name both flags:\n{message}"
    );
}

#[test]
fn run_refuses_pgo_because_it_would_train_and_then_run() {
    let dir = work_dir("pgo-run");
    write(&dir, "main.ya", SOURCE);

    let output = run(&dir, &["run", "main.ya", "--pgo"]);
    assert_eq!(output.status.code(), Some(2), "a usage error exits 2");
    assert!(
        stderr(&output).contains("build --pgo"),
        "the message should point at the build command:\n{}",
        stderr(&output)
    );
}

#[test]
fn pgo_trains_and_rebuilds_when_the_profile_runtime_exists() {
    let dir = work_dir("pgo-round");
    write(&dir, "main.ya", SOURCE);

    let output = run(
        &dir,
        &["build", "main.ya", "--release", "--pgo", "--timings"],
    );
    let report = stderr(&output);
    if !output.status.success()
        && (report.contains("libclang_rt.profile") || report.contains("profile runtime"))
    {
        // The clang on this host has no profiler runtime for the target triple
        // (a Windows GNU install ships the MSVC profile library only). The
        // driver's diagnostic already names the missing file; the CI toolchain
        // is MSVC, which has it.
        eprintln!("skipping: this clang cannot link the profile runtime");
        return;
    }
    assert!(output.status.success(), "the --pgo build failed:\n{report}");
    for stage in ["pgo=instrumented", "pgo=trained", "pgo=applied"] {
        assert!(
            report.contains(stage),
            "the build must report `{stage}`:\n{report}"
        );
    }

    let profile = dir.join(".yashy").join("pgo");
    let merged: Vec<_> = std::fs::read_dir(&profile)
        .expect("the profile directory must exist")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "profdata")
        })
        .collect();
    assert_eq!(
        merged.len(),
        1,
        "one merged profile must live under .yashy/pgo/: {merged:?}"
    );

    let program =
        launch_output(&common::exe(&dir, "main")).expect("the rebuilt program must start");
    assert!(
        program.status.success(),
        "the rebuilt program must run:\n{}",
        String::from_utf8_lossy(&program.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&program.stdout),
        "2\n",
        "the profile must not change what the program prints"
    );
}
