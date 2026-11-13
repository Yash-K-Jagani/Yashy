//! The playground: the sandbox behind an HTTP front end.
//!
//! The point of the playground is that it is *not* a second sandbox. Every
//! run is a `yashy run --sandbox`, so the phase 11 escape corpus is the test
//! that matters: the same attempts, through the same code, must all be
//! denied.

mod common;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use yashy_driver::playground::{Limits, Reply, Request, Server};

fn compiler() -> PathBuf {
    PathBuf::from(common::yashy())
}

/// A server with the default limits, driving the compiler the tests built.
fn server(limits: Limits) -> Server {
    // One root per test process. `work_dir` wipes the directory it returns,
    // so every test asking for a fresh `playground-runs` would delete another
    // test's in-flight run directory, and the spawn that follows fails with
    // "the directory name is invalid" (os error 267).
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    let root = ROOT
        .get_or_init(|| common::work_dir("playground-runs"))
        .clone();
    Server::with_paths(limits, compiler(), root)
}

/// The escape corpus: the ten attempts that must not escape.
fn corpus() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("sandbox_corpus");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("the corpus must be readable")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("ya"))
        .collect();
    files.sort();
    files
}

#[test]
fn every_escape_attempt_is_denied() {
    let mut server = server(Limits::default());
    let files = corpus();
    assert!(files.len() >= 10, "the corpus must have its ten attempts");
    for path in files {
        let source = std::fs::read_to_string(&path).expect("the corpus file must be readable");
        let reply = server.handle(
            "192.0.2.1",
            Request {
                source,
                args: Vec::new(),
            },
        );
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("");
        let Reply::Ran {
            stdout,
            stderr,
            exit,
        } = reply
        else {
            // A refusal is also a denial, as long as it is the sandbox's and
            // not a crash.
            panic!("{name} must be answered by a run");
        };
        let text = format!("{stdout}{stderr}");
        match name {
            // A denied read or write is a value the program matches on, so the
            // program itself prints the denial; the sandbox's job is that the
            // file outside the allowlist was never touched.
            "abs_path" | "traversal" | "symlink" | "env_path" | "write_out" | "socket"
            | "proc_mem" | "big_alloc" => {
                assert!(
                    exit == 0 && text.contains("denied"),
                    "{name} must be denied:\n{text} (exit {exit})"
                );
            }
            // A denied spawn exits loudly; an allowed one would print the
            // worker's answer. Either way the answer must not appear.
            "spawn_child" => {
                assert!(
                    exit != 0 || !text.contains("\n1\n"),
                    "a task must not start when spawn is off:\n{text}"
                );
            }
            // The time limit is enforced by the driver, so the process is
            // killed: a non-zero exit the program did not choose, plus E1901.
            "infinite_loop" => {
                assert!(
                    exit != 0 && text.contains("E1901"),
                    "{name} must end with the time limit:\n{text} (exit {exit})"
                );
            }
            other => panic!("the corpus has an attempt the test does not know: {other}"),
        }
    }
}

#[test]
fn a_source_over_the_cap_is_refused() {
    let mut server = server(Limits::default());
    let reply = server.handle(
        "192.0.2.2",
        Request {
            source: "x".repeat(64 * 1024 + 1),
            args: Vec::new(),
        },
    );
    match reply {
        Reply::Refused { status, message } => {
            assert_eq!(status, 413, "{message}");
            assert!(message.contains("65536"), "{message}");
        }
        other => panic!("a source over the cap must be refused, got {other:?}"),
    }
}

#[test]
fn a_caller_over_its_rate_is_refused() {
    // The window is an hour: these requests compile real programs, and three
    // compiles can take longer than a minute on a loaded machine. A rolling
    // minute would then let the fourth request through and fail a limit that
    // is working exactly as designed.
    let mut server = server(Limits {
        requests_per_minute: 3,
        rate_window: std::time::Duration::from_secs(3600),
        ..Limits::default()
    });
    for index in 0..3 {
        let reply = server.handle(
            "192.0.2.3",
            Request {
                source: "fn main() {}".to_string(),
                args: Vec::new(),
            },
        );
        assert!(
            matches!(reply, Reply::Ran { .. }),
            "request {index} must be answered"
        );
    }
    let reply = server.handle(
        "192.0.2.3",
        Request {
            source: "fn main() {}".to_string(),
            args: Vec::new(),
        },
    );
    match reply {
        Reply::Refused { status, message } => {
            assert_eq!(status, 429, "{message}");
            assert!(message.contains("too many requests"), "{message}");
        }
        other => panic!("a caller over its rate must be refused, got {other:?}"),
    }
    // Another caller is not affected: the limit is per caller.
    let reply = server.handle(
        "192.0.2.4",
        Request {
            source: "fn main() {}".to_string(),
            args: Vec::new(),
        },
    );
    assert!(
        matches!(reply, Reply::Ran { .. }),
        "another caller is not refused"
    );
}

#[test]
fn a_full_queue_is_refused_rather_than_queued() {
    let mut server = server(Limits {
        queue_depth: 0,
        ..Limits::default()
    });
    let reply = server.handle(
        "192.0.2.5",
        Request {
            source: "fn main() {}".to_string(),
            args: Vec::new(),
        },
    );
    match reply {
        Reply::Refused { status, message } => {
            assert_eq!(status, 429, "{message}");
            assert!(message.contains("busy"), "{message}");
        }
        other => panic!("a full queue must be refused, got {other:?}"),
    }
}

#[test]
fn a_program_that_prints_is_answered() {
    let mut server = server(Limits::default());
    let reply = server.handle(
        "192.0.2.6",
        Request {
            source: "fn main() {\n    print(\"hello\");\n}\n".to_string(),
            args: Vec::new(),
        },
    );
    match reply {
        Reply::Ran {
            stdout,
            stderr,
            exit,
        } => {
            assert_eq!(exit, 0, "the program must exit zero: {stderr}");
            // The stdout assertion carries the run's whole reply: an empty
            // stdout with a zero exit is either a program that printed
            // somewhere else or a build that was never run, and the stderr
            // is what tells those apart.
            assert_eq!(
                stdout, "hello\n",
                "the program printed nothing: exit {exit}, stderr {stderr:?}"
            );
        }
        other => panic!("the program must run, got {other:?}"),
    }
}

#[test]
fn a_program_that_does_not_compile_is_answered_with_the_diagnostic() {
    let mut server = server(Limits::default());
    let reply = server.handle(
        "192.0.2.7",
        Request {
            source: "fn main() {\n    let int x = \"three\";\n}\n".to_string(),
            args: Vec::new(),
        },
    );
    match reply {
        Reply::Ran { stderr, exit, .. } => {
            assert_ne!(
                exit, 0,
                "a program that does not compile must not exit zero"
            );
            assert!(
                stderr.contains("E0100"),
                "the diagnostic must be there: {stderr}"
            );
        }
        other => panic!("the answer must be a run, got {other:?}"),
    }
}

#[test]
fn the_reply_is_a_json_document_either_way() {
    let mut server = server(Limits::default());
    let reply = server.handle(
        "192.0.2.8",
        Request {
            source: "fn main() {}".to_string(),
            args: Vec::new(),
        },
    );
    let json = reply.json();
    assert!(json.contains("\"ok\": true"), "{json}");
    assert!(json.contains("\"stdout\""), "{json}");
    assert!(json.contains("\"exit\""), "{json}");

    let refused = Reply::Refused {
        status: 429,
        message: "busy".to_string(),
    };
    let json = refused.json();
    assert!(json.contains("\"ok\": false"), "{json}");
    assert!(json.contains("\"status\": 429"), "{json}");
}
