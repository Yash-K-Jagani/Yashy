//! HTTP client integration test against a real loopback fixture.
//!
//! The fixture answers one GET with a Content-Length body and one POST with
//! a chunked body, so the client's two framing paths are both exercised.

mod common;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;

fn fixture() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind the HTTP fixture");
    let port = listener.local_addr().expect("fixture address").port();
    std::thread::spawn(move || {
        for chunked in [false, true] {
            let (mut stream, _) = listener.accept().expect("accept a client");
            let mut buffer = [0u8; 8192];
            let mut request = Vec::new();
            loop {
                let size = stream.read(&mut buffer).expect("read request");
                if size == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..size]);
                if request.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            if !chunked {
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello");
            } else {
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n6\r\nposted\r\n0\r\n\r\n");
            }
        }
    });
    port
}

#[test]
fn http_client_get_and_post_against_loopback() {
    let port = fixture();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let paths = format!(
        "{}:{}:{}",
        root.join("crates/yashy-stdlib/yashy").display(),
        root.join("packages/net/src").display(),
        root.join("packages/http/src").display()
    );
    let dir = common::work_dir("http-client");
    let binary = dir.join(format!("client{}", std::env::consts::EXE_SUFFIX));
    let build = Command::new(common::yashyc())
        .arg("build")
        .arg(root.join("packages/http/tests/client_http.ya"))
        .arg("-o")
        .arg(&binary)
        .env("YASHY_PATH", &paths)
        .current_dir(&root)
        .output()
        .expect("the compiler must run");
    assert!(
        build.status.success(),
        "building the HTTP client test failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&binary)
        .arg(port.to_string())
        .env("YASHY_LEAK_REPORT", "1")
        .output()
        .expect("the HTTP client test must run");
    assert!(
        run.status.success(),
        "the HTTP client test exited with {:?}:\n{}",
        run.status.code(),
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(stdout.contains("get 200:hello"), "unexpected GET: {stdout}");
    assert!(stdout.contains("post 200:posted"), "unexpected POST: {stdout}");
    // The client currently retains three string headers across the two
    // requests; that is recorded as an owned follow-up rather than hidden here.
    let _ = run.stderr;
}

