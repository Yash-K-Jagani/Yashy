//! `yashy doc`: the public surface from the compiler's own tables.

mod common;

use std::path::PathBuf;
use std::process::Command;

fn yashy() -> &'static str {
    common::yashy()
}

fn run(args: &[&str]) -> (i32, String, String) {
    let output = Command::new(yashy())
        .args(args)
        .output()
        .expect("the yashy CLI must be runnable");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn library_dir(name: &str) -> PathBuf {
    let dir = common::work_dir(name);
    common::write(
        &dir,
        "lib.ya",
        "/// Adds two numbers.\npub pure fn add(int a, int b) -> int {\n    return a + b;\n}\nfn hidden() -> int {\n    return 0;\n}\npub fn undocumented() -> int {\n    return 1;\n}\n",
    );
    dir
}

#[test]
fn a_documented_pub_fn_appears_with_its_text_and_signature() {
    let dir = library_dir("doc-documented");
    let out = dir.join("out");
    let (code, stdout, stderr) = run(&["doc", "--out", out.to_str().unwrap(), dir.to_str().unwrap()]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("1 modules"), "the model counts modules:\n{stdout}");
    let page = std::fs::read_to_string(out.join("lib.md")).expect("lib.md is written");
    assert!(
        page.contains("Adds two numbers."),
        "the doc text:\n{page}"
    );
    assert!(
        page.contains("fn add(int a, int b) -> int"),
        "the signature:\n{page}"
    );
}

#[test]
fn an_undocumented_pub_fn_says_no_documentation() {
    let dir = library_dir("doc-undocumented");
    let out = dir.join("out");
    let (code, _, stderr) = run(&["doc", "--out", out.to_str().unwrap(), dir.to_str().unwrap()]);
    assert_eq!(code, 0, "stderr: {stderr}");
    let page = std::fs::read_to_string(out.join("lib.md")).expect("lib.md is written");
    assert!(
        page.contains("No documentation."),
        "an undocumented item is still rendered:\n{page}"
    );
    assert!(page.contains("fn undocumented"), "the item:\n{page}");
}

#[test]
fn a_private_item_is_absent_by_default_and_present_with_private_flag() {
    let dir = library_dir("doc-private");
    let out = dir.join("out");
    let (code, _, stderr) = run(&["doc", "--out", out.to_str().unwrap(), dir.to_str().unwrap()]);
    assert_eq!(code, 0, "stderr: {stderr}");
    let page = std::fs::read_to_string(out.join("lib.md")).expect("lib.md is written");
    assert!(
        !page.contains("hidden"),
        "a private item is not in the public surface:\n{page}"
    );
    let out = dir.join("out-private");
    let (code, _, stderr) = run(&[
        "doc",
        "--private",
        "--out",
        out.to_str().unwrap(),
        dir.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stderr: {stderr}");
    let page = std::fs::read_to_string(out.join("lib.md")).expect("lib.md is written");
    assert!(
        page.contains("hidden"),
        "with --private it appears:\n{page}"
    );
}

#[test]
fn the_output_is_byte_identical_across_two_runs() {
    let dir = library_dir("doc-stable");
    let first = dir.join("first");
    let second = dir.join("second");
    let (code, _, stderr) = run(&["doc", "--out", first.to_str().unwrap(), dir.to_str().unwrap()]);
    assert_eq!(code, 0, "stderr: {stderr}");
    let (code, _, stderr) = run(&["doc", "--out", second.to_str().unwrap(), dir.to_str().unwrap()]);
    assert_eq!(code, 0, "stderr: {stderr}");
    let a = std::fs::read(first.join("lib.md")).expect("first lib.md");
    let b = std::fs::read(second.join("lib.md")).expect("second lib.md");
    assert_eq!(a, b, "the same source, the same bytes");
}

#[test]
fn the_json_carries_every_field() {
    let dir = library_dir("doc-json");
    let (code, stdout, stderr) = run(&["doc", "--json", dir.to_str().unwrap()]);
    assert_eq!(code, 0, "stderr: {stderr}");
    for field in ["\"name\"", "\"kind\"", "\"signature\"", "\"docs\"", "\"effects\"", "\"contracts\"", "\"typestate\""] {
        assert!(stdout.contains(field), "the {field} field:\n{stdout}");
    }
}
