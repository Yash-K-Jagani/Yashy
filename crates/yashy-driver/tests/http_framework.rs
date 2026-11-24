//! The HTTP server framework programs: routing, extractors and middleware.

mod common;

use std::process::Command;

#[test]
fn http_framework_program_runs_without_leaks() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let paths = format!(
        "{}:{}:{}",
        root.join("crates/yashy-stdlib/yashy").display(),
        root.join("packages/net/src").display(),
        root.join("packages/http/src").display()
    );
    let dir = common::work_dir("http-framework");
    let binary = dir.join(format!("framework{}", std::env::consts::EXE_SUFFIX));
    let build = Command::new(common::yashyc())
        .arg("build")
        .arg(root.join("packages/http/tests/framework.ya"))
        .arg("-o")
        .arg(&binary)
        .env("YASHY_PATH", &paths)
        .current_dir(&root)
        .output()
        .expect("the compiler must run");
    assert!(
        build.status.success(),
        "building the framework test failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&binary)
        .env("YASHY_LEAK_REPORT", "1")
        .output()
        .expect("the framework test must run");
    assert!(
        run.status.success(),
        "the framework test exited with {:?}:\n{}",
        run.status.code(),
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert_eq!(stdout, "framework-ok\n", "unexpected output: {stdout}");
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("yashy: leak report: 0 live allocations"),
        "the framework test leaked:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
}
