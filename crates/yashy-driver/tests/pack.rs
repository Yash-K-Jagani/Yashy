//! Structured context packs: the excerpts are compiler facts with paths,
//! spans and hashes, and a cut pack names everything that did not fit.

mod common;

use std::path::PathBuf;
use std::process::Command;

fn yashyc() -> &'static str {
    common::yashyc()
}

fn run(args: &[&str]) -> (bool, String, String) {
    let output = Command::new(yashyc())
        .args(args)
        .output()
        .expect("the yashyc CLI must be runnable");
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
        "tests/lib.ya",
        "/// Adds two numbers.\npub pure fn add(int a, int b) -> int {\n    return a + b;\n}\n",
    );
    common::write(
        &dir,
        "tests/smoke.ya",
        "import lib;\nfn main() {\n    print(lib.add(1, 2));\n}\n",
    );
    dir
}

#[test]
fn a_focus_pack_returns_structured_excerpts_and_a_revision() {
    let dir = fixture_dir("pack-focus");
    let root = dir.to_str().unwrap();
    let (ok, stdout, stderr) = run(&[
        "context",
        "--json",
        "--task",
        "fix the bug in add",
        "--mode",
        "focus",
        root,
    ]);
    assert!(ok, "a focus pack must succeed: {stderr}");
    for field in [
        "\"excerpts\"",
        "\"file\"",
        "\"hash\"",
        "\"score\"",
        "\"revision\"",
        "\"signatures\"",
    ] {
        assert!(stdout.contains(field), "{field} is missing:\n{stdout}");
    }
    assert!(
        stdout.contains("add"),
        "the focus signature is in the pack: {stdout}"
    );
    assert!(
        stdout.contains("smoke.ya"),
        "the covering test is in the pack: {stdout}"
    );
}

#[test]
fn the_same_request_has_the_same_revision() {
    let dir = fixture_dir("pack-stable");
    let root = dir.to_str().unwrap();
    let (_, first, _) = run(&[
        "context",
        "--json",
        "--task",
        "fix the bug in add",
        "--mode",
        "focus",
        root,
    ]);
    let (_, second, _) = run(&[
        "context",
        "--json",
        "--task",
        "fix the bug in add",
        "--mode",
        "focus",
        root,
    ]);
    assert_eq!(first, second, "packs are byte-stable");
}

#[test]
fn a_small_budget_drops_excerpts_with_a_reason() {
    let dir = fixture_dir("pack-budget");
    let (ok, stdout, stderr) = run(&[
        "context",
        "--json",
        "--task",
        "fix the bug in add",
        "--mode",
        "focus",
        "--budget",
        "10",
        dir.to_str().unwrap(),
    ]);
    assert!(ok, "a cut pack still exits 0: {stderr}");
    assert!(stdout.contains("\"dropped\""), "drops are named: {stdout}");
    assert!(stdout.contains("over the 10 token budget"), "{stdout}");
}

#[test]
fn an_error_pack_carries_the_diagnostic_and_an_edited_file_pack_carries_the_file() {
    let dir = fixture_dir("pack-modes");
    let root = dir.to_str().unwrap();
    let (ok, stdout, stderr) = run(&[
        "context",
        "--json",
        "--task",
        "fix the bug in add",
        "--mode",
        "error",
        "--diagnostic",
        "E0100 at line 2",
        root,
    ]);
    assert!(ok, "an error pack must succeed: {stderr}");
    assert!(stdout.contains("E0100 at line 2"), "{stdout}");

    let (ok, stdout, stderr) = run(&[
        "context",
        "--json",
        "--task",
        "fix the bug in add",
        "--mode",
        "diff",
        "--changed",
        "tests/lib.ya",
        root,
    ]);
    assert!(ok, "a diff pack must succeed: {stderr}");
    assert!(stdout.contains("tests/lib.ya"), "{stdout}");
}

fn ranking_dir(name: &str) -> PathBuf {
    let dir = common::work_dir(name);
    common::write(
        &dir,
        "tests/lib.ya",
        "/// Adds two numbers.\npub pure fn add(int a, int b) -> int {\n    return a + b;\n}\n",
    );
    common::write(
        &dir,
        "tests/near.ya",
        "import lib;\nfn near(int a) -> int {\n    return lib.add(a, 1);\n}\n",
    );
    common::write(
        &dir,
        "tests/far.ya",
        "import lib;\nfn far(int a) -> int {\n    return a + 1;\n}\n",
    );
    common::write(
        &dir,
        "tests/peer.ya",
        "import lib;\nfn peer(int a) -> int {\n    return a + 2;\n}\n",
    );
    common::write(
        &dir,
        "tests/unlinked.ya",
        "fn unlinked(int a) -> int {\n    return a + 3;\n}\n",
    );
    dir
}

#[test]
fn graph_distance_ranks_a_neighbour_before_an_unconnected_file() {
    let dir = ranking_dir("pack-rank-distance");
    let (ok, stdout, stderr) = run(&[
        "context",
        "--json",
        "--task",
        "fix the bug in add",
        "--mode",
        "diff",
        "--changed",
        "tests/unlinked.ya,tests/far.ya",
        dir.to_str().unwrap(),
    ]);
    assert!(ok, "{stderr}");
    let near = stdout
        .find("tests/far.ya")
        .expect("the neighbour is in the pack");
    let far = stdout
        .find("tests/unlinked.ya")
        .expect("the unconnected file is in the pack");
    assert!(
        near < far,
        "a direct import outranks no path at all: {stdout}"
    );
}

#[test]
fn edit_recency_breaks_a_tie_between_equally_near_files() {
    let dir = ranking_dir("pack-rank-recency");
    let (ok, stdout, stderr) = run(&[
        "context",
        "--json",
        "--task",
        "fix the bug in add",
        "--mode",
        "diff",
        "--changed",
        "tests/far.ya,tests/peer.ya",
        dir.to_str().unwrap(),
    ]);
    assert!(ok, "{stderr}");
    let first = stdout.find("tests/far.ya").expect("first edited");
    let second = stdout.find("tests/peer.ya").expect("second edited");
    assert!(
        first < second,
        "the caller's order is the edit recency: {stdout}"
    );
}

#[test]
fn an_unknown_mode_is_a_usage_error() {
    let dir = fixture_dir("pack-bad-mode");
    let (ok, _, stderr) = run(&[
        "context",
        "--task",
        "fix the bug in add",
        "--mode",
        "prompt",
        dir.to_str().unwrap(),
    ]);
    assert!(!ok, "an unknown mode exits 2: {stderr}");
    assert!(stderr.contains("unknown pack mode"), "{stderr}");
}

#[test]
fn a_stale_revision_is_refused() {
    let dir = fixture_dir("pack-stale");
    let root = dir.to_str().unwrap();
    let args = [
        "context",
        "--json",
        "--task",
        "fix the bug in add",
        "--mode",
        "focus",
        root,
    ];
    let (ok, stdout, stderr) = run(&args);
    assert!(ok, "first pack succeeds: {stderr}");
    let value = yashy_common::json_rpc::parse(&stdout).expect("valid pack JSON");
    let revision = value
        .get("revision")
        .and_then(yashy_common::json_rpc::Value::as_str)
        .expect("a revision")
        .to_string();

    let expected = [
        "context",
        "--json",
        "--task",
        "fix the bug in add",
        "--mode",
        "focus",
        "--expect-revision",
        &revision,
        root,
    ];
    let (ok, _, stderr) = run(&expected);
    assert!(ok, "a matching revision is accepted: {stderr}");

    common::write(
        &dir,
        "tests/lib.ya",
        "/// Adds two numbers.\npub pure fn add(int a, int b) -> int {\n    return a + b + 1;\n}\n",
    );
    let (ok, _, stderr) = run(&expected);
    assert!(!ok, "a changed workspace invalidates the pack: {stderr}");
    assert!(stderr.contains("E2204"), "{stderr}");
    assert!(stderr.contains("stale context pack revision"), "{stderr}");
    assert!(
        stderr.contains("docs: https://yashy.dev/errors/E2204"),
        "{stderr}"
    );
}

/// 4546 functions of eleven lines each: a workspace above 50k lines where
/// the focus is one small function, so the pack has to select rather than
/// dump. Generated in the test rather than checked in: the point is the
/// size, not a fixture a reviewer has to scroll past.
fn large_fixture_dir(name: &str) -> PathBuf {
    let dir = common::work_dir(name);
    let mut big = String::new();
    for index in 0..4546 {
        big.push_str(&format!(
            "fn big_f{index}(int x) -> int {{\n    let int a = x + 1;\n    let int b = a + 2;\n    let int c = b + 3;\n    let int d = c + 4;\n    let int e = d + 5;\n    let int f = e + 6;\n    let int g = f + 7;\n    return g;\n}}\n\n"
        ));
    }
    common::write(&dir, "tests/big.ya", &big);
    common::write(
        &dir,
        "tests/util.ya",
        "/// A callee in another file.\npub pure fn helper(int x) -> int {\n    return x + 1;\n}\n",
    );
    common::write(
        &dir,
        "tests/focus.ya",
        "import util;\n/// Does the work.\npub fn work(int x) -> int {\n    return util.helper(x);\n}\n",
    );
    common::write(
        &dir,
        "tests/work_test.ya",
        "import focus;\nfn main() {\n    print(focus.work(1));\n}\n",
    );
    dir
}

#[test]
fn a_focus_pack_cuts_a_fifty_thousand_line_workspace_to_its_budget() {
    let dir = large_fixture_dir("pack-50k");
    let (ok, stdout, stderr) = run(&[
        "context",
        "--json",
        "--task",
        "fix the bug in work",
        "--mode",
        "focus",
        "--budget",
        "8000",
        dir.to_str().unwrap(),
    ]);
    assert!(ok, "a large workspace still produces a pack: {stderr}");
    let value = yashy_common::json_rpc::parse(&stdout).expect("valid pack JSON");
    let tokens = value
        .get("tokens")
        .and_then(yashy_common::json_rpc::Value::as_i64)
        .expect("a token count") as usize;
    assert!(tokens <= 8000, "the pack fits its budget: {tokens}");
    assert!(stdout.contains("work"), "the focus signature is present");
    assert!(
        stdout.contains("helper"),
        "the called signature is present: {stdout}"
    );
    assert!(
        stdout.contains("work_test.ya"),
        "the covering test is present: {stdout}"
    );
}
