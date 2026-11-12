//! `cfg`: the target condition, and the promise that a disabled branch is
//! never even resolved.
//!
//! The tests here are the ones the feature is for. A `cfg` that did nothing
//! would still compile the program it guards, so the proof that it works is a
//! branch that calls a function which exists nowhere: if that program compiles,
//! the disabled branch was not resolved.

mod common;

use common::{work_dir, yashy};
use std::fs;
use std::path::{Path, PathBuf};
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

/// The programs directory, where the worked examples live.
fn programs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("programs")
}

/// The `cfg` keys that are true for the host, as `--print-cfg` reports them.
fn host_keys() -> Vec<String> {
    let output = run(&["check", "--print-cfg"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "--print-cfg must succeed:\n{}",
        stderr_of(&output)
    );
    stdout_of(&output)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

#[test]
fn the_program_reports_the_platform_it_was_built_for() {
    let source = programs_dir().join("cfg_platform.ya");
    let dir = work_dir("cfg-platform");
    let exe = dir.join(format!("cfg_platform{}", std::env::consts::EXE_SUFFIX));

    let build = run(&[
        "build",
        source.to_str().unwrap(),
        "-o",
        exe.to_str().unwrap(),
    ]);
    assert!(
        build.status.success(),
        "cfg_platform.ya must build:\n{}",
        stderr_of(&build)
    );

    let keys = host_keys();
    let expected = if keys.iter().any(|key| key == "windows") {
        "windows"
    } else if keys.iter().any(|key| key == "macos") {
        "unix"
    } else {
        "unix"
    };
    let output = common::launch_output(&exe).expect("the built program must run");
    assert!(output.status.success(), "the program must exit zero");
    assert_eq!(
        stdout_of(&output),
        format!("{expected}\n"),
        "the program must print the platform its `cfg` chose"
    );
}

#[test]
fn exactly_one_branch_is_selected() {
    let dir = work_dir("cfg-one-branch");
    let file = dir.join("main.ya");
    fs::write(
        &file,
        "fn main() {\n    cfg(windows) {\n        print(\"windows\");\n    }\n    cfg(unix) {\n        print(\"unix\");\n    }\n}\n",
    )
    .expect("the test source must be writable");

    let output = run(&["run", file.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "the program must run:\n{}",
        stderr_of(&output)
    );
    let printed = stdout_of(&output);
    assert!(
        printed == "windows\n" || printed == "unix\n",
        "exactly one branch must run, got {printed:?}"
    );
}

#[test]
fn a_disabled_branch_is_never_resolved() {
    // The function called in the disabled branch exists nowhere. If the filter
    // ran after name resolution this program would not compile.
    //
    // The key is chosen for the host, not hardcoded: `cfg(unix)` is *enabled*
    // on the Linux runner, so a fixed key would leave the branch resolved and
    // the test would fail for a reason that has nothing to do with the filter.
    let keys = host_keys();
    let disabled = if keys.iter().any(|key| key == "windows") {
        "unix"
    } else {
        "windows"
    };
    let dir = work_dir("cfg-disabled");
    let file = dir.join("main.ya");
    fs::write(
        &file,
        format!(
            "fn main() {{\n    cfg({disabled}) {{\n        a_function_that_exists_nowhere();\n    }}\n    print(\"compiled\");\n}}\n"
        ),
    )
    .expect("the test source must be writable");

    let output = run(&["check", file.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "a disabled branch must not be resolved:\n{}",
        stderr_of(&output)
    );
}

#[test]
fn a_disabled_item_is_not_declared() {
    // The disabled item declares `platform`; the enabled one declares it too.
    // Only one may reach the checker, or the name is defined twice.
    let dir = work_dir("cfg-item");
    let file = dir.join("main.ya");
    fs::write(
        &file,
        "cfg(windows) fn platform() -> string {\n    return \"windows\";\n}\n\ncfg(unix) fn platform() -> string {\n    return \"unix\";\n}\n\nfn main() {\n    print(platform());\n}\n",
    )
    .expect("the test source must be writable");

    let output = run(&["check", file.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "one guarded item must survive:\n{}",
        stderr_of(&output)
    );
    assert!(
        !stderr_of(&output).contains("duplicate definition"),
        "the disabled item must not be declared:\n{}",
        stderr_of(&output)
    );
}

#[test]
fn an_unknown_key_is_an_error_and_not_a_false_condition() {
    let dir = work_dir("cfg-unknown-key");
    let file = dir.join("main.ya");
    fs::write(&file, "cfg(window) fn main() {\n    print(\"typo\");\n}\n")
        .expect("the test source must be writable");

    let output = run(&["check", file.to_str().unwrap()]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "a misspelled key must fail the build:\n{}",
        stderr_of(&output)
    );
    let stderr = stderr_of(&output);
    assert!(stderr.contains("E2202"), "{stderr}");
    assert!(stderr.contains("window"), "{stderr}");
    assert!(stderr.contains("known keys"), "{stderr}");
}

#[test]
fn cfg_on_a_let_is_refused() {
    let dir = work_dir("cfg-on-let");
    let file = dir.join("main.ya");
    fs::write(
        &file,
        "fn main() {\n    let int x = cfg(windows) 1;\n    print(\"x\");\n}\n",
    )
    .expect("the test source must be writable");

    let output = run(&["check", file.to_str().unwrap()]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "`cfg` cannot guard a binding:\n{}",
        stderr_of(&output)
    );
    let stderr = stderr_of(&output);
    assert!(stderr.contains("E2203"), "{stderr}");
    assert!(
        stderr.contains("statement block") && stderr.contains("struct field"),
        "the message must name the positions that work:\n{stderr}"
    );
}

#[test]
fn cfg_on_a_parameter_is_refused() {
    let dir = work_dir("cfg-on-param");
    let file = dir.join("main.ya");
    fs::write(
        &file,
        "fn f(cfg(windows) int x) {\n    print(\"x\");\n}\n\nfn main() {\n    f(1);\n}\n",
    )
    .expect("the test source must be writable");

    let output = run(&["check", file.to_str().unwrap()]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "`cfg` cannot guard a parameter:\n{}",
        stderr_of(&output)
    );
    assert!(
        stderr_of(&output).contains("E2203"),
        "{}",
        stderr_of(&output)
    );
}

#[test]
fn print_cfg_lists_the_keys_that_are_true_for_the_host() {
    let keys = host_keys();
    assert!(
        keys.contains(&"windows".to_string())
            || keys.contains(&"linux".to_string())
            || keys.contains(&"macos".to_string()),
        "the host must answer one of the platform keys: {keys:?}"
    );
    assert!(
        keys.contains(&"debug".to_string()),
        "a build without `--release` is a debug build: {keys:?}"
    );
    assert!(
        keys.iter()
            .any(|key| key == "arch_x86_64" || key == "arch_aarch64"),
        "the host must answer one architecture: {keys:?}"
    );
    // The list is sorted, so a reader can compare two of them.
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(sorted, keys, "the keys must be printed in order: {keys:?}");
}

#[test]
fn the_combinators_read_as_they_are_written() {
    let dir = work_dir("cfg-combinators");
    let file = dir.join("main.ya");
    fs::write(
        &file,
        "fn main() {\n    cfg(not(windows)) {\n        print(\"not windows\");\n    }\n    cfg(any(windows, linux)) {\n        print(\"any\");\n    }\n    cfg(all(unix, not(arch_x86_64))) {\n        print(\"all\");\n    }\n}\n",
    )
    .expect("the test source must be writable");

    let output = run(&["check", file.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "the combinators must parse and evaluate:\n{}",
        stderr_of(&output)
    );
}

#[test]
fn the_formatter_round_trips_the_three_positions() {
    let dir = work_dir("cfg-format");
    let file = dir.join("main.ya");
    let source = "cfg(windows) fn platform() -> string {\n    return \"windows\";\n}\n\nstruct Config {\n    cfg(windows) handle: int,\n    name: string,\n}\n\nfn main() {\n    cfg(unix) {\n        print(\"unix\");\n    }\n}\n";
    fs::write(&file, source).expect("the test source must be writable");

    let formatted = run(&["fmt", file.to_str().unwrap()]);
    assert_eq!(
        formatted.status.code(),
        Some(0),
        "the file must format:\n{}",
        stderr_of(&formatted)
    );
    let once = stdout_of(&formatted);
    assert!(
        once.contains("cfg(windows) fn platform()"),
        "an item-level `cfg` must keep its shape:\n{once}"
    );
    assert!(
        once.contains("cfg(windows) handle: int,"),
        "a field-level `cfg` must keep its shape:\n{once}"
    );
    assert!(
        once.contains("cfg(unix) {"),
        "a block-level `cfg` must keep its shape:\n{once}"
    );

    // Formatting the formatted text changes nothing: that is what makes the
    // gate usable.
    let second = run(&["fmt", file.to_str().unwrap()]);
    assert_eq!(stdout_of(&second), once, "the format must be idempotent");
}
