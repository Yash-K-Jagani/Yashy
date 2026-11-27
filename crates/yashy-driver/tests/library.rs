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
    assert!(header.contains("int64_t yashy_fn_twice"), "{header}");
    assert!(header.contains("yashy_str"), "{header}");
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
