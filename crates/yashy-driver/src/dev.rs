//! `yashyc dev`: rebuild and restart a program when its sources change.
//!
//! The loop reuses the CLI's watcher, so it registers directories and notices
//! changes to any `.ya` file under the root, not just `main.ya`. It uses the
//! same `build` command the shell would use, so there is no second compiler
//! path. A failed generation leaves the last working process alone.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use yashy_common::json_rpc::{number, object, string, to_string, Value};

use crate::watch::{WatchEvent, Watcher};

pub struct DevOptions {
    pub root: PathBuf,
    pub only: Option<PathBuf>,
    pub json: bool,
    pub interval_ms: u64,
}

/// Run the development loop until the process is stopped.
pub fn dev(options: DevOptions) -> io::Result<()> {
    let mut watcher = Watcher::new(&options.root, options.only.as_deref())?;
    let interval = Duration::from_millis(options.interval_ms);
    let mut running: Option<Child> = None;
    let mut generation = 0u64;

    // Generation 1 is the initial build, so a caller always knows the loop is
    // alive before it makes its first edit.
    generation += 1;
    let entry = options.only.clone().unwrap_or_else(|| options.root.join("main.ya"));
    emit(
        &options,
        generation,
        &[entry.to_string_lossy().to_string()],
        "restarted",
        0,
        build_and_run(&entry, &mut running),
    );

    loop {
        std::thread::sleep(interval);
        let events = watcher.poll();
        if events.is_empty() {
            continue;
        }
        generation += 1;
        let started = Instant::now();
        let mut changed = Vec::new();
        let mut diagnostics = 0;
        let mut entry = entry.clone();
        for event in events {
            match event {
                WatchEvent::Changed(path) => changed.push(path.to_string_lossy().to_string()),
                WatchEvent::Diagnostics { .. } | WatchEvent::Error(_) => diagnostics += 1,
                WatchEvent::Success { file } => entry = file,
            }
        }
        let status = if diagnostics > 0 { "diagnostics" } else { "restarted" };
        let live = if diagnostics == 0 {
            build_and_run(&entry, &mut running)
        } else {
            running.is_some()
        };
        emit(
            &options,
            generation,
            &changed,
            status,
            started.elapsed().as_millis() as i64,
            live,
        );
    }
}

fn emit(options: &DevOptions, generation: u64, changed: &[String], status: &str, build_ms: i64, live: bool) {
    let fields = [
        ("event", string("generation")),
        ("generation", number(generation as i64)),
        (
            "changed",
            Value::Arr(changed.iter().map(|path| string(path.as_str())).collect()),
        ),
        ("status", string(status)),
        ("build_ms", number(build_ms)),
        ("running", Value::Bool(live)),
    ];
    if options.json {
        println!("{}", to_string(&object(fields)));
    } else {
        println!(
            "generation {generation}: {status} ({} file(s), {build_ms} ms)",
            changed.len()
        );
    }
}

/// Build through the CLI's own `build` command, then replace the old process.
/// The old process is killed and waited for before the new one starts, so at
/// most one copy is alive at a time. A hung old process is therefore bounded
/// by the kill, not by its own shutdown path; socket inheritance is not
/// attempted because it is not portable across the two target OSes.
fn build_and_run(source: &Path, running: &mut Option<Child>) -> bool {
    let Some(file) = source.file_name() else {
        return false;
    };
    let out = std::env::temp_dir().join(format!(
        "yashy-dev-{}-{}.bin",
        std::process::id(),
        file.to_string_lossy()
    ));
    let compiler = match std::env::current_exe() {
        Ok(path) => path,
        Err(_) => return false,
    };
    let build = Command::new(compiler)
        .arg("build")
        .arg(source)
        .arg("-o")
        .arg(&out)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if !build.map(|status| status.success()).unwrap_or(false) {
        return false;
    }
    if let Some(mut child) = running.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    match Command::new(&out).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        Ok(child) => {
            *running = Some(child);
            true
        }
        Err(_) => false,
    }
}

/// Remove the build artifact when the loop ends; the process is killed by the
/// caller, so this is a best-effort helper for tests and wrappers.
pub fn cleanup(source: &Path) {
    if let Some(file) = source.file_name() {
        let _ = fs::remove_file(std::env::temp_dir().join(format!(
            "yashy-dev-{}-{}.bin",
            std::process::id(),
            file.to_string_lossy()
        )));
    }
}
