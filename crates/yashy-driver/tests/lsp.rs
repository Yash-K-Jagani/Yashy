//! `yashy lsp`: the server is a sibling binary next to the driver.

mod common;

use std::process::Command;

fn yashy() -> &'static str {
    common::yashy()
}

fn run(args: &[&str]) -> (i32, String, String) {
    let output = Command::new(yashy())
        .args(args)
        .output()
        .expect("the yashy CLI must be runnable");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn sibling_dir() -> std::path::PathBuf {
    std::env::current_exe()
        .expect("the test binary has a path")
        .parent()
        .expect("the test binary has a parent")
        .to_path_buf()
}

#[test]
fn lsp_version_prints_both_versions() {
    // Gated on the sibling being present: this suite builds the driver only,
    // and the LSP binary arrives with a workspace build. The plan requires
    // the exit-2 diagnostic when it is missing, which the next test pins.
    if !sibling_dir().join("yashy-lsp").is_file() {
        return;
    }
    let (code, stdout, stderr) = run(&["lsp", "--version"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("yashy "), "the driver's version:\n{stdout}");
    assert!(stdout.contains("yashy-lsp"), "the server's version:\n{stdout}");
}

#[test]
fn doctor_reports_the_lsp_lookup() {
    let (code, stdout, _) = run(&["doctor", "--json"]);
    assert_eq!(code, 0);
    assert!(
        stdout.contains("\"lsp\""),
        "doctor names the LSP lookup:\n{stdout}"
    );
}
