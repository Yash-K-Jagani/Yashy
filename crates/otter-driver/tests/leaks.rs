//! Leak checks: a program that releases what it allocates reports zero live
//! allocations at exit.
//!
//! The checker is built into the runtime rather than borrowed from the host
//! (`valgrind` on Linux, the debug CRT on Windows), for two reasons:
//!
//! - The two surfaces the compiler inserts ownership for -- manual
//!   `alloc`/`dealloc` and refcounted strings -- both go through the runtime's
//!   own `malloc`/`free`, so the runtime knows exactly which allocations are
//!   still live. No malloc interposer is needed.
//! - It works on every machine the compiler itself runs on, so the check is
//!   never silently skipped. The plan allowed a printed skip when no external
//!   checker was installed; a check that always runs is strictly better.
//!
//! Set `OTTER_LEAK_REPORT=1` and the process prints one line to stderr at exit
//! (including the exit a panic takes):
//!
//! ```text
//! otter: leak report: 0 live allocations
//! ```
//!
//! Zero is the only acceptable count. The printed output is asserted too, so a
//! program that released everything by doing nothing cannot pass.

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
    let dir = std::env::temp_dir().join(format!("otter-leaks-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("the test work directory must be creatable");
    dir
}

/// Build `name.ot` and run it with the leak report enabled.
fn build_and_run(name: &str) -> Output {
    let dir = work_dir(name);
    let source = programs_dir().join(format!("{name}.ot"));
    let exe = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    let compile = Command::new(otter())
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("the otter CLI must be runnable");
    assert!(
        compile.status.success(),
        "compiling {name}.ot failed:\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );

    Command::new(&exe)
        .env("OTTER_LEAK_REPORT", "1")
        .output()
        .unwrap_or_else(|error| {
            // The same Device Guard caveat as `e2e.rs`: a freshly linked
            // executable can be blocked by content policy, which is an
            // environment result rather than a compiler defect.
            let policy = error.to_string().contains("blocked")
                || error.to_string().contains("Application Control")
                || error.raw_os_error() == Some(4551);
            let hint = if policy {
                "\n\nthe operating system refused to start the binary: an \
                 Application Control / Device Guard policy blocked it. Rebuilding \
                 produces a different binary (the linker stamps a timestamp)."
            } else {
                ""
            };
            panic!("the executable at {} must be runnable: {error}{hint}", exe.display())
        })
}

/// Assert the program exited zero and reported no live allocations.
fn assert_no_leaks(name: &str, output: &Output) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "{name} exited with {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status.code()
    );
    assert!(
        stderr.contains("otter: leak report: 0 live allocations"),
        "{name} must report zero live allocations; a missing line means the \
         runtime never armed the report, and a nonzero count is a leak:\n{stderr}"
    );
}

#[test]
fn a_manual_buffer_returns_every_allocation() {
    // `buffer.ot` allocates 1024 `i64`s, fills and sums them through pointer
    // indexing, and releases the block with `defer`. The sum proves the memory
    // was really written and read; the report proves the block came back.
    let output = build_and_run("buffer");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout, "1047552\n",
        "2 * (0 + ... + 1023) is 1023 * 1024:\n{stdout}"
    );
    assert_no_leaks("buffer", &output);
}

#[test]
fn built_strings_return_every_reference() {
    // `builder.ot` rebinds an accumulator 100 times and builds 100 fresh
    // strings, each of which the loop body's scope releases. The lengths prove
    // the values are right; the report proves the references came back. Before
    // the scope-exit release, this count grew with the loop.
    let output = build_and_run("builder");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout, "100\n7\n",
        "a 100-byte accumulator and the last `item 99`:\n{stdout}"
    );
    assert_no_leaks("builder", &output);
}
