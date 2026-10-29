//! `--no-verify`: the escape hatch, and every promise it makes.
//!
//! The flag exists for the case where the verifier is wrong and a user still
//! needs a binary. That is a narrow, loud, traceable thing, and each test here
//! pins one part of the bargain:
//!
//! - the build succeeds and warns;
//! - the binary carries a `.unverified` stamp a later reader can find;
//! - a verified build retires that stamp;
//! - a program with a contract is refused, because only the verifier can honour
//!   a contract;
//! - a pipeline that demands proof (`YASHY_REQUIRE_VERIFY`) refuses the flag;
//! - `--emit-ir` is not a binary, so the stamp has nothing to describe.
//!
//! A refusal is a usage error (exit 2), not a diagnostic: the program is fine,
//! the command line is not.

mod common;

use common::{exe, work_dir, write, yashy};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(yashy())
        .args(args)
        .env_remove("YASHY_REQUIRE_VERIFY")
        .current_dir(dir)
        .output()
        .expect("the yashy CLI must be runnable")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The stamp beside a binary: `out.exe` becomes `out.exe.unverified`.
fn sidecar(output: &Path) -> PathBuf {
    let mut path = output.as_os_str().to_owned();
    path.push(".unverified");
    PathBuf::from(path)
}

const HELLO: &str = "fn main() {\n    println(\"hello\");\n}\n";

#[test]
fn no_verify_writes_a_sidecar_and_warns() {
    let dir = work_dir("no-verify-stamp");
    write(&dir, "hello.ya", HELLO);
    let output = exe(&dir, "out");

    let build = run(
        &dir,
        &[
            "build",
            "hello.ya",
            "-o",
            output.to_str().expect("the test path must be UTF-8"),
            "--no-verify",
        ],
    );
    let errors = stderr(&build);
    assert_eq!(
        build.status.code(),
        Some(0),
        "the build must succeed:\n{errors}"
    );
    assert!(
        errors.contains("warning:") && errors.contains("--no-verify"),
        "an unverified build must say so:\n{errors}"
    );
    assert!(output.is_file(), "the binary must exist");

    let stamp = sidecar(&output);
    let text = fs::read_to_string(&stamp).expect("the stamp must be written beside the binary");
    assert!(
        text.contains("\"verified\":false"),
        "the stamp must deny verification:\n{text}"
    );
    assert!(
        text.contains("\"schema_version\":1") && text.contains("\"--no-verify\""),
        "the stamp must be machine-readable and name the reason:\n{text}"
    );
}

#[test]
fn a_verified_build_removes_a_stale_sidecar() {
    let dir = work_dir("no-verify-stale-stamp");
    write(&dir, "hello.ya", HELLO);
    let output = exe(&dir, "out");
    let stamp = sidecar(&output);
    fs::write(&stamp, "stale").expect("the stale stamp must be writable");

    let build = run(
        &dir,
        &[
            "build",
            "hello.ya",
            "-o",
            output.to_str().expect("the test path must be UTF-8"),
        ],
    );
    let errors = stderr(&build);
    assert_eq!(
        build.status.code(),
        Some(0),
        "the build must succeed:\n{errors}"
    );
    assert!(
        !stamp.exists(),
        "a verified build must not leave an unverified stamp behind"
    );
}

#[test]
fn no_verify_is_refused_when_the_program_has_a_contract() {
    let dir = work_dir("no-verify-contract");
    write(
        &dir,
        "contract.ya",
        "fn positive(n: int) requires n > 0 { }\n\nfn main() { }\n",
    );
    let output = exe(&dir, "out");

    let build = run(
        &dir,
        &[
            "build",
            "contract.ya",
            "-o",
            output.to_str().expect("the test path must be UTF-8"),
            "--no-verify",
        ],
    );
    let errors = stderr(&build);
    assert_eq!(
        build.status.code(),
        Some(2),
        "a contract is a usage error for this flag:\n{errors}"
    );
    assert!(
        errors.contains("positive"),
        "the refusal must name the contract-bearing function:\n{errors}"
    );
    assert!(
        errors.contains("--no-verify"),
        "the refusal must name the flag:\n{errors}"
    );
    assert!(!output.exists(), "a refused build must produce no binary");
    assert!(
        !sidecar(&output).exists(),
        "a refused build must produce no stamp"
    );
}

#[test]
fn no_verify_is_refused_when_the_program_has_a_loop_invariant() {
    let dir = work_dir("no-verify-invariant");
    write(
        &dir,
        "invariant.ya",
        "fn main() {\n    let i = 0;\n    while i < 3 invariant i >= 0 {\n        i = i + 1;\n    }\n}\n",
    );
    let output = exe(&dir, "out");

    let build = run(
        &dir,
        &[
            "build",
            "invariant.ya",
            "-o",
            output.to_str().expect("the test path must be UTF-8"),
            "--no-verify",
        ],
    );
    let errors = stderr(&build);
    assert_eq!(
        build.status.code(),
        Some(2),
        "a loop invariant is a contract too:\n{errors}"
    );
    assert!(
        errors.contains("main"),
        "the refusal must name the function holding the invariant:\n{errors}"
    );
    assert!(!output.exists(), "a refused build must produce no binary");
}

#[test]
fn require_verify_makes_no_verify_a_usage_error() {
    let dir = work_dir("no-verify-required");
    write(&dir, "hello.ya", HELLO);
    let output = exe(&dir, "out");

    let refused = Command::new(yashy())
        .args([
            "build",
            "hello.ya",
            "-o",
            output.to_str().expect("the test path must be UTF-8"),
            "--no-verify",
        ])
        .env("YASHY_REQUIRE_VERIFY", "1")
        .current_dir(&dir)
        .output()
        .expect("the yashy CLI must be runnable");
    let errors = stderr(&refused);
    assert_eq!(
        refused.status.code(),
        Some(2),
        "a pipeline that demands proof must refuse the flag:\n{errors}"
    );
    assert!(
        errors.contains("YASHY_REQUIRE_VERIFY"),
        "the refusal must name the variable that caused it:\n{errors}"
    );
    assert!(!output.exists(), "a refused build must produce no binary");

    // The variable demands verification; it does not forbid building. An
    // ordinary build still verifies and still succeeds.
    let verified = Command::new(yashy())
        .args([
            "build",
            "hello.ya",
            "-o",
            output.to_str().expect("the test path must be UTF-8"),
        ])
        .env("YASHY_REQUIRE_VERIFY", "1")
        .current_dir(&dir)
        .output()
        .expect("the yashy CLI must be runnable");
    assert_eq!(
        verified.status.code(),
        Some(0),
        "a verified build must still work with the variable set:\n{}",
        stderr(&verified)
    );
}

#[test]
fn no_verify_builds_a_program_the_verifier_rejects() {
    let dir = work_dir("no-verify-skips-proof");
    // A leak is an error the verifier reports. Without the flag the build
    // stops; with it the binary exists and is stamped.
    write(
        &dir,
        "leak.ya",
        "fn main() {\n    let buf = alloc(16);\n}\n",
    );
    let output = exe(&dir, "out");
    let output_arg = output.to_str().expect("the test path must be UTF-8");

    let verified = run(&dir, &["build", "leak.ya", "-o", output_arg]);
    assert_eq!(
        verified.status.code(),
        Some(1),
        "the verifier must reject the leak:\n{}",
        stderr(&verified)
    );
    assert!(!output.exists(), "a rejected build must produce no binary");

    let unverified = run(&dir, &["build", "leak.ya", "-o", output_arg, "--no-verify"]);
    let errors = stderr(&unverified);
    assert_eq!(
        unverified.status.code(),
        Some(0),
        "`--no-verify` must skip the proof:\n{errors}"
    );
    assert!(output.is_file(), "the unverified binary must exist");
    assert!(
        sidecar(&output).is_file(),
        "the unverified binary must be stamped"
    );
}

#[test]
fn no_verify_is_refused_with_emit_ir() {
    let dir = work_dir("no-verify-emit-ir");
    write(&dir, "hello.ya", HELLO);

    let build = run(&dir, &["build", "hello.ya", "--emit-ir", "--no-verify"]);
    let errors = stderr(&build);
    assert_eq!(
        build.status.code(),
        Some(2),
        "IR is not a binary, so there is nothing to stamp:\n{errors}"
    );
    assert!(
        errors.contains("IR is not a binary"),
        "the refusal must be about IR, not an unknown argument:\n{errors}"
    );
}
