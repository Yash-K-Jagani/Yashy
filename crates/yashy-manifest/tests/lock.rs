//! The lockfile's promises: sorted bytes, a content hash that follows the
//! source, and `E1802` when the manifest and the lock disagree.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use yashy_manifest::lock::{hash_tree, LockedPackage, Lockfile};
use yashy_manifest::{Dependency, Manifest};

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

fn temp_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "yashy-lock-test-{}-{}-{label}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("create the test directory");
    path
}

fn package(name: &str, version: &str, source: &str, hash: &str) -> LockedPackage {
    LockedPackage {
        name: name.to_string(),
        version: version.to_string(),
        source: source.to_string(),
        hash: format!("sha256:{}", hash.repeat(64 / hash.len())),
        dependencies: Vec::new(),
    }
}

fn manifest(text: &str) -> Manifest {
    Manifest::parse(text).expect("the test manifest parses")
}

#[test]
fn the_manifest_version_change_names_both_versions() {
    let lock = Lockfile::new(vec![package(
        "http",
        "0.3.1",
        "registry+file:///registry",
        "a",
    )]);
    let manifest = manifest(
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nhttp = \"0.3.2\"\n",
    );
    let error = lock
        .check_manifest(&manifest, Path::new("."))
        .expect_err("the versions differ");
    assert_eq!(error.code(), yashy_common::codes::LOCKFILE_MISMATCH);
    assert!(error.message.contains("0.3.2"), "{error}");
    assert!(error.message.contains("0.3.1"), "{error}");
    assert!(error.message.contains("yashy update"), "{error}");
}

#[test]
fn a_missing_lock_entry_names_the_package_and_the_command() {
    let lock = Lockfile::new(Vec::new());
    let manifest = manifest(
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nhttp = \"0.3.1\"\n",
    );
    let error = lock
        .check_manifest(&manifest, Path::new("."))
        .expect_err("the lock is empty");
    assert!(error.message.contains("http"), "{error}");
    assert!(error.message.contains("yashy update"), "{error}");
}

#[test]
fn a_hand_edited_hash_is_e1802() {
    let root = temp_dir("hash-mismatch");
    std::fs::create_dir_all(root.join("net/src")).unwrap();
    std::fs::write(root.join("net/src/net.ya"), "pub fn open() {}\n").unwrap();
    std::fs::write(
        root.join("yashy.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies.net]\npath = \"net\"\n",
    )
    .unwrap();

    let actual = hash_tree(&root.join("net")).expect("hash the dependency");
    let lock = Lockfile::new(vec![LockedPackage {
        name: "net".into(),
        version: "0.1.0".into(),
        source: LockedPackage::path_source("net"),
        hash: format!("sha256:{}", "0".repeat(64)),
        dependencies: Vec::new(),
    }]);
    let manifest = Manifest::load(&root.join("yashy.toml")).expect("the manifest loads");
    let error = lock
        .check_manifest(&manifest, &root)
        .expect_err("the hash is wrong");
    assert_eq!(error.code(), yashy_common::codes::LOCKFILE_MISMATCH);
    assert!(error.message.contains(&actual), "{error}");
    assert!(error.message.contains(&"0".repeat(64)), "{error}");
    assert!(error.message.contains("yashy update"), "{error}");

    // The same lock with the real hash passes.
    let good = Lockfile::new(vec![LockedPackage {
        name: "net".into(),
        version: "0.1.0".into(),
        source: LockedPackage::path_source("net"),
        hash: actual,
        dependencies: Vec::new(),
    }]);
    good.check_manifest(&manifest, &root)
        .expect("the real hash matches");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_path_dependency_with_the_wrong_source_is_named() {
    let root = temp_dir("source-mismatch");
    std::fs::create_dir_all(root.join("net")).unwrap();
    std::fs::write(root.join("net/lib.ya"), "pub fn x() {}\n").unwrap();
    let manifest = manifest(
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies.net]\npath = \"net\"\n",
    );
    let lock = Lockfile::new(vec![package(
        "net",
        "0.1.0",
        "path+somewhere-else",
        "a",
    )]);
    let error = lock
        .check_manifest(&manifest, &root)
        .expect_err("the sources differ");
    assert!(error.message.contains("path+net"), "{error}");
    assert!(error.message.contains("does not record"), "{error}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_lockfile_is_sorted_by_name_then_version() {
    let lock = Lockfile::new(vec![
        package("string", "1.0.0", "registry+r", "a"),
        package("http", "0.4.0", "registry+r", "b"),
        package("http", "0.3.1", "registry+r", "c"),
    ]);
    let rendered = lock.render();
    let http_old = rendered.find("version = \"0.3.1\"").expect("http 0.3.1");
    let http_new = rendered.find("version = \"0.4.0\"").expect("http 0.4.0");
    let string = rendered.find("name = \"string\"").expect("string");
    assert!(http_old < http_new, "name then version, ascending");
    assert!(http_new < string, "http sorts before string");
    assert_eq!(Lockfile::parse(&rendered).expect("reparse"), lock);
}

#[test]
fn a_lock_missing_a_package_field_is_e1802() {
    let text = "version = 1\n\n[[package]]\nname = \"a\"\nversion = \"1.0.0\"\n\
                source = \"registry+r\"\n";
    let error = Lockfile::parse(text).expect_err("no hash");
    assert_eq!(error.code(), yashy_common::codes::LOCKFILE_MISMATCH);
    assert!(error.message.contains("hash"), "{error}");
}

#[test]
fn a_registry_dependency_is_checked_by_name_and_version() {
    let lock = Lockfile::new(vec![package("http", "0.3.1", "registry+file:///r", "a")]);
    let manifest = manifest(
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nhttp = \"0.3.1\"\n",
    );
    lock.check_manifest(&manifest, Path::new("."))
        .expect("the pin matches");
    assert_eq!(
        manifest.dependencies["http"],
        Dependency::Registry {
            version: "0.3.1".into()
        }
    );
}

#[test]
fn git_and_path_sources_have_one_spelling() {
    assert_eq!(LockedPackage::path_source("../net"), "path+../net");
    assert_eq!(
        LockedPackage::git_source("https://example.invalid/a", "0123456789abcdef"),
        "git+https://example.invalid/a?rev=0123456789abcdef"
    );
}
