//! `yashy build --opt-report`: the reason every check and allocation survived.
//!
//! The report is produced while the program is lowered, not by a second
//! analysis: it names the checks the verifier could not delete and the
//! allocations escape analysis could not promote. These tests drive the CLI
//! because the promises are about what a user -- or an agent -- sees.

mod common;

use common::{work_dir, write, yashy};
use std::path::Path;
use std::process::{Command, Output};

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(yashy())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("the yashy CLI must be runnable")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Run `--opt-report` over `source` and return its stdout.
fn report(name: &str, source: &str, extra: &[&str]) -> String {
    let dir = work_dir(name);
    write(&dir, "main.ya", source);
    let mut args = vec!["build", "main.ya", "--opt-report"];
    args.extend_from_slice(extra);
    let output = run(&dir, &args);
    assert!(
        output.status.success(),
        "`--opt-report` must succeed:\n{}",
        stderr(&output)
    );
    stdout(&output)
}

/// An index on a parameter: nothing proves the length, so the runtime decides.
const UNPROVEN: &str = "\
fn first([]int xs) -> int {
    let o = xs[0];
    return switch o { case Some(v) { v } case None { 0 } };
}

fn main() {
}
";

/// The same read on a literal slice: `xs[0]` is proved in bounds.
const PROVEN: &str = "\
fn main() {
    let xs = []int { 7 };
    let o = switch xs[0] { case Some(v) { v } case None { 0 } };
    print(o);
    xs.free();
}
";

/// A non-escaping literal allocation: escape analysis promotes it to the frame.
const PROMOTED: &str = "\
fn f() -> int {
    let p = alloc(8);
    p[0] = 1;
    let v = p[0];
    p.free();
    return v;
}

fn main() {
}
";

/// Nothing to report: no index, no checked arithmetic, no allocation.
const NOTHING: &str = "\
fn main() {
    print(1);
}
";

#[test]
fn a_proven_index_has_no_bounds_entry() {
    let text = report("opt-report-proven", PROVEN, &[]);
    assert!(
        !text.contains("bounds check kept"),
        "a proved index must produce no `bounds` entry:\n{text}"
    );
}

#[test]
fn an_unproven_index_is_one_entry_with_the_requires_fix() {
    let text = report("opt-report-unproven", UNPROVEN, &[]);
    assert_eq!(
        text.matches("bounds check kept").count(),
        1,
        "an unproven index is exactly one entry:\n{text}"
    );
    assert!(
        text.contains("fn first"),
        "the entry is grouped under its function:\n{text}"
    );
    assert!(
        text.contains("requires"),
        "the fix names the `requires` clause:\n{text}"
    );
}

#[test]
fn a_promoted_allocation_reports_its_placement() {
    let text = report("opt-report-promoted", PROMOTED, &[]);
    assert!(
        text.contains("promoted"),
        "the allocation was promoted and the report must say so:\n{text}"
    );
    assert!(
        text.contains("stack"),
        "the report names where it was promoted to:\n{text}"
    );
    assert!(
        !text.contains("heap allocation"),
        "a promoted allocation is not a heap allocation:\n{text}"
    );
}

#[test]
fn the_json_report_is_stable_and_carries_the_schema() {
    let first = report("opt-report-json-a", UNPROVEN, &["--opt-report=json"]);
    let second = report("opt-report-json-b", UNPROVEN, &["--opt-report=json"]);
    assert_eq!(first, second, "two runs must agree byte for byte");
    assert!(
        first.contains("\"schema_version\": 1"),
        "schema v1:\n{first}"
    );
    assert!(first.contains("\"tool\": \"yashy\""), "the tool:\n{first}");
    assert!(first.contains("\"functions\""), "the grouping:\n{first}");
    assert!(
        first.contains("\"kind\": \"bounds\""),
        "the item kind:\n{first}"
    );
    assert!(
        first.contains("\"reason\""),
        "every item has a reason:\n{first}"
    );
    assert!(
        first.contains("\"fix\""),
        "every item has a fix field:\n{first}"
    );
    assert!(
        first.contains("\"span\""),
        "every item points at its source:\n{first}"
    );
}

#[test]
fn an_empty_report_says_so_in_one_line() {
    let text = report("opt-report-empty", NOTHING, &[]);
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    assert_eq!(
        lines.len(),
        1,
        "an empty report is one line, not nothing:\n{text}"
    );
    assert!(!text.trim().is_empty(), "the one line must say something");
}

#[test]
fn json_stays_empty_for_a_program_with_nothing_to_say() {
    let text = report("opt-report-empty-json", NOTHING, &["--opt-report=json"]);
    assert!(
        text.contains("\"functions\": []"),
        "no function has anything to report:\n{text}"
    );
}
