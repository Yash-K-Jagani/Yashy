//! `yashyc watch`: re-check on change and print diagnostics.
//!
//! The watcher is a thin loop over the same checker the CLI uses. It polls
//! file metadata (portable, dependency-free) rather than depending on an OS
//! notification API, and debounces by waiting before it re-reads a changed
//! file. It is deliberately separate from the compiler: a watcher that dies
//! never takes a build with it.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use yashy_common::json_rpc::Value;
use yashy_common::SourceMap;
use yashy_verify::{ContractMode, VerifyOptions};

use crate::pipeline::check_and_verify;

const VERIFY_OPTIONS: VerifyOptions = VerifyOptions {
    contracts: ContractMode::Enforce,
};

/// What happened since the last poll.
#[derive(Clone, Debug, PartialEq)]
pub enum WatchEvent {
    Changed(PathBuf),
    Diagnostics {
        file: PathBuf,
        diagnostics: Vec<Value>,
    },
    Success {
        file: PathBuf,
    },
    Error(String),
}

impl WatchEvent {
    pub fn to_json(&self) -> Value {
        use yashy_common::json_rpc::{object, string};
        match self {
            WatchEvent::Changed(path) => object([
                ("event", string("changed")),
                ("file", string(path.to_string_lossy().replace('\\', "/"))),
            ]),
            WatchEvent::Diagnostics { file, diagnostics } => object([
                ("event", string("diagnostics")),
                ("file", string(file.to_string_lossy().replace('\\', "/"))),
                ("diagnostics", Value::Arr(diagnostics.clone())),
            ]),
            WatchEvent::Success { file } => object([
                ("event", string("success")),
                ("file", string(file.to_string_lossy().replace('\\', "/"))),
            ]),
            WatchEvent::Error(message) => {
                object([("event", string("error")), ("message", string(message))])
            }
        }
    }
}

/// A file's observed state. Length and modified time together are enough to
/// notice an edit without reading the file on every poll.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
}

fn stamp(path: &Path) -> Option<Stamp> {
    let metadata = fs::metadata(path).ok()?;
    Some(Stamp {
        len: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

/// Polls a tree (or one file) and checks whatever changed.
pub struct Watcher {
    root: PathBuf,
    /// When set, only this file is watched.
    only: Option<PathBuf>,
    seen: HashMap<PathBuf, Stamp>,
}

impl Watcher {
    pub fn new(root: &Path, only: Option<&Path>) -> io::Result<Self> {
        let mut watcher = Watcher {
            root: root.to_path_buf(),
            only: only.map(Path::to_path_buf),
            seen: HashMap::new(),
        };
        watcher.refresh();
        Ok(watcher)
    }

    fn paths(&self) -> Vec<PathBuf> {
        if let Some(file) = &self.only {
            return vec![file.clone()];
        }
        let mut files = Vec::new();
        collect(&self.root, &mut files);
        files
    }

    fn refresh(&mut self) {
        self.seen.clear();
        for path in self.paths() {
            if let Some(stamp) = stamp(&path) {
                self.seen.insert(path, stamp);
            }
        }
    }

    /// Check every file that changed since the last poll.
    pub fn poll(&mut self) -> Vec<WatchEvent> {
        let mut events = Vec::new();
        let paths = self.paths();
        // Drop files that have been deleted.
        self.seen.retain(|path, _| paths.contains(path));
        for path in paths {
            let now = stamp(&path);
            if now.is_some() && self.seen.get(&path) == now.as_ref() {
                continue;
            }
            self.seen.insert(
                path.clone(),
                now.unwrap_or(Stamp {
                    len: 0,
                    modified: None,
                }),
            );
            events.push(WatchEvent::Changed(path.clone()));
            let diagnostics = check(&path);
            match diagnostics {
                Ok(list) if list.is_empty() => {
                    events.push(WatchEvent::Success { file: path });
                }
                Ok(list) => events.push(WatchEvent::Diagnostics {
                    file: path,
                    diagnostics: list,
                }),
                Err(error) => events.push(WatchEvent::Error(error)),
            }
        }
        events
    }
}

fn collect(root: &Path, files: &mut Vec<PathBuf>) {
    if root.is_file() {
        files.push(root.to_path_buf());
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "target" {
            continue;
        }
        if path.is_dir() {
            collect(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("ya") {
            files.push(path);
        }
    }
    files.sort();
}

fn check(path: &Path) -> Result<Vec<Value>, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let mut sources = SourceMap::new();
    let display = path.to_string_lossy().replace('\\', "/");
    let outcome = check_and_verify(
        &mut sources,
        Path::new(&display),
        source,
        false,
        &VERIFY_OPTIONS,
        0,
    );
    let rendered = match outcome {
        Ok((_, report)) => crate::diagnostics_json(&sources, &report.diagnostics),
        Err(list) => crate::diagnostics_json(&sources, &list),
    };
    Ok(rendered
        .iter()
        .map(|value| {
            yashy_common::json_rpc::parse(&value.render())
                .unwrap_or(yashy_common::json_rpc::Value::Null)
        })
        .collect())
}

/// Options for the long-running CLI loop.
pub struct WatchOptions {
    pub root: PathBuf,
    pub only: Option<PathBuf>,
    pub json: bool,
    pub run: bool,
    pub test: bool,
    pub interval_ms: u64,
}

/// Run the watch loop until the process is stopped.
pub fn watch(options: WatchOptions) -> io::Result<()> {
    let mut watcher = Watcher::new(&options.root, options.only.as_deref())?;
    let interval = Duration::from_millis(options.interval_ms);
    // The last successfully built program is the one kept running. A failed
    // edit leaves this child alone: a server must stay up while the user fixes
    // the source, which is the point of watch --run.
    let mut running: Option<std::process::Child> = None;
    loop {
        std::thread::sleep(interval);
        for event in watcher.poll() {
            if options.json {
                println!("{}", yashy_common::json_rpc::to_string(&event.to_json()));
            } else {
                print_event(&event);
            }
            match &event {
                WatchEvent::Success { file } => {
                    if options.run {
                        if let Some(mut child) = running.take() {
                            let _ = child.kill();
                            let _ = child.wait();
                        }
                        running = run_program(file);
                    }
                    if options.test {
                        run_tests(&options.root);
                    }
                }
                WatchEvent::Diagnostics { .. } | WatchEvent::Error(_) => {
                    // Keep `running` untouched; report the failure and go on.
                }
                WatchEvent::Changed(_) => {}
            }
        }
    }
}

/// Build and start the program, returning the live child. The caller owns it:
/// a short-lived program is reaped by waiting, and a server keeps running
/// until a later successful edit replaces it.
fn run_program(file: &Path) -> Option<std::process::Child> {
    let out = std::env::temp_dir().join(format!(
        "yashy-watch-{}-{}.bin",
        std::process::id(),
        file.file_stem()?.to_string_lossy()
    ));
    let source = match fs::read_to_string(file) {
        Ok(source) => source,
        Err(_) => return None,
    };
    let toolchain = match crate::detect() {
        Ok(toolchain) => toolchain,
        Err(_) => return None,
    };
    if crate::compile_to_binary(
        &source,
        &file.to_string_lossy(),
        &out,
        &toolchain,
        crate::BuildOptions::default(),
    )
    .is_err()
    {
        return None;
    }
    std::process::Command::new(&out).spawn().ok()
}

fn run_tests(root: &Path) {
    let toolchain = match crate::detect() {
        Ok(toolchain) => toolchain,
        Err(_) => return,
    };
    let mut tests = Vec::new();
    collect(&root.join("tests"), &mut tests);
    let report = crate::test_harness::run_suite(
        &tests,
        &toolchain,
        yashy_codegen::host_target_triple(),
        0,
        &[],
    );
    eprintln!(
        "watch: tests: {} passed, {} failed, {} compile errors",
        report.passed, report.failed, report.compile_errors
    );
}

/// Print one event.
///
/// A terminal gets a cleared screen before the new report, so a long watch
/// session reads as a live view; a pipe gets no escape and one line per
/// diagnostic, so a log file or a CI job sees exactly what happened, in order.
fn print_event(event: &WatchEvent) {
    let clear = crate::term::clear_screen();
    match event {
        WatchEvent::Changed(path) => println!("{clear}changed {}", path.display()),
        WatchEvent::Diagnostics { file, diagnostics } => {
            if clear.is_empty() {
                for diagnostic in diagnostics {
                    let message = diagnostic
                        .get("message")
                        .and_then(|value| value.as_str())
                        .unwrap_or("diagnostic");
                    let code = diagnostic
                        .get("code")
                        .and_then(|value| value.as_str())
                        .unwrap_or("error");
                    println!("{}: {}: {message}", file.display(), code);
                }
            } else {
                println!(
                    "{clear}{}: {} diagnostic(s)",
                    file.display(),
                    diagnostics.len()
                );
                for diagnostic in diagnostics {
                    if let Some(message) =
                        diagnostic.get("message").and_then(|value| value.as_str())
                    {
                        println!("  {message}");
                    }
                }
            }
        }
        WatchEvent::Success { file } => println!("{clear}{}: ok", file.display()),
        WatchEvent::Error(message) => eprintln!("watch: {message}"),
    }
}
