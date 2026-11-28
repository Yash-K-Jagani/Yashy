//! Library output: a C header generated from checked public functions and a
//! shared object linked with the same code. Private helpers stay out of the
//! header; signatures with no C representation are named as excluded.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace(name: &str) -> PathBuf {
    let dir = common::work_dir(name);
    common::write(
        &dir,
        "main.ya",
        "/// Doubles a number.\npub fn twice(int n) -> int {\n    return n + n;\n}\n\n\
         /// Greets a name; the result belongs to the caller.\npub fn greet(string name) -> string {\n    return name;\n}\n\n\
         fn hidden(int n) -> int {\n    return n;\n}\n\n\
         pub fn no_c_type() -> map[int]int {\n    return map_new();\n}\n\n\
         fn main() {\n    print(twice(2));\n}\n",
    );
    dir
}

fn build(dir: &Path, out: &str) -> (bool, String) {
    let output = Command::new(common::yashyc())
        .args([
            "build",
            "main.ya",
            "--emit=cdylib",
            "--header",
            "out.h",
            "-o",
            out,
        ])
        .current_dir(dir)
        .output()
        .expect("the CLI must run");
    (
        output.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

#[test]
fn a_header_is_generated_from_public_c_compatible_functions() {
    let dir = workspace("library-header");
    let exe = format!("libdemo{}", std::env::consts::DLL_SUFFIX);
    let (ok, output) = build(&dir, &exe);
    assert!(ok, "{output}");
    let header = fs::read_to_string(dir.join("out.h")).expect("the header is written");
    assert!(header.contains("YASHY_GENERATED_H"), "{header}");
    assert!(header.contains("#define YASHY_ABI_VERSION 1"), "{header}");
    assert!(
        header.contains("void yashy_str_release(yashy_str *s);"),
        "{header}"
    );
    assert!(header.contains("int64_t yashy_fn_twice"), "{header}");
    assert!(header.contains("yashy_str"), "{header}");
    assert!(
        header.contains("const yashy_str *name"),
        "a borrowed parameter is const: {header}"
    );
    assert!(
        header.contains("YASHY_OWNED yashy_str *yashy_fn_greet"),
        "a returned string is owned by the caller: {header}"
    );
    assert!(
        header.contains("owned; release with yashy_str_release"),
        "{header}"
    );
    assert!(
        !header.contains("yashy_fn_hidden"),
        "private stays out: {header}"
    );
    assert!(
        header.contains("Not exported (no C-compatible signature): no_c_type"),
        "{header}"
    );
    assert!(dir.join(&exe).is_file(), "the shared object exists");
}

#[test]
fn a_checked_in_header_is_compared_not_rewritten() {
    let dir = workspace("library-header-check");
    let build = |extra: &[&str]| {
        Command::new(common::yashyc())
            .args(["build", "main.ya", "--header", "out.h", "--emit-ir"])
            .args(extra)
            .current_dir(&dir)
            .output()
            .expect("the CLI must run")
    };
    let output = build(&[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let fresh = build(&["--check"]);
    assert!(
        fresh.status.success(),
        "a freshly written header passes: {}",
        String::from_utf8_lossy(&fresh.stderr)
    );

    fs::write(dir.join("out.h"), "/* edited by hand */\n").expect("edit the header");
    let drifted = build(&["--check"]);
    assert!(
        !drifted.status.success(),
        "a drifted header fails the check"
    );
    let stderr = String::from_utf8_lossy(&drifted.stderr);
    assert!(stderr.contains("is stale"), "{stderr}");
    assert!(stderr.contains("yashyc build --header"), "{stderr}");

    let regenerated = build(&[]);
    assert!(regenerated.status.success());
    let clean = build(&["--check"]);
    assert!(
        clean.status.success(),
        "regenerating clears the drift: {}",
        String::from_utf8_lossy(&clean.stderr)
    );
}

#[test]
fn a_header_check_without_a_header_is_a_usage_error() {
    let dir = workspace("library-check-no-header");
    let output = Command::new(common::yashyc())
        .args(["build", "main.ya", "--check"])
        .current_dir(&dir)
        .output()
        .expect("the CLI must run");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("`--check` needs `--header"), "{stderr}");
}

#[test]
fn an_unknown_emit_kind_is_a_usage_error() {
    let dir = workspace("library-bad-emit");
    let output = Command::new(common::yashyc())
        .args(["build", "main.ya", "--emit=staticlib"])
        .current_dir(&dir)
        .output()
        .expect("the CLI must run");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown emit kind"), "{stderr}");
}
