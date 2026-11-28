//! `yashyc migrate`: edition rewrite with an honest dry run.

mod common;

use std::fs;
use std::process::Command;

fn yashyc() -> &'static str {
    common::yashyc()
}

#[test]
fn dry_run_prints_the_diff_and_changes_nothing() {
    let dir = common::work_dir("migrate-dry-run");
    let manifest = "[package]\nname = \"demo\"\nversion = \"1.0.0\"\n";
    common::write(&dir, "yashy.toml", manifest);
    let output = Command::new(yashyc())
        .args(["migrate", "--dry-run", "yashy.toml"])
        .current_dir(&dir)
        .output()
        .expect("the CLI must run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("+ edition = \"2027\""), "{stdout}");
    assert_eq!(
        fs::read_to_string(dir.join("yashy.toml")).unwrap(),
        manifest
    );
}

#[test]
fn migrate_adds_the_edition_key() {
    let dir = common::work_dir("migrate-apply");
    common::write(
        &dir,
        "yashy.toml",
        "[package]\nname = \"demo\"\nversion = \"1.0.0\"\n\n[dependencies]\n",
    );
    let output = Command::new(yashyc())
        .args(["migrate", "yashy.toml"])
        .current_dir(&dir)
        .output()
        .expect("the CLI must run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let migrated = fs::read_to_string(dir.join("yashy.toml")).unwrap();
    assert!(
        migrated.contains("[package]\nedition = \"2027\"\nname"),
        "{migrated}"
    );
    assert!(migrated.contains("\n[dependencies]\n"), "{migrated}");
}

#[test]
fn an_unknown_target_edition_is_refused_by_the_library() {
    let source = "[package]\nname = \"demo\"\n";
    assert!(yashy_driver::migrate::rewrite(source, "2026").is_err());
}

#[test]
fn source_migration_rewrites_the_e0109_spellings() {
    let dir = common::work_dir("migrate-source");
    common::write(
        &dir,
        "main.ya",
        "struct Point { x: int }\nfn add(a: int, b: int) -> int {\n    let sum: int = a + b;\n    return sum;\n}\nfn main() {\n    print(add(1, 2));\n}\n",
    );
    let output = Command::new(yashyc())
        .args(["migrate", "--source", "main.ya"])
        .current_dir(&dir)
        .output()
        .expect("the CLI must run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let migrated = fs::read_to_string(dir.join("main.ya")).unwrap();
    assert!(migrated.contains("fn add(int a, int b)"), "{migrated}");
    assert!(migrated.contains("let int sum = a + b;"), "{migrated}");
    assert!(migrated.contains("struct Point { x: int }"), "{migrated}");

    let checked = Command::new(yashyc())
        .args(["check", "main.ya"])
        .current_dir(&dir)
        .output()
        .expect("the CLI must run");
    assert!(
        checked.status.success(),
        "the migrated source checks: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
}

#[test]
fn a_source_dry_run_prints_the_diff_and_changes_nothing() {
    let dir = common::work_dir("migrate-source-dry-run");
    let original = "fn add(a: int) -> int {\n    let value: int = a;\n    return value;\n}\n";
    common::write(&dir, "main.ya", original);
    let output = Command::new(yashyc())
        .args(["migrate", "--dry-run", "--source", "main.ya"])
        .current_dir(&dir)
        .output()
        .expect("the CLI must run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("+ fn add(int a)"), "{stdout}");
    assert!(stdout.contains("- fn add(a: int)"), "{stdout}");
    assert!(stdout.contains("let int value = a;"), "{stdout}");
    assert_eq!(fs::read_to_string(dir.join("main.ya")).unwrap(), original);
}
