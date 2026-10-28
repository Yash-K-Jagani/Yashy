//! `AGENTS.md` is executable documentation for coding agents. These tests keep
//! its test targets real and run the diagnostic commands it tells an agent to
//! use. The link checks for `AGENTS.md` and `llms.txt` live in `docs.rs`.
//!
//! The cargo commands in the file cannot all run from inside a test: a
//! `cargo test --workspace` would recurse. The test target names are checked
//! against the files on disk, and the CLI commands are executed for real.

mod common;

use common::yashy;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn read(relative: &str) -> String {
    let path = root().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()))
}

/// The non-empty lines inside the first fenced block after `heading`.
fn block_after(markdown: &str, heading: &str) -> Vec<String> {
    let mut lines = markdown.lines();
    let found = lines.by_ref().any(|line| line.trim() == heading);
    assert!(found, "AGENTS.md must contain the `{heading}` heading");

    let mut block = Vec::new();
    let mut in_fence = false;
    let mut closed = false;
    for line in lines {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            if in_fence {
                closed = true;
                break;
            }
            in_fence = true;
            continue;
        }
        if in_fence && !trimmed.is_empty() {
            block.push(trimmed.to_string());
        }
    }

    assert!(
        in_fence && closed,
        "`{heading}` must be followed by a closed fenced block"
    );
    block
}

#[test]
fn agents_cargo_test_commands_name_real_targets() {
    let markdown = read("AGENTS.md");
    let commands = block_after(&markdown, "## Commands");

    let mut targets = 0;
    for command in &commands {
        let Some(target) = command.strip_prefix("cargo test -p yashy-driver --test ") else {
            continue;
        };
        let target = target.trim();
        let path = root()
            .join("crates/yashy-driver/tests")
            .join(format!("{target}.rs"));
        assert!(
            path.is_file(),
            "AGENTS.md runs `{command}`, but {} is missing",
            path.display()
        );
        targets += 1;
    }

    assert!(
        targets >= 8,
        "AGENTS.md should name the driver test targets; found {targets}"
    );
}

#[test]
fn agents_diagnostic_commands_run() {
    let markdown = read("AGENTS.md");
    let commands: Vec<String> = block_after(&markdown, "## Diagnostics and JSON")
        .into_iter()
        .filter(|line| line.starts_with("yashy "))
        .collect();

    assert_eq!(
        commands.len(),
        5,
        "AGENTS.md should document five diagnostic commands: {commands:?}"
    );
    for expected in [
        "yashy check --json",
        "yashy verify --json",
        "yashy test --json",
        "yashy explain",
        "yashy explain --json",
    ] {
        assert!(
            commands.iter().any(|command| command.contains(expected)),
            "AGENTS.md does not document `{expected}`: {commands:?}"
        );
    }

    for command in commands {
        let args: Vec<&str> = command.split_whitespace().skip(1).collect();
        let output = Command::new(yashy())
            .args(&args)
            .current_dir(root())
            .output()
            .unwrap_or_else(|error| panic!("`{command}` must start: {error}"));

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "`{command}` failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );

        if command.contains("--json") {
            assert!(
                stdout.contains("\"schema_version\": 1"),
                "`{command}` must print schema v1:\n{stdout}"
            );
            assert!(
                stdout.contains("\"success\": true"),
                "`{command}` must report success:\n{stdout}"
            );
        }
        if command.starts_with("yashy check --json") {
            assert!(
                stdout.contains("\"diagnostics\": []"),
                "the example must be clean:\n{stdout}"
            );
        }
        if command.starts_with("yashy verify --json") {
            assert!(
                stdout.contains("\"tool\": \"verify\""),
                "`{command}` must be a verifier report:\n{stdout}"
            );
            assert!(
                stdout.contains("\"diagnostics\": []"),
                "the example must be sound:\n{stdout}"
            );
        }
        if command.starts_with("yashy test --json") {
            assert!(
                stdout.contains("\"tool\": \"test\""),
                "`{command}` must be a test report:\n{stdout}"
            );
            assert!(
                stdout.contains("\"tests\": ["),
                "`{command}` must list the tests it ran:\n{stdout}"
            );
        }
        if command == "yashy explain E0100" {
            assert!(
                stdout.contains("# E0100"),
                "`{command}` must print the catalog page:\n{stdout}"
            );
            assert!(
                stdout.contains("## Fix"),
                "`{command}` must print the fix section:\n{stdout}"
            );
        }
    }
}
