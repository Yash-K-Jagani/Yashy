//! The architecture doc is a claim about the workspace. This test makes the
//! claim checkable: every crate and every dispatch verb in `docs/architecture.md`
//! must exist, and every active phase plan named by the master plan must exist.

mod common;

use std::fs;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

#[test]
fn architecture_lists_every_workspace_crate() {
    let root = root();
    let text = fs::read_to_string(root.join("docs/architecture.md"))
        .expect("the architecture doc must exist");
    let cargo = fs::read_to_string(root.join("Cargo.toml"))
        .expect("the workspace manifest must exist");
    // Every `crates/yashy-*` directory is a workspace member.
    for entry in fs::read_dir(root.join("crates")).expect("crates/ must exist") {
        let path = entry.expect("a readable entry").path();
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !name.starts_with("yashy-") {
            continue;
        }
        assert!(
            cargo.contains(&format!("crates/{name}")),
            "the workspace manifest must list crates/{name}"
        );
        assert!(
            text.contains(name),
            "docs/architecture.md must name the `{name}` crate"
        );
    }
}

#[test]
fn architecture_and_cli_spec_agree_on_verbs() {
    let text = fs::read_to_string(root().join("docs/architecture.md"))
        .expect("the architecture doc must exist");
    for command in yashy_driver::cli_spec::COMMANDS {
        assert!(
            text.contains(command.name),
            "docs/architecture.md must name the `{}` verb",
            command.name
        );
    }
}

#[test]
fn the_master_plan_names_every_active_phase_file() {
    let root = root();
    let plan = fs::read_to_string(root.join("docs/superpowers/plans/2026-11-15-yashy-2.0-master-plan.md"))
        .expect("the master plan must exist");
    let plans = root.join("docs/superpowers/plans");
    let mut active = 0;
    for entry in fs::read_dir(&plans).expect("plans/ must exist") {
        let path = entry.expect("a readable entry").path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        if name.starts_with("2026-11-15-phase-") && name.ends_with(".md") {
            active += 1;
            assert!(
                plan.contains(&name),
                "the master plan must name the active phase file {name}"
            );
        }
    }
    assert!(active >= 16, "expected at least 16 active phase files, found {active}");
}
