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
        "fn main() {\n    let xs = []int {};\n    xs.push(1);\n    xs.free();\n    print(\"done\");\n}\n",
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
fn trace_events_name_a_clock_read() {
    // `now()` goes through one runtime clock. With the clock fixed for a
    // deterministic run, that read has to be visible as a `clock` event so a
    // trace says which program observed it.
    let dir = common::work_dir("trace-clock");
    common::write(
        &dir,
        "main.ya",
        "fn main() {\n    print(\"${now()}\");\n}\n",
    );
    let output = Command::new(common::yashyc())
        .args(["trace", "--events", "--json", "main.ya"])
        .current_dir(&dir)
        .env("YASHY_DETERMINISTIC", "1")
        .env("YASHY_DETERMINISTIC_SEED", "1700000000000")
        .output()
        .expect("yashyc must run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "tracing a clock read must succeed:\n{}\n{stdout}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("\"clock\""), "a named clock event: {stdout}");
}

#[test]
fn trace_without_events_still_prints_the_rule_matrix() {
    // The static matrix reads repository documentation, so it has to run from
    // the repository root rather than the crate's test working directory.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new(common::yashyc())
        .args(["trace", "--json"])
        .current_dir(root)
        .output()
        .expect("yashyc must run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"rules\""), "{stdout}");
    assert_eq!(stdout.matches("\"mode\": \"events\"").count(), 0);
}
