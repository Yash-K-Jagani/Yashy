//! Impact analysis names the tests, packages, examples and docs a change
//! reaches. It reads the same index model review reads, so a symbol that is
//! not in the graph cannot be named.

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
        "docs/guide.md",
        "# Guide\n\n`add` combines two numbers.\n",
    );
    common::write(
        &dir,
        "examples/demo/main.ya",
        "fn add(int a, int b) -> int {\n    return a + b;\n}\n\
         fn main() {\n    print(add(2, 3));\n}\n",
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

fn impact(dir: &Path, baseline: &str, human: bool) -> String {
    let file = dir.join("baseline.json");
    std::fs::write(&file, baseline).unwrap();
    let mut args = vec!["impact", "--baseline", "baseline.json"];
    if !human {
        args.push("--json");
    }
    let (ok, stdout, stderr) = run(dir, &args);
    assert!(ok, "impact must succeed: {stderr}");
    stdout
}

#[test]
fn a_changed_symbol_names_its_test_example_and_doc() {
    let dir = workspace("impact-fanout");
    let base = baseline(&dir);
    common::write(
        &dir,
        "main.ya",
        "pub fn add(int a, int b) -> string {\n    return \"changed\";\n}\n\n\
         fn main() {\n    print(add(1, 2));\n}\n",
    );
    let json = impact(&dir, &base, false);
    assert!(json.contains("\"add\""), "{json}");
    assert!(json.contains("tests/smoke.ya"), "{json}");
    assert!(json.contains("examples/demo/main.ya"), "{json}");
    assert!(json.contains("docs/guide.md"), "{json}");
    assert!(json.contains("\"demo\""), "{json}");
    let human = impact(&dir, &base, true);
    assert!(human.contains("Impact:"), "{human}");
    assert!(human.contains("tests/smoke.ya"), "{human}");
}

#[test]
fn no_semantic_change_gives_an_empty_impact() {
    let dir = workspace("impact-empty");
    let base = baseline(&dir);
    let json = impact(&dir, &base, false);
    assert!(json.contains("\"symbols\": []"), "{json}");
    assert!(json.contains("\"tests\": []"), "{json}");
    assert!(json.contains("\"docs\": []"), "{json}");
}

#[test]
fn a_baseline_is_required() {
    let dir = workspace("impact-no-baseline");
    let (ok, _, stderr) = run(&dir, &["impact"]);
    assert!(!ok, "missing --baseline is a usage error: {stderr}");
    assert!(stderr.contains("--baseline"), "{stderr}");
}
