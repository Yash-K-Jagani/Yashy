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
        "fn main() {\n    var int old = 1;\n    let int unused = 2;\n    const int used = 3;\n    print(used);\n}\n",
    );
    let output = run(&dir, &["lint", "--json", "main.ya"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("LINT-VAR"), "{stdout}");
    assert!(stdout.contains("LINT-UNUSED"), "{stdout}");
}

#[test]
fn a_typed_lint_finds_an_ignored_user_defined_result() {
    let dir = work_dir("lint-typed");
    write(
        &dir,
        "main.ya",
        "fn maybe() -> Result[int, string] { return Ok(1); }\nfn main() { maybe(); print(1); }\n",
    );
    let plain_output = run(&dir, &["lint", "--json", "main.ya"]);
    let plain = String::from_utf8_lossy(&plain_output.stdout).into_owned();
    assert!(
        !plain.contains("LINT-RESULT"),
        "the text pass knows only builtins: {plain}"
    );
    let typed_output = run(&dir, &["lint", "--typed", "--json", "main.ya"]);
    let typed = String::from_utf8_lossy(&typed_output.stdout).into_owned();
    assert!(
        typed.contains("LINT-RESULT"),
        "the checked type is a Result: {typed}"
    );

    write(
        &dir,
        "main.ya",
        "fn maybe() -> Result[int, string] { return Ok(1); }\nfn main() { switch maybe() { case Ok(v) { print(v); } case Err(_) { print(0); } } }\n",
    );
    let handled_output = run(&dir, &["lint", "--typed", "--json", "main.ya"]);
    let handled = String::from_utf8_lossy(&handled_output.stdout).into_owned();
    assert!(
        !handled.contains("LINT-RESULT"),
        "a handled result is not advice: {handled}"
    );
}

#[test]
fn lint_never_fails_the_build_by_itself() {
    let dir = work_dir("lint-nonfatal");
    write(&dir, "main.ya", "fn main() { var int x = 1; }\n");
    let output = run(&dir, &["lint", "main.ya"]);
    assert!(output.status.success());
}

#[test]
fn deny_promotes_a_lint_to_an_error() {
    let dir = work_dir("lint-deny");
    write(&dir, "main.ya", "fn main() { var int x = 1; }\n");
    let output = run(&dir, &["lint", "--deny", "LINT-VAR", "main.ya"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "a denied lint must fail the command:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn group_filters_the_reported_lints() {
    let dir = work_dir("lint-group");
    write(&dir, "main.ya", "fn main() { var int x = 1; }\n");
    let output = run(&dir, &["lint", "--json", "--group", "style", "main.ya"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("LINT-VAR"), "{stdout}");
    assert!(!stdout.contains("LINT-UNUSED"), "{stdout}");
}

#[test]
fn the_var_lint_carries_a_machine_applicable_fix() {
    let dir = work_dir("lint-suggestion");
    write(&dir, "main.ya", "fn main() { var int x = 1; }\n");
    let output = run(&dir, &["lint", "--json", "main.ya"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"replacement\": \"let\""), "{stdout}");
    assert!(
        stdout.contains("\"applicability\": \"machine-applicable\""),
        "{stdout}"
    );
}

#[test]
fn the_committed_lint_page_is_the_rendered_registry() {
    let rendered = yashy_driver::lint::render_docs();
    let page = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/lints.md"),
    )
    .expect("docs/lints.md must exist");
    assert_eq!(page, rendered, "run `yashyc lint --docs > docs/lints.md`");
}

#[test]
fn the_switch_while_and_result_lints_fire() {
    let dir = work_dir("lint-three-rules");
    write(
        &dir,
        "main.ya",
        "fn main() {\n  if x == 1 { } else if x == 2 { } else if x == 3 { }\n  while true { }\n  let _x = read_file(\"a.txt\");\n}\n",
    );
    let output = run(&dir, &["lint", "--json", "main.ya"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    for code in ["LINT-SWITCH", "LINT-WHILE", "LINT-RESULT"] {
        assert!(stdout.contains(code), "{code} is missing from {stdout}");
    }
}

#[test]
fn a_handled_result_is_not_linted() {
    let dir = work_dir("lint-result-handled");
    write(
        &dir,
        "main.ya",
        "fn main() {\n  let x = read_file(\"a.txt\")?;\n}\n",
    );
    let output = run(&dir, &["lint", "--json", "main.ya"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("LINT-RESULT"), "{stdout}");
}

#[test]
fn a_bare_condvar_wait_is_advised_and_a_predicate_loop_is_not() {
    let dir = work_dir("lint-condwait");
    write(
        &dir,
        "bare.ya",
        "import sync;\nfn main() {\n    let cv = sync.condvar();\n    cv.wait(0);\n}\n",
    );
    let output = run(&dir, &["lint", "--json", "bare.ya"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("LINT-CONDWAIT"), "{stdout}");

    write(
        &dir,
        "guarded.ya",
        "import sync;\nfn main() {\n    let cv = sync.condvar();\n    while cv.generation() == 0 {\n        cv.wait(0);\n    }\n}\n",
    );
    let output = run(&dir, &["lint", "--json", "guarded.ya"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("LINT-CONDWAIT"), "{stdout}");
}
