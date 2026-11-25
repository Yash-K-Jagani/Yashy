//! The traceability matrix is generated, machine-readable and committed.

mod common;

use std::path::Path;
use std::process::Command;

#[test]
fn trace_json_and_markdown_come_from_the_same_matrix() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new(common::yashyc())
        .args(["trace", "--json", "."])
        .current_dir(&root)
        .output()
        .expect("the yashyc CLI must run");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let json = String::from_utf8_lossy(&output.stdout);
    assert!(json.contains("\"rules\""), "no rules in the matrix: {json}");
    assert!(json.contains("ALIAS-1"), "no rule ids in the matrix: {json}");
    assert!(json.contains("orphan"), "no orphan accounting in the matrix: {json}");

    let markdown = std::fs::read_to_string(root.join("docs/traceability.md"))
        .expect("the generated traceability page must be committed");
    assert!(markdown.contains("ALIAS-1"), "the page is not generated from the matrix");
}
