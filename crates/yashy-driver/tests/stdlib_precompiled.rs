//! The stdlib is a package, and its contribution to a build is visible.
//!
//! These tests answer three questions the plan for the precompiled stdlib
//! asks. Is the stdlib versioned like a package? Does a program that imports
//! nothing from it load no stdlib module? Does the emitted IR come from the
//! content-addressed cache on a second build, and is that build byte-for-byte
//! the one a from-source build produces?
//!
//! Every test owns a private `YASHY_CACHE_DIR`, so the warm and cold builds it
//! compares cannot see another test's artifacts.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::{exe, launch_output, work_dir, yashy};
use yashy_manifest::Manifest;

/// The repository root: `crates/yashy-driver/tests/stdlib_precompiled.rs`.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// Run the CLI from the repository root so an entry path in the IR key is the
/// same string for every build in one test.
fn cli(cache: &Path, stdlib: Option<&Path>, args: &[&str]) -> Output {
    let mut command = Command::new(yashy());
    command
        .args(args)
        .current_dir(root())
        .env("YASHY_CACHE_DIR", cache);
    if let Some(stdlib) = stdlib {
        command.env("YASHY_STDLIB", stdlib);
    }
    command.output().expect("the yashy CLI must be runnable")
}

/// Build `entry` to `out` with `--timings`, optionally from source.
fn build(cache: &Path, stdlib: Option<&Path>, entry: &str, out: &Path, extra: &[&str]) -> Output {
    let out_text = out.to_string_lossy().into_owned();
    let mut args = vec!["build", entry, "-o", out_text.as_str(), "--timings"];
    args.extend_from_slice(extra);
    cli(cache, stdlib, &args)
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn the_stdlib_declares_itself_as_a_versioned_package() {
    let manifest = Manifest::load(&root().join("crates/yashy-stdlib/yashy.toml"))
        .expect("the stdlib must have a manifest");
    assert_eq!(manifest.package.name, "stdlib");
    assert_eq!(manifest.package.version, "1.0.0");
    // The compiler's fallback constant and the shipped manifest agree.
    assert_eq!(yashy_driver::modules::stdlib_version(), "1.0.0");
}

#[test]
fn hello_loads_no_stdlib_module() {
    let cache = work_dir("stdlib-hello-cache");
    let out = exe(&work_dir("stdlib-hello"), "hello");
    let output = build(&cache, None, "examples/hello/main.ya", &out, &[]);
    assert!(
        output.status.success(),
        "hello must build:\n{}",
        stderr(&output)
    );
    let log = stderr(&output);
    // The stdlib is not ambient: `hello` imports nothing, so it pays for no
    // stdlib module and never typechecks one.
    assert!(
        log.contains("stdlib_modules=0"),
        "hello must report no stdlib modules:\n{log}"
    );
}

#[test]
fn a_warm_ir_cache_skips_emit_and_reproduces_the_binary() {
    let cache = work_dir("stdlib-ir-cache");
    let dir = work_dir("stdlib-ir-out");
    let first = exe(&dir, "first");
    let second = exe(&dir, "second");
    let from_source = exe(&dir, "from_source");

    let cold = build(&cache, None, "examples/json-report/main.ya", &first, &[]);
    assert!(
        cold.status.success(),
        "the cold build must pass:\n{}",
        stderr(&cold)
    );
    let cold_log = stderr(&cold);
    assert!(
        cold_log.contains("ir_cache=miss"),
        "the first build must emit:\n{cold_log}"
    );
    // `json-report` imports `JSON`, so exactly one stdlib module is loaded.
    assert!(
        cold_log.contains("stdlib_modules=1"),
        "importing JSON must load one stdlib module:\n{cold_log}"
    );

    let warm = build(&cache, None, "examples/json-report/main.ya", &second, &[]);
    assert!(
        warm.status.success(),
        "the warm build must pass:\n{}",
        stderr(&warm)
    );
    let warm_log = stderr(&warm);
    assert!(
        warm_log.contains("ir_cache=hit"),
        "the second build must reuse the IR:\n{warm_log}"
    );
    assert!(
        warm_log.contains("timing: emit 0 ms"),
        "a cache hit must not emit:\n{warm_log}"
    );

    // The escape hatch never reads or writes the cache, so it reports no
    // decision at all and must still produce the same bytes.
    let source_build = build(
        &cache,
        None,
        "examples/json-report/main.ya",
        &from_source,
        &["--stdlib-source"],
    );
    assert!(
        source_build.status.success(),
        "a from-source build must pass:\n{}",
        stderr(&source_build)
    );
    let source_log = stderr(&source_build);
    assert!(
        !source_log.contains("ir_cache="),
        "`--stdlib-source` must bypass the cache:\n{source_log}"
    );

    let cold_bytes = fs::read(&first).expect("the first binary must exist");
    let warm_bytes = fs::read(&second).expect("the second binary must exist");
    let source_bytes = fs::read(&from_source).expect("the from-source binary must exist");
    assert_eq!(
        cold_bytes.len(),
        warm_bytes.len(),
        "a cache hit must produce the same image size"
    );
    assert_eq!(
        cold_bytes.len(),
        source_bytes.len(),
        "a from-source build must produce the same image size"
    );
    assert_same_binary(
        &without_link_timestamp(&cold_bytes),
        &without_link_timestamp(&warm_bytes),
        "a cache hit must reproduce the binary",
    );
    assert_same_binary(
        &without_link_timestamp(&cold_bytes),
        &without_link_timestamp(&source_bytes),
        "a from-source build must reproduce the cached binary",
    );

    // Both binaries agree about what the program prints, not just about the
    // bytes they were built from.
    let cached_run = launch_output(&first).expect("the cached binary must start");
    let source_run = launch_output(&from_source).expect("the from-source binary must start");
    assert!(cached_run.status.success(), "the cached binary must run");
    assert!(
        source_run.status.success(),
        "the from-source binary must run"
    );
    assert_eq!(cached_run.stdout, source_run.stdout);
}

#[test]
fn a_stdlib_version_change_invalidates_the_ir_cache() {
    // A private copy of the stdlib, so the manifest can be edited without
    // touching the repository.
    let copy = work_dir("stdlib-copy");
    fs::create_dir_all(&copy).expect("the copy must be creatable");
    copy_tree(
        &root().join("crates/yashy-stdlib/yashy"),
        &copy.join("yashy"),
    );
    fs::copy(
        root().join("crates/yashy-stdlib/yashy.toml"),
        copy.join("yashy.toml"),
    )
    .expect("the manifest must be copied");
    let stdlib = copy.join("yashy");

    let cache = work_dir("stdlib-version-cache");
    let dir = work_dir("stdlib-version-out");
    let first = exe(&dir, "first");
    let second = exe(&dir, "second");
    let third = exe(&dir, "third");

    let cold = build(
        &cache,
        Some(&stdlib),
        "examples/json-report/main.ya",
        &first,
        &[],
    );
    assert!(
        cold.status.success(),
        "the cold build must pass:\n{}",
        stderr(&cold)
    );
    assert!(
        stderr(&cold).contains("ir_cache=miss"),
        "the first build must emit:\n{}",
        stderr(&cold)
    );

    let warm = build(
        &cache,
        Some(&stdlib),
        "examples/json-report/main.ya",
        &second,
        &[],
    );
    assert!(
        warm.status.success(),
        "the warm build must pass:\n{}",
        stderr(&warm)
    );
    assert!(
        stderr(&warm).contains("ir_cache=hit"),
        "an unchanged stdlib must hit the cache:\n{}",
        stderr(&warm)
    );

    // Only the manifest changes. The module bytes are identical, so a cache
    // key without the stdlib version would serve the old IR here.
    fs::write(
        copy.join("yashy.toml"),
        "[package]\nname = \"stdlib\"\nversion = \"1.0.1\"\n",
    )
    .expect("the copied manifest must be writable");
    let rebuilt = build(
        &cache,
        Some(&stdlib),
        "examples/json-report/main.ya",
        &third,
        &[],
    );
    assert!(
        rebuilt.status.success(),
        "the build after a stdlib bump must pass:\n{}",
        stderr(&rebuilt)
    );
    assert!(
        stderr(&rebuilt).contains("ir_cache=miss"),
        "a changed stdlib version must invalidate the entry:\n{}",
        stderr(&rebuilt)
    );
}

/// A binary with the one field the linker fills in from the clock cleared.
///
/// A PE image carries `TimeDateStamp` in its COFF header, which the linker sets
/// to the current time, so two identical builds made a second apart differ in
/// exactly that four-byte field. Everything the compiler controls is still
/// compared byte-for-byte. A non-PE image is returned unchanged: ELF embeds no
/// timestamp, and a GNU build-id is a hash of the image contents, so identical
/// builds are identical files there.
fn without_link_timestamp(bytes: &[u8]) -> Vec<u8> {
    let mut bytes = bytes.to_vec();
    if bytes.len() < 0x40 || &bytes[0..2] != b"MZ" {
        return bytes;
    }
    let offset = u32::from_le_bytes([bytes[0x3c], bytes[0x3d], bytes[0x3e], bytes[0x3f]]) as usize;
    if offset + 4 <= bytes.len() && &bytes[offset..offset + 4] == b"PE\0\0" {
        // The COFF header starts just past the four-byte signature, and its
        // `TimeDateStamp` is the third field.
        let stamp = offset + 8;
        if stamp + 4 <= bytes.len() {
            bytes[stamp..stamp + 4].fill(0);
        }
    }
    bytes
}

/// Assert two images are identical, naming the first differing byte.
///
/// A plain `assert_eq!` on two quarter-megabyte images dumps thousands of
/// numbers into the failure message and hides the one fact worth knowing: the
/// offset where they parted.
fn assert_same_binary(left: &[u8], right: &[u8], what: &str) {
    assert_eq!(
        left.len(),
        right.len(),
        "{what}: image lengths differ ({} and {} bytes)",
        left.len(),
        right.len()
    );
    if let Some(offset) = left.iter().zip(right).position(|(a, b)| a != b) {
        panic!(
            "{what}: images differ at byte {offset} of {} ({} and {})",
            left.len(),
            left[offset],
            right[offset]
        );
    }
}

/// Copy a directory tree recursively, without following into a symlink.
fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("the destination must be creatable");
    for entry in fs::read_dir(from).expect("the stdlib source must be readable") {
        let entry = entry.expect("a directory entry must be readable");
        let source = entry.path();
        let destination = to.join(entry.file_name());
        if source.is_dir() {
            copy_tree(&source, &destination);
        } else {
            fs::copy(&source, &destination).expect("a stdlib file must be copyable");
        }
    }
}
