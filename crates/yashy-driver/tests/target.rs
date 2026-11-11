//! `--target`: the triple a build is for, and the toolchain it needs.
//!
//! A cross build is not self-contained: the runtime is C and the program is
//! linked by `clang`, so the target's C toolchain has to be installed. These
//! tests therefore accept two honest outcomes for a cross build -- it works,
//! or it fails with `E2201` naming the program it looked for -- and refuse
//! every other one.

mod common;

use common::{work_dir, yashy};
use std::fs;
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(yashy())
        .args(args)
        .output()
        .expect("the yashy CLI must be runnable")
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A triple that is not the host, chosen from the six the compiler supports.
fn cross_triple() -> &'static str {
    if cfg!(windows) {
        "aarch64-apple-darwin"
    } else if cfg!(target_os = "macos") {
        "x86_64-unknown-linux-gnu"
    } else {
        "x86_64-pc-windows-msvc"
    }
}

#[test]
fn an_unsupported_triple_is_refused_and_the_supported_ones_are_listed() {
    let dir = work_dir("target-unsupported");
    let file = dir.join("main.ya");
    fs::write(&file, "fn main() {\n    print(\"hi\");\n}\n").expect("the source must be writable");

    let output = run(&[
        "build",
        "--target",
        "riscv64gc-unknown-linux-gnu",
        file.to_str().unwrap(),
    ]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "a triple the compiler does not emit for must fail:\n{}",
        stderr_of(&output)
    );
    let stderr = stderr_of(&output);
    assert!(stderr.contains("E2200"), "{stderr}");
    for triple in [
        "x86_64-pc-windows-msvc",
        "aarch64-pc-windows-msvc",
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
    ] {
        assert!(
            stderr.contains(triple),
            "the message must list {triple}: {stderr}"
        );
    }
}

#[test]
fn the_emitted_ir_names_the_target_it_was_built_for() {
    let dir = work_dir("target-ir");
    let file = dir.join("main.ya");
    fs::write(&file, "fn main() {\n    print(\"hi\");\n}\n").expect("the source must be writable");

    // `--emit-ir` needs no C toolchain, so this works on a machine that cannot
    // cross compile at all. It is the reason the IR header is the thing the
    // test looks at.
    let output = run(&[
        "build",
        "--target",
        "aarch64-apple-darwin",
        "--emit-ir",
        file.to_str().unwrap(),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "emitting IR for a target needs no linker:\n{}",
        stderr_of(&output)
    );
    let ir = stdout_of(&output);
    assert!(
        ir.contains("target triple = \"aarch64-apple-darwin\""),
        "the IR must name the target:\n{ir}"
    );
    // The data layout is the other half of the same fact: a Mach-O module with
    // the ELF layout is a miscompile waiting to happen.
    assert!(
        ir.contains("target datalayout = \"e-m:o-"),
        "the data layout must match the target:\n{ir}"
    );
    assert!(
        !ir.contains("target triple = \"x86_64-pc-windows-msvc\""),
        "the host triple must not leak into a cross build:\n{ir}"
    );
}

#[test]
fn a_cross_build_either_works_or_names_the_missing_toolchain() {
    let dir = work_dir("target-cross");
    let file = dir.join("main.ya");
    fs::write(&file, "fn main() {\n    print(\"hi\");\n}\n").expect("the source must be writable");

    let triple = cross_triple();
    let output = run(&["build", "--target", triple, file.to_str().unwrap()]);
    let stderr = stderr_of(&output);
    if output.status.success() {
        return;
    }
    assert_eq!(
        output.status.code(),
        Some(1),
        "a cross build that fails must fail with a diagnostic:\n{stderr}"
    );
    assert!(
        stderr.contains("E2201"),
        "a cross build without the target's toolchain is E2201:\n{stderr}"
    );
    assert!(
        stderr.contains("clang"),
        "the message must name the program it looked for:\n{stderr}"
    );
}

#[test]
fn the_target_decides_the_cfg_keys() {
    let output = run(&["check", "--print-cfg", "--target", "aarch64-apple-darwin"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "--print-cfg must succeed:\n{}",
        stderr_of(&output)
    );
    let printed = stdout_of(&output);
    let keys: Vec<&str> = printed.lines().map(str::trim).collect();
    for expected in ["macos", "unix", "arch_aarch64"] {
        assert!(
            keys.contains(&expected),
            "`aarch64-apple-darwin` must answer `{expected}`: {keys:?}"
        );
    }
    assert!(
        !keys.contains(&"windows"),
        "`aarch64-apple-darwin` must not answer `windows`: {keys:?}"
    );

    let output = run(&[
        "check",
        "--print-cfg",
        "--target",
        "x86_64-unknown-linux-gnu",
    ]);
    let printed = stdout_of(&output);
    let keys: Vec<&str> = printed.lines().map(str::trim).collect();
    for expected in ["linux", "unix", "arch_x86_64"] {
        assert!(
            keys.contains(&expected),
            "`x86_64-unknown-linux-gnu` must answer `{expected}`: {keys:?}"
        );
    }
    assert!(
        !keys.contains(&"macos"),
        "`x86_64-unknown-linux-gnu` must not answer `macos`: {keys:?}"
    );
}

#[test]
fn doctor_reports_the_toolchain_for_the_target_it_is_asked_about() {
    let triple = cross_triple();
    let output = run(&["doctor", "--target", triple, "--json"]);
    let json = stdout_of(&output);
    assert!(
        json.contains(&format!("\"target\": \"{triple}\"")),
        "the report must name the target it checked:\n{json}"
    );
    assert!(
        json.contains("\"cross\""),
        "the report must say whether the cross toolchain is there:\n{json}"
    );
    // Either the toolchain is there, or the report says why it is not. A
    // report that says nothing about the target is not an answer.
    if !json.contains("\"ok\": true") {
        assert!(
            json.contains("E2201") || json.contains("is not installed"),
            "a missing cross toolchain must be explained:\n{json}"
        );
    }
}

#[test]
fn a_cfg_branch_follows_the_target_not_the_host() {
    let dir = work_dir("target-cfg");
    let file = dir.join("main.ya");
    fs::write(
        &file,
        "fn main() {\n    cfg(windows) {\n        print(\"windows\");\n    }\n    cfg(unix) {\n        print(\"unix\");\n    }\n}\n",
    )
    .expect("the source must be writable");

    // The branch that is selected is the target's, so the program for a Linux
    // target prints the Unix line even when it is built on Windows.
    let output = run(&[
        "build",
        "--target",
        "x86_64-unknown-linux-gnu",
        "--emit-ir",
        file.to_str().unwrap(),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "emitting IR needs no linker:\n{}",
        stderr_of(&output)
    );
    let ir = stdout_of(&output);
    assert!(
        ir.contains("unix"),
        "the Unix branch must be the one kept:\n{ir}"
    );
    assert!(
        !ir.contains("windows"),
        "the Windows branch must have been dropped:\n{ir}"
    );
}
