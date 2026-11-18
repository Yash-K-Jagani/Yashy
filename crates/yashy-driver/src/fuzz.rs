//! Fuzz corpus inventory.
//!
//! A finding is a file, and a seed is a fact. This module lists the corpus and
//! the findings the repository keeps so a reader can see what the fuzzer has
//! already learned without running it. The target itself is a plain Yashy
//! function in `packages/fuzz`; no attributes and no separate build system.

use std::path::{Path, PathBuf};
use yashy_common::Json;

/// One corpus or finding file, with its size in bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub bytes: u64,
}

/// List files under `root` whose extension is `.bin` or has no extension.
///
/// A directory is walked recursively; paths are relative and forward-slash so
/// the listing is byte-stable across operating systems.
pub fn list(root: &Path) -> Result<Vec<Entry>, String> {
    fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
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
                walk(&path, files);
            } else {
                let extension = path.extension().and_then(|value| value.to_str());
                if extension.is_none() || extension == Some("bin") {
                    files.push(path);
                }
            }
        }
        files.sort();
    }

    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    walk(root, &mut files);
    let mut entries = Vec::new();
    for path in files {
        let bytes = std::fs::metadata(&path)
            .map(|meta| meta.len())
            .map_err(|error| format!("cannot stat {}: {error}", path.display()))?;
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        entries.push(Entry {
            path: relative,
            bytes,
        });
    }
    Ok(entries)
}

/// The same entries in a schema-v1 envelope.
pub fn to_json(entries: &[Entry]) -> Json {
    let rows: Vec<Json> = entries
        .iter()
        .map(|entry| {
            let mut object = Json::object();
            object.set("path", Json::string(&entry.path));
            object.set("bytes", Json::number(entry.bytes as i64));
            object
        })
        .collect();
    let mut report = Json::object();
    report.set("schema_version", Json::number(1));
    report.set("tool", Json::string("fuzz"));
    report.set("yashy_version", Json::string(env!("CARGO_PKG_VERSION")));
    report.set("entries", Json::array(rows));
    report
}
