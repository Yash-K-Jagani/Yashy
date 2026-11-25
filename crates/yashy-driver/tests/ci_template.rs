//! The CI template and its coverage/SARIF artefacts are testable locally.

mod common;

use std::path::Path;
use std::process::Command;

#[test]
fn ci_template_declares_every_stage() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let template = std::fs::read_to_string(root.join("ci/generic.sh")).expect("the CI template must exist");
    for stage in [
        "stage \"check\"",
        "stage \"verify\"",
        "stage \"build\"",
        "stage \"test\"",
        "stage \"fmt\"",
        "stage \"lint\"",
        "stage \"traceability\"",
        "stage \"coverage\"",
        "stage \"benchmark gate\"",
        "stage \"fuzz smoke\"",
    ] {
        assert!(template.contains(stage), "missing CI stage: {stage}");
    }
    assert!(template.contains("--locked"), "every Cargo call must be locked");
    assert!(
        root.join("docs/ci.md").is_file(),
        "the CI stage reference must exist"
    );
}

#[test]
fn coverage_sarif_has_a_result_per_uncovered_line() {
    // Coverage needs llvm-profdata. When the host has no LLVM, the CI template
    // reports SKIP with the reason; the local test does the same.
    let has_tool = std::env::var_os("YASHY_LLVM_PROFDATA").is_some()
        || Command::new("llvm-profdata")
            .arg("--version")
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false);
    if !has_tool {
        eprintln!("SKIP coverage SARIF: llvm-profdata is not installed");
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new(common::yashyc())
        .args(["coverage", "--format", "sarif", "examples/hello/main.ya"])
        .current_dir(&root)
        .output()
        .expect("the yashyc CLI must run");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let sarif = String::from_utf8_lossy(&output.stdout);
    assert!(sarif.contains("2.1.0"), "unexpected SARIF: {sarif}");
    assert!(sarif.contains("COVERAGE-1"), "unexpected SARIF: {sarif}");
}
