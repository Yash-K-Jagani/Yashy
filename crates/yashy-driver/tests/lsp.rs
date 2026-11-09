//! `yashy lsp`: the server is a sibling binary next to the driver.

mod common;

use std::io::Write;
use std::process::{Command, Stdio};

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
    if !sibling_dir()
        .join(format!("yashy-lsp{}", std::env::consts::EXE_SUFFIX))
        .is_file()
    {
        return;
    }
    let (code, stdout, stderr) = run(&["lsp", "--version"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("yashy "), "the driver's version:\n{stdout}");
    assert!(
        stdout.contains("yashy-lsp"),
        "the server's version:\n{stdout}"
    );
}

#[test]
fn a_missing_sibling_exits_2_and_names_the_path() {
    let output = Command::new(yashy())
        .arg("lsp")
        .env("YASHY_LSP_SIBLING", "no-such-language-server.binary")
        .output()
        .expect("the yashy CLI must be runnable");
    let code = output.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(code, 2, "stderr: {stderr}");
    assert!(
        stderr.contains("no-such-language-server.binary"),
        "the missing path is named:\n{stderr}"
    );
}

#[test]
fn a_stub_siblings_nonzero_exit_becomes_the_drivers_exit() {
    // The driver itself exits 2 with no command, so pointing the lookup at
    // it is a stub that answers non-zero immediately.
    let output = Command::new(yashy())
        .arg("lsp")
        .env("YASHY_LSP_SIBLING", yashy())
        .stdin(Stdio::null())
        .output()
        .expect("the yashy CLI must be runnable");
    assert_ne!(
        output.status.code().unwrap_or(0),
        0,
        "the sibling's exit propagates, not a clean exit"
    );
}

#[test]
fn the_real_handshake_is_answered_through_the_driver() {
    if !sibling_dir()
        .join(format!("yashy-lsp{}", std::env::consts::EXE_SUFFIX))
        .is_file()
    {
        return;
    }
    let mut child = Command::new(yashy())
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the yashy CLI must be runnable");
    let mut stdin = child.stdin.take().expect("stdin is piped");
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{}}}}"#
    )
    .expect("write the request");
    stdin.flush().expect("flush the request");
    drop(stdin);
    let output = child
        .wait_with_output()
        .expect("the server exits on closed stdin");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout.contains("protocolVersion"),
        "one JSON-RPC round trip through `yashy lsp`:\n{stdout}"
    );
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
