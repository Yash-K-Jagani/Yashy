//! SQLite CRUD against the pinned amalgamation.
//!
//! The fixture is an in-memory database, so the test runs on every host. A
//! zero leak report is asserted alongside the program output.

mod common;

use std::process::Command;

#[test]
fn sqlite_crud_program_runs_without_leaks() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let paths = format!(
        "{}:{}",
        root.join("crates/yashy-stdlib/yashy").display(),
        root.join("packages/sqlite/src").display()
    );
    let dir = common::work_dir("sqlite-crud");
    let binary = dir.join(format!("sqlite{}", std::env::consts::EXE_SUFFIX));
    let build = Command::new(common::yashyc())
        .arg("build")
        .arg(root.join("packages/sqlite/tests/crud.ya"))
        .arg("-o")
        .arg(&binary)
        .env("YASHY_PATH", &paths)
        .current_dir(&root)
        .output()
        .expect("the compiler must run");
    assert!(
        build.status.success(),
        "building the SQLite test failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&binary)
        .env("YASHY_LEAK_REPORT", "1")
        .output()
        .expect("the SQLite test must run");
    assert!(
        run.status.success(),
        "the SQLite test exited with {:?}:\n{}",
        run.status.code(),
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert_eq!(stdout, "sqlite-ok\n", "unexpected output: {stdout}");
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("yashy: leak report: 0 live allocations"),
        "the SQLite test leaked:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
}
