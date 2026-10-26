mod common;

use common::{work_dir, yashy};
use std::fs;
use std::process::Command;

#[test]
fn init_creates_a_buildable_and_testable_project() {
    let parent = work_dir("init-create");
    let project = parent.join("hello");

    let output = Command::new(yashy())
        .arg("init")
        .arg(&project)
        .output()
        .expect("the yashy CLI must be runnable");
    assert!(
        output.status.success(),
        "init must succeed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(project.join("yashy.toml").is_file());
    assert!(project.join("main.ya").is_file());
    assert!(project.join("tests/smoke.ya").is_file());
    assert!(project.join("README.md").is_file());
    assert!(project.join("AGENTS.md").is_file());

    let gitignore =
        fs::read_to_string(project.join(".gitignore")).expect("the .gitignore must be readable");
    assert!(gitignore.contains("*.ll"), ".gitignore:\n{gitignore}");
    assert!(gitignore.contains("*.yashy/"), ".gitignore:\n{gitignore}");
    assert!(gitignore.contains("/hello"), ".gitignore:\n{gitignore}");

    let readme =
        fs::read_to_string(project.join("README.md")).expect("the README must be readable");
    assert!(readme.contains("yashy run main.ya"), "README:\n{readme}");
    assert!(readme.contains("yashy test"), "README:\n{readme}");
    assert!(readme.contains("yashy build main.ya"), "README:\n{readme}");

    let agents = fs::read_to_string(project.join("AGENTS.md")).expect("AGENTS.md must be readable");
    assert!(agents.contains("## Commands"), "AGENTS.md:\n{agents}");
    assert!(agents.contains("## Layout"), "AGENTS.md:\n{agents}");
    assert!(agents.contains("## Rules"), "AGENTS.md:\n{agents}");
    assert!(agents.contains("## Docs"), "AGENTS.md:\n{agents}");
    assert!(agents.contains("yashy test"), "AGENTS.md:\n{agents}");

    let build = Command::new(yashy())
        .arg("build")
        .arg(project.join("main.ya"))
        .arg("-o")
        .arg(project.join("hello"))
        .output()
        .expect("the yashy CLI must be runnable");
    assert!(
        build.status.success(),
        "the generated main.ya must compile:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let test = Command::new(yashy())
        .arg("test")
        .current_dir(&project)
        .output()
        .expect("the yashy CLI must be runnable");
    assert!(
        test.status.success(),
        "the generated smoke test must pass:\n{}\n{}",
        String::from_utf8_lossy(&test.stdout),
        String::from_utf8_lossy(&test.stderr)
    );
}

#[test]
fn init_refuses_a_non_empty_directory() {
    let dir = work_dir("init-refuse");
    fs::write(dir.join("keep.txt"), "keep").expect("the fixture must be writable");

    let output = Command::new(yashy())
        .arg("init")
        .arg(&dir)
        .output()
        .expect("the yashy CLI must be runnable");

    assert!(!output.status.success());
    assert!(dir.join("keep.txt").is_file());
    assert!(!dir.join("main.ya").exists());
}

#[test]
fn init_uses_the_current_directory_when_no_path_is_given() {
    let dir = work_dir("init-current");
    let output = Command::new(yashy())
        .arg("init")
        .current_dir(&dir)
        .output()
        .expect("the yashy CLI must be runnable");

    assert!(
        output.status.success(),
        "init must succeed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(dir.join("main.ya").is_file());
}
