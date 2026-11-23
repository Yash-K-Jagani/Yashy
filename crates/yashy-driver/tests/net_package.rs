//! The `net` package's own end-to-end test: TCP echo, UDP datagram, DNS and
//! an absolute deadline, all on loopback. Package tests are not part of the
//! `stdlib` sysroot, so the harness supplies both search paths explicitly.

mod common;

use std::process::Command;

#[test]
fn net_package_sockets_udp_dns_and_deadline() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let stdlib = root.join("crates/yashy-stdlib/yashy");
    let net_src = root.join("packages/net/src");
    let source = root.join("packages/net/tests/sockets.ya");
    let dir = common::work_dir("net-package");
    let binary = dir.join(format!("sockets{}", std::env::consts::EXE_SUFFIX));
    let build = Command::new(common::yashyc())
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .env("YASHY_PATH", format!("{}:{}", stdlib.display(), net_src.display()))
        .current_dir(&root)
        .output()
        .expect("the compiler must run");
    assert!(
        build.status.success(),
        "building the net package test failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&binary)
        .env("YASHY_LEAK_REPORT", "1")
        .output()
        .expect("the net package test must run");
    assert!(
        run.status.success(),
        "the net package test exited with {:?}:\n{}",
        run.status.code(),
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8_lossy(&run.stdout);
    let mut expected = vec!["tcp 5", "udp 8", "dns true", "deadline timeout", "cancel cancelled"];
    if cfg!(unix) {
        expected.push("unix 5");
    }
    for line in expected {
        assert!(stdout.contains(line), "missing {line} in:\n{stdout}");
    }
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("yashy: leak report: 0 live allocations"),
        "the net package test leaked"
    );
}
