//! Phase 3 Task 12: the package query and mutation surface.

mod common;

use std::path::Path;

use yashy_manifest::{LockedPackage, Lockfile};

fn fixture(dir: &Path) {
    common::write(
        dir,
        "yashy.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies.lib]\npath = \"lib\"\n",
    );
    common::write(dir, "main.ya", "fn main() {}\n");
    let lock = Lockfile::new(vec![
        LockedPackage {
            name: "app".to_string(),
            version: "0.1.0".to_string(),
            source: "path+.".to_string(),
            hash: "sha256:aaa".to_string(),
            dependencies: vec!["lib 1.0.0".to_string()],
        },
        LockedPackage {
            name: "lib".to_string(),
            version: "1.0.0".to_string(),
            source: "path+lib".to_string(),
            hash: "sha256:bbb".to_string(),
            dependencies: Vec::new(),
        },
    ]);
    common::write(dir, "yashy.lock", &lock.render());
}

#[test]
fn tree_and_why_read_the_same_graph() {
    let dir = common::work_dir("pkg-tree");
    fixture(&dir);
    let lock = Lockfile::load(&dir.join("yashy.lock")).unwrap();
    let lines = yashy_driver::pkg_cmd::tree_lines(&lock, 2);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].starts_with("app"), "{lines:?}");
    assert!(lines[1].contains("lib"), "{lines:?}");
    let paths = yashy_driver::pkg_cmd::why(&lock, "lib");
    assert_eq!(paths, vec![vec!["app".to_string(), "lib".to_string()]]);
}

#[test]
fn info_names_reverse_dependencies() {
    let dir = common::work_dir("pkg-info");
    fixture(&dir);
    let lock = Lockfile::load(&dir.join("yashy.lock")).unwrap();
    let report = yashy_driver::pkg_cmd::info(&lock, "lib", &dir).unwrap();
    let text = report.render();
    assert!(text.contains("\"name\": \"lib\""), "{text}");
    assert!(text.contains("app"), "reverse dependency missing: {text}");
}

#[test]
fn remove_writes_a_backup_and_drops_the_dependency() {
    let dir = common::work_dir("pkg-remove");
    fixture(&dir);
    let files = yashy_driver::pkg_cmd::remove(&dir, "lib").unwrap();
    assert!(files
        .iter()
        .any(|path| path.to_string_lossy().ends_with("yashy.toml.bak")));
    let manifest = std::fs::read_to_string(dir.join("yashy.toml")).unwrap();
    assert!(!manifest.contains("dependencies.lib"), "{manifest}");
    let lock = Lockfile::load(&dir.join("yashy.lock")).unwrap();
    assert!(lock.packages.iter().all(|package| package.name != "lib"));
}
