mod common;

use common::yashy;
use std::process::Command;

#[test]
fn doctor_reports_the_toolchain_and_stdlib() {
    let output = Command::new(yashy())
        .arg("doctor")
        .output()
        .expect("the yashy CLI must be runnable");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "doctor must succeed on a working development machine:\n{}\n{}",
        stdout,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("yashy"), "stdout:\n{stdout}");
    assert!(stdout.contains("clang"), "stdout:\n{stdout}");
    assert!(stdout.contains("stdlib"), "stdout:\n{stdout}");
}
