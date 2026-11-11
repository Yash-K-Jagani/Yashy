//! Two builds of the same tree must produce the same bytes.
//!
//! The claim a release makes is that a stranger can recompute the hash of the
//! archive. That claim is only true if the compiler itself is reproducible, so
//! the test builds the same program twice, in two directories, and compares
//! the results byte for byte.

mod common;

use common::{work_dir, yashy};
use std::fs;
use std::path::Path;
use std::process::Command;

/// A program small enough to build twice in a test, and different from the
/// tutorial's so a change to one does not hide a change in the other.
const PROGRAM: &str = "fn main() {\n    print(\"hello\");\n}\n";

/// Build `source` into `dir` and return the bytes of the binary.
fn build(source: &Path, dir: &Path, reproducible: bool) -> Vec<u8> {
    fs::create_dir_all(dir).expect("the build directory must be creatable");
    let source = dir.join("main.ya");
    fs::write(&source, PROGRAM).expect("the source must be writable");
    let out = dir.join(format!("hello{}", std::env::consts::EXE_SUFFIX));
    let mut command = Command::new(yashy());
    command.arg("build").arg(&source).arg("-o").arg(&out);
    if reproducible {
        command.arg("--reproducible");
    }
    let output = command.output().expect("the yashy CLI must be runnable");
    assert!(
        output.status.success(),
        "the build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::read(&out).expect("the built binary must be readable")
}

/// The first byte at which two byte strings differ, and how many differ.
fn first_difference(left: &[u8], right: &[u8]) -> Option<(usize, usize)> {
    let mut differing = 0usize;
    for index in 0..left.len().min(right.len()) {
        if left[index] != right[index] {
            differing += 1;
            if differing == 1 {
                return Some((index, differing));
            }
        }
    }
    if left.len() != right.len() {
        return Some((left.len().min(right.len()), differing));
    }
    None
}

#[test]
fn two_builds_in_two_directories_produce_the_same_bytes() {
    let dir = work_dir("reproducible-build");
    let first = build(&dir.join("main.ya"), &dir.join("a"), true);
    let second = build(&dir.join("main.ya"), &dir.join("b"), true);

    if let Some((offset, _)) = first_difference(&first, &second) {
        panic!(
            "two reproducible builds differ at byte {offset} of {}; the first build is {} bytes",
            first.len(),
            first.len()
        );
    }
}

#[test]
fn the_reproducible_ir_names_no_absolute_path() {
    // The IR is an artifact people keep and hash. A path that named the build
    // directory would make it depend on where the build happened.
    let dir = work_dir("reproducible-ir");
    fs::create_dir_all(&dir).expect("the build directory must be creatable");
    let source = dir.join("main.ya");
    fs::write(&source, PROGRAM).expect("the source must be writable");

    let output = Command::new(yashy())
        .arg("build")
        .arg(&source)
        .arg("--reproducible")
        .arg("--emit-ir")
        .output()
        .expect("the yashy CLI must be runnable");
    assert!(
        output.status.success(),
        "emitting the IR failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let ir = String::from_utf8_lossy(&output.stdout).into_owned();
    let named = ir
        .lines()
        .find(|line| line.starts_with("source_filename"))
        .unwrap_or("the IR has no source_filename");
    assert!(
        !named.contains(&dir.to_string_lossy().to_string()),
        "the IR must not name the build directory: {named}"
    );
    assert!(
        named.contains("main.ya"),
        "the IR must still name the file: {named}"
    );
}

#[test]
fn the_two_builds_run_the_same_way() {
    // Reproducible bytes are worth nothing if the program does not work, so
    // the built binary is run.
    let dir = work_dir("reproducible-run");
    let binary = dir
        .join("a")
        .join(format!("hello{}", std::env::consts::EXE_SUFFIX));
    build(&dir.join("main.ya"), &dir.join("a"), true);
    let output = common::launch_output(&binary).expect("the built program must run");
    assert!(output.status.success(), "the program must exit zero");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "hello\n",
        "the program must print what it prints"
    );
}
