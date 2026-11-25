//! Differential testing across two compiler binaries.
//!
//! A corpus file is checked by both compilers. Exit status and normalised
//! output must agree. Paths and timings are replaced before comparison, so a
//! directory name or a slower machine is not a finding.

use std::path::Path;
use std::process::Command;

#[derive(Clone, Debug, PartialEq)]
pub struct Mismatch {
    pub file: String,
    pub left: String,
    pub right: String,
}

fn normalise(text: &str, file: &Path) -> String {
    let name = file.to_string_lossy();
    text.replace(name.as_ref(), "<file>")
        .lines()
        .filter(|line| !line.contains("\"timings\"") && !line.contains("\"duration_ms\""))
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

fn check(binary: &Path, file: &Path) -> (i32, String, String) {
    let output = Command::new(binary)
        .arg("check")
        .arg(file)
        .output()
        .expect("both compilers must be runnable");
    (
        output.status.code().unwrap_or(-1),
        normalise(&String::from_utf8_lossy(&output.stdout), file),
        normalise(&String::from_utf8_lossy(&output.stderr), file),
    )
}

/// Compare every corpus file under two compilers. An empty vector is green.
pub fn run(corpus: &[std::path::PathBuf], left: &Path, right: &Path) -> Vec<Mismatch> {
    let mut out = Vec::new();
    for file in corpus {
        let a = check(left, file);
        let b = check(right, file);
        if a != b {
            out.push(Mismatch {
                file: file.to_string_lossy().to_string(),
                left: format!("{}|{}|{}", a.0, a.1, a.2),
                right: format!("{}|{}|{}", b.0, b.1, b.2),
            });
        }
    }
    out
}
