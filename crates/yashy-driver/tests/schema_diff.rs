//! Schema v1 freeze: every command's JSON is pinned by a golden payload.
//!
//! The test regenerates each payload on a fixed fixture and compares it
//! against the committed golden. A field removed or retyped fails; a field
//! added shows up as a diff the reviewer must accept by updating the
//! golden (and recording it in `docs/schema/CHANGELOG.md`).

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

fn yashy() -> &'static str {
    common::yashy()
}

fn goldens() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/schema_golden")
}

fn fixture_dir(name: &str) -> PathBuf {
    let dir = common::work_dir(&format!("schema-{name}"));
    common::write(&dir, "main.ya", "fn main() {\n    print(\"hi\");\n}\n");
    dir
}

fn payload(command: &str, dir: &Path, extra: &[&str]) -> String {
    let mut args = vec![command];
    args.extend(extra);
    let target = if command == "context" || command == "index" {
        dir.to_path_buf()
    } else {
        dir.join("main.ya")
    };
    args.push(target.to_str().unwrap());
    let output = Command::new(yashy())
        .args(&args)
        .output()
        .expect("the yashy CLI must be runnable");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    // Absolute paths are machine-specific: replace the fixture directory
    // with a fixed marker so the golden is portable.
    let dir_text = dir.to_string_lossy().into_owned();
    let stdout = stdout.replace(&dir_text, "<workspace>");
    let stdout = stdout.replace(&dir_text.replace('\\', "/"), "<workspace>");
    let stderr = stderr.replace(&dir_text, "<workspace>");
    let stderr = stderr.replace(&dir_text.replace('\\', "/"), "<workspace>");
    let stdout = stdout.trim_end_matches('\n').to_string();
    let stderr = stderr.trim_end_matches('\n').to_string();
    format!(
        "exit={}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status.code().unwrap_or(-1)
    )
}

/// The frozen commands and the argument shape each golden is captured with.
const COMMANDS: &[(&str, &[&str])] = &[
    ("check", &["--json"]),
    ("verify", &["--json"]),
    ("index", &["--json"]),
    (
        "context",
        &[
            "--task",
            "fix the bug in main",
            "--budget",
            "8000",
            "--json",
        ],
    ),
];

/// The top-level keys each frozen command's schema requires. A payload that
/// loses one fails here even before the byte comparison runs.
const REQUIRED: &[(&str, &[&str])] = &[
    (
        "check",
        &[
            "schema_version",
            "tool",
            "yashy_version",
            "success",
            "diagnostics",
            "summary",
        ],
    ),
    (
        "verify",
        &[
            "schema_version",
            "tool",
            "yashy_version",
            "success",
            "diagnostics",
            "summary",
        ],
    ),
    (
        "index",
        &[
            "schema_version",
            "tool",
            "files",
            "symbols",
            "imports",
            "tests",
        ],
    ),
    (
        "context",
        &[
            "schema_version",
            "tool",
            "tokens",
            "budget",
            "included",
            "dropped",
        ],
    ),
];

#[test]
fn every_frozen_command_matches_its_golden() {
    let dir = fixture_dir("golden");
    for (command, extra) in COMMANDS {
        let golden = std::fs::read_to_string(goldens().join(format!("{command}.json")))
            .unwrap_or_else(|error| panic!("golden for {command} must exist: {error}"));
        let current = payload(command, &dir, extra);
        assert_eq!(
            current, golden,
            "`yashy {command}` JSON drifted from its v1 golden; if the change is additive, \
             update the golden and record it in docs/schema/CHANGELOG.md"
        );
    }
}

#[test]
fn every_golden_stdout_parses_and_carries_its_schema_keys() {
    let dir = fixture_dir("golden-keys");
    for (command, extra) in COMMANDS {
        let required = REQUIRED
            .iter()
            .find(|(name, _)| *name == *command)
            .map(|(_, keys)| *keys)
            .expect("the command has a schema row");
        let schema_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/schema/v1")
            .join(format!("{command}.schema.json"));
        assert!(
            schema_path.is_file(),
            "the JSON Schema for {command} must exist at {}",
            schema_path.display()
        );
        let current = payload(command, &dir, extra);
        let stdout = current
            .split_once("stdout:\n")
            .map(|(_, rest)| rest.split("\nstderr:").next().unwrap_or(rest))
            .unwrap_or("");
        let value = yashy_common::json_rpc::parse(stdout)
            .unwrap_or_else(|error| panic!("`yashy {command}` stdout must parse: {error}"));
        for key in required.iter() {
            assert!(
                value.get(key).is_some(),
                "`yashy {command}` lost the required field `{key}`"
            );
        }
    }
}
