//! The daemon is a cache in front of the compiler, not a second compiler.
//!
//! A check reuses the driver's checker; a second check of the same bytes is a
//! cache hit; changing the file produces new diagnostics without restarting.

mod common;

use common::{work_dir, write};
use yashy_driver::daemon::DaemonHandle;
use yashy_driver::daemon_protocol::{Request, ResponseKind};

fn diagnostics(result: ResponseKind) -> (Vec<yashy_common::json_rpc::Value>, u64) {
    match result {
        ResponseKind::Diagnostics {
            diagnostics,
            cache_hits,
        } => (diagnostics, cache_hits),
        other => panic!("expected diagnostics, got {other:?}"),
    }
}

#[test]
fn a_check_is_cached_and_a_change_produces_new_diagnostics() {
    let dir = work_dir("daemon-check");
    write(
        &dir,
        "bad.ya",
        "fn main() {\n    const int count = \"three\";\n}\n",
    );

    let handle = DaemonHandle::start(&dir).expect("the daemon must start");
    let mut client = handle.client().expect("the client must connect");

    let (first, hits) = diagnostics(
        client
            .call(Request::Check {
                file: "bad.ya".to_string(),
            })
            .expect("the first check must answer"),
    );
    assert!(!first.is_empty(), "a type error must produce a diagnostic");
    assert_eq!(hits, 0, "the first check is a miss");

    let (again, hits) = diagnostics(
        client
            .call(Request::Check {
                file: "bad.ya".to_string(),
            })
            .expect("the second check must answer"),
    );
    assert_eq!(
        again, first,
        "the same bytes must give the same diagnostics"
    );
    assert!(
        hits > 0,
        "a second check of the same bytes must hit the cache"
    );

    write(&dir, "bad.ya", "fn main() {}\n");
    let (fixed, _) = diagnostics(
        client
            .call(Request::Check {
                file: "bad.ya".to_string(),
            })
            .expect("the check after the edit must answer"),
    );
    assert!(fixed.is_empty(), "a fixed file must have no diagnostics");

    handle.shutdown();
}

#[test]
fn a_path_outside_the_root_is_refused() {
    let dir = work_dir("daemon-escape");
    let handle = DaemonHandle::start(&dir).expect("the daemon must start");
    let mut client = handle.client().expect("the client must connect");

    let result = client.call(Request::Check {
        file: "../secret.ya".to_string(),
    });
    match result {
        Err(message) => assert!(
            message.contains("cannot resolve") || message.contains("outside"),
            "the refusal must name the reason: {message}"
        ),
        Ok(ResponseKind::Diagnostics { diagnostics, .. }) => {
            let text = format!("{diagnostics:?}");
            assert!(
                text.contains("outside") || text.contains("cannot resolve"),
                "the refusal must name the reason: {text}"
            );
        }
        Ok(other) => panic!("an escaped path must be refused, got {other:?}"),
    }

    handle.shutdown();
}

#[test]
fn diagnostics_and_shutdown_are_answered() {
    let dir = work_dir("daemon-verbs");
    write(&dir, "ok.ya", "fn main() {}\n");
    let handle = DaemonHandle::start(&dir).expect("the daemon must start");
    let mut client = handle.client().expect("the client must connect");

    let (empty, _) = diagnostics(
        client
            .call(Request::Diagnostics)
            .expect("diagnostics must answer"),
    );
    assert!(empty.is_empty(), "a fresh daemon has no last diagnostics");

    let result = client
        .call(Request::Shutdown)
        .expect("shutdown must answer");
    assert_eq!(result, ResponseKind::Ok);
    handle.shutdown();
}
