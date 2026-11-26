//! `yashyc fuzz --run` drives a bounded seeded campaign over a target module and
//! writes a reproducible finding; `--replay` re-runs the recorded seed and
//! refuses a target that no longer reproduces it.

mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

fn target(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
    let path = dir.join(format!("{name}.ya"));
    fs::write(
        &path,
        format!("pub fn accepts(string value) -> bool {{\n    return {body};\n}}\n"),
    )
    .expect("the target must be writable");
    path
}

#[test]
fn a_finding_is_written_and_replayed() {
    let dir = common::work_dir("fuzz-cli");
    let failing = target(&dir, "failing", "false");
    let finding = dir.join("finding.txt");

    let run = Command::new(common::yashyc())
        .args(["fuzz", "--run"])
        .arg(&failing)
        .args(["--iterations", "20", "--seed", "7", "--findings"])
        .arg(&finding)
        .current_dir(&dir)
        .output()
        .expect("the yashyc CLI must run");
    assert_eq!(
        run.status.code(),
        Some(1),
        "a finding must exit 1: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let text = fs::read_to_string(&finding).expect("the finding must exist");
    assert!(text.contains("seed=7"), "no seed in: {text}");
    assert!(text.contains("iterations=20"), "no iterations in: {text}");
    assert!(text.contains("input="), "no input in: {text}");

    let replay = Command::new(common::yashyc())
        .args(["fuzz", "--run"])
        .arg(&failing)
        .arg("--replay")
        .arg(&finding)
        .current_dir(&dir)
        .output()
        .expect("the yashyc CLI must run");
    assert!(
        replay.status.success(),
        "replay failed: {}",
        String::from_utf8_lossy(&replay.stderr)
    );
    assert!(
        String::from_utf8_lossy(&replay.stdout).contains("replay ok"),
        "{}",
        String::from_utf8_lossy(&replay.stdout)
    );

    let passing = target(&dir, "passing", "true");
    let refused = Command::new(common::yashyc())
        .args(["fuzz", "--run"])
        .arg(&passing)
        .arg("--replay")
        .arg(&finding)
        .current_dir(&dir)
        .output()
        .expect("the yashyc CLI must run");
    assert!(
        !refused.status.success(),
        "a target that no longer fails must not replay clean"
    );
}

#[test]
fn a_clean_campaign_reports_no_finding() {
    let dir = common::work_dir("fuzz-cli-clean");
    let passing = target(&dir, "passing", "true");

    let run = Command::new(common::yashyc())
        .args(["fuzz", "--run"])
        .arg(&passing)
        .args(["--iterations", "30", "--seed", "3", "--json"])
        .current_dir(&dir)
        .output()
        .expect("the yashyc CLI must run");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(stdout.contains("\"finding\":null"), "unexpected: {stdout}");
}

#[test]
fn a_finding_defaults_to_a_hashed_file() {
    let dir = common::work_dir("fuzz-cli-default");
    let failing = target(&dir, "failing", "false");

    let run = Command::new(common::yashyc())
        .args(["fuzz", "--run"])
        .arg(&failing)
        .args(["--iterations", "5", "--seed", "1"])
        .current_dir(&dir)
        .output()
        .expect("the yashyc CLI must run");
    assert_eq!(run.status.code(), Some(1));
    let findings = dir.join("fuzz/findings");
    let entries = fs::read_dir(&findings).expect("the default findings directory must exist");
    let paths: Vec<_> = entries
        .map(|entry| entry.expect("the entry must be readable").path())
        .collect();
    assert_eq!(paths.len(), 1, "one finding expected: {paths:?}");
    assert_eq!(
        paths[0].extension().and_then(|value| value.to_str()),
        Some("bin"),
        "unexpected finding name: {:?}",
        paths[0]
    );
}
