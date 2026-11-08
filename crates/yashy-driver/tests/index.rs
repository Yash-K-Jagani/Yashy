//! `yashy index`: the symbol index over a directory.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

fn yashy() -> &'static str {
    common::yashy()
}

fn index_of(dir: &Path) -> String {
    let output = Command::new(yashy())
        .arg("index")
        .arg("--json")
        .arg(dir)
        .output()
        .expect("the yashy CLI must be runnable");
    assert!(
        output.status.success(),
        "index failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn an_index_of_examples_contains_every_top_level_symbol() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let output = index_of(&root);
    assert!(
        output.contains("\"schema_version\": 1"),
        "json must be schema v1:\n{output}"
    );
    assert!(output.contains("\"kind\": \"fn\""), "a function symbol:\n{output}");
    assert!(
        output.contains("\"signature\": \"fn main() -> void\""),
        "main's signature:\n{output}"
    );
}

#[test]
fn a_pure_function_has_no_io_effect_and_a_contract_is_carried() {
    let dir = common::work_dir("index-effects");
    common::write(
        &dir,
        "main.ya",
        "/// Fetches a URL.\npure fn fetch(string url) -> string {\n    return url;\n}\nfn main() {\n    print(fetch(\"x\"));\n}\n",
    );
    let output = index_of(&dir);
    assert!(
        output.contains("\"name\": \"fetch\""),
        "the pure function is indexed:\n{output}"
    );
    assert!(
        output.contains("\"docs\": \"Fetches a URL.\""),
        "the doc block is carried:\n{output}"
    );
    assert!(
        output.contains("\"io\": false"),
        "a pure function has effects.io == false:\n{output}"
    );
    assert!(
        output.contains("\"signature\": \"fn fetch(string url) -> string\""),
        "the signature is rendered from the checked item:\n{output}"
    );
}

#[test]
fn the_json_is_byte_identical_across_two_runs() {
    let dir = common::work_dir("index-stable");
    common::write(
        &dir,
        "main.ya",
        "pure fn add(int a, int b) -> int {\n    return a + b;\n}\nfn main() {\n    print(add(1, 2));\n}\n",
    );
    let first = index_of(&dir);
    let second = index_of(&dir);
    assert_eq!(first, second, "the index must not depend on run order");
}

#[test]
fn the_tests_section_lists_the_functions_a_test_program_calls() {
    let dir = common::work_dir("index-tests");
    common::write(
        &dir,
        "main.ya",
        "pure fn add(int a, int b) -> int {\n    return a + b;\n}\nfn main() {\n    print(add(1, 2));\n}\n",
    );
    common::write(
        &dir,
        "tests/smoke.ya",
        "fn main() {\n    print(add(1, 2));\n}\n",
    );
    let output = index_of(&dir);
    assert!(
        output.contains("\"tests\""),
        "the index has a tests section:\n{output}"
    );
    assert!(
        output.contains("smoke.ya"),
        "the test program is listed:\n{output}"
    );
}
