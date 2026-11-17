//! `yashyc debug`: choose a native debugger, write its script, and run it.
//!
//! The script is generated from the same compiler facts the build used. It
//! sets `YASHY_DEBUG=1` so the runtime's task-aware stop handler is active and
//! breaks on named functions. Source-line breakpoints need LLVM debug metadata,
//! which the backend does not emit yet; this command does not pretend they
//! work. See `docs/debugging.md` for the honest limit.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

/// A native debugger found on `PATH`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Backend {
    Lldb(PathBuf),
    Cdb(PathBuf),
}

impl Backend {
    pub fn name(&self) -> &'static str {
        match self {
            Backend::Lldb(_) => "lldb",
            Backend::Cdb(_) => "cdb",
        }
    }

    pub fn executable(&self) -> &Path {
        match self {
            Backend::Lldb(path) | Backend::Cdb(path) => path,
        }
    }
}

fn on_path(names: &[&str]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        for name in names {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Prefer LLDB everywhere it is installed; on Windows fall back to `cdb`.
pub fn detect_backend() -> Result<Backend, String> {
    let lldb_names = if cfg!(windows) {
        &["lldb.exe", "lldb"][..]
    } else {
        &["lldb"][..]
    };
    if let Some(path) = on_path(lldb_names) {
        return Ok(Backend::Lldb(path));
    }
    if cfg!(windows) {
        if let Some(path) = on_path(&["cdb.exe", "cdb"]) {
            return Ok(Backend::Cdb(path));
        }
    }
    Err(if cfg!(windows) {
        "no debugger found: install LLDB or the Windows Debugging Tools (`cdb`), or add one to PATH"
    } else {
        "no debugger found: install LLDB and add it to PATH"
    }
    .to_string())
}

/// The LLDB command file. Values are quoted so a path with spaces works.
fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

pub fn script(
    backend: &Backend,
    program: &Path,
    breakpoints: &[String],
    args: &[String],
) -> String {
    match backend {
        Backend::Lldb(_) => {
            let mut out = format!(
                "settings set target.env-vars YASHY_DEBUG=1\ntarget create {}\n",
                quote(&program.to_string_lossy())
            );
            for name in breakpoints {
                out.push_str(&format!("breakpoint set --name {}\n", quote(name)));
            }
            if breakpoints.is_empty() {
                out.push_str("breakpoint set --name main\n");
            }
            out.push_str("breakpoint command add\nframe variable\nbt\ncontinue\nDONE\n");
            out.push_str("run");
            for argument in args {
                out.push(' ');
                out.push_str(&quote(argument));
            }
            out.push('\n');
            out
        }
        Backend::Cdb(_) => {
            let mut out = String::new();
            for name in breakpoints {
                out.push_str(&format!("bp {}\n", name));
            }
            if breakpoints.is_empty() {
                out.push_str("bp main\n");
            }
            out.push_str("g");
            for argument in args {
                out.push(' ');
                out.push_str(argument);
            }
            out.push('\n');
            out
        }
    }
}

/// Launch the backend with the generated script.
pub fn launch(backend: &Backend, script: &Path) -> std::io::Result<ExitStatus> {
    match backend {
        Backend::Lldb(path) => Command::new(path).arg("--source").arg(script).status(),
        Backend::Cdb(path) => Command::new(path).arg("-cf").arg(script).status(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_lldb_script_breaks_on_every_named_function() {
        let backend = Backend::Lldb(PathBuf::from("lldb"));
        let text = script(
            &backend,
            Path::new("app.exe"),
            &["main".to_string(), "work".to_string()],
            &["one".to_string(), "two words".to_string()],
        );
        assert!(text.contains("YASHY_DEBUG=1"), "{text}");
        assert!(text.contains("breakpoint set --name \"main\""), "{text}");
        assert!(text.contains("breakpoint set --name \"work\""), "{text}");
        assert!(text.contains("run \"one\" \"two words\""), "{text}");
        assert!(text.contains("frame variable"), "{text}");
    }

    #[test]
    fn a_cdb_script_uses_its_own_command_syntax() {
        let backend = Backend::Cdb(PathBuf::from("cdb.exe"));
        let text = script(&backend, Path::new("app.exe"), &[], &[]);
        assert!(text.contains("bp main"), "{text}");
        assert!(text.contains('g'), "{text}");
        assert!(!text.contains("breakpoint set"), "{text}");
    }

    #[test]
    fn a_default_breakpoint_is_the_entry_point() {
        let backend = Backend::Lldb(PathBuf::from("lldb"));
        let text = script(&backend, Path::new("app"), &[], &[]);
        assert!(text.contains("breakpoint set --name \"main\""), "{text}");
    }
}
