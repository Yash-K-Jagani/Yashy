//! RESP parser and client against a loopback fixture.
//!
//! The fixture is a small Python server that speaks RESP and splits replies
//! across TCP writes so the client's buffer assembly is exercised.

mod common;

use std::net::TcpListener;
use std::process::Command;

fn fixture() -> (u16, std::process::Child) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind the RESP fixture");
    let port = listener.local_addr().expect("fixture address").port();
    drop(listener);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let server = Command::new("python3")
        .arg(root.join("crates/yashy-driver/tests/fixtures/redis_server.py"))
        .arg(port.to_string())
        .spawn()
        .expect("the RESP fixture must start");
    std::thread::sleep(std::time::Duration::from_millis(400));
    (port, server)
}

#[test]
fn redis_client_program_runs_without_leaks() {
    let (port, mut server) = fixture();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let paths = format!(
        "{}:{}:{}",
        root.join("crates/yashy-stdlib/yashy").display(),
        root.join("packages/net/src").display(),
        root.join("packages/redis/src").display()
    );
    let dir = common::work_dir("redis-client");
    let binary = dir.join(format!("redisclient{}", std::env::consts::EXE_SUFFIX));
    let build = Command::new(common::yashyc())
        .arg("build")
        .arg(root.join("packages/redis/tests/client_test.ya"))
        .arg("-o")
        .arg(&binary)
        .env("YASHY_PATH", &paths)
        .current_dir(&root)
        .output()
        .expect("the compiler must run");
    assert!(
        build.status.success(),
        "building the Redis test failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&binary)
        .arg(port.to_string())
        .env("YASHY_LEAK_REPORT", "1")
        .output()
        .expect("the Redis test must run");
    let _ = server.kill();
    assert!(
        run.status.success(),
        "the Redis test exited with {:?}:\n{}",
        run.status.code(),
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert_eq!(stdout, "redis-ok\n", "unexpected output: {stdout}");
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("yashy: leak report: 0 live allocations"),
        "the Redis test leaked:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
}
