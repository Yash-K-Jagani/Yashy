//! `yashyc audit`: manifests, lock agreement, licenses, and build scripts.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::{work_dir, write, yashyc};

fn cli(directory: &Path, args: &[&str]) -> Output {
    let store = work_dir("audit-store");
    Command::new(yashyc())
        .args(args)
        .current_dir(directory)
        .env("YASHY_PACKAGE_STORE", &store)
        .env("YASHY_CACHE_DIR", store.join("cache"))
        .output()
        .expect("the yashyc CLI must be runnable")
}

fn write_project(root: &Path, license: bool) {
    write(
        root,
        "yashy.toml",
        &format!(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n{}\n\n[dependencies.lib]\npath = \"../lib\"\n",
            if license { "license = \"MIT\"" } else { "" }
        ),
    );
    write(root, "main.ya", "fn main() { }\n");
}

fn write_lib(root: &Path, license: bool) {
    write(
        root,
        "yashy.toml",
        &format!(
            "[package]\nname = \"lib\"\nversion = \"0.1.0\"\n{}\n",
            if license { "license = \"MIT\"" } else { "" }
        ),
    );
    write(root, "src/lib.ya", "pub fn value() -> int { return 7; }\n");
}

fn clean_project(name: &str) -> (PathBuf, PathBuf) {
    let dir = work_dir(name);
    write_project(&dir.join("app"), true);
    write_lib(&dir.join("lib"), true);
    (dir.join("app"), dir.join("lib"))
}

#[test]
fn a_clean_project_is_ok_in_json() {
    let (app, _) = clean_project("audit-clean");
    let update = cli(&app, &["update"]);
    assert!(
        update.status.success(),
        "update:\n{}",
        String::from_utf8_lossy(&update.stderr)
    );
    let output = cli(&app, &["audit", "--json"]);
    assert!(
        output.status.success(),
        "clean project:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("\"schema_version\": 1"));
    assert!(text.contains("\"ok\": true"));
    assert!(text.contains("\"findings\": []"));
}

#[test]
fn a_signed_advisory_is_matched_against_the_resolved_project() {
    use yashy_driver::advisory::{sign_feed, Advisory};

    let (app, _) = clean_project("audit-advisory");
    assert!(cli(&app, &["update"]).status.success());
    let advisory = Advisory {
        id: "YASHY-2027-0042".to_string(),
        package: "app".to_string(),
        range: "<1.0.0".to_string(),
        severity: "high".to_string(),
        fixed: "1.0.0".to_string(),
        title: "affected release".to_string(),
    };
    let feed = sign_feed(vec![advisory], "release", &[9u8; 32]).expect("a signed feed");
    std::fs::write(app.join("feed.json"), feed).unwrap();

    let output = cli(&app, &["audit", "--json", "--feed", "feed.json"]);
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("YASHY-2027-0042"), "{text}");
    assert!(text.contains("fixed in 1.0.0"), "{text}");

    // A range that does not contain the resolved version leaves the audit ok.
    let advisory = Advisory {
        id: "YASHY-2027-0043".to_string(),
        package: "app".to_string(),
        range: "<0.0.1".to_string(),
        severity: "low".to_string(),
        fixed: "0.0.1".to_string(),
        title: "unrelated release".to_string(),
    };
    let feed = sign_feed(vec![advisory], "release", &[9u8; 32]).expect("a signed feed");
    std::fs::write(app.join("feed.json"), feed).unwrap();
    let output = cli(&app, &["audit", "--json", "--feed", "feed.json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"ok\": true"));
}

#[test]
fn an_unreadable_advisory_feed_is_a_usage_error() {
    let (app, _) = clean_project("audit-bad-feed");
    let output = cli(&app, &["audit", "--feed", "missing.json"]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn a_lockfile_mismatch_is_an_e1802_finding() {
    let (app, lib) = clean_project("audit-mismatch");
    assert!(cli(&app, &["update"]).status.success());
    write(&lib, "src/lib.ya", "pub fn value() -> int { return 8; }\n");
    let output = cli(&app, &["audit", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(text.contains("E1802"), "{text}");
    assert!(!text.contains("\"ok\": true"));
}

#[test]
fn a_git_dependency_without_a_rev_is_e1800() {
    let dir = work_dir("audit-git-no-rev");
    write(
        &dir,
        "yashy.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nlicense = \"MIT\"\n\n[dependencies.extra]\ngit = \"https://example.invalid/repo.git\"\n",
    );
    write(&dir, "main.ya", "fn main() { }\n");
    let output = cli(&dir, &["audit", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(text.contains("E1800"), "{text}");
}

#[test]
fn a_package_without_a_license_is_e1804() {
    let (app, _) = clean_project("audit-no-license");
    write_lib(&app.parent().unwrap().join("lib"), false);
    let output = cli(&app, &["audit", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(text.contains("E1804"), "{text}");
    assert!(text.contains("lib"), "{text}");
}

#[test]
fn a_package_with_a_build_script_is_e1804() {
    let (app, lib) = clean_project("audit-build-script");
    write(&lib, "build.rs", "fn main() {}\n");
    let output = cli(&app, &["audit", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(text.contains("E1804"), "{text}");
    assert!(text.contains("build.rs"), "{text}");
}

#[test]
fn audit_policy_denies_a_disallowed_license() {
    let dir = work_dir("audit-policy");
    write(
        &dir,
        "yashy.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nlicense = \"GPL-3.0\"\n\n\
         [audit]\nallowed-licenses = [\"MIT\"]\n",
    );
    write(&dir, "main.ya", "fn main() {}\n");
    let report = yashy_driver::audit::audit_project(&dir, &yashy_driver::Store::open().unwrap())
        .expect("audit runs");
    assert!(!report.ok);
    assert!(report
        .findings
        .iter()
        .any(|finding| finding.code == yashy_common::codes::POLICY_DENIED));
}
