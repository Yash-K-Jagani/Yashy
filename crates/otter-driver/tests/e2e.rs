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
