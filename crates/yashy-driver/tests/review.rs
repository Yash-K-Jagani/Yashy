//! Semantic review: the baseline is an index snapshot, not a line diff.
//! Only changes to signatures, effects, contracts, ownership and visibility
//! count as semantic; documentation alone does not.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

fn yashyc() -> &'static str {
    common::yashyc()
}

fn workspace(name: &str) -> PathBuf {
    let dir = common::work_dir(name);
    common::write(
        &dir,
        "main.ya",
        "pub fn add(int a, int b) -> int {\n    return a + b;\n}\n\n\
         fn helper(int x) -> int {\n    return x + 1;\n}\n\n\
         fn main() {\n    print(add(1, 2));\n}\n",
    );
    common::write(
        &dir,
        "tests/smoke.ya",
        "fn add(int a, int b) -> int {\n    return a + b;\n}\n\
         fn main() {\n    assert(add(1, 2) == 3);\n}\n",
    );
    common::write(
        &dir,
        "yashy.toml",
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    );
    dir
}

fn run(dir: &Path, args: &[&str]) -> (bool, String, String) {
    let output = Command::new(yashyc())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("the CLI must run");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn baseline(dir: &Path) -> String {
    let (ok, stdout, stderr) = run(dir, &["index", "--json"]);
    assert!(ok, "the index must succeed: {stderr}");
    stdout
}

fn review(dir: &Path, baseline: &str, human: bool) -> String {
    let file = dir.join("baseline.json");
    std::fs::write(&file, baseline).unwrap();
    let mut args = vec!["review", "--baseline", "baseline.json"];
    if !human {
        args.push("--json");
    }
    let (ok, stdout, stderr) = run(dir, &args);
    assert!(ok, "review must succeed: {stderr}");
    stdout
}

#[test]
fn a_public_signature_change_is_a_break() {
    let dir = workspace("review-public-break");
    let base = baseline(&dir);
    common::write(
        &dir,
        "main.ya",
        "pub fn add(int a, int b) -> string {\n    return \"changed\";\n}\n\n\
         fn helper(int x) -> int {\n    return x + 1;\n}\n\n\
         fn main() {\n    print(add(1, 2));\n}\n",
    );
    let json = review(&dir, &base, false);
    assert!(json.contains("\"kind\": \"signature\""), "{json}");
    assert!(json.contains("\"public\": true"), "{json}");
    assert!(json.contains("\"public_break\": true"), "{json}");
    assert!(json.contains("\"risk\": \"high\""), "{json}");
    let human = review(&dir, &base, true);
    assert!(human.contains("public break: yes"), "{human}");
    assert!(human.contains("signature pub add"), "{human}");
}

#[test]
fn a_comment_only_change_is_not_semantic() {
    let dir = workspace("review-comments");
    let base = baseline(&dir);
    common::write(
        &dir,
        "main.ya",
        "/// Adds two numbers.\npub fn add(int a, int b) -> int {\n    return a + b;\n}\n\n\
         fn helper(int x) -> int {\n    return x + 1;\n}\n\n\
         fn main() {\n    print(add(1, 2));\n}\n",
    );
    let json = review(&dir, &base, false);
    assert!(json.contains("\"changes\": []"), "{json}");
    assert!(json.contains("\"public_break\": false"), "{json}");
    assert!(json.contains("\"risk\": \"low\""), "{json}");
}

#[test]
fn a_private_rename_is_bounded_and_not_a_break() {
    let dir = workspace("review-private-rename");
    let base = baseline(&dir);
    common::write(
        &dir,
        "main.ya",
        "pub fn add(int a, int b) -> int {\n    return a + b;\n}\n\n\
         fn helper_two(int x) -> int {\n    return x + 1;\n}\n\n\
         fn main() {\n    print(add(1, 2));\n}\n",
    );
    let json = review(&dir, &base, false);
    assert!(json.contains("\"public_break\": false"), "{json}");
    assert!(json.contains("\"risk\": \"medium\""), "{json}");
    assert!(json.contains("helper"), "{json}");
}

#[test]
fn capability_and_test_deltas_are_reported() {
    let dir = workspace("review-deltas");
    let base = baseline(&dir);
    common::write(
        &dir,
        "yashy.toml",
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n\
         [capabilities]\nnet = [\"*\"]\n",
    );
    std::fs::remove_file(dir.join("tests/smoke.ya")).unwrap();
    let json = review(&dir, &base, false);
    assert!(json.contains("\"capability_delta\""), "{json}");
    assert!(json.contains("\"net\""), "{json}");
    assert!(json.contains("\"test_delta\": -1"), "{json}");
    assert!(json.contains("\"risk\": \"high\""), "{json}");
}
