//! The build daemon: a warm cache in front of the one-shot compiler.
//!
//! It is optional by design. `yashyc serve` starts it; every other command
//! keeps working without one. The daemon owns the same `SourceMap`, query
//! cache and toolchain the CLI uses and routes every verb through the same
//! driver functions — it never re-implements checking or code generation.

use std::collections::HashMap;
use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use yashy_common::json_rpc::{self, number, object, string, Value};
use yashy_common::SourceMap;
use yashy_verify::{ContractMode, VerifyOptions};

use crate::daemon_protocol::{read_message, write_message, Request, Response, ResponseKind};
use crate::pipeline::{check_and_verify, compile_to_binary, BuildOptions};
use crate::toolchain::{detect, Toolchain};

const VERIFY_OPTIONS: VerifyOptions = VerifyOptions {
    contracts: ContractMode::Enforce,
};

/// Bump on any incompatible change to the request or response shape.
pub const PROTOCOL_VERSION: u32 = 1;

/// Where a running daemon publishes its address for clients to find.
fn addr_file(root: &Path) -> PathBuf {
    root.join(".yashy").join("daemon.addr")
}

/// The address of a compatible daemon for this workspace, if one is running.
/// A stale protocol or compiler version makes the file unusable; the caller
/// starts a fresh daemon, which overwrites it.
pub fn connect(root: &Path) -> io::Result<Option<SocketAddr>> {
    let text = match std::fs::read_to_string(addr_file(root)) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let value = match json_rpc::parse(&text) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let protocol = value.get("protocol").and_then(Value::as_i64).unwrap_or(0);
    let version = value.get("version").and_then(Value::as_str).unwrap_or("");
    if protocol != i64::from(PROTOCOL_VERSION) || version != env!("CARGO_PKG_VERSION") {
        return Ok(None);
    }
    let address = value
        .get("addr")
        .and_then(Value::as_str)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "daemon address is missing"))?;
    address
        .parse::<SocketAddr>()
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// The daemon's mutable state, touched only by its server thread.
struct Daemon {
    root: PathBuf,
    toolchain: Toolchain,
    /// Source bytes hash -> rendered diagnostics. A hit avoids re-checking an
    /// unchanged file, which is the whole point of keeping a daemon warm.
    checks: HashMap<String, (u64, Vec<Value>)>,
    cache_hits: u64,
    last_diagnostics: Vec<Value>,
}

impl Daemon {
    fn new(root: &Path, toolchain: Toolchain) -> Self {
        Daemon {
            root: root.to_path_buf(),
            toolchain,
            checks: HashMap::new(),
            cache_hits: 0,
            last_diagnostics: Vec::new(),
        }
    }

    /// Resolve a path the client named, refusing anything outside the root.
    fn resolve(&self, name: &str) -> Result<PathBuf, String> {
        let path = Path::new(name);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };
        let canonical = path
            .canonicalize()
            .map_err(|error| format!("cannot resolve {}: {error}", path.display()))?;
        let root = self
            .root
            .canonicalize()
            .map_err(|error| format!("cannot resolve root: {error}"))?;
        if !canonical.starts_with(&root) {
            return Err(format!("path outside the workspace root: {name}"));
        }
        Ok(path)
    }

    fn handle(&mut self, request: Request) -> ResponseKind {
        match request {
            Request::Check { file } => self.check(&file),
            Request::Verify { file } => self.check(&file),
            Request::Build { file, out } => self.build(&file, &out),
            Request::Test { root } => self.test(&root),
            Request::Diagnostics => ResponseKind::Diagnostics {
                diagnostics: self.last_diagnostics.clone(),
                cache_hits: self.cache_hits,
            },
            Request::Shutdown => ResponseKind::Ok,
        }
    }

    fn check(&mut self, name: &str) -> ResponseKind {
        let path = match self.resolve(name) {
            Ok(path) => path,
            Err(error) => return self.fail(error),
        };
        let source = match std::fs::read(&path) {
            Ok(source) => source,
            Err(error) => return self.fail(format!("cannot read {}: {error}", path.display())),
        };
        let hash = fnv1a(&source);
        let key = path.to_string_lossy().into_owned();
        if let Some((seen, diagnostics)) = self.checks.get(&key) {
            if *seen == hash {
                self.cache_hits += 1;
                self.last_diagnostics = diagnostics.clone();
                return ResponseKind::Diagnostics {
                    diagnostics: diagnostics.clone(),
                    cache_hits: self.cache_hits,
                };
            }
        }

        let mut sources = SourceMap::new();
        let display = path.to_string_lossy().replace('\\', "/");
        let outcome = check_and_verify(
            &mut sources,
            Path::new(&display),
            String::from_utf8_lossy(&source).into_owned(),
            false,
            &VERIFY_OPTIONS,
            0,
        );
        let rendered = match outcome {
            Ok((_, report)) => crate::diagnostics_json(&sources, &report.diagnostics),
            Err(list) => crate::diagnostics_json(&sources, &list),
        };
        let diagnostics: Vec<Value> = rendered.iter().map(json_to_value).collect();
        self.checks.insert(key, (hash, diagnostics.clone()));
        self.last_diagnostics = diagnostics.clone();
        ResponseKind::Diagnostics {
            diagnostics,
            cache_hits: self.cache_hits,
        }
    }

    fn build(&mut self, name: &str, out: &str) -> ResponseKind {
        let path = match self.resolve(name) {
            Ok(path) => path,
            Err(error) => return self.fail(error),
        };
        let source = match std::fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) => return self.fail(format!("cannot read {}: {error}", path.display())),
        };
        let output = PathBuf::from(out);
        let entry = path.to_string_lossy().replace('\\', "/");
        match compile_to_binary(
            &source,
            &entry,
            &output,
            &self.toolchain,
            BuildOptions::default(),
        ) {
            Ok(()) => ResponseKind::Built {
                path: output.to_string_lossy().into_owned(),
            },
            Err(diagnostics) => self.fail(format!(
                "{} diagnostic(s) while building",
                diagnostics.len()
            )),
        }
    }

    fn test(&mut self, name: &str) -> ResponseKind {
        let root = match self.resolve(name) {
            Ok(root) => root,
            Err(error) => return self.fail(error),
        };
        let tests = collect_tests(&root);
        let report = crate::test_harness::run_suite(
            &tests,
            &self.toolchain,
            yashy_codegen::host_target_triple(),
            0,
            &[],
        );
        if let Some(error) = report.internal_error {
            return self.fail(error);
        }
        ResponseKind::Test {
            passed: report.passed,
            failed: report.failed,
            compile_errors: report.compile_errors,
        }
    }

    fn fail(&mut self, message: String) -> ResponseKind {
        let diagnostic = error_value(&message);
        self.last_diagnostics = vec![diagnostic.clone()];
        ResponseKind::Diagnostics {
            diagnostics: vec![diagnostic],
            cache_hits: self.cache_hits,
        }
    }
}

/// Bridge the CLI's schema-v1 `Json` tree into the protocol's `Value` tree.
fn json_to_value(value: &yashy_common::Json) -> Value {
    json_rpc::parse(&value.render()).unwrap_or(Value::Null)
}

fn error_value(message: &str) -> Value {
    object([
        ("code", string("daemon")),
        ("severity", string("error")),
        ("message", string(message)),
    ])
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

fn collect_tests(root: &Path) -> Vec<PathBuf> {
    let mut tests = Vec::new();
    let tests_root = if root.is_dir() && root.join("tests").is_dir() {
        root.join("tests")
    } else {
        root.to_path_buf()
    };
    if let Ok(entries) = std::fs::read_dir(&tests_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                tests.extend(collect_tests(&path));
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("ya") {
                tests.push(path);
            }
        }
    }
    tests.sort();
    tests
}

/// A running daemon and the handle used to talk to (and stop) it.
pub struct DaemonHandle {
    addr: SocketAddr,
    root: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl DaemonHandle {
    /// Start a daemon bound to a free loopback port.
    pub fn start(root: &Path) -> io::Result<Self> {
        let toolchain = detect().map_err(io::Error::other)?;
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let root = root.to_path_buf();
        let thread_root = root.clone();
        let thread_stop = Arc::clone(&stop);
        // Publish the address so `watch` and the LSP can find this daemon. A
        // stale file (old compiler or protocol) is replaced, never trusted.
        let file = addr_file(&root);
        if let Some(parent) = file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let record = json_rpc::to_string(&object([
            ("protocol", number(i64::from(PROTOCOL_VERSION))),
            ("version", string(env!("CARGO_PKG_VERSION"))),
            ("addr", string(addr.to_string())),
        ]));
        let _ = std::fs::write(&file, record);

        let thread = thread::spawn(move || {
            let mut daemon = Daemon::new(&thread_root, toolchain);
            while !thread_stop.load(Ordering::Relaxed) {
                let stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(_) => break,
                };
                serve_connection(stream, &mut daemon, &thread_stop);
            }
        });
        Ok(DaemonHandle {
            addr,
            root,
            stop,
            thread: Some(thread),
        })
    }

    /// The loopback address clients should connect to.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Open a client connection to this daemon.
    pub fn client(&self) -> io::Result<Client> {
        Ok(Client {
            stream: TcpStream::connect(self.addr)?,
            next_id: 1,
        })
    }

    /// Ask the daemon to stop and join its server thread.
    pub fn shutdown(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Wake a blocked accept() so the thread can notice the flag.
        let _ = TcpStream::connect(self.addr);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_file(addr_file(&self.root));
    }
}

fn serve_connection(stream: TcpStream, daemon: &mut Daemon, stop: &AtomicBool) {
    let mut stream = stream;
    // A client that connects and then goes quiet must not keep the server
    // thread in read() forever: shutdown wakes accept() and sets the flag, and
    // this timeout is what lets the thread observe it.
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(1)));
    let mut reader = io::BufReader::new(match stream.try_clone() {
        Ok(clone) => clone,
        Err(_) => return,
    });
    loop {
        let message = match read_message(&mut reader) {
            Ok(Some(message)) => message,
            _ => return,
        };
        let id = message.get("id").and_then(Value::as_i64).unwrap_or(0);
        let request = match Request::from_json(&message) {
            Ok(request) => request,
            Err(error) => {
                let response = Response {
                    id,
                    result: Err(error),
                };
                let _ = write_message(&mut stream, &response.to_json());
                continue;
            }
        };
        let shutdown = request == Request::Shutdown;
        let result = daemon.handle(request);
        let response = Response {
            id,
            result: Ok(result),
        };
        if write_message(&mut stream, &response.to_json()).is_err() {
            return;
        }
        if shutdown {
            stop.store(true, Ordering::Relaxed);
            return;
        }
    }
}

/// A synchronous client for the daemon protocol.
pub struct Client {
    stream: TcpStream,
    next_id: i64,
}

impl Client {
    pub fn call(&mut self, request: Request) -> Result<ResponseKind, String> {
        let id = self.next_id;
        self.next_id += 1;
        write_message(&mut self.stream, &request.to_json(id))
            .map_err(|error| format!("cannot send request: {error}"))?;
        let mut reader = io::BufReader::new(
            self.stream
                .try_clone()
                .map_err(|error| format!("cannot clone stream: {error}"))?,
        );
        let message = read_message(&mut reader)
            .map_err(|error| format!("cannot read response: {error}"))?
            .ok_or_else(|| "the daemon closed the connection".to_string())?;
        Response::from_json(&message)?.result
    }
}
