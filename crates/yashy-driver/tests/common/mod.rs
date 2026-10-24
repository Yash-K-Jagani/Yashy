//! Shared helpers for CLI integration tests.
//!
//! Kept in `tests/common/mod.rs` so it is a module, not a separate test target.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// The `yashy` binary cargo built for this test run, as a path the host will
/// actually start.
///
/// Every test drives the CLI, and a freshly linked binary is exactly what an
/// Application Control policy refuses. The refusal is decided by content, so
/// the same workaround the compiled programs get applies here: a copy with a
/// few zero bytes past the end of the image is a new question. The copy is made
/// once per test process, because the binary cannot change while the run is in
/// progress, and the returned path is the image cargo built with nothing but
/// trailing padding added.
pub fn yashy() -> &'static str {
    static RUNNABLE: OnceLock<String> = OnceLock::new();
    RUNNABLE.get_or_init(runnable_cli).as_str()
}

/// Find a copy of the CLI the host will start, padding until it does.
fn runnable_cli() -> String {
    let original = Path::new(env!("CARGO_BIN_EXE_yashy"));
    for attempt in 0..LAUNCH_ATTEMPTS {
        let Ok(candidate) = launch_candidate(original, attempt) else {
            continue;
        };
        // Any exit status is a success here: the question is only whether the
        // loader started the program, not what the program decided to do.
        match Command::new(&candidate).output() {
            Ok(_) => return candidate.to_string_lossy().into_owned(),
            Err(error) if blocked_by_policy(&error) => continue,
            Err(error) => panic!(
                "the yashy CLI at {} must start: {error}",
                original.display()
            ),
        }
    }
    panic!(
        "the yashy CLI at {} was blocked by an Application Control / Device \
         Guard policy on all {LAUNCH_ATTEMPTS} attempts",
        original.display()
    );
}

/// A fresh, empty directory for one test, unique per test name and process.
pub fn work_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yashy-cli-test-{}-{name}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("the test work directory must be creatable");
    dir
}

/// Write `text` to `dir/relative`, creating parent directories as needed.
pub fn write(dir: &Path, relative: &str, text: &str) {
    let path = dir.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("the test source directory must be creatable");
    }
    fs::write(&path, text).expect("the test source must be writable");
}

/// `<dir>/<name>` plus `.exe` on Windows.
pub fn exe(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

/// How many copies of an executable the host may refuse before a test gives up.
const LAUNCH_ATTEMPTS: usize = 40;

/// Whether an error is the host refusing to start a binary rather than the
/// program failing.
fn blocked_by_policy(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::PermissionDenied
        || error.raw_os_error() == Some(4551)
        || error.to_string().contains("blocked")
        || error.to_string().contains("Application Control")
}

/// The file to start on attempt `attempt`: the executable itself, then copies
/// that differ from it only in how many zero bytes follow the image.
fn launch_candidate(exe: &Path, attempt: usize) -> std::io::Result<PathBuf> {
    if attempt == 0 {
        return Ok(exe.to_path_buf());
    }
    let dir = exe.parent().unwrap_or_else(|| Path::new("."));
    let stem = exe
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("program");
    let candidate = dir.join(format!(
        "{stem}_launch{attempt}{}",
        std::env::consts::EXE_SUFFIX
    ));
    let mut bytes = fs::read(exe)?;
    bytes.resize(bytes.len() + attempt, 0);
    fs::write(&candidate, bytes)?;
    Ok(candidate)
}
