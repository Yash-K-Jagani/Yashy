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
fn info_reports_link_values_and_not_the_abi() {
    // `native_libraries` is what a package manager reads to know which system
    // libraries an install needs. The ABI is `"C"`, not a library, so reporting
    // it would ask the user to install a library named C. A vendored source is
    // a build input, not a system library, so it is not reported either.
    let dir = common::work_dir("pkg-info-native");
    fixture(&dir);
    common::write(
        &dir,
        "bindings.ya",
        "extern \"C\" link \"m\" {\n    unsafe fn c_sqrt(f64 x) -> f64 link_name \"sqrt\";\n}\n\n\
         extern \"C\" link \"fixture.c\" {\n    unsafe fn add(int a, int b) -> int;\n}\n",
    );
    let lock = Lockfile::load(&dir.join("yashy.lock")).unwrap();
    let report = yashy_driver::pkg_cmd::info(&lock, "lib", &dir).unwrap();
    let text = report.render();
    assert!(
        text.contains("\"native_libraries\": [\n    \"m\"\n  ]"),
        "the `link` clause is the library: {text}"
    );
    assert!(
        !text.contains("fixture.c"),
        "a vendored source is not a system library: {text}"
    );
    assert!(!text.contains("\"C\""), "the ABI is not a library: {text}");
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
#[test]
fn vendor_produces_a_complete_local_registry() {
    let dir = common::work_dir("pkg-vendor");
    let registry = dir.join("registry");
    let vendored = dir.join("vendor");
    common::write(&registry, "index.json", "{\"packages\":[]}\n");
    std::fs::create_dir_all(registry.join("packages")).unwrap();
    common::write(&registry, "packages/abc.tar", "binary");
    let (count, _) = yashy_driver::pkg_cmd::vendor(&registry, &vendored).unwrap();
    assert_eq!(count, 0, "an empty index has no entries to count");
    assert!(vendored.join("index.json").is_file());
    assert!(vendored.join("packages").is_dir());
    // The vendor tree is a registry source that can be read back.
    assert!(yashy_driver::registry::read_index(&vendored).is_ok());
}
#[test]
fn tree_marks_duplicates_and_enabled_features() {
    let dir = common::work_dir("pkg-tree-marks");
    let lock = Lockfile::new(vec![
        LockedPackage { name: "app".into(), version: "0.1.0".into(), source: "path+.".into(), hash: "sha256:a".into(), dependencies: vec![] },
        LockedPackage { name: "lib".into(), version: "1.0.0".into(), source: "path+lib".into(), hash: "sha256:b".into(), dependencies: vec![] },
        LockedPackage { name: "lib".into(), version: "2.0.0".into(), source: "path+lib2".into(), hash: "sha256:c".into(), dependencies: vec![] },
    ]);
    let features = std::collections::BTreeSet::from(["tls".to_string()]);
    let lines = yashy_driver::pkg_cmd::tree_lines_with_features(&lock, 2, &features);
    assert!(lines.iter().any(|line| line.contains("duplicate")), "{lines:?}");
    assert!(lines.iter().any(|line| line.contains("feature:tls")), "{lines:?}");
}
