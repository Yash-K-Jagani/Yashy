//! `yashyc inspect`: one symbol's facts, from the same index `index` renders.

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

fn project(dir: &std::path::Path) {
    write(
        dir,
        "main.ya",
        "/// Adds two numbers.\nfn add(int a, int b) -> int { return a + b; }\nfn main() { print(add(1, 2)); }\n",
    );
}

#[test]
fn inspect_json_names_a_symbol_and_its_span() {
    let dir = work_dir("inspect-json");
    project(&dir);
    let output = run(&dir, &["inspect", "--json", "main.ya", "add"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for field in [
        "\"name\": \"add\"",
        "\"signature\"",
        "\"file\"",
        "\"span\"",
        "\"docs\"",
    ] {
        assert!(stdout.contains(field), "missing {field} in {stdout}");
    }
}

#[test]
fn inspect_lists_symbols_when_no_name_is_given() {
    let dir = work_dir("inspect-list");
    project(&dir);
    let output = run(&dir, &["inspect", "--json", "main.ya"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("add"), "{stdout}");
    assert!(stdout.contains("main"), "{stdout}");
}

#[test]
fn an_unknown_symbol_is_a_diagnostic() {
    let dir = work_dir("inspect-missing");
    project(&dir);
    let output = run(&dir, &["inspect", "--json", "main.ya", "nonesuch"]);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("no symbol"));
}

#[test]
fn inspect_human_prints_a_path_line_and_column() {
    let dir = work_dir("inspect-human");
    project(&dir);
    let output = run(&dir, &["inspect", "--human", "main.ya", "add"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    // `add` is declared on the second line of the fixture, one column in.
    assert!(
        stdout.contains("main.ya:2:1:"),
        "the human form must be clickable `path:line:col`: {stdout}"
    );
    assert!(stdout.contains("fn add"), "{stdout}");
}

#[test]
fn inspect_layout_reports_size_alignment_and_fields() {
    let dir = work_dir("inspect-layout");
    write(
        &dir,
        "main.ya",
        "struct Point { x: int, y: int }\nfn main() { let p = Point { x: 1, y: 2 }; }\n",
    );
    let output = run(&dir, &["inspect", "--layout", "main.ya", "Point"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"size\": 16"), "{stdout}");
    assert!(stdout.contains("\"align\": 8"), "{stdout}");
    assert!(stdout.contains("\"field\": \"x\""), "{stdout}");
    assert!(stdout.contains("\"field\": \"y\""), "{stdout}");

    // The same facts are part of the symbol itself, so one `inspect --json`
    // call answers a function or a type.
    let symbol = run(&dir, &["inspect", "--json", "main.ya", "Point"]);
    assert!(
        symbol.status.success(),
        "{}",
        String::from_utf8_lossy(&symbol.stderr)
    );
    let stdout = String::from_utf8_lossy(&symbol.stdout);
    assert!(stdout.contains("\"kind\": \"struct\""), "{stdout}");
    assert!(stdout.contains("\"layout\""), "{stdout}");
    assert!(stdout.contains("\"size\": 16"), "{stdout}");
    assert!(stdout.contains("\"field\": \"x\""), "{stdout}");
}
