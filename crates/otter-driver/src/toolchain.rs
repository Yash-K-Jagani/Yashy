//! Finding the LLVM toolchain that turns our IR text into native code.

use std::env;
use std::path::{Path, PathBuf};

/// Environment variable that overrides the `llc` lookup entirely.
pub const LLC_ENV_VAR: &str = "OTTER_LLC";

/// The binaries we know how to drive, in preference order.
///
/// `clang` is accepted because `clang -x ir` does the same job as `llc`.
const CANDIDATES: [&str; 2] = ["llc", "clang"];

/// A located LLVM toolchain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toolchain {
    /// Path to the `llc`-or-`clang` executable to invoke.
    pub llc: PathBuf,
}

/// Locates the LLVM toolchain, honouring `OTTER_LLC` and then `PATH`.
///
/// The error message names every location that was tried, so a user with no
/// LLVM installed is told what to install rather than just "not found".
pub fn detect() -> Result<Toolchain, String> {
    detect_with(probe_environment)
}

/// `detect`, with the "where could this binary be?" question handed to `probe`.
///
/// The probe is generic rather than `&dyn Fn` so that a closure literal can be
/// passed straight in; `&closure` satisfies the same bound. Tests drive this
/// with a probe that answers from a table instead of the real filesystem, so
/// they never depend on what is installed on the machine running them. A
/// candidate path is only accepted if it really is a file: a probe that hands
/// back a stale or bogus path still produces the failure path rather than a
/// toolchain that cannot be run.
pub fn detect_with<F>(probe: F) -> Result<Toolchain, String>
where
    F: Fn(&str) -> Option<PathBuf>,
{
    let mut tried: Vec<String> = Vec::with_capacity(CANDIDATES.len());

    for name in CANDIDATES {
        match probe(name) {
            Some(path) if path.is_file() => return Ok(Toolchain { llc: path }),
            Some(path) => {
                tried.push(format!("{name} at {} (not an existing file)", path.display()));
            }
            None => tried.push(format!("{name} (no candidate found)")),
        }
    }

    Err(format!(
        "no LLVM toolchain found: Otter needs `llc` or `clang` to turn IR into a binary. \
         Set {LLC_ENV_VAR} to the executable, or put it on PATH. Tried: {}",
        tried.join("; ")
    ))
}

/// The real probe: `OTTER_LLC` first for `llc`, then every directory on `PATH`.
fn probe_environment(name: &str) -> Option<PathBuf> {
    if name == "llc" {
        if let Some(from_env) = llc_from_env() {
            return Some(from_env);
        }
    }
    which(name)
}

/// Reads `OTTER_LLC`, which may name either the executable or its directory.
///
/// An empty or whitespace-only value is treated as unset, so
/// `OTTER_LLC=` does not silently break the lookup.
fn llc_from_env() -> Option<PathBuf> {
    let raw = env::var_os(LLC_ENV_VAR)?;
    let trimmed = raw.to_string_lossy().trim().to_string();
    if trimmed.is_empty() {
        return None;
    }
    let path = PathBuf::from(trimmed);
    Some(if path.is_dir() { path.join(exe_name("llc")) } else { path })
}

/// Looks `name` up in every directory on `PATH`.
///
/// `PATH` is environment-derived, so a malformed or absent `PATH` yields
/// `None` rather than a panic.
fn which(name: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    env::split_paths(&path_var)
        .map(|dir| dir.join(exe_name(name)))
        .find(|candidate| is_usable(candidate))
}

/// The file name to look for: `llc` on Unix, `llc.exe` on Windows.
fn exe_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn is_usable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_probe_that_finds_a_real_file_wins() {
        // Any existing file stands in for llc: detect_with only asks "is this
        // runnable?", and the probe test must not depend on a real install.
        let Ok(exe) = env::current_exe() else { return };
        let outcome = detect_with(|_name| Some(exe.clone()));
        assert!(outcome.is_ok(), "an existing file must be accepted");
        let Ok(toolchain) = outcome else { return };
        assert!(toolchain.llc.is_file());
    }

    #[test]
    fn clang_is_used_when_llc_is_absent() {
        let probe = |name: &str| match name {
            "clang" => env::current_exe().ok(),
            _ => None,
        };
        if let Ok(exe) = env::current_exe() {
            assert!(exe.is_file());
            assert!(detect_with(probe).is_ok());
        }
    }

    #[test]
    fn error_names_every_candidate() {
        let err = detect_with(|_name| None).unwrap_err();
        assert!(err.contains("llc"), "{err}");
        assert!(err.contains("clang"), "{err}");
    }
}
