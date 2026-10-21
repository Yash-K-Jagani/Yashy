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

/// Build `name.ot` and return the work directory and the executable.
fn build(name: &str) -> (PathBuf, PathBuf) {
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
    (dir, exe)
}

/// The error a program that refuses to start should report, with the Device
/// Guard caveat when that is what happened.
fn start_failure(exe: &Path, error: &std::io::Error) -> String {
    // The same Device Guard caveat as `e2e.rs`: a freshly linked executable can
    // be blocked by content policy, which is an environment result rather than a
    // compiler defect.
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
    format!(
        "the executable at {} must be runnable: {error}{hint}",
        exe.display()
    )
}

/// Run a built program with the leak report enabled, optionally feeding standard
/// input. The write end is closed before waiting, so a read-until-EOF loop ends
/// instead of blocking on a pipe nobody will write to again.
fn run_program(exe: &Path, input: Option<&str>) -> Output {
    let mut command = Command::new(exe);
    command.env("OTTER_LEAK_REPORT", "1");
    if let Some(input) = input {
        use std::io::Write;
        use std::process::Stdio;

        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|error| panic!("{}", start_failure(exe, &error)));
        child
            .stdin
            .as_mut()
            .expect("standard input was piped")
            .write_all(input.as_bytes())
            .expect("writing the program's input must succeed");
        drop(child.stdin.take());
        return child
            .wait_with_output()
            .unwrap_or_else(|error| panic!("{}", start_failure(exe, &error)));
    }
    command
        .output()
        .unwrap_or_else(|error| panic!("{}", start_failure(exe, &error)))
}

/// Build `name.ot` and run it with the leak report enabled.
fn build_and_run(name: &str) -> Output {
    let (_dir, exe) = build(name);
    run_program(&exe, None)
}

/// Build `name.ot` and run it with `input` on standard input and the leak report
/// enabled.
fn build_and_run_with_input(name: &str, input: &str) -> Output {
    let (_dir, exe) = build(name);
    run_program(&exe, Some(input))
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

#[test]
fn a_slice_returns_its_backing_array() {
    // `slices.ot` starts with a two-element literal and pushes twice. The
    // second push grows the array from two elements to four: the runtime
    // allocates the larger block, copies, and frees the first one. `free_slice`
    // then releases the surviving block, so the reported live count is zero
    // only if both blocks came back exactly once.
    let output = build_and_run("slices");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout, "slices freed\n");
    assert_no_leaks("slices", &output);
}

#[test]
fn slice_operations_return_every_backing_array() {
    // `slices_ops.ot` grows `growing` from one element to four inside the loop
    // (three reallocations, each freeing the block it replaced), pops elements
    // without allocating, and frees all three slices. A non-zero count here
    // means a grow or a `free_slice` missed a block.
    let output = build_and_run("slices_ops");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout, "4\n10\n40\n-1\n-1\n100\n1\n0\n4\n40\n30\n2\n",
        "the program prints its witnesses before exiting:\n{stdout}"
    );
    assert_no_leaks("slices_ops", &output);
}

#[test]
fn a_map_releases_every_key_it_retained() {
    // `maps_heap.ot` builds two hundred string keys, inserts each one, and lets
    // the binding go at the end of its iteration. The table's own reference is
    // what keeps each key alive, and `map_free` must drop exactly those two
    // hundred. A non-zero count means a key was retained and never released, or
    // retained twice on an overwrite.
    let output = build_and_run("maps_heap");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout, "150\n200\n",
        "the program prints its witnesses before exiting:\n{stdout}"
    );
    assert_no_leaks("maps_heap", &output);
}

#[test]
fn a_closure_releases_its_environment() {
    // `closures.ot` makes two closures: one captures a number, the other a
    // string. The environment is a block of its own, and the string in it is a
    // reference the environment owns. Both have to come back, or the report
    // counts them.
    let output = build_and_run("closures");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout, "15\nhello\n",
        "the program prints its witnesses before exiting:\n{stdout}"
    );
    assert_no_leaks("closures", &output);
}

#[test]
fn the_higher_order_methods_release_what_they_build() {
    // `higher_order.ot` maps, filters and folds, and frees the arrays it
    // built. The mapped `[]string` holds three strings the runtime built, so
    // the report proves the new array's elements came back with it.
    let output = build_and_run("higher_order");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout, "2\n4\n6\n3\n6\n1\n2\n3\n",
        "the program prints its witnesses before exiting:\n{stdout}"
    );
    assert_no_leaks("higher_order", &output);
}

#[test]
fn a_string_slice_releases_every_element() {
    // `string_slices_heap.ot` pushes one hundred freshly built strings. The
    // slice owns a reference to each, so `free_slice` must release all hundred
    // and the backing array: anything less shows up as a live allocation.
    let output = build_and_run("string_slices_heap");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout, "190\n",
        "the sum of the decimal lengths of 0..99:\n{stdout}"
    );
    assert_no_leaks("string_slices_heap", &output);
}

#[test]
fn both_container_programs_leak_nothing() {
    // The two M6 showcase programs, with the report armed. `json_count` builds a
    // `[]string` of lines and a `map[string]i64` of keys; `wc` loops over lines
    // and counts. Both must release the slice, the map (including the key
    // references the map retained), and every intermediate string.
    let json = build_and_run_with_input(
        "json_count",
        "{\"name\": \"otter\"}\n{\"age\": 3}\n{\"name\": \"again\"}\n",
    );
    assert_eq!(
        String::from_utf8_lossy(&json.stdout),
        "3\n2\n",
        "three keys seen, two distinct"
    );
    assert_no_leaks("json_count", &json);

    let wc = build_and_run_with_input("wc", "hello world\nsecond line\n");
    assert_eq!(
        String::from_utf8_lossy(&wc.stdout),
        "2 4 24\n",
        "two lines, four words, twenty-four bytes"
    );
    assert_no_leaks("wc", &wc);
}
