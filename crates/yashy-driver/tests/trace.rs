//! Runtime event tracing: `trace --events` reports what the program did, with
//! source spans from the same debug info the debugger uses.

mod common;

use std::process::Command;

#[test]
fn trace_events_reports_allocations_frees_and_io() {
    let dir = common::work_dir("trace-events");
    common::write(
        &dir,
        "main.ya",
        "fn main() {\n    let buf = alloc(16);\n    buf.free();\n    print(\"done\");\n}\n",
    );
    let output = Command::new(common::yashyc())
        .args(["trace", "--events", "--json", "main.ya"])
        .current_dir(&dir)
        .output()
        .expect("yashyc must run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "tracing must succeed:\n{}\n{stdout}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("\"events\""), "{stdout}");
    assert!(stdout.contains("\"alloc\""), "{stdout}");
    assert!(stdout.contains("\"free\""), "{stdout}");
    assert!(stdout.contains("\"io\""), "{stdout}");
    assert!(
        stdout.contains("main.ya"),
        "spans must name the source: {stdout}"
    );
}

#[test]
fn trace_without_events_still_prints_the_rule_matrix() {
    let output = Command::new(common::yashyc())
        .args(["trace", "--json"])
        .output()
        .expect("yashyc must run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"rules\""), "{stdout}");
    assert_eq!(stdout.matches("\"mode\": \"events\"").count(), 0);
}
