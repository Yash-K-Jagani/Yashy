//! Phase 3 Task 2: workspace members share one root and one selection.

mod common;

use std::path::Path;
use std::process::Command;

fn project(dir: &Path) {
    common::write(
        dir,
        "yashy.toml",
        "[workspace]\nmembers = [\"app\", \"lib\"]\ndefault-members = [\"app\"]\n",
    );
    common::write(
        dir,
        "app/yashy.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n\
         [dependencies.lib]\npath = \"../lib\"\n",
    );
    common::write(dir, "app/main.ya", "fn main() { print(1); }\n");
    common::write(dir, "app/tests/smoke.ya", "fn main() { assert(true); }\n");
    common::write(
        dir,
        "lib/yashy.toml",
        "[package]\nname = \"lib\"\nversion = \"0.1.0\"\n",
    );
    common::write(
        dir,
        "lib/src/lib.ya",
        "pub fn answer() -> int { return 42; }\n",
    );
    common::write(dir, "lib/tests/lib.ya", "fn main() { assert(true); }\n");
}

fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(common::yashyc())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("yashyc must run")
}

#[test]
fn a_manifest_can_describe_a_workspace() {
    let manifest = yashy_manifest::Manifest::parse(
        "[workspace]\nmembers = [\"app\", \"lib\"]\ndefault-members = [\"app\"]\n",
    )
    .expect("a workspace-root manifest parses");
    let workspace = manifest.workspace.expect("workspace table");
    assert_eq!(workspace.members, vec!["app", "lib"]);
    assert_eq!(workspace.default_members, vec!["app"]);
}

#[test]
fn workspace_members_are_discovered_and_selected() {
    let dir = common::work_dir("workspace-members");
    project(&dir);
    let workspace = yashy_driver::workspace::load(&dir)
        .expect("load workspace")
        .expect("a workspace root");
    let all = workspace.select(None, true, &[]).expect("all members");
    assert_eq!(all.len(), 2);
    let one = workspace.select(Some("app"), false, &[]).expect("app");
    assert_eq!(one.len(), 1);
    assert!(one[0].ends_with("app"));
    let default = workspace.select(None, false, &[]).expect("default");
    assert_eq!(default, one);
}

#[test]
fn test_workspace_and_package_selection_are_accepted() {
    let dir = common::work_dir("workspace-test");
    project(&dir);
    for args in [
        vec!["test", "--workspace"],
        vec!["test", "-p", "app"],
    ] {
        let output = run(&dir, &args);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.contains("not a workspace root") && !stderr.contains("no workspace member"),
            "{args:?}: {stderr}"
        );
    }
}
