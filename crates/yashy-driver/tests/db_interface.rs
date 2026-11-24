//! The `db` interface program: one value model, typed rows, a fake driver,
//! transactions and a bounded pool, with a zero leak report.

mod common;

use std::process::Command;

#[test]
fn db_interface_program_runs_without_leaks() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let paths = format!(
        "{}:{}",
        root.join("crates/yashy-stdlib/yashy").display(),
        root.join("packages/db/src").display()
    );
    let dir = common::work_dir("db-interface");
    let binary = dir.join(format!("dbinterface{}", std::env::consts::EXE_SUFFIX));
    let build = Command::new(common::yashyc())
        .arg("build")
        .arg(root.join("packages/db/tests/interface.ya"))
        .arg("-o")
        .arg(&binary)
        .env("YASHY_PATH", &paths)
        .current_dir(&root)
        .output()
        .expect("the compiler must run");
    assert!(
        build.status.success(),
        "building the db interface test failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&binary)
        .env("YASHY_LEAK_REPORT", "1")
        .output()
        .expect("the db interface test must run");
    assert!(
        run.status.success(),
        "the db interface test exited with {:?}:\n{}",
        run.status.code(),
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert_eq!(stdout, "db-interface-ok\n", "unexpected output: {stdout}");
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("yashy: leak report: 0 live allocations"),
        "the db interface test leaked:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
}
