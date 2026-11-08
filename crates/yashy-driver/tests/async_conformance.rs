//! Async conformance: every program under `tests/programs/async` runs under
//! both executor modes and must print identical output.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn programs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("programs")
        .join("async")
}

fn program_names() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(programs_dir())
        .expect("the async programs directory must be readable")
        .filter_map(|entry| {
            let path = entry.expect("a directory entry must be readable").path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("ya") {
                path.file_stem()
                    .and_then(|stem| stem.to_str())
                    .map(str::to_string)
            } else {
                None
            }
        })
        .collect();
    names.sort();
    names
}

fn build(name: &str) -> PathBuf {
    let dir = common::work_dir(&format!("async-{name}"));
    let source = programs_dir().join(format!("{name}.ya"));
    let exe = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    let compile = Command::new(common::yashy())
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("the yashy CLI must be runnable");
    assert!(
        compile.status.success(),
        "compiling {name}.ya failed:\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    exe
}

fn run_with_threads(exe: &Path, threads: Option<&str>) -> Output {
    let mut child = common::launch_retrying(exe, |path| {
        let mut command = Command::new(path);
        command.env("YASHY_LEAK_REPORT", "1");
        if let Some(threads) = threads {
            command.env("YASHY_EXECUTOR_THREADS", threads);
        } else {
            command.env_remove("YASHY_EXECUTOR_THREADS");
        }
        command.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()
    })
    .expect("the built executable must start");
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return child
                    .wait_with_output()
                    .expect("the built executable must be waitable");
            }
            Ok(None) => {
                assert!(
                    started.elapsed() < Duration::from_secs(60),
                    "{} did not terminate within 60 seconds",
                    exe.display()
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("waiting for {} failed: {error}", exe.display()),
        }
    }
}

#[test]
fn every_async_program_agrees_across_both_executor_modes() {
    // Warm the shared CLI copy once: `common::yashy` hands one padded copy
    // to every thread, and its `OnceLock` makes that safe, after which the
    // per-program builds below run in parallel on their own directories.
    let _ = common::yashy();
    let names = program_names();
    assert_eq!(names.len(), 30, "the conformance suite has 30 programs");
    let failures: Vec<String> = std::thread::scope(|scope| {
        let handles: Vec<_> = names
            .iter()
            .map(|name| {
                scope.spawn(move || -> Result<(), String> {
                    let exe = build(name);
                    let single = run_with_threads(&exe, None);
                    let pool = run_with_threads(&exe, Some("4"));
                    for (mode, output) in [("single-threaded", &single), ("pool", &pool)] {
                        if !output.status.success() {
                            return Err(format!(
                                "{name} ({mode}) exited with {:?}; stderr:\n{}",
                                output.status.code(),
                                String::from_utf8_lossy(&output.stderr)
                            ));
                        }
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        if !stderr.contains("yashy: leak report: 0 live allocations") {
                            return Err(format!("{name} ({mode}) leaked:\n{stderr}"));
                        }
                    }
                    if single.stdout != pool.stdout {
                        return Err(format!(
                            "{name} differs between executor modes: {:?} vs {:?}",
                            String::from_utf8_lossy(&single.stdout),
                            String::from_utf8_lossy(&pool.stdout)
                        ));
                    }
                    Ok(())
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|handle| match handle.join() {
                Ok(Ok(())) => None,
                Ok(Err(message)) => Some(message),
                Err(_) => Some("a conformance thread panicked".to_string()),
            })
            .collect()
    });
    assert!(failures.is_empty(), "conformance failures: {failures:?}");
}
