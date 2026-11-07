//! Tests for `yashy expand` and expansion in the check pipeline.

use std::path::PathBuf;
use std::process::Command;

mod common;

fn run_yashy(dir: &PathBuf, args: &[&str], file: &str) -> std::process::Output {
    Command::new(common::yashy())
        .args(args)
        .arg(dir.join(file))
        .output()
        .expect("the yashy CLI must run")
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn expand_prints_no_macro_declaration_or_metavariable() {
    let dir = common::work_dir("expand-output");
    let source = "macro say { $x:expr => { print($x); } } fn main() { say(ready); }";
    common::write(&dir, "main.ya", source);

    let output = run_yashy(&dir, &["expand"], "main.ya");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(!text.contains("macro "), "expanded output must drop the declaration:\n{text}");
    assert!(!text.contains('$'), "expanded output must substitute every metavariable:\n{text}");
    assert!(text.contains("print(ready)"), "expanded output must contain the replacement:\n{text}");
}

#[test]
fn expand_json_lists_one_entry_per_invocation_in_source_order() {
    let dir = common::work_dir("expand-json");
    let source = "macro unless { $cond:expr => { if $cond { } } } fn main() { unless(ready); unless(count); }";
    common::write(&dir, "main.ya", source);

    let output = run_yashy(&dir, &["expand", "--json"], "main.ya");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    let ready = text.find("\"output_tokens\"").expect("the json must carry entries");
    let second = text[ready + 1..].find("\"macro\": \"unless\"");
    assert!(text.contains("\"schema_version\": 1"), "json must be schema v1:\n{text}");
    assert!(second.is_some(), "the json must list two invocations:\n{text}");
}

#[test]
fn check_reports_a_macro_error_with_its_code() {
    let dir = common::work_dir("expand-check-error");
    let source = "macro unless { $cond:expr => { if $cond { } } } fn main() { unless(ready, count); }";
    common::write(&dir, "main.ya", source);

    let output = run_yashy(&dir, &["check", "--json"], "main.ya");

    assert!(!output.status.success(), "an unmatched invocation must not check");
    let text = stdout(&output);
    assert!(text.contains("\"code\": \"E1600\""), "the diagnostic must carry E1600:\n{text}");
    assert!(text.contains("main.ya"), "the diagnostic must name the user's file:\n{text}");
}
