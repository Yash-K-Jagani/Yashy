//! Finding the LLVM toolchain that turns our IR text into native code.

use std::env;
use std::ffi::OsStr;
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
    ///
    /// The field is named for `llc` because that is the preferred tool, but on
    /// a machine that ships only `clang` it holds the `clang` path. Read
    /// [`Toolchain::found`] to know which binary this actually is.
    pub llc: PathBuf,

    /// Which candidate was selected: `"llc"` or `"clang"`.
    ///
    /// Callers must not infer this from `llc`'s file name: `OTTER_LLC` may
    /// legitimately point at something called anything at all.
    pub found: &'static str,
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
            Some(path) if path.is_file() => {
                return Ok(Toolchain { llc: path, found: name });
            }
            Some(path) => {
                tried.push(format!("{name} at {} (not an existing file)", path.display()));
            }
            None => tried.push(format!("{name} (no candidate found)")),
        }
    }

    Err(format!(
        "no LLVM toolchain found: Otter needs `llc` or `clang` to turn IR into a binary. \
         Set {LLC_ENV_VAR} to the executable, or put it on PATH. {} Tried: {}",
        install_hint(),
        tried.join("; ")
    ))
}

/// How to get an LLVM toolchain on this platform.
///
/// A "not found" message that does not say what to install leaves the user to
/// guess, which is the difference between a two-minute fix and giving up.
fn install_hint() -> &'static str {
    if cfg!(windows) {
        "Install it with `winget install LLVM.LLVM`."
    } else if cfg!(target_os = "macos") {
        "Install it with `brew install llvm`."
    } else {
        "Install it with your package manager, e.g. `apt install llvm clang`."
    }
}

/// The real probe: `OTTER_LLC` first for `llc`, then every directory on `PATH`.
///
/// Both inputs are threaded in as parameters so tests can drive the precedence
/// rules without touching the process environment.
fn probe_environment(name: &str) -> Option<PathBuf> {
    let env_value = env::var_os(LLC_ENV_VAR);
    probe_environment_with(name, env_value.as_deref(), &which)
}

/// The body of [`probe_environment`], with its two environment reads injected.
///
/// A `OTTER_LLC` that does not point at a real file is ignored rather than
/// fatal: the search continues with `which(name)` for the *same* candidate, so
/// a stale override degrades to the `PATH` lookup instead of demoting `llc` to
/// `clang` without saying so.
fn probe_environment_with(
    name: &str,
    env_value: Option<&OsStr>,
    which: &dyn Fn(&str) -> Option<PathBuf>,
) -> Option<PathBuf> {
    if name == "llc" {
        if let Some(from_env) = llc_from_env_value(env_value) {
            if from_env.is_file() {
                return Some(from_env);
            }
        }
    }
    which(name)
}

/// Interprets an `OTTER_LLC` value, which may name either the executable or its
/// directory.
///
/// An empty or whitespace-only value is treated as unset, so `OTTER_LLC=` does
/// not silently break the lookup. Returns `None` when there is nothing to use;
/// whether the result actually exists is the caller's decision.
fn llc_from_env_value(raw: Option<&OsStr>) -> Option<PathBuf> {
    let raw = raw?;
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

    /// A path guaranteed to exist and to be a *file*, so these tests never
    /// depend on what happens to be installed on the machine.
    ///
    /// The existence check is not paranoia for its own sake: a stand-in path
    /// that silently stops existing turns every ordering assertion into a
    /// no-op, because `detect_with` would reject the phantom and fall through
    /// to the other candidate anyway.
    fn an_existing_file() -> PathBuf {
        existing_fixture("Cargo.toml")
    }

    /// A second, distinct existing file, so a per-name table can tell the two
    /// candidates apart.
    fn another_existing_file() -> PathBuf {
        existing_fixture("src/main.rs")
    }

    fn existing_fixture(relative: &str) -> PathBuf {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
        assert!(
            path.is_file(),
            "test fixture {} must exist, or this test proves nothing",
            path.display()
        );
        path
    }

    /// A path guaranteed not to exist.
    fn a_missing_file() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("no-such-file-otter-test")
    }

    #[test]
    fn llc_wins_when_both_candidates_resolve() {
        // Distinct existing paths per name, and the assertion names the exact
        // path, so reversing CANDIDATES makes this fail rather than pass.
        let llc = an_existing_file();
        let clang = another_existing_file();
        assert_ne!(llc, clang, "the two stand-ins must be distinguishable");

        let toolchain = detect_with(|name| match name {
            "llc" => Some(llc.clone()),
            "clang" => Some(clang.clone()),
            other => panic!("unexpected candidate probed: {other}"),
        });

        let toolchain = match toolchain {
            Ok(toolchain) => toolchain,
            Err(message) => panic!("both candidates resolve, so this must succeed: {message}"),
        };
        assert_eq!(toolchain.llc, llc, "llc must win over clang");
        assert_eq!(toolchain.found, "llc");
    }

    #[test]
    fn clang_is_used_when_llc_is_absent() {
        let clang = an_existing_file();

        let toolchain = detect_with(|name| match name {
            "clang" => Some(clang.clone()),
            _ => None,
        });

        let toolchain = match toolchain {
            Ok(toolchain) => toolchain,
            Err(message) => panic!("clang resolves, so this must succeed: {message}"),
        };
        assert_eq!(toolchain.llc, clang);
        assert_eq!(toolchain.found, "clang");
    }

    #[test]
    fn a_candidate_that_is_not_a_file_is_rejected() {
        // Same shape as the brief's integration test: the probe answers, but
        // the answer is a path to nothing.
        let result = detect_with(|name| match name {
            "llc" => Some(a_missing_file()),
            _ => None,
        });
        assert!(result.is_err(), "a nonexistent path must not be accepted");
    }

    #[test]
    fn error_names_every_candidate() {
        let err = match detect_with(|_name| None) {
            Ok(toolchain) => panic!("no candidate resolves, yet found {:?}", toolchain.found),
            Err(message) => message,
        };
        assert!(err.contains("llc"), "{err}");
        assert!(err.contains("clang"), "{err}");
    }

    #[test]
    fn env_override_wins_over_the_path_lookup() {
        let from_env = an_existing_file();
        let from_path = another_existing_file();
        let which_never_called = |_name: &str| -> Option<PathBuf> { None };

        let probe = probe_environment_with("llc", Some(from_env.as_os_str()), &which_never_called);
        assert_eq!(probe, Some(from_env.clone()));

        // The same override must be ignored for clang: it names llc only.
        let probe = probe_environment_with("clang", Some(from_path.as_os_str()), &which_never_called);
        assert_eq!(probe, None, "OTTER_LLC must not hijack the clang lookup");
    }

    #[test]
    fn a_stale_env_override_falls_back_to_the_path_lookup_for_llc() {
        // The bug this pins: a bad OTTER_LLC used to make the search move on to
        // the *next candidate name*, silently handing back clang.
        let path_llc = an_existing_file();
        let which_returns_llc = |name: &str| match name {
            "llc" => Some(path_llc.clone()),
            _ => None,
        };

        let probe = probe_environment_with("llc", Some(a_missing_file().as_os_str()), &which_returns_llc);
        assert_eq!(probe, Some(path_llc.clone()), "must fall back to llc on PATH");

        let toolchain = match detect_with(|name| probe_environment_with(
            name,
            Some(a_missing_file().as_os_str()),
            &which_returns_llc,
        )) {
            Ok(toolchain) => toolchain,
            Err(message) => panic!("PATH still has an llc: {message}"),
        };
        assert_eq!(toolchain.llc, path_llc, "a stale override must not demote llc to clang");
        assert_eq!(toolchain.found, "llc");
    }

    #[test]
    fn an_unusable_env_override_still_allows_clang() {
        let clang = an_existing_file();
        let which_returns_clang = |name: &str| match name {
            "clang" => Some(clang.clone()),
            _ => None,
        };

        let probe = probe_environment_with(
            "clang",
            Some(a_missing_file().as_os_str()),
            &which_returns_clang,
        );
        assert_eq!(probe, Some(clang));
    }

    #[test]
    fn env_value_may_name_a_directory_or_an_executable() {
        assert_eq!(llc_from_env_value(None), None);
        assert_eq!(llc_from_env_value(Some(OsStr::new("   "))), None);

        let exe = an_existing_file();
        assert_eq!(llc_from_env_value(Some(exe.as_os_str())), Some(exe.clone()));

        // A directory gets the platform's executable name appended.
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let expected = dir.join(exe_name("llc"));
        assert_eq!(llc_from_env_value(Some(dir.as_os_str())), Some(expected));
    }
}
