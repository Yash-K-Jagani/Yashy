//! The standard library: the builtin-surface modules (`io`, `string`, `math`,
//! `os`, `alloc`) and the Plan B modules (`fs`, `path`, `args`, `env`,
//! `process`, `time`, `rand`, `JSON`), written in Yashy and compiled by the
//! compiler they ship with. The R1 process builtins -- `args()`, `getcwd()`,
//! `now()`, and `sleep(ms)` -- are in scope without an import; the `args`,
//! `time`, and `os` modules document them.
//!
//! Most programs here import the modules through `YASHY_PATH` so the test can
//! pin the exact source tree it is exercising. The loader also discovers the
//! bundled stdlib automatically; `stdlib_is_found_without_yashy_path` covers
//! that path on its own.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The `yashy` binary cargo built for this test run.
fn yashy() -> &'static str {
    common::yashy()
}

/// `crates/yashy-stdlib/yashy`, the directory holding `io.ya` and friends.
fn stdlib_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("yashy-stdlib")
        .join("yashy")
}

fn work_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yashy-stdlib-{}-{name}", std::process::id()));
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
    Command::new(yashy())
        .arg("build")
        .arg(source)
        .arg("-o")
        .arg(exe)
        .env("YASHY_PATH", stdlib_dir())
        .output()
        .expect("the yashy CLI must be runnable")
}

/// Run a built program in `dir`, with the leak report and one variable set.
fn run(dir: &Path, exe: &Path) -> Output {
    common::launch_retrying(exe, |path| {
        Command::new(path)
            .current_dir(dir)
            .env("YASHY_LEAK_REPORT", "1")
            .env("YASHY_STDLIB_PROBE", "from-env")
            .output()
    })
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
    let padded = \"  padded \\n\";
    println(padded.trim());
    let hello = \"hello\";
    if hello.contains(\"ell\") { println(\"contains\"); }
    if hello.starts_with(\"he\") { println(\"starts\"); }
    let csv = \"a,b,c\";
    let parts = csv.split(\",\");
    println_i64(parts.len());
    for part in parts { println(part); }
    parts.free();
    let one = alloc_one(5);
    println_i64(one[0]);
    one.free();
    let zeros = alloc_zeroed(3);
    println_i64(zeros[0] + zeros[2]);
    zeros.free();
    println(getenv(\"YASHY_STDLIB_PROBE\"));
}
";

#[test]
fn every_module_is_a_program_the_checker_accepts_together() {
    // All five modules in one program: this is also the collision check, because
    // a duplicate top-level name across modules is an error. `check` needs no C
    // toolchain, so the test still says something on a machine without clang.
    let dir = work_dir("check");
    write(&dir, "main.ya", DEMO);
    let output = Command::new(yashy())
        .arg("check")
        .arg(dir.join("main.ya"))
        .env("YASHY_PATH", stdlib_dir())
        .output()
        .expect("the yashy CLI must be runnable");
    assert!(
        output.status.success(),
        "the stdlib must check:\n{}",
        stderr_of(&output)
    );
}

#[test]
fn the_text_and_number_helpers_produce_the_expected_report() {
    let dir = work_dir("demo");
    write(&dir, "main.ya", DEMO);
    let program = exe(&dir, "main");
    let compile = build(&dir.join("main.ya"), &program);
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
        stderr_of(&output).contains("yashy: leak report: 0 live allocations"),
        "every helper must release what it allocates:\n{}",
        stderr_of(&output)
    );
}

const FILE_ROUND_TRIP: &str = "\
import io;
import string;

fn main() {
    let wrote = write_file(\"out.txt\", \"line one\\nline two\\n\");
    println_i64(wrote);
    let text = read_file(\"out.txt\");
    println_i64(text.len());
    if text.contains(\"two\") { println(\"two\"); }
    let lines = text.split(\"\\n\");
    println_i64(lines.len());
    for line in lines { println(line); }
    lines.free();
}
";

#[test]
fn a_file_round_trips_through_the_io_and_string_helpers() {
    // The program runs with its own directory as the working directory, so the
    // relative `out.txt` it writes lands beside it rather than in the repo.
    let dir = work_dir("files");
    write(&dir, "main.ya", FILE_ROUND_TRIP);
    let program = exe(&dir, "main");
    let compile = build(&dir.join("main.ya"), &program);
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
        stderr_of(&output).contains("yashy: leak report: 0 live allocations"),
        "the file text and every part are released:\n{}",
        stderr_of(&output)
    );

    let written = fs::read_to_string(dir.join("out.txt")).expect("the program wrote the file");
    assert_eq!(written, "line one\nline two\n");
}

#[test]
fn stdlib_is_found_without_yashy_path() {
    let dir = work_dir("bundled");
    write(
        &dir,
        "main.ya",
        "import io;\nfn main() { println(\"ok\"); }\n",
    );
    let output = Command::new(yashy())
        .arg("build")
        .arg(dir.join("main.ya"))
        .arg("-o")
        .arg(exe(&dir, "main"))
        .env_remove("YASHY_PATH")
        .env_remove("YASHY_STDLIB")
        .output()
        .expect("the yashy CLI must be runnable");

    assert!(
        output.status.success(),
        "the bundled stdlib must be discoverable without YASHY_PATH:\n{}",
        stderr_of(&output)
    );
}

const EVERY_MODULE: &str = "\
import io;
import string;
import math;
import os;
import alloc;
import fs;
import path;
import args;
import env;
import process;
import time;
import rand;
import JSON;

fn main() {
    println(join(\"a\", \"b\"));
    println(join(\"a/\", \"/b\"));
    println(join(\"\", \"b\"));
    println(join(\"a\", \"\"));
    println(basename(\"/usr/local/bin/yashy\"));
    println(basename(\"/\"));
    println(basename(\"\"));
    println(dirname(\"/usr/local/bin/yashy\"));
    println(dirname(\"\"));
    println(extension(\"archive.tar.gz\"));
    println(extension(\".gitignore\"));
    println(quote(\"a\\\"b\\n\"));
    let first = next(1);
    let second = next(1);
    if first == second { println(\"deterministic\"); }
    if first >= 0 { println(\"non-negative\"); }
    let argv = args();
    println_i64(argv.len());
    argv.free();
    let here = getcwd();
    if here.len() > 0 { println(\"cwd\"); }
    if now() > 0 { println(\"clock\"); }
    sleep(0);
}
";

#[test]
fn every_stdlib_module_imports_and_the_path_and_json_helpers_run() {
    // Every module the plan names, in one program. Before this test the Plan B
    // modules were not imported anywhere: seven of them did not even parse, and
    // the two builtin-wrapping ones would have been duplicate definitions. The
    // test is the collision check as well as the behaviour check.
    let dir = work_dir("every-module");
    write(&dir, "main.ya", EVERY_MODULE);
    let program = exe(&dir, "main");
    let compile = build(&dir.join("main.ya"), &program);
    assert!(
        compile.status.success(),
        "every stdlib module must compile together:\n{}",
        stderr_of(&compile)
    );

    let output = run(&dir, &program);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    assert_eq!(
        stdout_of(&output),
        "a/b\na/b\nb\na\nyashy\n/\n\n/usr/local/bin\n.\ngz\n\n\"a\\\"b\\n\"\ndeterministic\nnon-negative\n0\ncwd\nclock\n"
    );
    assert!(
        stderr_of(&output).contains("yashy: leak report: 0 live allocations"),
        "the helpers must release what they allocate:\n{}",
        stderr_of(&output)
    );
}
