//! Record/replay: a trace pins the program hash and target; replay refuses a
//! source that no longer matches.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn work_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yashy-replay-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("the replay work directory must be creatable");
    dir
}

#[test]
fn replay_validates_the_program_hash() {
    let dir = work_dir();
    let source = dir.join("case.ya");
    let trace = dir.join("trace.yat");
    fs::write(&source, "fn main() { print(\"record-ok\"); }\n").expect("the source must be writable");

    let recorded = Command::new(common::yashyc())
        .args(["record", "case.ya", "--out", "trace.yat"])
        .current_dir(&dir)
        .output()
        .expect("the yashyc CLI must run");
    assert!(recorded.status.success(), "{}", String::from_utf8_lossy(&recorded.stderr));
    let text = fs::read_to_string(&trace).expect("the trace must exist");
    assert!(text.contains("program_hash"), "unexpected trace: {text}");
    assert!(text.contains("events"), "unexpected trace: {text}");

    let replayed = Command::new(common::yashyc())
        .args(["replay", "trace.yat", "--source", "case.ya", "--json"])
        .current_dir(&dir)
        .output()
        .expect("the yashyc CLI must run");
    assert!(replayed.status.success(), "{}", String::from_utf8_lossy(&replayed.stderr));
    assert!(String::from_utf8_lossy(&replayed.stdout).contains("program_hash"));

    fs::write(&source, "fn main() { print(\"changed\"); }\n").expect("the edit must be writable");
    let refused = Command::new(common::yashyc())
        .args(["replay", "trace.yat", "--source", "case.ya"])
        .current_dir(&dir)
        .output()
        .expect("the yashyc CLI must run");
    assert!(!refused.status.success(), "a changed program must be refused");
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("hash mismatch"),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );
    let _ = fs::remove_dir_all(Path::new(&dir));
}
