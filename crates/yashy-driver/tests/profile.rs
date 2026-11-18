//! `yashyc profile`: a recorded report renders as a table sorted by self
//! time, with the instrumentation overhead as a number.

mod common;

use std::process::Command;

fn yashyc() -> &'static str {
    common::yashyc()
}

fn run(args: &[&str]) -> (i32, String, String) {
    let output = Command::new(yashyc())
        .args(args)
        .output()
        .expect("the yashyc CLI must be runnable");
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

#[test]
fn alloc_report_names_the_site_of_every_allocation() {
    let dir = common::work_dir("profile-alloc-site");
    common::write(
        &dir,
        "main.ya",
        "fn main() {\n    let buf = alloc(16);\n    buf.free();\n    print(\"done\");\n}\n",
    );
    let (code, stdout, stderr) = run(&[
        "profile",
        "--alloc-report",
        dir.join("main.ya").to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("\"allocations\""), "{stdout}");
    assert!(stdout.contains("\"sites\""), "{stdout}");
    assert!(
        stdout.contains("main.ya"),
        "the allocation site must name the source: {stdout}"
    );
    assert!(
        stdout.contains("\"live_allocations\": 0"),
        "the program frees its buffer: {stdout}"
    );
}

#[test]
fn a_recorded_report_has_the_same_model_as_json() {
    let dir = common::work_dir("profile-json");
    common::write(&dir, "report.json", REPORT);
    let (code, stdout, stderr) = run(&[
        "profile",
        "--json",
        "--report",
        dir.join("report.json").to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stderr: {stderr}");
    for field in [
        "\"schema_version\"",
        "\"tool\"",
        "\"program\"",
        "\"overhead_pct\"",
        "\"functions\"",
        "\"name\": \"hot\"",
        "\"self_ns\"",
    ] {
        assert!(stdout.contains(field), "missing {field}: {stdout}");
    }
}

#[test]
fn sample_names_its_counter_fallback_instead_of_inventing_samples() {
    let dir = common::work_dir("profile-sample-fallback");
    common::write(&dir, "main.ya", "fn main() {\n    print(\"hi\");\n}\n");
    let (code, stdout, stderr) =
        run(&["profile", "--sample", dir.join("main.ya").to_str().unwrap()]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stderr.contains("no native sampler"),
        "the fallback is stated, not hidden: {stderr}"
    );
    assert!(
        stdout.contains("wall clock"),
        "the counter path is what ran: {stdout}"
    );
}
