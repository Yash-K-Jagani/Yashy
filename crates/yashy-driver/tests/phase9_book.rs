//! The book and examples stay buildable: every chapter is linked and every
//! example project carries its own test.

mod common;

use std::path::Path;
use std::process::Command;

#[test]
fn book_links_and_examples_are_complete() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let summary = std::fs::read_to_string(root.join("book/src/SUMMARY.md"))
        .expect("the mdBook summary must exist");
    let mut links = 0;
    for line in summary.lines() {
        let Some(path) = line.find("](").and_then(|at| line[at + 2..].find(')').map(|end| &line[at + 2..at + 2 + end])) else {
            continue;
        };
        if path.starts_with("http") {
            continue;
        }
        links += 1;
        assert!(
            root.join("book/src").join(path).is_file(),
            "broken book link: {path}"
        );
    }
    assert!(links > 5, "the book has too few linked chapters");

    for name in ["api-service", "cli-tool", "web-app", "hello", "wordcount", "shop"] {
        let dir = root.join("examples").join(name);
        assert!(dir.join("yashy.toml").is_file(), "{name} is not a project");
        assert!(dir.join("main.ya").is_file(), "{name} has no main.ya");
        assert!(dir.join("tests/smoke.ya").is_file(), "{name} has no smoke test");
    }
}

#[test]
fn tutorial_harness_still_runs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new(common::yashyc())
        .arg("test")
        .arg("docs/tutorial")
        .current_dir(&root)
        .output()
        .expect("the yashyc CLI must run");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}
