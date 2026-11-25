//! Mutation scoring is deterministic; differential checking compares two
//! compiler binaries over a corpus.

mod common;

use std::path::Path;
use std::process::Command;

#[test]
fn mutation_score_is_deterministic_and_gates() {
    let args = ["mutate", "--score", "8:2:0:0", "--json"];
    let first = Command::new(common::yashyc())
        .args(args)
        .output()
        .expect("the yashyc CLI must run");
    assert!(first.status.success(), "{}", String::from_utf8_lossy(&first.stderr));
    let second = Command::new(common::yashyc())
        .args(args)
        .output()
        .expect("the yashyc CLI must run");
    assert_eq!(first.stdout, second.stdout, "the campaign must be deterministic");

    let strong = Command::new(common::yashyc())
        .args(["mutate", "--score", "8:2:0:0", "--fail-under", "70"])
        .output()
        .expect("the yashyc CLI must run");
    assert!(strong.status.success(), "a strong suite must pass the floor");
    let weak = Command::new(common::yashyc())
        .args(["mutate", "--score", "0:10:0:0", "--fail-under", "70"])
        .output()
        .expect("the yashyc CLI must run");
    assert!(!weak.status.success(), "a weak suite must fail the floor");
}

#[test]
fn differential_harness_agrees_with_itself() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = common::work_dir("p9-differential");
    let corpus_file = dir.join("case.ya");
    std::fs::write(&corpus_file, "fn main() { print(\"ok\"); }\n").expect("the corpus must be writable");
    let binary = Path::new(common::yashyc());
    let mismatches = yashy_driver::differential::run(
        std::slice::from_ref(&corpus_file),
        binary,
        binary,
    );
    assert!(mismatches.is_empty(), "unexpected mismatches: {mismatches:?}");
    let _ = root;
}
