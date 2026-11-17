//! Lints are data with codes and groups; they never change build success.

mod common;

use common::{work_dir, write, yashyc};
use std::process::Command;

fn run(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(yashyc())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("the CLI must run")
}

#[test]
fn lint_list_is_the_machine_form() {
    let dir = work_dir("lint-list");
    let output = run(&dir, &["lint", "--list", "--json"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    for code in ["LINT-UNUSED", "LINT-SHADOW", "LINT-VAR"] {
        assert!(stdout.contains(code), "{stdout}");
    }
    assert!(stdout.contains("\"group\""), "{stdout}");
}

#[test]
fn lint_reports_a_removed_keyword_and_an_unused_binding() {
    let dir = work_dir("lint-report");
    write(
        &dir,
        "main.ya",
        "fn main() {\n    var int unused = 1;\n    const int used = 2;\n    print(used);\n}\n",
    );
    let output = run(&dir, &["lint", "--json", "main.ya"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("LINT-VAR"), "{stdout}");
    assert!(stdout.contains("LINT-UNUSED"), "{stdout}");
}

#[test]
fn lint_never_fails_the_build_by_itself() {
    let dir = work_dir("lint-nonfatal");
    write(&dir, "main.ya", "fn main() { var int x = 1; }\n");
    let output = run(&dir, &["lint", "main.ya"]);
    assert!(output.status.success());
}
