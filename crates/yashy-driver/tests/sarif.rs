//! SARIF 2.1.0 is the CI interchange form of the same diagnostics the CLI
//! renders as schema v1 JSON.

mod common;

use common::{work_dir, write, yashyc};
use std::process::Command;

#[test]
fn check_sarif_reports_the_rules_and_locations() {
    let dir = work_dir("sarif-check");
    write(&dir, "main.ya", "fn main() { missing(); }\n");
    let output = Command::new(yashyc())
        .args(["check", "--sarif", "main.ya"])
        .current_dir(&dir)
        .output()
        .expect("the CLI must run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    for field in [
        "\"version\": \"2.1.0\"",
        "\"rules\"",
        "\"results\"",
        "\"ruleId\": \"E0001\"",
        "\"level\": \"error\"",
        "\"shortDescription\"",
    ] {
        assert!(stdout.contains(field), "missing {field} in:\n{stdout}");
    }
}

#[test]
fn a_clean_file_has_no_results() {
    let dir = work_dir("sarif-clean");
    write(&dir, "main.ya", "fn main() {}\n");
    let output = Command::new(yashyc())
        .args(["check", "--sarif", "main.ya"])
        .current_dir(&dir)
        .output()
        .expect("the CLI must run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"results\": []"), "{stdout}");
}
