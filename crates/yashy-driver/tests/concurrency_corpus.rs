//! Race and typestate corpus: known-bad programs must name a rule id and a
//! span; a clean concurrent program must have no diagnostics.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn work_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yashy-race-{}-{name}", std::process::id()));
    fs::create_dir_all(&dir).expect("the corpus work directory must be creatable");
    dir
}

fn check(root: &Path, source: &str, name: &str) -> (bool, String) {
    let dir = work_dir(name);
    let file = dir.join(format!("{name}.ya"));
    fs::write(&file, source).expect("the corpus file must be writable");
    let output = Command::new(common::yashyc())
        .arg("verify")
        .arg("--json")
        .arg(&file)
        .current_dir(root)
        .output()
        .expect("the yashyc CLI must run");
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text)
}

#[test]
fn hold_across_await_is_reported() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = "async fn g() -> int { return 1; }\nasync fn f() -> int {\n let m = mutex_new();\n m.lock();\n let n = g().await;\n m.unlock();\n return n;\n}\nfn main() { print(block_on(f())); }\n";
    let (ok, json) = check(&root, source, "hold_across_await");
    assert!(!ok, "a lock across await must not check clean");
    assert!(json.contains("E1702"), "no E1702 in: {json}");
    assert!(json.contains("span"), "no span in: {json}");
}

#[test]
fn task_escape_is_reported() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = "async fn g() -> int { return 1; }\nasync fn f() -> int {\n let t = g();\n scope {\n  t = spawn(g);\n }\n let n = t.await;\n return n;\n}\nfn main() { print(block_on(f())); }\n";
    let (ok, json) = check(&root, source, "task_escape");
    assert!(!ok, "an escaping task must not check clean");
    assert!(json.contains("E1701"), "no E1701 in: {json}");
}

#[test]
fn clean_corpus_has_no_diagnostics() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = fs::read_to_string(root.join("crates/yashy-driver/tests/programs/sync_locks.ya"))
        .expect("the clean corpus file must exist");
    let (ok, json) = check(&root, &source, "clean_locks");
    assert!(ok, "a clean concurrent program must check: {json}");
}
