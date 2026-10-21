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
use std::time::{Duration, Instant};

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
    Command::new(exe).output().unwrap_or_else(|error| {
        // A machine can refuse to start a binary it has just been handed: an
        // Application Control (Device Guard) policy blocks a content-defined
        // subset of newly linked executables, and the linker stamps a fresh
        // timestamp into every build, so the same source is judged differently
        // from one run to the next. That is an environment result, not evidence
        // about the compiler, and saying so here stops the failure being read as
        // a codegen bug.
        let policy = error.to_string().contains("blocked")
            || error.to_string().contains("Application Control")
            || error.raw_os_error() == Some(4551);
        let hint = if policy {
            "\n\nthe operating system refused to start the binary: an Application \
             Control / Device Guard policy blocked it. Rebuilding produces a \
             different binary (the linker stamps a timestamp), which usually has a \
             different verdict."
        } else {
            ""
        };
        panic!(
            "the compiled executable at {} must be runnable: {error}{hint}",
            exe.display()
        )
    })
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// How long a concurrent program may run before the test calls it hung.
///
/// The programs finish in about a second; the bound only has to be longer than
/// the slowest CI machine, because its job is to turn a lost wakeup into a
/// failure instead of a stuck test run.
const CONCURRENT_TIMEOUT: Duration = Duration::from_secs(60);

/// Run a built executable, killing it if it outlives `timeout`.
///
/// A concurrent program that loses a wakeup blocks in a channel call forever,
/// so a plain [`Command::output`] would hang the whole test binary. Polling
/// and killing keeps the failure attached to this test and names the timeout.
fn run_with_timeout(exe: &Path, timeout: Duration) -> Output {
    use std::process::Stdio;

    let mut child = Command::new(exe)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| {
            panic!(
                "the compiled executable at {} must start: {error}",
                exe.display()
            )
        });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return child
                    .wait_with_output()
                    .expect("the built executable must be waitable");
            }
            Ok(None) => {
                if started.elapsed() >= timeout {
                    let _ = child.kill();
                    let output = child
                        .wait_with_output()
                        .expect("the killed executable must be waitable");
                    panic!(
                        "{} did not terminate within {timeout:?}; stderr:\n{}",
                        exe.display(),
                        stderr_of(&output)
                    );
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("waiting for {} failed: {error}", exe.display()),
        }
    }
}

/// Run a built executable with `input` on its standard input.
///
/// The write end is closed before waiting, so the program sees end of input
/// after the last line rather than blocking on a pipe nobody will write to
/// again — which is what makes a read-until-EOF loop terminate.
fn run_with_input(exe: &Path, input: &str) -> Output {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new(exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the built executable must start");
    child
        .stdin
        .as_mut()
        .expect("standard input was piped")
        .write_all(input.as_bytes())
        .expect("writing the program's input must succeed");
    drop(child.stdin.take());
    child
        .wait_with_output()
        .expect("the built executable must be waitable")
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
fn string_methods_transform_bytes_and_ownership() {
    let output = build_and_run("string_methods");
    assert_eq!(
        stdout_of(&output),
        "16\nnot empty\n72\n72\n-1\nHello\ncontains Otter\nmissing cat\nstarts He\nends spaces\n9\n-1\nHello, Otter\n14\n14\n16\nHELLO, OTTER\nhello, otter\nHello, world\nababab\n0\n3\na\nb\nc\n1\n2\n3\none\ntwo\nthree\nempty is empty\n0\n42\nf64 parse ok\n"
    );
}

#[test]
fn array_search_and_copy_methods() {
    let output = build_and_run("array_search");
    assert_eq!(
        stdout_of(&output),
        "contains 20\nmissing 99\n2\n-1\n5\n10\n20\n30\n40\n50\n20\n30\n40\n10\n20\n30\n40\n50\ncontains c\n1\n4\nb\nc\n"
    );
}

#[test]
fn array_edit_methods_mutate_in_place() {
    let output = build_and_run("array_edit");
    assert_eq!(
        stdout_of(&output),
        "front ok\nmiddle ok\nend ok\npast end rejected\n6\n10\n-1\n5\n5\n15\n20\n30\n35\n35\n30\n20\n15\n5\n5\n15\n20\n30\n35\n0\ncleared\nstrings cleared\n"
    );
}

#[test]
fn array_query_methods_answer_in_options() {
    let output = build_and_run("array_query");
    assert_eq!(
        stdout_of(&output),
        "empty is empty\n-1\n-1\nxs is not empty\n20\n-1\n10\n30\nset in range\n99\nset out of range\n"
    );
}

#[test]
fn array_builtins_can_be_called_as_methods() {
    let output = build_and_run("array_methods");
    assert_eq!(stdout_of(&output), "2\n3\n");
}

#[test]
fn functions_can_be_passed_and_called_through_values() {
    let output = build_and_run("functions");
    assert_eq!(stdout_of(&output), "49\n16\n");
}

#[test]
fn a_non_capturing_lambda_is_a_function_value() {
    let output = build_and_run("lambdas");
    assert_eq!(stdout_of(&output), "42\n40\n");
}

#[test]
fn a_closure_outlives_its_creating_scope() {
    let output = build_and_run("closures");
    assert_eq!(stdout_of(&output), "15\nhello\n");
}

#[test]
fn let_var_and_const_behave_as_documented() {
    let output = build_and_run("bindings");
    assert_eq!(stdout_of(&output), "2\n42\n3\n");
}

#[test]
fn fibonacci_prints_the_tenth_number() {
    // 55, the tenth Fibonacci number, from an accumulating loop: this is the
    // end-to-end proof that `symbol = expr;` reaches a real binary and mutates a
    // binding (see fib.ot).
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
fn float_arithmetic_survives_the_whole_pipeline() {
    // Regression guard for a defect the unit tests structurally could not see:
    // `1.0 + 2.0` lowered to `add double` and `3.0 * 4.0` to `mul double`, and
    // the golden file recorded that invalid IR as the expected contract, so the
    // only thing that could reject it was `clang` itself. This test puts every
    // f64 operator, and an exponent-form literal, through the real backend.
    let output = build_and_run("floats");
    assert_eq!(stdout_of(&output), "3\n2\n12\n3\n1\n-2\n0\n");
}

#[test]
fn an_empty_option_can_be_constructed_and_matched() {
    // `None` was unusable: not treated as a constructor by sema, unable to infer
    // its payload because call arguments were checked before the callee was
    // resolved, and missing a codegen lowering. Every existing program only
    // *matched* on `None`, so the suite never built one.
    let output = build_and_run("none");
    assert_eq!(stdout_of(&output), "42\n0\n0\n");
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
fn building_leaves_no_intermediate_object_file() {
    // The generated IR and the runtime are compiled and linked by a *single*
    // `clang` invocation, so no per-build object file is written next to the
    // output. That is a deliberate shape, not an accident of the arguments:
    // splitting it back into "assemble, then link" measured 6-9% slower for a
    // byte-identical executable. Pinning it here means a change that
    // reintroduces a separate assemble step has to say so out loud.
    let dir = work_dir("no-object");
    let source = programs_dir().join("hello.ot");
    let exe = dir.join(format!("hello{}", std::env::consts::EXE_SUFFIX));
    let compile = Command::new(otter())
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("run the otter CLI");
    assert!(
        compile.status.success(),
        "compiling hello.ot failed:\n{}",
        stderr_of(&compile)
    );
    let object = exe.with_extension("o");
    assert!(
        !object.exists(),
        "an intermediate object was left at {}",
        object.display()
    );
    // The IR itself is *not* temporary: it is the documented debugging aid, so
    // it must still be beside the output.
    assert!(
        exe.with_extension("ll").is_file(),
        "the generated IR should be kept beside the executable"
    );
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
    // The entry point is an `i32` function whatever the Otter signature says,
    // because that is the return type the C runtime reads as the exit status.
    assert!(ir.contains("define i32 @main()"), "not this program:\n{ir}");
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
fn an_unknown_optimization_level_is_a_usage_error() {
    // The level is validated by the CLI, not clamped and not passed through to
    // `clang`, so a typo is reported as a usage error naming the valid levels.
    let dir = work_dir("bad-opt-level");
    let source = dir.join("hello.ot");
    fs::copy(programs_dir().join("hello.ot"), &source).expect("copy hello.ot");

    let output = Command::new(otter())
        .arg("build")
        .arg(&source)
        .arg("-O9")
        .output()
        .expect("run the otter CLI");

    assert_eq!(output.status.code(), Some(1), "-O9 must not build");
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("-O0, -O1, -O2 or -O3"),
        "the message should list the valid levels:\n{stderr}"
    );
}

#[test]
fn every_optimization_level_builds_a_working_binary() {
    // `-O2` is the default, and this is the evidence that asking for it does
    // not change what any program *does*. The runtime C source is compiled by
    // the same flag, so a level that broke the runtime would show up here too.
    let dir = work_dir("opt-levels");
    let source = programs_dir().join("hello.ot");

    for level in ["-O0", "-O1", "-O2", "-O3"] {
        let exe = dir.join(format!("hello{level}{}", std::env::consts::EXE_SUFFIX));
        let compile = Command::new(otter())
            .arg("build")
            .arg(&source)
            .arg("-o")
            .arg(&exe)
            .arg(level)
            .output()
            .expect("run the otter CLI");
        assert!(
            compile.status.success(),
            "building at {level} failed:\n{}",
            stderr_of(&compile)
        );
        assert_eq!(
            stdout_of(&run(&exe)),
            "Hello, Otter!\n",
            "the program printed differently at {level}"
        );
    }
}

#[test]
fn every_cpu_tier_builds() {
    // Building needs no particular CPU — only *running* a higher-tier binary
    // does — so this covers every tier, including `x86-64-v4`, without asking
    // the machine to execute instructions it may not have. `--native` is the
    // tier guaranteed to run here, and the test that runs a tiered binary
    // unsafely is not one this suite should contain.
    let dir = work_dir("cpu-tiers");
    let source = programs_dir().join("hello.ot");

    // Both spellings are covered: `--cpu=<tier>` and `--cpu <tier>`.
    let invocations: [&[&str]; 5] = [
        &["--cpu=portable"],
        &["--cpu", "x86-64-v2"],
        &["--cpu=x86-64-v3"],
        &["--cpu", "x86-64-v4"],
        &["--cpu=native"],
    ];
    for (index, extra) in invocations.iter().enumerate() {
        let exe = dir.join(format!("hello-{index}{}", std::env::consts::EXE_SUFFIX));
        let mut command = Command::new(otter());
        command.arg("build").arg(&source).arg("-o").arg(&exe);
        command.args(*extra);
        let compile = command.output().expect("run the otter CLI");
        assert!(
            compile.status.success(),
            "building with {extra:?} failed:\n{}",
            stderr_of(&compile)
        );
        assert!(exe.is_file(), "no executable for {extra:?}");
    }

    // `--native` is a synonym for `--cpu native`, so it must still work.
    let exe = dir.join(format!("hello-alias{}", std::env::consts::EXE_SUFFIX));
    let compile = Command::new(otter())
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(&exe)
        .arg("--native")
        .output()
        .expect("run the otter CLI");
    assert!(
        compile.status.success(),
        "the `--native` alias failed:\n{}",
        stderr_of(&compile)
    );
    assert_eq!(stdout_of(&run(&exe)), "Hello, Otter!\n");
}

#[test]
fn a_cpu_tier_is_a_backend_choice_not_an_ir_change() {
    // A tier is an argument to `clang`, not something `otter-codegen` knows
    // about: `-march=x86-64-v3` changes the instructions the backend is allowed
    // to pick, not the IR it is handed. If the two dumps differed, the tier
    // would have leaked into codegen and the language's output would depend on
    // the machine it was built on.
    let source = programs_dir().join("floats.ot");

    let dump = |extra: &[&str]| {
        let mut command = Command::new(otter());
        command
            .arg("build")
            .arg(&source)
            .arg("--emit-ir")
            .args(extra);
        let output = command.output().expect("run the otter CLI");
        assert!(
            output.status.success(),
            "--emit-ir {extra:?} failed:\n{}",
            stderr_of(&output)
        );
        stdout_of(&output)
    };

    let portable = dump(&[]);
    assert_eq!(
        portable,
        dump(&["--cpu=x86-64-v3"]),
        "the feature level must not change the emitted IR"
    );
}

#[test]
fn an_unrecognised_cpu_target_is_a_usage_error() {
    // The tier is validated by the CLI, not clamped and not passed through to
    // `clang`, so a typo is a usage error naming the valid tiers.
    let dir = work_dir("bad-cpu");
    let source = dir.join("hello.ot");
    fs::copy(programs_dir().join("hello.ot"), &source).expect("copy hello.ot");

    let output = Command::new(otter())
        .arg("build")
        .arg(&source)
        .arg("--cpu=x86-64-v9")
        .output()
        .expect("run the otter CLI");
    assert_eq!(output.status.code(), Some(1), "a bad tier must not build");
    let stderr = stderr_of(&output);
    for tier in ["portable", "x86-64-v2", "x86-64-v3", "x86-64-v4", "native"] {
        assert!(
            stderr.contains(tier),
            "the message should list `{tier}`:\n{stderr}"
        );
    }

    // A bare `--cpu` with nothing after it is a usage error too, not a panic
    // and not a silent default.
    let output = Command::new(otter())
        .arg("build")
        .arg(&source)
        .arg("--cpu")
        .output()
        .expect("run the otter CLI");
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr_of(&output).contains("needs a value"));
}

#[test]
fn fast_math_reaches_codegen_and_leaves_a_non_fusing_program_alone() {
    let source = programs_dir().join("floats.ot");

    // 1. The flag has to reach codegen, and codegen has to mark the arithmetic.
    //    `--emit-ir` shows that deterministically, without depending on which
    //    instructions the machine running the test can execute.
    let dump = |extra: &[&str]| {
        let mut command = Command::new(otter());
        command
            .arg("build")
            .arg(&source)
            .arg("--emit-ir")
            .args(extra);
        let output = command.output().expect("run the otter CLI");
        assert!(
            output.status.success(),
            "--emit-ir {extra:?} failed:\n{}",
            stderr_of(&output)
        );
        stdout_of(&output)
    };
    let exact = dump(&[]);
    assert!(
        !exact.contains("contract"),
        "contraction must be off unless it is asked for:\n{exact}"
    );
    let fast = dump(&["--fast-math"]);
    assert!(
        fast.contains("fmul contract double"),
        "`--fast-math` must mark the multiply:\n{fast}"
    );
    assert!(
        fast.contains("fadd contract double"),
        "`--fast-math` must mark the add:\n{fast}"
    );

    // 2. And the whole pipeline still works with the flag on, on the *native*
    //    target so the backend really has a fused multiply-add to use.
    //
    //    `floats.ot` has a multiply and an add but never a multiply feeding an
    //    add, so contraction has nothing to fuse here and the output must be
    //    exactly the same as without the flag. That is what makes this program
    //    the right one to assert an exact output on under a flag whose entire
    //    purpose is that it can change one.
    let dir = work_dir("fast-math");
    let exe = dir.join(format!("floats{}", std::env::consts::EXE_SUFFIX));
    let compile = Command::new(otter())
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(&exe)
        .arg("--native")
        .arg("--fast-math")
        .output()
        .expect("run the otter CLI");
    assert!(
        compile.status.success(),
        "building with `--fast-math` failed:\n{}",
        stderr_of(&compile)
    );
    assert_eq!(stdout_of(&run(&exe)), "3\n2\n12\n3\n1\n-2\n0\n");
}

#[test]
fn native_optimization_does_not_change_what_the_program_prints() {
    // `--native` lets the backend use this CPU's instruction set, which on a
    // machine with AVX2 is a large speed-up on a vectorisable loop. What it
    // must *not* do is change the answer: a program may not print something
    // different because it was compiled for a newer CPU. `floats` is the
    // interesting case, because floating-point contraction is the one place a
    // faster CPU could quietly change a result — and this compiler does not
    // enable it, so the value must be identical.
    let dir = work_dir("native");
    let source = programs_dir().join("floats.ot");

    let builds = [
        ("portable", None),
        ("native", Some("--native")),
    ];
    let mut outputs = Vec::new();
    for (label, flag) in builds {
        let exe = dir.join(format!("floats-{label}{}", std::env::consts::EXE_SUFFIX));
        let mut command = Command::new(otter());
        command.arg("build").arg(&source).arg("-o").arg(&exe);
        if let Some(flag) = flag {
            command.arg(flag);
        }
        let compile = command.output().expect("run the otter CLI");
        assert!(
            compile.status.success(),
            "building the {label} variant failed:\n{}",
            stderr_of(&compile)
        );
        outputs.push((label, stdout_of(&run(&exe))));
    }

    let expected = "3\n2\n12\n3\n1\n-2\n0\n";
    for (label, actual) in &outputs {
        assert_eq!(
            actual, expected,
            "the {label} build printed a different result"
        );
    }
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

#[test]
fn heap_ownership_survives_the_whole_pipeline() {
    // alloc / move / dealloc / overwrite, compiled and run. This is the only
    // test that proves the heap symbols resolve against the runtime that the
    // driver actually links, and that a pointer can cross a function boundary
    // in a real binary.
    let output = build_and_run("memory");
    assert_eq!(stdout_of(&output), "memory ok\n");
}

#[test]
fn strings_concatenate_compare_and_measure() {
    // `+`, `len`, `==` and `!=` across a function boundary, compiled and run:
    // string values are returned from a helper, and the helper's concatenation
    // happens in a different frame from the caller's.
    let output = build_and_run("strings");
    assert_eq!(
        stdout_of(&output),
        "Hello, Otter!\n13\n[x]\n0\ndifferent\nequal\n"
    );
}

#[test]
fn a_string_binding_owns_its_reference_until_the_scope_ends() {
    // The refcounted representation, checked behaviourally: an alias survives its
    // source being overwritten, a loop's intermediate strings are released each
    // iteration, a function's result outlives the local it was built from, and a
    // `defer` still sees a string local because the release happens after the
    // defers. Garbage output or a crash here would mean a release landed too
    // early; see `ownership.ot`.
    let output = build_and_run("ownership");
    assert_eq!(
        stdout_of(&output),
        "AB\nAB!\n[x]\nzzz\nin\ndeferred in\nn = 42\n"
    );
}

#[test]
fn string_interpolation_builds_the_same_text_as_the_written_out_form() {
    // `"n = ${n}"` must equal `"n = " + str_from_i64(n)`, because the checker
    // lowers the first into the second. This is also the end-to-end proof that
    // the parser's sub-parse of the expression inside `${ ... }` reaches code
    // generation intact: the `${n + 1}` line is an arithmetic expression, not a
    // name, and it has to come out as 8.
    let output = build_and_run("interp");
    assert_eq!(
        stdout_of(&output),
        "n = 7\nd = 1.5\nflag = true\nhello, Otter!\n7 + 1 = 8\nsame, written out: 7\n"
    );
}

#[test]
fn a_unit_main_exits_zero_even_when_its_last_call_returns_nonzero() {
    // The entry point is emitted as `define i32 @main()`; a `void main` would
    // leave the process exit status as whatever the last call left in the return
    // register. Here that last call is a string comparison, which returns 1 when
    // the strings are equal -- so a `void main` exits 1 with the right output,
    // and only the exit code gives it away.
    let dir = work_dir("entry-exit-status");
    let source = dir.join("entry_exit_status.ot");
    fs::write(
        &source,
        "fn main() {\n    if \"a\" == \"a\" {\n        print_str(\"equal\\n\");\n    }\n}\n",
    )
    .expect("write entry_exit_status.ot");
    let exe = dir.join(format!(
        "entry_exit_status{}",
        std::env::consts::EXE_SUFFIX
    ));
    let compile = Command::new(otter())
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("run the otter CLI");
    assert!(
        compile.status.success(),
        "compiling entry_exit_status.ot failed:\n{}",
        stderr_of(&compile)
    );

    let output = run(&exe);
    assert_eq!(stdout_of(&output), "equal\n");
    assert_eq!(
        output.status.code(),
        Some(0),
        "the entry point must return an explicit zero, not the last call's register"
    );
}

#[test]
fn a_program_can_read_its_input() {
    // `read_line` and `parse_i64` through a real pipe: the loop runs until end
    // of input, and a line that is not an integer contributes zero.
    let (_dir, exe, compile) = build_program("adder");
    assert!(
        compile.status.success(),
        "compiling adder.ot failed:\n{}",
        stderr_of(&compile)
    );

    let counted = run_with_input(&exe, "1\n2\n3\n");
    assert_eq!(stdout_of(&counted), "6\n");
    assert_eq!(counted.status.code(), Some(0));

    let with_junk = run_with_input(&exe, "1\nx\n3\n");
    assert_eq!(
        stdout_of(&with_junk),
        "4\n",
        "a line that is not an integer parses as zero"
    );

    let empty = run_with_input(&exe, "");
    assert_eq!(stdout_of(&empty), "0\n", "end of input is the empty string");
}

#[test]
fn a_while_loop_runs_until_its_condition_stops_holding() {
    // `while` is the condition-only loop; the count of Collatz steps from each
    // start is a fact about the number, so the output pins the loop's control
    // flow rather than anything the program chose.
    let output = build_and_run("collatz");
    assert_eq!(stdout_of(&output), "1: 0\n2: 1\n3: 7\n4: 2\n5: 5\n");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn a_program_can_build_a_string_with_a_number_in_it() {
    // The conversions put a value inside a string, so a whole sentence is one
    // string rather than a sequence of `print_*` calls.
    let (_dir, exe, compile) = build_program("greet");
    assert!(
        compile.status.success(),
        "compiling greet.ot failed:\n{}",
        stderr_of(&compile)
    );

    let output = run_with_input(&exe, "Ada\n36\n");
    assert_eq!(stdout_of(&output), "name? age? Ada is 36 years old\n");
    assert_eq!(output.status.code(), Some(0));

    // A line that is not an integer contributes zero, which is what
    // `parse_i64` promises -- and the sentence is still well formed.
    let junk = run_with_input(&exe, "Bo\nnot a number\n");
    assert_eq!(stdout_of(&junk), "name? age? Bo is 0 years old\n");
}

#[test]
fn a_struct_field_is_read_by_name_whatever_order_it_was_written() {
    // The literal writes `y` first; `p.x` must still be the declared `x`.
    let output = build_and_run("structs");
    assert_eq!(stdout_of(&output), "3 4\n5\n3\n");
}

#[test]
fn a_struct_field_can_be_written_and_the_other_fields_survive() {
    // A struct value is an aggregate, not an address, so each write rebuilds the
    // whole value: `p.x = 10` must leave `p.y`, and `line.from.x = 100` must
    // leave `from.y` and both of `to`'s fields. The output is the proof, because
    // a lost field would print a wrong number rather than fail to compile.
    let output = build_and_run("fields");
    assert_eq!(stdout_of(&output), "10 2\n7\n102 7\n15\n");
}

#[test]
fn defer_runs_at_scope_exit_and_sees_registration_time_values() {
    // Last registered first, and the deferred free releases the block that was
    // live when the `defer` was reached.
    let output = build_and_run("defer");
    assert_eq!(stdout_of(&output), "body\nfirst\nsecond\n");
}

#[test]
fn a_deferred_free_after_a_rebind_does_not_double_free() {
    // Before the capture, the deferred body read `buf`'s slot at exit -- the
    // *second* allocation -- and freed it a second time after the explicit
    // `dealloc`, which corrupted the heap and aborted the process
    // (STATUS_HEAP_CORRUPTION). The two frees must now name different blocks:
    // the deferred one releases the allocation that was live at registration,
    // and the body's own `dealloc` releases the rebind.
    let dir = work_dir("deferred-free-rebind");
    let source = dir.join("deferred_free_rebind.ot");
    fs::write(
        &source,
        "fn main() {\n\
         \x20   var buf = alloc(16);\n\
         \x20   defer {\n\
         \x20       dealloc(buf);\n\
         \x20   }\n\
         \x20   buf = alloc(32);\n\
         \x20   dealloc(buf);\n\
         \x20   print_str(\"survived\\n\");\n\
         }\n",
    )
    .expect("write deferred_free_rebind.ot");
    let exe = dir.join(format!(
        "deferred_free_rebind{}",
        std::env::consts::EXE_SUFFIX
    ));
    let compile = Command::new(otter())
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("run the otter CLI");
    assert!(
        compile.status.success(),
        "compiling deferred_free_rebind.ot failed:\n{}",
        stderr_of(&compile)
    );

    let output = run(&exe);
    assert_eq!(stdout_of(&output), "survived\n");
    assert_eq!(
        output.status.code(),
        Some(0),
        "a second free of the same block aborts the process; the deferred body \
         must free the registration-time value instead"
    );
}

#[test]
fn a_manual_buffer_is_filled_and_summed_through_pointer_indexing() {
    // `buffer.ot`: 1024 `i64`s written and read through `p[i]`, then released
    // with a `defer` that is registered *after* the loops -- registering it
    // moves `buf`, so it is the last use. `leaks.rs` asserts the allocation came
    // back; this asserts the arithmetic was right.
    let output = build_and_run("buffer");
    assert_eq!(stdout_of(&output), "1047552\n");
}

#[test]
fn a_string_built_in_a_loop_releases_its_previous_value() {
    // `builder.ot`: a rebound accumulator and a fresh string per iteration. The
    // lengths are the value check; `leaks.rs` is the ownership check.
    let output = build_and_run("builder");
    assert_eq!(stdout_of(&output), "100\n7\n");
}

#[test]
fn an_arena_hands_out_blocks_that_all_survive_until_the_release() {
    // `arena.ot`: two allocations from one handle, written and read through
    // pointer indices, released in one call. The value proves the second
    // allocation did not disturb the first; the golden IR proves the byte counts
    // and the runtime tests prove the allocator, but only a link-and-run proves
    // the two agree on the ABI.
    let output = build_and_run("arena");
    assert_eq!(stdout_of(&output), "47\n");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn a_double_free_is_a_compile_error_with_a_caret() {
    // Ownership is checked, not assumed: freeing the same binding twice never
    // reaches the backend, so the runtime's `otter_free` cannot be handed the
    // same address twice by a program this compiler accepted.
    let dir = work_dir("double-free");
    let source = dir.join("double_free.ot");
    fs::write(
        &source,
        "fn main() {\n    var buf = alloc(16);\n    dealloc(buf);\n    dealloc(buf);\n}\n",
    )
    .expect("write double_free.ot");

    let compile = Command::new(otter())
        .arg("build")
        .arg(&source)
        .output()
        .expect("run the otter CLI");

    assert_eq!(compile.status.code(), Some(1), "a double free is a compile error");
    let stderr = stderr_of(&compile);
    assert!(
        stderr.contains("already been moved"),
        "the diagnostic must explain that the binding was spent:\n{stderr}"
    );
    assert!(
        stderr.lines().any(|line| line.trim_start().starts_with('^')),
        "expected a caret underlining the second use:\n{stderr}"
    );
}

#[test]
fn a_panic_prints_its_message_and_the_position_it_was_written_at() {
    // The whole pipeline on the diagnostic path: the message and the source
    // position reach standard error, the exit status is a failure, and nothing
    // after the panic runs. `checked_halve` panics one call deeper than `main`,
    // which is what makes "there is no recovery" a real claim about the program
    // rather than a reading of its source.
    let (_dir, exe, compile) = build_program("panic");
    assert!(
        compile.status.success(),
        "compiling panic.ot failed:\n{}",
        stderr_of(&compile)
    );
    let output = run(&exe);
    assert_eq!(stdout_of(&output), "10\n", "the line printed before the panic");
    assert_eq!(
        output.status.code(),
        Some(1),
        "a panic is a failure, not a normal exit: {:?}",
        output.status
    );
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("panic.ot:10:9: panic: cannot halve an odd number"),
        "the message and the position it was written at must reach standard \
         error:\n{stderr}"
    );
    assert!(
        !stdout_of(&output).contains("never be printed"),
        "nothing after the panic may run:\n{stderr}"
    );
}

#[test]
fn an_integer_literal_that_does_not_fit_is_a_compile_error_with_a_caret() {
    // `9223372036854775808` is one past `i64::MAX` and a number a user can type by
    // accident. Compiling it to `0` would be a wrong answer wearing the clothes of
    // a right one, so the whole CLI is pinned here: the diagnostic names the
    // literal and the caret sits under it, and nothing is emitted.
    let dir = work_dir("huge-literal");
    let source = dir.join("huge_literal.ot");
    fs::write(
        &source,
        "fn main() {\n    print_i64(9223372036854775808);\n}\n",
    )
    .expect("write huge_literal.ot");

    let exe = dir.join(format!("huge_literal{}", std::env::consts::EXE_SUFFIX));
    let compile = Command::new(otter())
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("run the otter CLI");

    assert_eq!(compile.status.code(), Some(1), "the literal cannot compile");
    let stderr = stderr_of(&compile);
    assert!(
        stderr.contains("integer literal does not fit an i64"),
        "the diagnostic must name the mistake:\n{stderr}"
    );
    // The caret is checked for position, not just for existence: it has to sit
    // under the digits, and cover all nineteen of them. A caret one column off
    // points at `(` and is worse than none.
    let lines: Vec<&str> = stderr.lines().collect();
    let underline = lines
        .iter()
        .position(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with('^') && trimmed.chars().all(|c| c == '^')
        })
        .unwrap_or_else(|| panic!("expected a caret line underlining the literal:\n{stderr}"));
    let carets = lines[underline];
    let source_line = lines[underline - 1];
    let column = |line: &str| line.len() - line.trim_start().len();
    assert_eq!(
        column(carets),
        source_line.find("9223372036854775808").unwrap_or(usize::MAX),
        "the caret must start under the first digit:\n{stderr}"
    );
    assert_eq!(
        carets.trim().len(),
        "9223372036854775808".len(),
        "one caret per digit:\n{stderr}"
    );
    assert!(
        !exe.is_file(),
        "a rejected program must not produce an executable at {}",
        exe.display()
    );
}

#[test]
fn the_smallest_i64_can_be_written_and_printed() {
    // `-9223372036854775808` is the one integer a leading minus makes readable
    // that its digits alone are not: the digits are one past `i64::MAX`, so the
    // literal under the minus cannot stand on its own and `i64::MIN` would have
    // no spelling at all without the fold. It is also the value that makes
    // `x / -1` a fault, so it has to be writable for that path to be testable.
    let dir = work_dir("i64-min");
    let source = dir.join("i64_min.ot");
    fs::write(
        &source,
        "fn main() {\n    print_i64(-9223372036854775808);\n    print_str(\"\\n\");\n}\n",
    )
    .expect("write i64_min.ot");
    let exe = dir.join(format!("i64_min{}", std::env::consts::EXE_SUFFIX));

    let compile = Command::new(otter())
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("run the otter CLI");
    assert!(
        compile.status.success(),
        "compiling i64_min.ot failed:\n{}",
        stderr_of(&compile)
    );

    let output = run(&exe);
    assert_eq!(
        stdout_of(&output),
        "-9223372036854775808\n",
        "the folded literal must survive to run time"
    );
    assert_eq!(output.status.code(), Some(0));
}

/// Write `source` into a fresh work directory, compile it, and run it.
///
/// The compile must succeed or the test would be measuring the front end instead
/// of the runtime, so a failed compile fails the test with the compiler's own
/// message rather than with whatever the run then says.
fn compile_and_run(name: &str, source: &str) -> Output {
    compile_and_run_with(name, source, &[])
}

/// The same, with extra `otter build` flags -- `--release`, say.
fn compile_and_run_with(name: &str, source: &str, flags: &[&str]) -> Output {
    let dir = work_dir(name);
    let file = dir.join(format!("{name}.ot"));
    fs::write(&file, source).expect("writing the test program");
    let exe = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    let mut command = Command::new(otter());
    command.arg("build").arg(&file).arg("-o").arg(&exe).args(flags);
    let compile = command.output().expect("run the otter CLI");
    assert!(
        compile.status.success(),
        "compiling {name}.ot failed:\n{}",
        stderr_of(&compile)
    );
    run(&exe)
}

#[test]
fn os_exit_ends_the_process_with_the_status_it_names() {
    let output = compile_and_run(
        "exit-status",
        concat!(
            "fn main() {\n",
            "    print_str(\"before\\n\");\n",
            "    os.exit(7);\n",
            "    print_str(\"after\\n\");\n",
            "}\n"
        ),
    );
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(
        stdout_of(&output),
        "before\n",
        "output written before the exit is flushed, and nothing after it runs"
    );
    assert_eq!(stderr_of(&output), "");
}

#[test]
fn os_exit_accepts_zero() {
    let output = compile_and_run(
        "exit-zero",
        "fn main() { os.exit(0); }\n",
    );
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn a_returned_i64_becomes_the_exit_status() {
    // `main` may return nothing or `i64`, which is the process status. The
    // `i32` the specification names is the C `main`'s type; Otter has no `i32`,
    // so `i64` is the spelling, truncated to the width the operating system reads.
    let output = compile_and_run(
        "exit-return",
        "fn main() -> i64 { return 3; }\n",
    );
    assert_eq!(output.status.code(), Some(3));
}

#[test]
fn os_exit_can_be_reached_through_a_helper() {
    // The whole point of the builtin over `main`'s return value: a helper can
    // stop the program without its caller learning the status.
    let output = compile_and_run(
        "exit-helper",
        concat!(
            "fn quit(code: i64) {\n",
            "    print_str(\"leaving\\n\");\n",
            "    os.exit(code);\n",
            "    print_str(\"never\\n\");\n",
            "}\n",
            "\n",
            "fn main() {\n",
            "    print_str(\"start\\n\");\n",
            "    quit(7);\n",
            "    print_str(\"after\\n\");\n",
            "}\n"
        ),
    );
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(stdout_of(&output), "start\nleaving\n");
}

#[test]
fn a_switch_arm_may_end_with_a_semicolon() {
    // Statement-shaped arms are the common case, and the habitual trailing
    // `;` must not make them fail to parse.
    let output = compile_and_run(
        "switch-semi",
        concat!(
            "fn main() {\n",
            "    var o = Some(42);\n",
            "    switch o {\n",
            "        case Some(v) { print_i64(v); }\n",
            "        case None { print_str(\"none\"); }\n",
            "    }\n",
            "    print_str(\"\\n\");\n",
            "}\n"
        ),
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout_of(&output), "42\n");
}

#[test]
fn literal_switch_arms_compare_the_subject() {
    // Every arm here has a value that must not match. That is the half the
    // original lowering got wrong: a literal arm had no test, so the first one
    // was the fall-through and everything matched it.
    let output = build_and_run("switch_literals");
    assert_eq!(
        stdout_of(&output),
        "zero one many\nanimal shout other\none and a half other\nyes no\n"
    );
}

#[test]
fn user_enums_construct_bind_and_match() {
    // The tag is the variant's declaration index, but payloads are packed
    // after it with no gaps, so tag and field index agree only when every
    // earlier variant carried exactly one field. `Message` declares its unit
    // variant first specifically to break that coincidence, and `Rect` writes
    // its named payload out of order to prove the layout comes from the
    // declaration.
    let output = build_and_run("enums");
    assert_eq!(
        stdout_of(&output),
        "12\n12\n0\nhi\ncount\nnothing\n7\n5\n0\n"
    );
}

#[test]
fn an_overflowing_addition_reports_the_operands_and_ends_the_program() {
    // Integer arithmetic is checked. The alternative is a wrapped number that
    // looks exactly like an answer, and a language with no exceptions has
    // nowhere else to put the error (spec 6.1). The message names the numbers as
    // well as the operation, because "it overflowed" is far less useful than
    // "these two added to too much".
    let output = compile_and_run(
        "overflow-add",
        "fn main() {\n    print_str(\"before\\n\");\n    var big = 9223372036854775807;\n    print_i64(big + 1);\n    print_str(\"after\\n\");\n}\n",
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "a fault is a failure status: {:?}",
        output.status
    );
    assert_eq!(
        stdout_of(&output),
        "before\n",
        "output from before the fault must survive it, and nothing after may run"
    );
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("panic: integer overflow: 9223372036854775807 + 1"),
        "the operands belong in the message:\n{stderr}"
    );
    assert!(
        stderr.contains("overflow-add.ot:4:15:"),
        "and so does the position the addition was written at:\n{stderr}"
    );
}

#[test]
fn an_overflowing_multiplication_is_a_fault_too() {
    // The multiply is the one that would have needed a hand-rolled test -- the
    // sign of the result is not enough to know whether the product fit -- which
    // is why it goes through LLVM's `smul.with.overflow`.
    let output = compile_and_run(
        "overflow-mul",
        "fn main() {\n    var big = 4000000000000000000;\n    print_i64(big * 3);\n}\n",
    );
    assert_eq!(output.status.code(), Some(1));
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("panic: integer overflow: 4000000000000000000 * 3"),
        "the message names the multiplication:\n{stderr}"
    );
}

#[test]
fn dividing_or_taking_the_remainder_by_zero_names_the_operator() {
    // A `sdiv` by zero does not return a wrong answer, it raises `#DE`, so this
    // is the check that keeps a division from reaching the instruction at all.
    // Both operators share one runtime function, and the symbol is what tells
    // them apart.
    for (name, source, expected) in [
        (
            "divide-by-zero",
            "fn main() {\n    var a = 5;\n    var b = 0;\n    print_i64(a / b);\n}\n",
            "panic: division by zero: 5 / 0",
        ),
        (
            "remainder-by-zero",
            "fn main() {\n    var a = 5;\n    var b = 0;\n    print_i64(a % b);\n}\n",
            "panic: division by zero: 5 % 0",
        ),
    ] {
        let output = compile_and_run(name, source);
        assert_eq!(output.status.code(), Some(1), "{name}");
        let stderr = stderr_of(&output);
        assert!(
            stderr.contains(expected),
            "{name} should report `{expected}`:\n{stderr}"
        );
    }
}

#[test]
fn dividing_the_smallest_i64_by_minus_one_is_an_overflow() {
    // The other way a division has no answer: the result is one past `i64::MAX`.
    // `srem` is undefined there too, so both operators are guarded, and the
    // runtime names this case from the operands rather than being told -- the
    // check in generated code is a single branch for both.
    let output = compile_and_run(
        "overflow-div-min",
        "fn main() {\n    var m = -9223372036854775808;\n    var minus_one = 0 - 1;\n    print_i64(m / minus_one);\n}\n",
    );
    assert_eq!(output.status.code(), Some(1));
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains(
            "panic: integer overflow: -9223372036854775808 / -1"
        ),
        "the division that has no answer must say so:\n{stderr}"
    );
}

#[test]
fn negating_the_smallest_i64_is_an_overflow() {
    // `-i64::MIN` wraps to itself, which is the one negation that cannot be
    // right. It reports as an overflow of a subtraction from zero, because that
    // is what negation is.
    let output = compile_and_run(
        "overflow-neg",
        "fn main() {\n    var m = -9223372036854775808;\n    print_i64(-m);\n}\n",
    );
    assert_eq!(output.status.code(), Some(1));
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("panic: integer overflow: 0 - -9223372036854775808"),
        "the negation that has no answer must say so:\n{stderr}"
    );
    assert!(
        stderr.contains("overflow-neg.ot:3:15:"),
        "and point at the negation:\n{stderr}"
    );
}

// --- end-to-end fault programs (M4 Task 13) --------------------------------
//
// The four programs below are the definition of done for M3-M4's fault half:
// each exits with a diagnostic that names the source position and the failed
// operation, and never as a signal or a hardware trap. They live in
// `tests/programs` rather than as inline sources so the exact text the
// diagnostic points at is part of the repository.
//
// The runtime's diagnostic format is `<file>:<line>:<column>: panic: <message>`
// rather than the plan's sketched `error:` line plus a caret. A caret needs the
// source text to underline, and the runtime is handed a path, a line and a
// column -- not the file. Compile-time diagnostics still render a caret; a
// runtime fault names the same position in the machine-readable form an editor
// or a test can consume.

#[test]
fn division_by_zero_in_a_program_reports_the_position_and_exits_1() {
    let (_dir, exe, compile) = build_program("divzero");
    assert!(
        compile.status.success(),
        "compiling divzero.ot failed:\n{}",
        stderr_of(&compile)
    );
    let output = run(&exe);
    assert_eq!(
        stdout_of(&output),
        "5\n",
        "the division before the fault must print, and nothing after may run"
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "a fault is an exit through the diagnostic path, not a signal: {:?}",
        output.status
    );
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("divzero.ot:11:12: panic: division by zero: 10 / 0"),
        "the position, the operator and both operands belong in the message:\n{stderr}"
    );
}

#[test]
fn integer_overflow_does_not_wrap() {
    // The first line is the largest `i64`; the faulting line is that plus one.
    // If arithmetic wrapped, the last thing printed would be `i64::MIN`, which
    // this asserts must not appear.
    let (_dir, exe, compile) = build_program("overflow");
    assert!(
        compile.status.success(),
        "compiling overflow.ot failed:\n{}",
        stderr_of(&compile)
    );
    let output = run(&exe);
    assert_eq!(stdout_of(&output), "9223372036854775807\n");
    assert_eq!(output.status.code(), Some(1));
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("overflow.ot:11:15: panic: integer overflow: 9223372036854775807 + 1"),
        "the message must name the operands and the position:\n{stderr}"
    );
    assert!(
        !stdout_of(&output).contains("-9223372036854775808"),
        "the operands must not have wrapped:\n{stderr}"
    );
}

#[test]
fn every_pending_defer_runs_before_a_panic_message_is_printed() {
    // Four handlers across three nesting levels, innermost first and
    // last-registered first within a scope. The message goes to stderr and the
    // handler output to stdout, so the two streams cannot be compared by
    // position in a captured `Output`; what proves the handlers ran before the
    // panic is that they ran at all -- the panic is the next statement, and the
    // program prints nothing else.
    let (_dir, exe, compile) = build_program("cleanup");
    assert!(
        compile.status.success(),
        "compiling cleanup.ot failed:\n{}",
        stderr_of(&compile)
    );
    let output = run(&exe);
    assert_eq!(
        stdout_of(&output),
        "inner\nmiddle 2\nmiddle 1\nouter\n",
        "all four defers, innermost scope first and reverse registration order \
         within a scope"
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_of(&output).contains("cleanup.ot:30:13: panic: cleanup"),
        "the panic its own position reaches stderr:\n{}",
        stderr_of(&output)
    );
}

#[test]
fn a_fault_deep_in_a_recursion_still_exits_through_the_diagnostic_path() {
    // 5000 non-tail frames. Whether the stack is unwound is a separate question
    // (it is not -- see Task 10); what this pins is that reaching the bottom
    // with 5000 frames alive ends the process through the runtime, with status
    // 1 and the message, rather than overflowing the C stack.
    let (_dir, exe, compile) = build_program("deeprecurse");
    assert!(
        compile.status.success(),
        "compiling deeprecurse.ot failed:\n{}",
        stderr_of(&compile)
    );
    let output = run(&exe);
    assert_eq!(stdout_of(&output), "");
    assert_eq!(
        output.status.code(),
        Some(1),
        "a stack overflow is not a defined stop; this must be the runtime's exit: {:?}",
        output.status
    );
    assert!(
        stderr_of(&output).contains("deeprecurse.ot:10:9: panic: bottom reached"),
        "the diagnostic from depth must reach stderr:\n{}",
        stderr_of(&output)
    );
}

#[test]
fn byte_at_reads_a_byte_and_answers_zero_out_of_range() {
    // The language half of ledger L1's `otter_str_byte_at`, end to end. `"abc"`
    // is the literal path (a static string, refcount -1) and `98` is `'b'`; the
    // out-of-range index is an answer (`0`), not a fault.
    let output = compile_and_run(
        "byte-at",
        "fn main() {\n\
         \x20   const abc = \"abc\";\n\
         \x20   print_i64(abc.byte_at(1));\n\
         \x20   print_str(\"\\n\");\n\
         \x20   print_i64(abc.byte_at(99));\n\
         \x20   print_str(\"\\n\");\n\
         }",
    );
    assert_eq!(stdout_of(&output), "98\n0\n");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn the_arithmetic_opt_outs_do_what_their_names_say() {
    // One program, all nine builtins, at the values where the three behaviours
    // differ from each other and from the checked default: the largest `i64`
    // plus one, the smallest minus one, and `i64::MAX` times two.
    //
    //   wrapping    gives the two's-complement answer -- `MAX + 1` is `MIN`
    //   saturating  clamps to the nearest end
    //   checked     says `None`, and the program decides
    //
    // `show` maps an absent answer to 999, a number none of the real answers is.
    let output = compile_and_run(
        "arithmetic-opt-outs",
        concat!(
            "fn show(o: Option[i64]) -> i64 {\n",
            "    return switch o {\n",
            "        case None { 999 }\n",
            "        case Some(v) { v }\n",
            "    };\n",
            "}\n",
            "\n",
            "fn main() {\n",
            "    var big = 9223372036854775807;\n",
            "    var small = -9223372036854775808;\n",
            "    print_i64(wrapping_add(big, 1));\n",
            "    print_str(\"\\n\");\n",
            "    print_i64(saturating_add(big, 1));\n",
            "    print_str(\"\\n\");\n",
            "    print_i64(wrapping_sub(small, 1));\n",
            "    print_str(\"\\n\");\n",
            "    print_i64(saturating_sub(small, 1));\n",
            "    print_str(\"\\n\");\n",
            "    print_i64(wrapping_mul(big, 2));\n",
            "    print_str(\"\\n\");\n",
            "    print_i64(saturating_mul(big, 2));\n",
            "    print_str(\"\\n\");\n",
            "    print_i64(saturating_mul(small, 0 - 2));\n",
            "    print_str(\"\\n\");\n",
            "    print_i64(saturating_mul(3, 4));\n",
            "    print_str(\"\\n\");\n",
            "    print_i64(show(checked_add(big, 1)));\n",
            "    print_str(\"\\n\");\n",
            "    print_i64(show(checked_add(1, 2)));\n",
            "    print_str(\"\\n\");\n",
            "    print_i64(show(checked_sub(small, 1)));\n",
            "    print_str(\"\\n\");\n",
            "    print_i64(show(checked_mul(big, 2)));\n",
            "    print_str(\"\\n\");\n",
            "}\n"
        ),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "none of these may stop the program: {}",
        stderr_of(&output)
    );
    assert_eq!(
        stdout_of(&output),
        "-9223372036854775808\n\
         9223372036854775807\n\
         9223372036854775807\n\
         -9223372036854775808\n\
         -2\n\
         9223372036854775807\n\
         9223372036854775807\n\
         12\n\
         999\n\
         3\n\
         999\n\
         999\n",
        "wrapping wraps, saturating clamps, checked says no"
    );
}

#[test]
fn the_saturating_multiply_clamps_at_both_ends_and_is_exact_inside() {
    // `smul.sat` is not implemented in LLVM, so this one is written by hand from
    // the overflow flag and the sign of the true product -- which is negative
    // exactly when the operands' signs differ. That predicate is only correct if
    // it is right for both ends and does not fire when the product fits, so all
    // four cases are pinned here.
    let output = compile_and_run(
        "saturating-mul-edges",
        concat!(
            "fn main() {\n",
            "    var big = 9223372036854775807;\n",
            "    var small = -9223372036854775808;\n",
            "    print_i64(saturating_mul(big, -1));\n",
            "    print_str(\"\\n\");\n",
            "    print_i64(saturating_mul(big, 2));\n",
            "    print_str(\"\\n\");\n",
            "    print_i64(saturating_mul(small, 2));\n",
            "    print_str(\"\\n\");\n",
            "    print_i64(saturating_mul(small, -1));\n",
            "    print_str(\"\\n\");\n",
            "    print_i64(saturating_mul(4611686018427387903, 2));\n",
            "    print_str(\"\\n\");\n",
            "    print_i64(saturating_mul(-3, 4));\n",
            "    print_str(\"\\n\");\n",
            "    print_i64(saturating_mul(6, 7));\n",
            "    print_str(\"\\n\");\n",
            "}\n"
        ),
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    assert_eq!(
        stdout_of(&output),
        "-9223372036854775807\n\
         9223372036854775807\n\
         -9223372036854775808\n\
         9223372036854775807\n\
         9223372036854775806\n\
         -12\n\
         42\n",
        "the clamp has to be right at both ends and silent in the middle"
    );
}

#[test]
fn arithmetic_that_fits_runs_to_completion() {
    // The check must not turn working programs into faults. A loop that adds to
    // a running total, an exact division, a signed remainder and a negation all
    // stay on the succeeding path -- and the loop is the interesting one, since
    // the check on `sum + i` is inside it.
    let output = compile_and_run(
        "arithmetic-ok",
        "fn main() {\n    var sum = 0;\n    for i in 0..10 {\n        sum = sum + i;\n    }\n    print_i64(sum);\n    print_str(\"\\n\");\n    print_i64(7 / 2);\n    print_str(\"\\n\");\n    print_i64(-7 % 2);\n    print_str(\"\\n\");\n    print_i64(-sum);\n    print_str(\"\\n\");\n}\n",
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "nothing here overflows: {}",
        stderr_of(&output)
    );
    assert_eq!(stdout_of(&output), "45\n3\n-1\n-45\n");
}

// -- assert (spec 6.2) --------------------------------------------------------

#[test]
fn assertions_that_hold_do_nothing_at_all() {
    // The passing path is a test and a branch: no message is built, nothing is
    // printed, and the program carries on. All four comparable types are here,
    // because each one branches through a different comparison.
    let output = compile_and_run(
        "assert-ok",
        concat!(
            "fn main() {\n",
            "    var n = 3;\n",
            "    assert(n > 0);\n",
            "    assert_eq(n, 3);\n",
            "    assert_ne(n, 4);\n",
            "    assert_eq(n * 2, 6);\n",
            "    assert_eq(1.5 + 0.5, 2.0);\n",
            "    assert_eq(true, true);\n",
            "    assert_eq(\"otter\", \"otter\");\n",
            "    assert_ne(\"otter\", \"other\");\n",
            "    print_str(\"done\\n\");\n",
            "}\n"
        ),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "nothing here fails: {}",
        stderr_of(&output)
    );
    assert_eq!(stdout_of(&output), "done\n");
    assert_eq!(
        stderr_of(&output),
        "",
        "an assertion that holds says nothing"
    );
}

#[test]
fn a_failed_assertion_quotes_the_condition_it_was_given() {
    // The message is the expression's own source text, which is the part a
    // program cannot supply for itself and a reader needs most: "assertion
    // failed" without the condition says nothing about what was expected.
    let output = compile_and_run(
        "assert-fail",
        concat!(
            "fn main() {\n",
            "    var n = 0;\n",
            "    print_str(\"before\\n\");\n",
            "    assert(n > 0);\n",
            "    print_str(\"after\\n\");\n",
            "}\n"
        ),
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "a failed assertion is a failure, not a normal exit: {:?}",
        output.status
    );
    assert_eq!(
        stdout_of(&output),
        "before\n",
        "output from before the fault must survive it, and nothing after may run"
    );
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("panic: assertion failed: n > 0"),
        "the condition belongs in the message:\n{stderr}"
    );
    assert!(
        stderr.contains("assert-fail.ot:4:5:"),
        "and so does the position it was written at:\n{stderr}"
    );
}

#[test]
fn assert_eq_reports_both_values_and_assert_ne_does_too() {
    // The point of the comparison forms: the two numbers, not just the fact that
    // they disagreed.
    let output = compile_and_run(
        "assert-eq-fail",
        concat!(
            "fn main() {\n",
            "    var n = 3;\n",
            "    assert_eq(n, 4);\n",
            "}\n"
        ),
    );
    assert_eq!(output.status.code(), Some(1));
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("panic: assertion failed: n == 4, left = 3, right = 4"),
        "both operands and the comparison belong in the message:\n{stderr}"
    );

    // `assert_ne` prints the same two values, which for this failure are equal:
    // that is exactly what a reader needs to see.
    let output = compile_and_run(
        "assert-ne-fail",
        concat!(
            "fn main() {\n",
            "    var n = 3;\n",
            "    assert_ne(n, 3);\n",
            "}\n"
        ),
    );
    assert_eq!(output.status.code(), Some(1));
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("panic: assertion failed: n != 3, left = 3, right = 3"),
        "the operator that was written must survive into the message:\n{stderr}"
    );
}

#[test]
fn a_comparison_assertion_prints_floats_and_strings_as_text() {
    let output = compile_and_run(
        "assert-types-fail",
        concat!(
            "fn main() {\n",
            "    assert_eq(1.5, 2.5);\n",
            "}\n"
        ),
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_of(&output).contains("assertion failed: 1.5 == 2.5, left = 1.5, right = 2.5"),
        "a float is printed the way the runtime prints one:\n{}",
        stderr_of(&output)
    );

    let output = compile_and_run(
        "assert-string-fail",
        concat!(
            "fn main() {\n",
            "    var s = \"abc\";\n",
            "    assert_eq(s, \"x\");\n",
            "}\n"
        ),
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_of(&output).contains("assertion failed: s == \"x\", left = abc, right = x"),
        "a string is quoted by its own expression and printed as its text:\n{}",
        stderr_of(&output)
    );
}

#[test]
fn an_asserted_operand_runs_exactly_once() {
    // The comparison and the message read the same value. Re-evaluating the
    // operand to print it would run its side effect twice -- and, on the failure
    // path, could print a number that is not the one that failed.
    let source = concat!(
        "fn tick() -> i64 {\n",
        "    print_str(\"tick\\n\");\n",
        "    return 1;\n",
        "}\n",
        "\n",
        "fn main() {\n",
        "    assert_eq(tick(), 1);\n",
        "    assert_eq(tick(), 2);\n",
        "}\n"
    );
    let output = compile_and_run("assert-once", source);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout_of(&output),
        "tick\ntick\n",
        "one `tick` per assertion: the second failing must not run it a third time"
    );
    assert!(
        stderr_of(&output).contains("left = 1, right = 2"),
        "the message reports the value that was compared:\n{}",
        stderr_of(&output)
    );
}

#[test]
fn a_release_build_compiles_debug_asserts_away_and_keeps_asserts() {
    // The one user-visible difference a build mode makes. A `debug_assert` is a
    // claim about the program that is worth checking while writing it and worth
    // nothing in front of a user; `assert` is a claim the program depends on.
    let source = concat!(
        "fn main() {\n",
        "    print_str(\"start\\n\");\n",
        "    var n = 0;\n",
        "    debug_assert(n > 0);\n",   // false, and dropped by --release
        "    print_str(\"checked\\n\");\n",
        "    assert(n == 0);\n",         // true, and kept by both
        "    print_str(\"done\\n\");\n",
        "}\n"
    );
    let release = compile_and_run_with("debug-release", source, &["--release"]);
    assert_eq!(
        release.status.code(),
        Some(0),
        "a debug build's assertion must not stop a release build: {}",
        stderr_of(&release)
    );
    assert_eq!(stdout_of(&release), "start\nchecked\ndone\n");

    let debug = compile_and_run_with("debug-debug", source, &[]);
    assert_eq!(
        debug.status.code(),
        Some(1),
        "and a debug build must still stop on it"
    );
    assert_eq!(
        stdout_of(&debug),
        "start\n",
        "nothing after the failed assertion may run"
    );
    assert!(
        stderr_of(&debug).contains("panic: assertion failed: n > 0"),
        "with the same message a debug build always had:\n{}",
        stderr_of(&debug)
    );
    assert!(
        stderr_of(&debug).contains("debug-debug.ot:4:5:"),
        "reported at the position it was written at:\n{}",
        stderr_of(&debug)
    );
}

#[test]
fn the_release_flag_reaches_the_ir_dump() {
    // `--emit-ir` is a front-end path of its own, taken before the toolchain is
    // even looked for, so the flag has to be plumbed there too. The proof is in
    // the text: the debug assertion's message is not interned and its branch is
    // gone, while `assert`'s is untouched.
    let dir = work_dir("release-ir");
    let file = dir.join("release-ir.ot");
    fs::write(
        &file,
        concat!(
            "fn main() {\n",
            "    var n = 0;\n",
            "    debug_assert(n > 0);\n",
            "    assert(n == 0);\n",
            "}\n"
        ),
    )
    .expect("writing the test program");
    let dump = |extra: &[&str]| {
        let mut command = Command::new(otter());
        command.arg("build").arg(&file).arg("--emit-ir").args(extra);
        let output = command.output().expect("run the otter CLI");
        assert!(
            output.status.success(),
            "--emit-ir {extra:?} failed:\n{}",
            stderr_of(&output)
        );
        stdout_of(&output)
    };

    let debug = dump(&[]);
    assert!(
        debug.contains("assertion failed: n > 0"),
        "a debug build emits both assertions:\n{debug}"
    );
    let release = dump(&["--release"]);
    assert!(
        !release.contains("assertion failed: n > 0"),
        "a release build drops the debug one:\n{release}"
    );
    assert!(
        release.contains("assertion failed: n == 0"),
        "and keeps `assert`:\n{release}"
    );
}

#[test]
fn libc_is_reachable_from_otter() {
    // The whole FFI chain in one program: declaration, copied string argument,
    // `usize` cast in and out, a linear C pointer through `malloc`/`free`, and
    // a link directive for the platform that keeps libm separate.
    let (_dir, exe, built) = build_program("libc");
    assert!(
        built.status.success(),
        "libc.ot must build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let output = run(&exe);
    assert!(
        output.status.success(),
        "libc.ot must exit 0; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(stdout_of(&output), "hello from C\n5\n");
}

#[test]
fn a_slice_literal_grows_and_is_freed_at_run_time() {
    // `slices.ot`: a two-element literal, two pushes (the second grows the
    // backing array, which the runtime implements by allocating, copying and
    // freeing the old block), then one `free_slice`. Only a link-and-run shows
    // the generated header and `otter_slice` agree; `leaks.rs` proves the
    // reallocation handed the old block back.
    let output = build_and_run("slices");
    assert_eq!(stdout_of(&output), "slices freed\n");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn a_deferred_body_runs_at_its_own_scope_exit() {
    // `defer_scope.ot` writes its `defer` before an `if`, a slice loop and a
    // range loop. The deferred body must run exactly once, after "after\n", and
    // not at the end of the `if` branch or of either iteration.
    let output = build_and_run("defer_scope");
    assert_eq!(stdout_of(&output), "if\n7 8 0 1 after\ndeferred\n");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn slice_operations_answer_in_options_and_iterate_in_order() {
    // `slices_ops.ot` prints, in order: the length, the first and last
    // elements, `None` for both ends of the range, the sum of every element, the
    // count before a `break`, zero iterations of an empty slice, the length
    // after three pushes made from inside the loop, two popped values, and the
    // length that is left. The pushes inside the loop are the important ones:
    // they prove the condition re-reads the header instead of caching it.
    let output = build_and_run("slices_ops");
    assert_eq!(
        stdout_of(&output),
        "4\n10\n40\n-1\n-1\n100\n1\n0\n4\n40\n30\n2\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn maps_insert_overwrite_remove_and_measure() {
    // `maps.ot` inserts "apple" twice (the second value wins), removes "pear",
    // then asks for "pear" again: a removed key is indistinguishable from one
    // that was never there, which is what `Option` is for. The final count is
    // one, and every line is a separate witness.
    let output = build_and_run("maps");
    assert_eq!(stdout_of(&output), "5\n3\nnone\n1\n");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn a_string_slice_can_be_built_iterated_and_freed() {
    // `string_slices.ot` starts from a two-element literal, pushes a third
    // literal and a freshly built number, then prints all four in order. The
    // elements are handles the slice retains, so the iteration prints text that
    // is still owned while `free_slice` takes the references back.
    let output = build_and_run("string_slices");
    assert_eq!(stdout_of(&output), "alpha\nbeta\ngamma\n42\n");
    assert_eq!(output.status.code(), Some(0));
}

/// Build `name.ot` and run it with `input` on standard input.
fn build_and_run_with_input(name: &str, input: &str) -> Output {
    let (_dir, exe, compile) = build_program(name);
    assert!(
        compile.status.success(),
        "compiling {name}.ot failed:\n{}",
        stderr_of(&compile)
    );
    let output = run_with_input(&exe, input);
    assert!(
        output.status.success(),
        "{name} exited with {:?}\nstderr:\n{}",
        output.status.code(),
        stderr_of(&output)
    );
    output
}

#[test]
fn a_multi_file_program_counts_json_keys() {
    // `json_count.ot` and `json_keys.ot`: the entry feeds every line into a
    // `[]string`, the module slices the quoted run out of each line, and a
    // `map[string]i64` deduplicates. Three lines, three keys, two distinct.
    let output = build_and_run_with_input(
        "json_count",
        "{\"name\": \"otter\"}\n{\"age\": 3}\n{\"name\": \"again\"}\n",
    );
    assert_eq!(stdout_of(&output), "3\n2\n");
}

#[test]
fn a_word_count_program_works_on_a_multi_line_input() {
    // `wc.ot` and `wc_words.ot`: two lines, four words, and 24 bytes counting
    // the two newlines `read_line` removed. The helper module owns the word
    // boundary rule, which is the part worth reading twice.
    let output = build_and_run_with_input("wc", "hello world\nsecond line\n");
    assert_eq!(stdout_of(&output), "2 4 24\n");
}

// --- M7: concurrency end to end -------------------------------------------------
//
// These four are the only tests that run more than one OS thread through the
// whole pipeline: two of them prove an exact answer, one proves that closing a
// channel releases a blocked sender rather than hanging, and one repeats a
// concurrent program to catch an answer that is only right sometimes.

#[test]
fn a_task_pool_computes_a_correct_sum() {
    // Eight workers, 10_000 jobs, one answer that cannot depend on scheduling:
    // 0 + 1 + ... + 9999.
    let (_dir, exe, compile) = build_program("pool");
    assert!(
        compile.status.success(),
        "compiling pool.ot failed:\n{}",
        stderr_of(&compile)
    );
    let output = run_with_timeout(&exe, CONCURRENT_TIMEOUT);
    assert!(
        output.status.success(),
        "pool exited with {:?}",
        output.status.code()
    );
    assert_eq!(stdout_of(&output), "49995000\n");
}

#[test]
fn a_ping_pong_between_two_threads_terminates() {
    // 10_000 alternating round trips over two capacity-one channels. A lost
    // wakeup on either condition variable would deadlock the join, which is
    // what the timeout turns into a test failure.
    let (_dir, exe, compile) = build_program("pingpong");
    assert!(
        compile.status.success(),
        "compiling pingpong.ot failed:\n{}",
        stderr_of(&compile)
    );
    let output = run_with_timeout(&exe, CONCURRENT_TIMEOUT);
    assert!(
        output.status.success(),
        "pingpong exited with {:?}",
        output.status.code()
    );
    assert_eq!(stdout_of(&output), "pingpong ok\n");
}

#[test]
fn closing_a_channel_unblocks_a_sender() {
    // The producer's second send to the capacity-one `slot` blocks until the
    // main thread closes that channel; close must answer 0 and let the send
    // return. The join is the proof that it did.
    let (_dir, exe, compile) = build_program("close_sender");
    assert!(
        compile.status.success(),
        "compiling close_sender.ot failed:\n{}",
        stderr_of(&compile)
    );
    let output = run_with_timeout(&exe, CONCURRENT_TIMEOUT);
    assert!(
        output.status.success(),
        "close_sender exited with {:?}",
        output.status.code()
    );
    assert_eq!(stdout_of(&output), "full\nunblocked\n");
}

#[test]
fn the_concurrent_programs_are_deterministic_across_ten_runs() {
    // A worker pool is the shape where a race shows up as an occasionally
    // wrong sum, so one successful run proves little. Ten runs of the same
    // binary must print the same bytes every time.
    let (_dir, exe, compile) = build_program("pool");
    assert!(
        compile.status.success(),
        "compiling pool.ot failed:\n{}",
        stderr_of(&compile)
    );
    let first = run_with_timeout(&exe, CONCURRENT_TIMEOUT);
    assert!(
        first.status.success(),
        "the first pool run exited with {:?}",
        first.status.code()
    );
    for run_number in 1..10 {
        let output = run_with_timeout(&exe, CONCURRENT_TIMEOUT);
        assert!(
            output.status.success(),
            "pool run {run_number} exited with {:?}",
            output.status.code()
        );
        assert_eq!(
            stdout_of(&output),
            stdout_of(&first),
            "pool run {run_number} printed a different answer"
        );
    }
}


// --- M8: the formatter's command line ---------------------------------------------

#[test]
fn fmt_rewrites_a_file_in_place_and_refuses_a_broken_one() {
    // `-w` is the only part of `otter fmt` a library test cannot prove: that a
    // good file is replaced with the canonical text, and a file that does not
    // parse is left byte for byte as it was.
    let dir = work_dir("fmt-cli");

    let messy = dir.join("messy.ot");
    fs::write(&messy, "fn  main( ){print_str(\"hi\") ;}\n")
        .expect("the test file must be writable");
    let output = Command::new(otter())
        .arg("fmt")
        .arg("-w")
        .arg(&messy)
        .output()
        .expect("the otter CLI must be runnable");
    assert!(
        output.status.success(),
        "fmt -w failed:\n{}",
        stderr_of(&output)
    );
    assert_eq!(
        fs::read_to_string(&messy).expect("the formatted file must be readable"),
        "fn main() {\n    print_str(\"hi\");\n}\n"
    );

    let broken = dir.join("broken.ot");
    fs::write(&broken, "fn main( {\n").expect("the test file must be writable");
    let output = Command::new(otter())
        .arg("fmt")
        .arg("-w")
        .arg(&broken)
        .output()
        .expect("the otter CLI must be runnable");
    assert!(
        !output.status.success(),
        "a file that does not parse must fail"
    );
    assert_eq!(
        fs::read_to_string(&broken).expect("the untouched file must be readable"),
        "fn main( {\n"
    );
}
