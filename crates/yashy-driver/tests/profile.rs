//! `yashy profile`: a recorded report renders as a table sorted by self
//! time, with the instrumentation overhead as a number.

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

const REPORT: &str = r#"{
  "program": "workload",
  "overhead_pct": 12.5,
  "functions": [
    {"name": "small", "calls": 10, "self_ns": 1000, "total_ns": 2000},
    {"name": "hot", "calls": 1, "self_ns": 9000, "total_ns": 10000}
  ]
}"#;

#[test]
fn a_recorded_report_renders_sorted_with_the_overhead() {
    let dir = common::work_dir("profile-report");
    let report = dir.join("report.json");
    common::write(&dir, "report.json", REPORT);
    let _ = report;
    let (code, stdout, stderr) = run(&[
        "profile",
        "--report",
        dir.join("report.json").to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stderr: {stderr}");
    let hot = stdout.find("hot").expect("the dominant row");
    let small = stdout.find("small").expect("the second row");
    assert!(hot < small, "sorted by self time:\n{stdout}");
    assert!(
        stdout.contains("12.50") && stdout.contains("overhead"),
        "the overhead is a number:\n{stdout}"
    );
}

#[test]
fn a_malformed_report_is_an_error_not_a_table() {
    let dir = common::work_dir("profile-bad");
    common::write(&dir, "bad.json", "{}");
    let (code, stdout, stderr) = run(&[
        "profile",
        "--report",
        dir.join("bad.json").to_str().unwrap(),
    ]);
    assert_ne!(code, 0, "an empty report is refused:\n{stdout}");
    assert!(!stderr.is_empty() || !stdout.is_empty());
}

#[test]
fn a_debug_run_generates_a_syntactically_valid_lldb_file() {
    let dir = common::work_dir("profile-debug");
    common::write(&dir, "main.ya", "fn main() {\n    print(\"hi\");\n}\n");
    let (code, _, stderr) = run(&["run", "--debug", dir.join("main.ya").to_str().unwrap()]);
    assert_eq!(code, 0, "stderr: {stderr}");
    // The file lands in the driver's working directory (`.yashy/debug.lldb`
    // beside where the test process runs), so look for it beside the test
    // work directory's parent first, then the current directory.
    let candidates = [
        std::env::current_dir().unwrap().join(".yashy/debug.lldb"),
        dir.join(".yashy/debug.lldb"),
    ];
    let Some(file) = candidates.iter().find(|path| path.is_file()) else {
        panic!("the LLDB command file was generated");
    };
    let text = std::fs::read_to_string(file).expect("the generated file");
    assert!(text.contains("main"), "the entry breakpoint:\n{text}");
    assert_eq!(
        text.matches('{').count(),
        text.matches('}').count(),
        "balanced braces:\n{text}"
    );
    let _ = std::fs::remove_dir_all(".yashy");
}
