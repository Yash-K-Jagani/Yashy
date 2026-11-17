//! `yashyc trace`: the generated rule matrix, not a hand-written spreadsheet.

mod common;

use std::path::Path;
use std::process::Command;

fn root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn the_matrix_has_one_row_per_rule() {
    let rows = yashy_driver::trace::matrix(&root()).expect("the matrix must build");
    assert!(
        rows.len() > 20,
        "the catalog has many rule families: {}",
        rows.len()
    );
    let mut names: Vec<&str> = rows.iter().map(|row| row.rule.as_str()).collect();
    names.sort_unstable();
    let count = names.len();
    names.dedup();
    assert_eq!(names.len(), count, "a rule must have one row");
    let codes: usize = rows.iter().map(|row| row.codes.len()).sum();
    assert_eq!(codes, yashy_common::codes::CODES.len());
}

#[test]
fn runtime_events_are_not_claimed() {
    let output = Command::new(common::yashyc())
        .args(["trace", "--events"])
        .output()
        .expect("the CLI must run");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not implemented"), "{stderr}");
}
