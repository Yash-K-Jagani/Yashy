//! Package dependencies, end to end: a path dependency builds, a diamond
//! resolves one copy, a cycle is `E1801`, a pinned git dependency is
//! reproducible, and `--offline` is honest about the store.
//!
//! Each test owns a private package store (`YASHY_PACKAGE_STORE`) and cache, so
//! the suite can run in parallel and never sees another test's checkout.

mod common;

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use common::{exe, launch_output, work_dir, write, yashy};
use yashy_manifest::Lockfile;

/// Run the CLI in `directory` with a private store and cache.
fn cli(directory: &Path, store: &Path, args: &[&str]) -> Output {
    Command::new(yashy())
        .args(args)
        .current_dir(directory)
        .env("YASHY_PACKAGE_STORE", store)
        .env("YASHY_CACHE_DIR", store.join("cache"))
        .output()
        .expect("the yashy CLI must be runnable")
}

/// A path package named `name` with one module exposing `value`.
fn write_path_package(root: &Path, name: &str, value: i64) {
    write(
        root,
        "yashy.toml",
        &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
    );
    write(
        root,
        &format!("src/{name}.ya"),
        &format!("pub fn value() -> int {{\n    return {value};\n}}\n"),
    );
}

/// A git repository holding a package named `extra`, or `None` when `git` is
/// not on `PATH` (the fetcher itself documents that case; the test skips).
fn git_package(root: &Path) -> Option<(String, String)> {
    if !Command::new("git")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
    {
        eprintln!("skipping: `git` is not on PATH");
        return None;
    }
    let repo = root.join("repo");
    fs::create_dir_all(&repo).expect("the test repository must be creatable");
    write_path_package(&repo, "extra", 41);
    let run = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(&repo)
            .env("GIT_AUTHOR_NAME", "Yashy Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
            .env("GIT_COMMITTER_NAME", "Yashy Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
            .output()
            .expect("git must start");
        assert!(
            output.status.success(),
            "git {args:?} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };
    run(&["init", "--quiet"]);
    run(&["add", "."]);
    run(&[
        "-c",
        "user.name=Yashy Test",
        "-c",
        "user.email=test@example.invalid",
        "commit",
        "--quiet",
        "-m",
        "initial",
    ]);
    let rev = String::from_utf8_lossy(&run(&["rev-parse", "HEAD"]).stdout)
        .trim()
        .to_string();
    let url = format!("file:///{}", repo.display().to_string().replace('\\', "/"));
    Some((rev, url))
}

fn git_manifest(rev: &str, url: &str) -> String {
    format!(
        "[package]\nname = \"git-app\"\nversion = \"0.1.0\"\n\n\
         [dependencies.extra]\ngit = \"{url}\"\nrev = \"{rev}\"\n"
    )
}

#[test]
fn a_path_dependency_builds_and_runs() {
    let dir = work_dir("dep-path");
    let store = work_dir("dep-path-store");
    write_path_package(&dir.join("net"), "net", 7);
    let app = dir.join("app");
    write(
        &app,
        "yashy.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n\
         [dependencies.net]\npath = \"../net\"\n",
    );
    write(
        &app,
        "main.ya",
        "import net;\n\nfn main() {\n    assert(net.value() == 7);\n    print(\"path-ok\");\n}\n",
    );

    let binary = exe(&app, "app");
    let output = cli(
        &app,
        &store,
        &["build", "main.ya", "-o", &binary.to_string_lossy()],
    );
    assert!(
        output.status.success(),
        "a path dependency must build:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = launch_output(&binary).expect("the built program must start");
    assert!(run.status.success(), "the built program must pass");
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "path-ok");
}

#[test]
fn a_diamond_dependency_resolves_to_one_copy() {
    let dir = work_dir("dep-diamond");
    let store = work_dir("dep-diamond-store");
    // `a` and `b` both depend on `c` at the same path; the graph has one `c`.
    write_path_package(&dir.join("c"), "c", 3);
    write_path_package(&dir.join("a"), "a", 1);
    write(
        &dir.join("a"),
        "yashy.toml",
        "[package]\nname = \"a\"\nversion = \"0.1.0\"\n\n\
         [dependencies.c]\npath = \"../c\"\n",
    );
    write_path_package(&dir.join("b"), "b", 2);
    write(
        &dir.join("b"),
        "yashy.toml",
        "[package]\nname = \"b\"\nversion = \"0.1.0\"\n\n\
         [dependencies.c]\npath = \"../c\"\n",
    );
    let app = dir.join("app");
    write(
        &app,
        "yashy.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n\
         [dependencies.a]\npath = \"../a\"\n\n[dependencies.b]\npath = \"../b\"\n",
    );
    write(&app, "main.ya", "fn main() { }\n");

    let update = cli(&app, &store, &["update"]);
    assert!(
        update.status.success(),
        "update must resolve the diamond:\n{}",
        String::from_utf8_lossy(&update.stderr)
    );
    let lock = Lockfile::load(&app.join("yashy.lock")).expect("update wrote a lock");
    assert_eq!(
        lock.packages.len(),
        3,
        "a, b and one c: {:?}",
        lock.packages
            .iter()
            .map(|package| package.name.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        lock.packages
            .iter()
            .filter(|package| package.name == "c")
            .count(),
        1
    );
    // `a` and `b` each list `c 0.1.0` once, not twice.
    for name in ["a", "b"] {
        let package = lock.package(name).expect("the package is locked");
        assert_eq!(package.dependencies, vec!["c 0.1.0".to_string()]);
    }
}

#[test]
fn a_path_cycle_is_e1801_with_the_path() {
    let dir = work_dir("dep-cycle");
    let store = work_dir("dep-cycle-store");
    write_path_package(&dir.join("a"), "a", 1);
    write_path_package(&dir.join("b"), "b", 2);
    write(
        &dir.join("a"),
        "yashy.toml",
        "[package]\nname = \"a\"\nversion = \"0.1.0\"\n\n\
         [dependencies.b]\npath = \"../b\"\n",
    );
    write(
        &dir.join("b"),
        "yashy.toml",
        "[package]\nname = \"b\"\nversion = \"0.1.0\"\n\n\
         [dependencies.a]\npath = \"../a\"\n",
    );
    let app = dir.join("app");
    write(
        &app,
        "yashy.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n\
         [dependencies.a]\npath = \"../a\"\n",
    );
    write(&app, "main.ya", "fn main() { }\n");

    let output = cli(&app, &store, &["build", "main.ya"]);
    assert_eq!(output.status.code(), Some(1), "a cycle must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("E1801"), "{stderr}");
    assert!(stderr.contains("a -> b -> a"), "{stderr}");
    assert!(stderr.contains("docs:"), "{stderr}");
}

#[test]
fn a_git_dependency_at_a_fixed_rev_is_reproducible() {
    let dir = work_dir("dep-git-repro");
    let Some((rev, url)) = git_package(&dir) else {
        return;
    };
    let app = dir.join("app");
    write(&app, "yashy.toml", &git_manifest(&rev, &url));
    write(&app, "main.ya", "fn main() { }\n");

    // Two independent stores fetch the same commit; the lock must be identical.
    let first_store = work_dir("dep-git-repro-store-1");
    let second_store = work_dir("dep-git-repro-store-2");
    let first = cli(&app, &first_store, &["update"]);
    assert!(
        first.status.success(),
        "the first update must fetch:\n{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first_lock = fs::read_to_string(app.join("yashy.lock")).expect("a lock");
    let second = cli(&app, &second_store, &["update"]);
    assert!(
        second.status.success(),
        "the second update must fetch:\n{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second_lock = fs::read_to_string(app.join("yashy.lock")).expect("a lock");
    assert_eq!(
        first_lock, second_lock,
        "the same commit yields the same lock"
    );
    assert!(
        first_lock.contains(&format!("git+{url}?rev={rev}")),
        "{first_lock}"
    );
    let lock = Lockfile::parse(&first_lock).expect("the lock parses");
    let extra = lock.package("extra").expect("extra is locked");
    assert!(extra.hash.starts_with("sha256:"), "{}", extra.hash);
    assert_ne!(extra.hash, format!("sha256:{}", "0".repeat(64)));
}

#[test]
fn offline_with_a_cold_store_is_e1803() {
    let dir = work_dir("dep-offline-cold");
    let store = work_dir("dep-offline-cold-store");
    let app = dir.join("app");
    write(
        &app,
        "yashy.toml",
        &git_manifest(
            "0123456789abcdef0123456789abcdef01234567",
            "file:///definitely/not/a/repository",
        ),
    );
    write(&app, "main.ya", "fn main() { }\n");

    let output = cli(&app, &store, &["build", "--offline", "main.ya"]);
    assert_eq!(output.status.code(), Some(1), "a cold offline build fails");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("E1803"), "{stderr}");
    assert!(stderr.contains("not in the local store"), "{stderr}");
    assert!(stderr.contains("yashy fetch"), "{stderr}");
    assert!(stderr.contains("--offline"), "{stderr}");
    assert!(stderr.contains("docs:"), "{stderr}");
    assert!(
        !stderr.contains("error[E1801]"),
        "the store miss is not a cycle:\n{stderr}"
    );
}

#[test]
fn offline_with_a_warm_store_builds() {
    let dir = work_dir("dep-offline-warm");
    let store = work_dir("dep-offline-warm-store");
    let Some((rev, url)) = git_package(&dir) else {
        return;
    };
    let app = dir.join("app");
    write(&app, "yashy.toml", &git_manifest(&rev, &url));
    write(
        &app,
        "main.ya",
        "import extra;\n\nfn main() {\n    assert(extra.value() == 41);\n    print(\"git-ok\");\n}\n",
    );

    // Warm the store once with network/git allowed.
    let fetch = cli(&app, &store, &["fetch"]);
    assert!(
        fetch.status.success(),
        "fetch must populate the store:\n{}",
        String::from_utf8_lossy(&fetch.stderr)
    );

    // Now the same build must not need git or the network at all.
    let binary = exe(&app, "app");
    let output = cli(
        &app,
        &store,
        &[
            "build",
            "--offline",
            "main.ya",
            "-o",
            &binary.to_string_lossy(),
        ],
    );
    assert!(
        output.status.success(),
        "a warm offline build must succeed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = launch_output(&binary).expect("the built program must start");
    assert!(run.status.success(), "the built program must pass");
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "git-ok");
}
