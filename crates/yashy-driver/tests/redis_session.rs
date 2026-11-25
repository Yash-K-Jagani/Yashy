//! Redis-backed session store against the local RESP fixture.

mod common;

use std::net::TcpListener;
use std::path::Path;
use std::process::Command;

fn fixture() -> (u16, std::process::Child) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind the RESP fixture");
    let port = listener.local_addr().expect("fixture address").port();
    drop(listener);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let server = Command::new("python3")
        .arg(root.join("crates/yashy-driver/tests/fixtures/redis_server.py"))
        .arg(port.to_string())
        .spawn()
        .expect("the RESP fixture must start");
    std::thread::sleep(std::time::Duration::from_millis(400));
    (port, server)
}

#[test]
fn redis_session_store_runs_without_leaks() {
    let (port, mut server) = fixture();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let paths = format!(
        "{}:{}:{}:{}",
        root.join("crates/yashy-stdlib/yashy").display(),
        root.join("packages/net/src").display(),
        root.join("packages/redis/src").display(),
        root.join("packages/session/src").display()
    );
    let dir = common::work_dir("redis-session");
    let binary = dir.join(format!("redissession{}", std::env::consts::EXE_SUFFIX));
    let build = Command::new(common::yashyc())
        .arg("build")
        .arg(root.join("packages/session/tests/redis_session.ya"))
        .arg("-o")
        .arg(&binary)
        .env("YASHY_PATH", &paths)
        .current_dir(&root)
        .output()
        .expect("the compiler must run");
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new(&binary)
        .arg(port.to_string())
        .env("YASHY_LEAK_REPORT", "1")
        .output()
        .expect("the program must run");
    let _ = server.kill();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "redis-session-ok");
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("yashy: leak report: 0 live allocations"),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}
