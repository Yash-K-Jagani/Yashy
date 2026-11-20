//! The v2 cache key names every input that can change the bytes, and a no-op
//! build reuses what it already produced. A stale hit is the one unacceptable
//! outcome, so the test mutates inputs and names each change.

mod common;

use common::{work_dir, write, yashyc};
use std::process::Command;

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

fn verify_with_cache(dir: &std::path::Path, cache: &std::path::Path, file: &str) -> String {    let output = Command::new(yashyc())
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

/// A ten-module fixture: `main.ya` plus nine modules it imports.
///
/// Every module holds one function, because the thing under test is how many
/// *objects* a build has to produce when one of them changes.
fn ten_modules(dir: &std::path::Path) {
    let mut main = String::new();
    for index in 0..9 {
        main.push_str(&format!("import m{index};\n"));
    }
    main.push_str("fn main() {\n    let total = 0;\n");
    for index in 0..9 {
        main.push_str(&format!("    total = total + m{index}({index});\n"));
    }
    main.push_str("    print(total);\n}\n");
    write(dir, "main.ya", &main);
    for index in 0..9 {
        write(
            dir,
            &format!("m{index}.ya"),
            &format!("pub fn m{index}(int x) -> int {{\n    return x + {index};\n}}\n"),
        );
    }
}

#[test]
fn a_one_function_edit_rebuilds_one_module_object() {
    // One object per source file: a build that changed one function must
    // compile that file and link the nine it did not touch.
    let dir = work_dir("incremental-modules");
    let cache = dir.join("cache");
    let out = common::exe(&dir, "modules");
    ten_modules(&dir);

    let build = || {
        Command::new(yashyc())
            .args(["build", "main.ya", "-o", "modules", "--timings"])
            .current_dir(&dir)
            .env("YASHY_CACHE_DIR", &cache)
            .output()
            .expect("the build must run")
    };

    let cold = build();
    assert!(
        cold.status.success(),
        "the ten-module build must pass:\n{}",
        String::from_utf8_lossy(&cold.stderr)
    );
    let cold_log = String::from_utf8_lossy(&cold.stderr).into_owned();
    assert!(
        cold_log.contains("timing: modules 10 rebuilt, 0 cached"),
        "a cold build must compile all ten modules:\n{cold_log}"
    );

    // The objects are not merely counted: the binary they link must be the
    // program the edit describes. `m_i(i)` is `i + i`, so the original total is
    // 2 * (0 + .. + 8) = 72; turning `m3`'s `+ 3` into `+ 40` moves it to 109.
    let before = common::launch_output(&out).expect("the built program must start");
    assert!(before.status.success(), "the built program must exit 0");
    assert!(
        String::from_utf8_lossy(&before.stdout).contains("72"),
        "the unedited binary must print its own total: {}",
        String::from_utf8_lossy(&before.stdout)
    );

    write(
        &dir,
        "m3.ya",
        "pub fn m3(int x) -> int {\n    return x + 40;\n}\n",
    );
    let edited = build();
    assert!(
        edited.status.success(),
        "the build after one edit must pass:\n{}",
        String::from_utf8_lossy(&edited.stderr)
    );
    let edited_log = String::from_utf8_lossy(&edited.stderr).into_owned();
    assert!(
        edited_log.contains("timing: modules 1 rebuilt, 9 cached"),
        "a one-function edit must rebuild one object:\n{edited_log}"
    );

    // The objects are not merely counted: the binary they link must be the
    // program the edit describes. 2 * (0 + .. + 8) = 72 before the edit and
    // 73 after it, because 3 became 40 in place of 3.
    let run = common::launch_output(&out).expect("the built program must start");
    assert!(run.status.success(), "the built program must exit 0");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    assert!(
        stdout.contains("109"),
        "the rebuilt binary must print the edited total: {stdout}"
    );

    // Nothing changed, so nothing is emitted and nothing is compiled.
    let unchanged = build();
    assert!(
        unchanged.status.success(),
        "a no-change build must pass:\n{}",
        String::from_utf8_lossy(&unchanged.stderr)
    );
    let unchanged_log = String::from_utf8_lossy(&unchanged.stderr).into_owned();
    assert!(
        unchanged_log.contains("ir_cache=hit"),
        "a no-change build must reuse the emitted IR:\n{unchanged_log}"
    );
    assert!(
        !unchanged_log.contains("timing: modules"),
        "a no-change build must not compile any module:\n{unchanged_log}"
    );
    assert!(
        unchanged_log.contains("timing: emit 0 ms"),
        "a no-change build must not emit:\n{unchanged_log}"
    );
}
