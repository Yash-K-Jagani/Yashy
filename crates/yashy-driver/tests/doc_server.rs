//! `yashyc doc --serve` serves the compiler's doc model on loopback only.

mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/tutorial")
}

fn get(port: u16, path: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("loopback must accept");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set timeout");
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .expect("send request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");
    response
}

#[test]
fn the_server_binds_loopback_and_answers_the_index_json_and_search() {
    let (listener, model, port) = yashy_driver::doc_server::bind(&root(), false, 0)
        .expect("the docs model must build and the port must bind");
    std::thread::spawn(move || yashy_driver::doc_server::serve_bound(listener, model));

    for path in ["/", "/json", "/search?q=fn"] {
        let response = get(port, path);
        assert!(
            response.starts_with("HTTP/1.1 200 OK"),
            "{path}: {response}"
        );
        assert!(response.contains("Content-Length:"), "{path}");
        assert!(response.contains("Cache-Control: no-store"), "{path}");
    }
    let not_found = get(port, "/../../secret");
    assert!(not_found.starts_with("HTTP/1.1 404"), "{not_found}");
}

#[test]
fn the_server_never_binds_a_non_loopback_address() {
    // `bind` takes only a port, so there is no spelling of an address to pass.
    // A direct listener on 0.0.0.0 would be a different function; check the
    // source of truth rather than claiming a runtime guarantee that is not
    // expressed in the API.
    let source = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/doc_server.rs"),
    )
    .expect("the doc server source exists");
    assert!(
        source.contains("TcpListener::bind((\"127.0.0.1\", port))"),
        "the docs server must bind loopback only"
    );
    assert!(
        !source.contains("0.0.0.0"),
        "no wildcard bind may appear in the docs server"
    );
}
