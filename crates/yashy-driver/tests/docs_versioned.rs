//! Versioned docs: `yashyc doc --versioned` writes one tree per version,
//! points `current` at the last one, and keeps a redirect map for moved pages.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace() -> PathBuf {
    let dir = common::work_dir("versioned-docs");
    common::write(
        &dir,
        "main.ya",
        "/// Adds two numbers.\npub fn add(int a, int b) -> int {\n    return a + b;\n}\n\nfn main() { print(add(1, 2)); }\n",
    );
    dir
}

fn run(dir: &Path, args: &[&str]) -> (bool, String, String) {
    let output = Command::new(common::yashyc())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("the CLI must run");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn two_versions_coexist_and_current_points_at_the_last() {
    let dir = workspace();
    let (ok, stdout, stderr) = run(&dir, &["doc", "--versioned", "--version", "1.0.0"]);
    assert!(ok, "{stdout} {stderr}");
    let (ok, stdout, stderr) = run(&dir, &["doc", "--versioned", "--version", "2.0.0"]);
    assert!(ok, "{stdout} {stderr}");
    assert!(dir.join("docs/versioned/1.0.0/manifest.json").is_file());
    assert!(dir.join("docs/versioned/2.0.0/manifest.json").is_file());
    assert_eq!(
        fs::read_to_string(dir.join("docs/versioned/current"))
            .unwrap()
            .trim(),
        "2.0.0"
    );
    assert!(dir.join("docs/versioned/redirects.json").is_file());
}

#[test]
fn a_default_version_uses_the_compiler_version() {
    let dir = workspace();
    let (ok, stdout, stderr) = run(&dir, &["doc", "--versioned"]);
    assert!(ok, "{stdout} {stderr}");
    let current = fs::read_to_string(dir.join("docs/versioned/current"))
        .unwrap()
        .trim()
        .to_string();
    assert!(!current.is_empty());
    assert!(dir.join("docs/versioned").join(&current).is_dir());
}

#[test]
fn json_and_versioned_cannot_be_combined() {
    let dir = workspace();
    let (ok, _, stderr) = run(&dir, &["doc", "--versioned", "--json"]);
    assert!(!ok, "{stderr}");
    assert!(stderr.contains("not a file write"), "{stderr}");
}
