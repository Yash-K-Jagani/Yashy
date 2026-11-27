//! Migration guides are tested programs: every `yashy` block must check with
//! the compiler that ships it, and the `edition` claim must match
//! `yashyc migrate --dry-run` on a real manifest.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the repo root is readable")
}

fn yashy_blocks(page: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut lines = page.lines();
    while let Some(line) = lines.next() {
        if line.trim_start().strip_prefix("```") != Some("yashy") {
            continue;
        }
        let mut body = Vec::new();
        for inner in lines.by_ref() {
            if inner.trim_start().starts_with("```") {
                break;
            }
            body.push(inner);
        }
        out.push(format!("{}\n", body.join("\n")));
    }
    out
}

fn check_source(dir: &Path, name: &str, source: &str) -> (bool, String) {
    common::write(dir, name, source);
    let output = Command::new(common::yashyc())
        .args(["check", name])
        .current_dir(dir)
        .output()
        .expect("the CLI must run");
    (
        output.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

#[test]
fn every_guide_program_checks() {
    let root = repo_root();
    let dir = common::work_dir("migrate-docs-programs");
    for page in ["1.0-to-2.0.md", "2.0-to-2027.md"] {
        let text = fs::read_to_string(root.join("docs/migrate").join(page))
            .unwrap_or_else(|error| panic!("{page}: {error}"));
        let blocks = yashy_blocks(&text);
        assert!(!blocks.is_empty(), "{page} has no yashy blocks");
        for (index, source) in blocks.iter().enumerate() {
            let (ok, output) = check_source(
                &dir,
                &format!("{}-{index}.ya", page.replace(".md", "")),
                source,
            );
            assert!(ok, "{page} block {index} must check:\n{output}\n{source}");
        }
    }
}

#[test]
fn the_edition_claim_matches_migrate_dry_run() {
    let dir = common::work_dir("migrate-docs-edition");
    let manifest = "[package]\nname = \"demo\"\nversion = \"1.0.0\"\nedition = \"2021\"\n";
    common::write(&dir, "yashy.toml", manifest);
    let output = Command::new(common::yashyc())
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
        manifest,
        "a dry run changes nothing"
    );
}
