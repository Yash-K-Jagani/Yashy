//! `yashy outline`, `yashy graph` and `yashy search`.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

fn yashy() -> &'static str {
    common::yashy()
}

fn run(args: &[&str]) -> (bool, String, String) {
    let output = Command::new(yashy())
        .args(args)
        .output()
        .expect("the yashy CLI must be runnable");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn fixture_dir(name: &str) -> PathBuf {
    let dir = common::work_dir(name);
    common::write(
        &dir,
        "main.ya",
        "/// Adds two numbers.\npure fn add(int a, int b) -> int {\n    return a + b;\n}\nfn main() {\n    print(add(1, 2));\n}\n",
    );
    dir
}

#[test]
fn outline_order_equals_source_order() {
    let dir = fixture_dir("outline-order");
    let source = dir.join("main.ya");
    let (ok, stdout, stderr) = run(&["outline", source.to_str().unwrap()]);
    assert!(ok, "outline must succeed: {stderr}");
    let add = stdout.find("fn add").expect("add is outlined");
    let main = stdout.find("fn main").expect("main is outlined");
    assert!(add < main, "source order:\n{stdout}");
}

#[test]
fn graph_has_one_edge_per_import() {
    let dir = common::work_dir("graph-imports");
    common::write(&dir, "helper.ya", "pub pure fn helper() -> int {\n    return 1;\n}\n");
    common::write(
        &dir,
        "main.ya",
        "import helper;\nfn main() {\n    print(helper());\n}\n",
    );
    let (ok, stdout, stderr) = run(&["graph", "--json", dir.to_str().unwrap()]);
    assert!(ok, "graph must succeed: {stderr}");
    assert!(
        stdout.contains("\"kind\": \"import\""),
        "an import edge:\n{stdout}"
    );
    assert!(
        stdout.contains("\"to\": \"helper\""),
        "the imported module is named:\n{stdout}"
    );
}

#[test]
fn the_dot_output_is_a_digraph() {
    let dir = fixture_dir("graph-dot");
    let (ok, stdout, stderr) = run(&["graph", "--dot", dir.to_str().unwrap()]);
    assert!(ok, "graph must succeed: {stderr}");
    assert!(stdout.starts_with("digraph yashy {"), "a digraph:\n{stdout}");
    assert!(stdout.trim_end().ends_with('}'), "closed:\n{stdout}");
}

#[test]
fn search_ranks_the_exact_name_first() {
    let dir = fixture_dir("search-exact");
    let (ok, stdout, stderr) = run(&["search", "add", dir.to_str().unwrap()]);
    assert!(ok, "search must succeed: {stderr}");
    let first = stdout.lines().next().expect("a result line");
    assert!(
        first.starts_with("fn add"),
        "the exact name ranks first:\n{stdout}"
    );
}

#[test]
fn search_with_no_match_exits_zero_with_an_empty_result() {
    let dir = fixture_dir("search-empty");
    let (ok, stdout, stderr) = run(&["search", "nosuchsymbol", dir.to_str().unwrap()]);
    assert!(ok, "no match is not an error: {stderr}");
    assert!(stdout.is_empty(), "an empty result set:\n{stdout}");
}

#[test]
fn search_finds_doc_text() {
    let dir = fixture_dir("search-docs");
    let (ok, stdout, stderr) = run(&["search", "Adds two", dir.to_str().unwrap()]);
    assert!(ok, "search must succeed: {stderr}");
    assert!(stdout.contains("fn add"), "a doc match:\n{stdout}");
}
