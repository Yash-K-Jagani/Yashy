//! End-to-end tests: compile a real program to a real executable and run it.
//!
//! These are the only tests in the workspace that prove the whole pipeline
//! works — lexer, parser, checker, code generator, `clang`, the runtime, and
//! the operating system's process loader all have to agree before one byte of
//! expected output appears.
//!
//! Every test drives the shipped CLI binary (not the library) so the argument
//! parsing and exit codes are covered too, and builds into a per-test temporary
//! directory so the tests can run in parallel.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The `otter` binary cargo built for this test run.
fn otter() -> &'static str {
    env!("CARGO_BIN_EXE_otter")
}

fn programs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("programs")
}

/// A fresh, empty directory for one test, unique per test name and process.
fn work_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("otter-e2e-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("the test work directory must be creatable");
    dir
}

/// Compile `name.ot` from `tests/programs` into `dir/name<exe suffix>`.
fn build_program(name: &str) -> (PathBuf, PathBuf, Output) {
    let dir = work_dir(name);
    let source = programs_dir().join(format!("{name}.ot"));
    let exe = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    let output = Command::new(otter())
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("the otter CLI must be runnable");
    (dir, exe, output)
}

/// Run a built executable and return its output.
fn run(exe: &Path) -> Output {
    Command::new(exe)
        .output()
        .expect("the compiled executable must be runnable")
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Build and run a program, asserting it compiled and exited zero.
fn build_and_run(name: &str) -> Output {
    let (_dir, exe, compile) = build_program(name);
    assert!(
        compile.status.success(),
        "compiling {name}.ot failed:\n{}",
        stderr_of(&compile)
    );
    assert!(exe.is_file(), "no executable at {}", exe.display());
    let output = run(&exe);
    assert!(
        output.status.success(),
        "{name} exited with {:?}",
        output.status.code()
    );
    output
}

#[test]
fn hello_world_prints_and_exits_zero() {
    let output = build_and_run("hello");
    assert_eq!(stdout_of(&output), "Hello, Otter!\n");
}

#[test]
fn fibonacci_prints_the_tenth_number() {
    // 55, the tenth Fibonacci number. Recursion rather than a loop: Otter has
    // no assignment statement for a loop body to accumulate into (see fib.ot).
    let output = build_and_run("fib");
    assert_eq!(stdout_of(&output), "55\n");
}

#[test]
fn option_matching_selects_the_some_payload() {
    let output = build_and_run("option");
    assert_eq!(stdout_of(&output), "42\n");
}

#[test]
fn result_propagation_returns_the_ok_value() {
    let output = build_and_run("result");
    assert_eq!(stdout_of(&output), "7\n");
}

#[test]
fn recursion_composes_in_hanoi() {
    // 2^3 - 1 moves.
    let output = build_and_run("hanoi");
    assert_eq!(stdout_of(&output), "7\n");
}

#[test]
fn the_default_output_path_sits_beside_the_source() {
    let dir = work_dir("default-output");
    let source = dir.join("hello.ot");
    fs::copy(programs_dir().join("hello.ot"), &source).expect("copy hello.ot");

    let compile = Command::new(otter())
        .arg("build")
        .arg(&source)
        .output()
        .expect("run the otter CLI");
    assert!(
        compile.status.success(),
        "compiling failed:\n{}",
        stderr_of(&compile)
    );

    let expected = dir.join(format!("hello{}", std::env::consts::EXE_SUFFIX));
    assert!(
        expected.is_file(),
        "the default output should be {}",
        expected.display()
    );
    assert_eq!(stdout_of(&run(&expected)), "Hello, Otter!\n");
}

#[test]
fn a_type_error_produces_a_caret_diagnostic_and_exit_1() {
    let dir = work_dir("type-error");
    let source = dir.join("bad.ot");
    fs::write(&source, "fn main() {\n    const x = 1 + true;\n}\n").expect("write bad.ot");

    let compile = Command::new(otter())
        .arg("build")
        .arg(&source)
        .output()
        .expect("run the otter CLI");

    assert_eq!(
        compile.status.code(),
        Some(1),
        "a type error is a compile error"
    );
    let stderr = stderr_of(&compile);
    assert!(stderr.contains("error:"), "expected an `error:` line:\n{stderr}");
    assert!(
        stderr.lines().any(|line| line.trim_start().starts_with('^')),
        "expected a caret line underlining the offending expression:\n{stderr}"
    );
}

#[test]
fn emit_ir_prints_llvm_without_invoking_llvm() {
    // PATH is cleared, so any attempt to find or run `clang`/`llc` fails. The
    // IR still comes out, which is what proves `--emit-ir` stops before the
    // backend.
    let dir = work_dir("emit-ir");
    let source = dir.join("hello.ot");
    fs::copy(programs_dir().join("hello.ot"), &source).expect("copy hello.ot");

    let output = Command::new(otter())
        .arg("build")
        .arg(&source)
        .arg("--emit-ir")
        .env("PATH", "")
        .env("OTTER_LLC", dir.join("no-such-llc"))
        .output()
        .expect("run the otter CLI");

    assert!(
        output.status.success(),
        "--emit-ir must not need a toolchain:\n{}",
        stderr_of(&output)
    );
    let ir = stdout_of(&output);
    assert!(ir.contains("target datalayout"), "not LLVM IR:\n{ir}");
    assert!(ir.contains("define void @main()"), "not this program:\n{ir}");
}

#[test]
fn missing_llvm_toolchain_fails_with_an_install_hint() {
    let dir = work_dir("no-toolchain");
    let source = dir.join("hello.ot");
    fs::copy(programs_dir().join("hello.ot"), &source).expect("copy hello.ot");

    // No PATH entry can resolve `llc` or `clang`, and the override points at
    // nothing, so detection has to fail before any compilation happens.
    let output = Command::new(otter())
        .arg("build")
        .arg(&source)
        .env("PATH", "")
        .env("OTTER_LLC", dir.join("no-such-llc"))
        .output()
        .expect("run the otter CLI");

    assert_eq!(output.status.code(), Some(1));
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("install"),
        "the message should say what to install:\n{stderr}"
    );
    if cfg!(windows) {
        assert!(
            stderr.contains("winget install LLVM.LLVM"),
            "the Windows hint should name the winget package:\n{stderr}"
        );
    }
}

#[test]
fn version_prints_a_version_and_exits_zero() {
    let output = Command::new(otter())
        .arg("version")
        .output()
        .expect("run the otter CLI");
    assert!(output.status.success());
    let stdout = stdout_of(&output);
    assert!(stdout.starts_with("otter "), "no version line:\n{stdout}");
}

#[test]
fn check_reports_a_type_error_without_building() {
    let dir = work_dir("check-error");
    let source = dir.join("bad.ot");
    fs::write(&source, "fn main() {\n    const x = 1 + true;\n}\n").expect("write bad.ot");

    let output = Command::new(otter())
        .arg("check")
        .arg(&source)
        .output()
        .expect("run the otter CLI");
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr_of(&output).contains("error:"));
}
