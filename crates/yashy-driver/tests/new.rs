//! `yashyc new <name>` creates a project whose first test passes.

mod common;

use common::{work_dir, yashyc};
use std::fs;
use std::path::Path;
use std::process::Command;

fn new(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(yashyc())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("the CLI must run")
}

#[test]
fn new_lists_its_templates() {
    let dir = work_dir("new-list");
    let output = new(&dir, &["new", "--list"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    for template in yashy_driver::scaffold::TEMPLATES {
        assert!(stdout.lines().any(|line| line == *template), "{stdout}");
    }
}

#[test]
fn new_creates_a_project_whose_test_passes() {
    let dir = work_dir("new-bin");
    let output = new(&dir, &["new", "hello", "--template", "bin"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let project = dir.join("hello");
    assert!(project.join("yashy.toml").is_file());
    assert!(project.join("main.ya").is_file());
    assert!(project.join("tests/smoke.ya").is_file());

    let tested = Command::new(yashyc())
        .arg("test")
        .current_dir(&project)
        .output()
        .expect("the CLI must run");
    assert!(
        tested.status.success(),
        "the new project's first test must pass:\n{}",
        String::from_utf8_lossy(&tested.stdout)
    );
}

#[test]
fn an_unknown_template_is_a_diagnostic_not_a_panic() {
    let dir = work_dir("new-bad-template");
    let output = new(&dir, &["new", "app", "--template", "nonesuch"]);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown template"), "{stderr}");
    assert!(
        !dir.join("app").exists(),
        "a failed new must not leave a project"
    );
}

#[test]
fn new_refuses_a_non_empty_directory() {
    let dir = work_dir("new-existing");
    fs::create_dir_all(dir.join("taken")).expect("the fixture directory");
    fs::write(dir.join("taken/keep.txt"), "keep").expect("the fixture file");
    let output = new(&dir, &["new", "taken"]);
    assert!(!output.status.success());
    assert!(fs::read_to_string(dir.join("taken/keep.txt"))
        .expect("the original file remains")
        .contains("keep"));
}
