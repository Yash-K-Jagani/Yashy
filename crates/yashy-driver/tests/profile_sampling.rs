//! Native sampling: the hot function owns the samples, and an allocation's
//! source span is the site the runtime recorded. The sampler is opt-in through
//! `profile --sample`; a machine without one falls back to the counter path and
//! says so, which is a different run and not this test's subject.

mod common;

use std::process::Command;

const WORKLOAD: &str = r#"fn hot(int n) -> int {
    let int total = 0;
    let i = 0;
    while i < n {
        total += i;
        i += 1;
    }
    return total;
}

fn cold(int x) -> int {
    return x + 1;
}

fn main() {
    // Long enough that a 1 ms sampler takes many ticks even when the OS
    // rounds a sleep up to its clock granularity; a short run could take
    // three samples, two of them outside the loop.
    let value = hot(500000000);
    print(cold(value));
}
"#;

/// The coverage case only needs every line to run once; a short loop keeps an
/// instrumented build from dominating the suite.
const COVERAGE_WORKLOAD: &str = r#"fn hot(int n) -> int {
    let int total = 0;
    let i = 0;
    while i < n {
        total += i;
        i += 1;
    }
    return total;
}

fn cold(int x) -> int {
    return x + 1;
}

fn main() {
    let value = hot(2000);
    print(cold(value));
}
"#;

#[test]
fn the_sampler_attributes_the_hot_loop_to_its_function() {
    let dir = common::work_dir("profile-sampling-hot");
    common::write(&dir, "main.ya", WORKLOAD);
    let output = Command::new(common::yashyc())
        .args(["profile", "--sample", "--json", "main.ya"])
        .current_dir(&dir)
        .output()
        .expect("the yashyc CLI must be runnable");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "sampling must succeed:\n{stderr}\n{stdout}"
    );
    assert!(
        stdout.contains("\"mode\": \"sample\""),
        "the native sampler must have run, not the fallback:\n{stderr}\n{stdout}"
    );
    let value = yashy_common::json_rpc::parse(&stdout).expect("the sample report is JSON");
    let functions = value
        .get("functions")
        .and_then(yashy_common::json_rpc::Value::as_array)
        .expect("the report has function rows");
    let hot = functions
        .iter()
        .find(|row| {
            row.get("name")
                .and_then(yashy_common::json_rpc::Value::as_str)
                == Some("hot")
        })
        .expect("the hot function is named");
    let percent = match hot.get("percent") {
        Some(yashy_common::json_rpc::Value::Num(percent)) => *percent,
        other => panic!("the hot row has a numeric percent: {other:?}"),
    };
    assert!(
        percent >= 80.0,
        "at least 80% of samples name the hot function: {stdout}"
    );
    assert!(
        value
            .get("hot")
            .and_then(yashy_common::json_rpc::Value::as_str)
            == Some("hot"),
        "the report's attribution verdict names `hot`: {stdout}"
    );
}

#[test]
fn an_allocation_site_carries_its_source_span_and_a_leak_is_named() {
    let dir = common::work_dir("profile-sampling-alloc");
    // The deliberate leak is the point of the report. The verifier would
    // refuse it, so the measurement is opt-in with `--no-verify`, which stamps
    // the binary as unverified in the usual way.
    common::write(
        &dir,
        "main.ya",
        "fn main() {\n    let buf = alloc(16);\n    print(\"leaking on purpose\");\n}\n",
    );
    let output = Command::new(common::yashyc())
        .args(["profile", "--alloc-report", "--no-verify", "main.ya"])
        .current_dir(&dir)
        .output()
        .expect("the yashyc CLI must be runnable");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "the allocation report must succeed:\n{}\n{stdout}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("main.ya"),
        "the allocation's site names the source: {stdout}"
    );
    assert!(
        stdout.contains("\"live_allocations\": 1"),
        "the deliberate leak is still live at exit: {stdout}"
    );
    assert!(
        stdout.contains("\"sites\""),
        "the live block is reported as a site: {stdout}"
    );
}

#[test]
fn coverage_reports_executed_lines() {
    let dir = common::work_dir("profile-sampling-coverage");
    common::write(&dir, "main.ya", COVERAGE_WORKLOAD);
    let output = Command::new(common::yashyc())
        .args(["coverage", "--json", "main.ya"])
        .current_dir(&dir)
        .output()
        .expect("the yashyc CLI must be runnable");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() && stderr.contains("llvm-profdata") {
        // A host without the coverage helpers is a stated usage error, not a
        // silent pass. CI installs them with the toolchain.
        return;
    }
    assert!(
        output.status.success(),
        "coverage must succeed:\n{stderr}\n{stdout}"
    );
    assert!(stdout.contains("\"covered_lines\""), "{stdout}");
    assert!(stdout.contains("main.ya"), "{stdout}");
}
