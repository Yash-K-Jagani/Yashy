//! Query builder and codec programs for the db package.

mod common;

use std::process::Command;

fn build_and_run(root: &std::path::Path, source: &str, name: &str, expected: &str) {
    let paths = format!(
        "{}:{}",
        root.join("crates/yashy-stdlib/yashy").display(),
        root.join("packages/db/src").display()
    );
    let dir = common::work_dir(name);
    let binary = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    let build = Command::new(common::yashyc())
        .arg("build")
        .arg(root.join(source))
        .arg("-o")
        .arg(&binary)
        .env("YASHY_PATH", &paths)
        .current_dir(root)
        .output()
        .expect("the compiler must run");
    assert!(
        build.status.success(),
        "building {name} failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&binary)
        .env("YASHY_LEAK_REPORT", "1")
        .output()
        .expect("the program must run");
    assert!(
        run.status.success(),
        "{name} exited with {:?}:\n{}",
        run.status.code(),
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert_eq!(
        stdout,
        format!("{expected}\n"),
        "unexpected output: {stdout}"
    );
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("yashy: leak report: 0 live allocations"),
        "{name} leaked:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn query_builder_program_is_parameter_bound() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    build_and_run(
        &root,
        "packages/db/tests/query_test.ya",
        "db-query",
        "query-ok",
    );
    build_and_run(
        &root,
        "crates/yashy-driver/tests/programs/db_query_full.ya",
        "db-query-full",
        "query-full-ok",
    );
}

#[test]
fn codec_program_round_trips_every_field() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    build_and_run(
        &root,
        "crates/yashy-driver/tests/programs/db_codec.ya",
        "db-codec",
        "codec-ok",
    );
}

#[test]
fn migration_and_pool_program_reports_checksums_and_counts() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    build_and_run(
        &root,
        "crates/yashy-driver/tests/programs/db_migrate.ya",
        "db-migrate",
        "migrate-ok",
    );
}
