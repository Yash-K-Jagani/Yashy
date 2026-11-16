//! The v2 cache key names every input that can change the bytes, and a no-op
//! build reuses what it already produced. A stale hit is the one unacceptable
//! outcome, so the test mutates inputs and names each change.

mod common;

use common::{work_dir, write, yashyc};
use std::process::Command;

fn verify(dir: &std::path::Path, file: &str) -> String {
    let output = Command::new(yashyc())
        .args(["cache", "verify", file])
        .current_dir(dir)
        .output()
        .expect("the CLI must run");
    assert!(
        output.status.success(),
        "cache verify must succeed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[test]
fn the_v2_key_changes_when_any_named_input_changes() {
    use yashy_cache::program_key_v2;
    let sources = [("main.ya".to_string(), "fn main() {}".to_string())];
    let base = program_key_v2(
        &sources,
        "-O0",
        "1.0",
        "x86_64-unknown-linux-gnu",
        &[("runtime", "abc")],
    );
    assert_eq!(
        base,
        program_key_v2(
            &sources,
            "-O0",
            "1.0",
            "x86_64-unknown-linux-gnu",
            &[("runtime", "abc")]
        ),
        "the same inputs must give the same key"
    );
    assert_ne!(
        base,
        program_key_v2(
            &sources,
            "-O2",
            "1.0",
            "x86_64-unknown-linux-gnu",
            &[("runtime", "abc")]
        ),
        "flags are part of the key"
    );
    assert_ne!(
        base,
        program_key_v2(
            &sources,
            "-O0",
            "1.1",
            "x86_64-unknown-linux-gnu",
            &[("runtime", "abc")]
        ),
        "the compiler version is part of the key"
    );
    assert_ne!(
        base,
        program_key_v2(
            &sources,
            "-O0",
            "1.0",
            "aarch64-apple-darwin",
            &[("runtime", "abc")]
        ),
        "the target is part of the key"
    );
    assert_ne!(
        base,
        program_key_v2(
            &sources,
            "-O0",
            "1.0",
            "x86_64-unknown-linux-gnu",
            &[("runtime", "def")]
        ),
        "the runtime hash is part of the key"
    );
    assert_ne!(
        base,
        program_key_v2(
            &sources,
            "-O0",
            "1.0",
            "x86_64-unknown-linux-gnu",
            &[("runtime", "abc"), ("lock", "x")]
        ),
        "the lockfile hash is part of the key"
    );
}

#[test]
fn a_build_populates_the_cache_and_an_edit_invalidates_it() {
    let dir = work_dir("incremental-cache");
    let cache = dir.join("cache");
    write(&dir, "main.ya", "fn main() { print(1); }\n");

    let output = Command::new(yashyc())
        .arg("build")
        .arg("main.ya")
        .arg("-o")
        .arg("main.bin")
        .current_dir(&dir)
        .env("YASHY_CACHE_DIR", &cache)
        .output()
        .expect("the build must run");
    assert!(
        output.status.success(),
        "the build must succeed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let first = verify_with_cache(&dir, &cache, "main.ya");
    assert!(
        first.ends_with(" hit"),
        "a build must fill the cache: {first}"
    );
    assert_eq!(
        verify_with_cache(&dir, &cache, "main.ya"),
        first,
        "a no-op verify is stable"
    );

    write(&dir, "main.ya", "fn main() { print(2); }\n");
    let changed = verify_with_cache(&dir, &cache, "main.ya");
    assert!(
        changed.ends_with(" miss"),
        "an edit must invalidate: {changed}"
    );
    assert_ne!(
        first.split_whitespace().next(),
        changed.split_whitespace().next(),
        "the key must change with the source"
    );
}

#[test]
fn a_second_build_reuses_the_binary_without_linking() {
    let dir = work_dir("incremental-binary");
    let cache = dir.join("cache");
    write(&dir, "main.ya", "fn main() { print(7); }\n");

    let build = || {
        Command::new(yashyc())
            .args(["build", "main.ya", "-o", "main.bin", "--timings"])
            .current_dir(&dir)
            .env("YASHY_CACHE_DIR", &cache)
            .output()
            .expect("the build must run")
    };
    let first = build();
    assert!(
        first.status.success(),
        "the first build must succeed:\n{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let second = build();
    assert!(
        second.status.success(),
        "the second build must succeed:\n{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let report = String::from_utf8_lossy(&second.stderr);
    assert!(
        report.contains("timing: link 0 ms"),
        "a no-change build must reuse the binary instead of linking:\n{report}"
    );
}

fn verify_with_cache(dir: &std::path::Path, cache: &std::path::Path, file: &str) -> String {
    let output = Command::new(yashyc())
        .args(["cache", "verify", file])
        .current_dir(dir)
        .env("YASHY_CACHE_DIR", cache)
        .output()
        .expect("cache verify must run");
    assert!(
        output.status.success(),
        "cache verify must succeed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}
