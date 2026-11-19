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

#[test]
fn the_workspace_template_creates_two_members() {
    let dir = work_dir("new-workspace-template");
    let output = new(&dir, &["new", "shop", "--template", "workspace"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let root = dir.join("shop");
    assert!(root.join("yashy.toml").is_file());
    assert!(root.join("app/yashy.toml").is_file());
    assert!(root.join("lib/src/lib.ya").is_file());
    let manifest = fs::read_to_string(root.join("yashy.toml")).unwrap();
    assert!(manifest.contains("[workspace]"));
    assert!(!manifest.contains("[package]"));
}

#[test]
fn service_and_native_templates_have_tests_and_docs() {
    for template in ["service", "native"] {
        let dir = work_dir(&format!("new-{template}-template"));
        let output = new(&dir, &["new", "demo", "--template", template]);
        assert!(
            output.status.success(),
            "{template}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let root = dir.join("demo");
        assert!(root.join("tests/smoke.ya").is_file(), "{template}");
        assert!(root.join("README.md").is_file(), "{template}");
        assert!(root.join("main.ya").is_file(), "{template}");
        if template == "native" {
            assert!(root.join("fixture.c").is_file());
            assert!(root.join("bindings.ya").is_file());
            assert!(root.join("safe.ya").is_file());
        }
    }
}

#[test]
fn the_native_template_links_its_vendored_c_source() {
    // The template is a `-sys` package: the ABI is in `bindings.ya`, the safe
    // wrapper is in `safe.ya`, and the C fixture is a build input named by
    // `link "fixture.c"`. Both commands therefore need no `cc` step of their
    // own, and the test really calls into C. A skeleton that compiled but never
    // reached the fixture would pass on this host and fail in CI, which is
    // exactly the "placeholder that lies" the plan refuses.
    let dir = work_dir("new-native-link");
    let output = new(&dir, &["new", "native-demo", "--template", "native"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let project = dir.join("native-demo");

    let tested = Command::new(yashyc())
        .arg("test")
        .current_dir(&project)
        .output()
        .expect("the CLI must run");
    assert!(
        tested.status.success(),
        "the native template's test must link and pass:\n{}\n{}",
        String::from_utf8_lossy(&tested.stdout),
        String::from_utf8_lossy(&tested.stderr)
    );

    let exe = project.join(format!("native-demo{}", std::env::consts::EXE_SUFFIX));
    let built = Command::new(yashyc())
        .arg("build")
        .arg("main.ya")
        .arg("-o")
        .arg(&exe)
        .current_dir(&project)
        .output()
        .expect("the CLI must run");
    assert!(
        built.status.success(),
        "the wrapper and fixture must link together:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let ran = Command::new(&exe)
        .output()
        .expect("the built program must run");
    assert!(
        ran.status.success(),
        "the built program must exit 0: {}",
        String::from_utf8_lossy(&ran.stderr)
    );
    let stdout = String::from_utf8_lossy(&ran.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines,
        vec!["3", "overflow"],
        "the wrapper turns C's unchecked add into a checked value: {stdout}"
    );
}
