//! `yashyc graph --from <symbol> --depth <n>`: a scoped query over the same
//! module graph the unfiltered command renders.

mod common;

use std::process::Command;

#[test]
fn a_scoped_graph_contains_only_reachable_edges() {
    let dir = common::work_dir("graph-scope");
    common::write(
        &dir,
        "main.ya",
        "import helper;\nfn main() { helper.run(); }\n",
    );
    common::write(&dir, "helper.ya", "pub fn run() {}\n");
    let output = Command::new(common::yashyc())
        .args(["graph", "--from", "main.ya", "--depth", "1"])
        .current_dir(&dir)
        .output()
        .expect("the CLI must run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"nodes\""), "{stdout}");
    assert!(stdout.contains("\"edges\""), "{stdout}");
    assert!(stdout.contains("helper"), "{stdout}");
}

#[test]
fn edge_filters_restrict_the_kinds_followed() {
    let dir = common::work_dir("graph-scope-edges");
    common::write(
        &dir,
        "tests/check.ya",
        "fn check() { run_helper(); }\nfn run_helper() {}\n",
    );
    let calls = Command::new(common::yashyc())
        .args([
            "graph",
            "--from",
            "tests/check.ya",
            "--depth",
            "1",
            "--edges",
            "calls",
        ])
        .current_dir(&dir)
        .output()
        .expect("the CLI must run");
    assert!(
        calls.status.success(),
        "{}",
        String::from_utf8_lossy(&calls.stderr)
    );
    let calls = String::from_utf8_lossy(&calls.stdout);
    assert!(calls.contains("\"kind\": \"call\""), "{calls}");
    assert!(calls.contains("run_helper"), "{calls}");

    let imports = Command::new(common::yashyc())
        .args([
            "graph",
            "--from",
            "tests/check.ya",
            "--depth",
            "1",
            "--edges",
            "imports",
        ])
        .current_dir(&dir)
        .output()
        .expect("the CLI must run");
    let imports = String::from_utf8_lossy(&imports.stdout);
    assert!(imports.contains("\"edges\": []"), "{imports}");
}

#[test]
fn an_unmodelled_edge_kind_is_a_usage_error() {
    let output = Command::new(common::yashyc())
        .args(["graph", "--from", "main.ya", "--edges", "owns"])
        .output()
        .expect("the CLI must run");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("import and call"), "{stderr}");
}

#[test]
fn depth_zero_has_no_edges() {
    let dir = common::work_dir("graph-scope-zero");
    common::write(&dir, "main.ya", "import helper;\nfn main() {}\n");
    common::write(&dir, "helper.ya", "pub fn run() {}\n");
    let output = Command::new(common::yashyc())
        .args(["graph", "--from", "main.ya", "--depth", "0"])
        .current_dir(&dir)
        .output()
        .expect("the CLI must run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"edges\": []"), "{stdout}");
}

#[test]
fn a_scope_cannot_be_rendered_as_dot() {
    let output = Command::new(common::yashyc())
        .args(["graph", "--from", "main.ya", "--dot"])
        .output()
        .expect("the CLI must run");
    assert_eq!(output.status.code(), Some(2));
}
