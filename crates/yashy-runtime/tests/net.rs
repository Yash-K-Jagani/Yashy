//! TCP sockets: the C contract the `net` package is built on.
//!
//! The language-level checks (linear handles, effects) live with the Yashy
//! program and the verifier; these tests pin the small C contract beneath
//! them. Every function returns an answer rather than aborting: `-1` is a
//! failure, `0` from `read` is a clean EOF after the peer closed its side.

use std::ffi::c_char;
use std::thread;
use yashy_runtime::{
    yashy_tcp_accept, yashy_tcp_close, yashy_tcp_connect, yashy_tcp_listen, yashy_tcp_read,
    yashy_tcp_write,
};

const HOST: &[u8] = b"127.0.0.1\0";

/// A high, uncommon port. The API contract does not expose the bound port, so
/// the tests share one deliberate constant.
fn port() -> i64 {
    18000 + (std::process::id() % 10_000) as i64
}

unsafe fn read_exact(fd: i64, buffer: &mut [u8]) -> i64 {
    unsafe { yashy_tcp_read(fd, buffer.as_mut_ptr(), buffer.len() as i64) }
}

#[test]
fn loopback_connect_accept_echo_and_eof() {
    let listen_port = port();
    unsafe {
        let listener = yashy_tcp_listen(HOST.as_ptr() as *const c_char, listen_port);
        assert!(listener >= 0, "loopback listen must succeed");

        let client = thread::spawn(move || {
            let client = yashy_tcp_connect(HOST.as_ptr() as *const c_char, listen_port);
            assert!(client >= 0, "loopback connect must succeed");
            let wrote = yashy_tcp_write(client, b"hello".as_ptr(), 5);
            assert_eq!(wrote, 5, "write must send every byte");
            let mut reply = [0u8; 5];
            assert_eq!(read_exact(client, &mut reply), 5);
            assert_eq!(&reply, b"world");
            yashy_tcp_close(client);
        });

        let server = yashy_tcp_accept(listener);
        assert!(server >= 0, "accept must succeed");
        let mut received = [0u8; 5];
        assert_eq!(read_exact(server, &mut received), 5);
        assert_eq!(&received, b"hello");
        assert_eq!(yashy_tcp_write(server, b"world".as_ptr(), 5), 5);

        // The client closed first; a read on the surviving socket is EOF.
        assert_eq!(read_exact(server, &mut received), 0, "EOF, not an error");
        client.join().expect("the client thread must finish");
        yashy_tcp_close(server);
        yashy_tcp_close(listener);
    }
}

#[test]
fn connect_to_a_closed_port_is_an_error_value() {
    let closed_port = port() + 1;
    unsafe {
        let fd = yashy_tcp_connect(HOST.as_ptr() as *const c_char, closed_port);
        assert_eq!(fd, -1, "a refused connect is a value, not a panic");
    }
}

#[test]
fn a_hundred_sequential_connections_do_not_accumulate_handles() {
    let listen_port = port() + 2;
    unsafe {
        let listener = yashy_tcp_listen(HOST.as_ptr() as *const c_char, listen_port);
        assert!(listener >= 0, "loopback listen must succeed");
        for _ in 0..100 {
            let client = yashy_tcp_connect(HOST.as_ptr() as *const c_char, listen_port);
            assert!(client >= 0, "each connect must succeed");
            assert_eq!(yashy_tcp_write(client, b"x".as_ptr(), 1), 1);
            let server = yashy_tcp_accept(listener);
            assert!(server >= 0, "each accept must succeed");
            let mut byte = [0u8; 1];
            assert_eq!(read_exact(server, &mut byte), 1);
            assert_eq!(byte, [b'x']);
            yashy_tcp_close(server);
            yashy_tcp_close(client);
        }
        yashy_tcp_close(listener);
    }
}

#[test]
fn invalid_arguments_return_errors_without_aborting() {
    unsafe {
        assert_eq!(yashy_tcp_listen(std::ptr::null(), port()), -1);
        assert_eq!(yashy_tcp_connect(std::ptr::null(), port()), -1);
        assert_eq!(yashy_tcp_read(-1, std::ptr::null_mut(), 1), -1);
        assert_eq!(yashy_tcp_write(-1, std::ptr::null(), 1), -1);
        // Closing an invalid descriptor is silently a no-op.
        yashy_tcp_close(-1);
    }
}
