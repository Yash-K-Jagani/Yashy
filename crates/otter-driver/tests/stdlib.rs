//! The core standard library (M6 Task 10): `io`, `string`, `math`, `os` and
//! `alloc`, written in Otter and compiled by the compiler they ship with.
//!
//! Every program here imports the modules through `OTTER_PATH`, which is how a
//! program finds them today: the loader's roots are the entry file's directory
//! unless the variable says otherwise, and the stdlib is not copied next to a
//! user's program. Locating a bundled stdlib automatically is recorded as a
//! follow-up in the plan.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The `otter` binary cargo built for this test run.
fn otter() -> &'static str {
    env!("CARGO_BIN_EXE_otter")
}

/// `crates/otter-stdlib/otter`, the directory holding `io.ot` and friends.
fn stdlib_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("otter-stdlib")
        .join("otter")
}

fn work_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("otter-stdlib-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("the test work directory must be creatable");
    dir
}

fn write(dir: &Path, name: &str, text: &str) {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("the test source directory must be creatable");
    }
    fs::write(&path, text).expect("the test source must be writable");
}

fn exe(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Build `source` with the stdlib on the module path.
fn build(source: &Path, exe: &Path) -> Output {
    Command::new(otter())
        .arg("build")
        .arg(source)
        .arg("-o")
        .arg(exe)
        .env("OTTER_PATH", stdlib_dir())
        .output()
        .expect("the otter CLI must be runnable")
}

/// Run a built program in `dir`, with the leak report and one variable set.
fn run(dir: &Path, exe: &Path) -> Output {
    Command::new(exe)
        .current_dir(dir)
        .env("OTTER_LEAK_REPORT", "1")
        .env("OTTER_STDLIB_PROBE", "from-env")
        .output()
        .expect("the compiled program must be runnable")
}

const DEMO: &str = "\
import io;
import string;
import math;
import os;
import alloc;

fn main() {
    println(\"hello\");
    println_i64(abs(-7));
    println_i64(min(3, 4));
    println_i64(max(3, 4));
    println_i64(pow(2, 10));
    println_f64(sqrt(9.0));
    println(concat(\"a\", \"b\"));
    println(trim(\"  padded \\n\"));
    if contains(\"hello\", \"ell\") { println(\"contains\"); }
    if starts_with(\"hello\", \"he\") { println(\"starts\"); }
    var parts = split(\"a,b,c\", \",\");
    println_i64(len(parts));
    for part in parts { println(part); }
    free_slice(parts);
    var one = alloc_one(5);
    println_i64(one[0]);
    dealloc(one);
    var zeros = alloc_zeroed(3);
    println_i64(zeros[0] + zeros[2]);
    dealloc(zeros);
    println(getenv(\"OTTER_STDLIB_PROBE\"));
}
";

#[test]
fn every_module_is_a_program_the_checker_accepts_together() {
    // All five modules in one program: this is also the collision check, because
    // a duplicate top-level name across modules is an error. `check` needs no C
    // toolchain, so the test still says something on a machine without clang.
    let dir = work_dir("check");
    write(&dir, "main.ot", DEMO);
    let output = Command::new(otter())
        .arg("check")
        .arg(dir.join("main.ot"))
        .env("OTTER_PATH", stdlib_dir())
        .output()
        .expect("the otter CLI must be runnable");
    assert!(
        output.status.success(),
        "the stdlib must check:\n{}",
        stderr_of(&output)
    );
}

#[test]
fn the_text_and_number_helpers_produce_the_expected_report() {
    let dir = work_dir("demo");
    write(&dir, "main.ot", DEMO);
    let program = exe(&dir, "main");
    let compile = build(&dir.join("main.ot"), &program);
    assert!(
        compile.status.success(),
        "the stdlib program must compile:\n{}",
        stderr_of(&compile)
    );

    let output = run(&dir, &program);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    assert_eq!(
        stdout_of(&output),
        "hello\n7\n3\n4\n1024\n3\nab\npadded\ncontains\nstarts\n3\na\nb\nc\n5\n0\nfrom-env\n"
    );
    assert!(
        stderr_of(&output).contains("otter: leak report: 0 live allocations"),
        "every helper must release what it allocates:\n{}",
        stderr_of(&output)
    );
}

const FILE_ROUND_TRIP: &str = "\
import io;
import string;

fn main() {
    var wrote = write_file(\"out.txt\", \"line one\\nline two\\n\");
    println_i64(wrote);
    var text = read_file(\"out.txt\");
    println_i64(len(text));
    if contains(text, \"two\") { println(\"two\"); }
    var lines = split(text, \"\\n\");
    println_i64(len(lines));
    for line in lines { println(line); }
    free_slice(lines);
}
";

#[test]
fn a_file_round_trips_through_the_io_and_string_helpers() {
    // The program runs with its own directory as the working directory, so the
    // relative `out.txt` it writes lands beside it rather than in the repo.
    let dir = work_dir("files");
    write(&dir, "main.ot", FILE_ROUND_TRIP);
    let program = exe(&dir, "main");
    let compile = build(&dir.join("main.ot"), &program);
    assert!(
        compile.status.success(),
        "the file program must compile:\n{}",
        stderr_of(&compile)
    );

    let output = run(&dir, &program);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    // `split` keeps the part after the final separator, so a trailing newline
    // yields a third, empty line. That is the documented contract, not an
    // accident: the parts together are exactly the input.
    assert_eq!(
        stdout_of(&output),
        "1\n18\ntwo\n3\nline one\nline two\n\n"
    );
    assert!(
        stderr_of(&output).contains("otter: leak report: 0 live allocations"),
        "the file text and every part are released:\n{}",
        stderr_of(&output)
    );

    let written = fs::read_to_string(dir.join("out.txt")).expect("the program wrote the file");
    assert_eq!(written, "line one\nline two\n");
}
