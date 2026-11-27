//! Versioned documentation trees.
//!
//! `yashyc doc --versioned` writes the compiler's own public-surface model into
//! `docs/versioned/<version>/`, leaving other version trees untouched. A
//! `current` pointer names the default version, and a redirect map records
//! pages that moved between versions. Nothing here invents a page from prose:
//! every tree comes from [`yashy_driver::doc`], the same model `yashyc doc`
//! renders for the current workspace.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use yashy_common::Json;

use crate::doc::DocModel;

/// What one generation wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub version: String,
    pub modules: usize,
    pub directory: PathBuf,
    pub moved: Vec<(String, String)>,
}

impl Report {
    pub fn to_json(&self) -> Json {
        let mut root = Json::object();
        root.set("schema_version", Json::number(1));
        root.set("tool", Json::string("yashyc"));
        root.set("version", Json::string(&self.version));
        root.set("modules", Json::number(self.modules as i64));
        root.set(
            "directory",
            Json::string(self.directory.to_string_lossy().into_owned()),
        );
        root.set(
            "moved",
            Json::array(
                self.moved
                    .iter()
                    .map(|(from, to)| {
                        let mut entry = Json::object();
                        entry.set("from", Json::string(from));
                        entry.set("to", Json::string(to));
                        entry
                    })
                    .collect(),
            ),
        );
        root
    }
}

fn version_dir(root: &Path, version: &str) -> PathBuf {
    root.join("docs").join("versioned").join(version)
}

/// Write one version tree and update the `current` pointer.
pub fn write(root: &Path, version: &str, model: &DocModel) -> Result<Report, String> {
    if version.is_empty() || version.contains('/') || version.contains('\\') || version == ".." {
        return Err(format!("`{version}` is not a version directory name"));
    }
    let directory = version_dir(root, version);
    model
        .write_to(&directory)
        .map_err(|error| format!("{}: {error}", directory.display()))?;
    let modules = model.modules.len();
    let mut manifest = Json::object();
    manifest.set("schema_version", Json::number(1));
    manifest.set("tool", Json::string("yashyc"));
    manifest.set("version", Json::string(version));
    manifest.set("yashy_version", Json::string(env!("CARGO_PKG_VERSION")));
    manifest.set("modules", Json::number(modules as i64));
    std::fs::write(directory.join("manifest.json"), manifest.render())
        .map_err(|error| format!("{}: {error}", directory.join("manifest.json").display()))?;

    let versions_root = root.join("docs").join("versioned");
    std::fs::create_dir_all(&versions_root)
        .map_err(|error| format!("{}: {error}", versions_root.display()))?;
    std::fs::write(versions_root.join("current"), format!("{version}\n"))
        .map_err(|error| format!("{}: {error}", versions_root.join("current").display()))?;

    let moved = read_redirects(&versions_root);
    let mut redirects = Json::object();
    redirects.set("schema_version", Json::number(1));
    redirects.set(
        "redirects",
        Json::array(
            moved
                .iter()
                .map(|(from, to)| {
                    let mut entry = Json::object();
                    entry.set("from", Json::string(from));
                    entry.set("to", Json::string(to));
                    entry
                })
                .collect(),
        ),
    );
    std::fs::write(versions_root.join("redirects.json"), redirects.render()).map_err(|error| {
        format!(
            "{}: {error}",
            versions_root.join("redirects.json").display()
        )
    })?;

    Ok(Report {
        version: version.to_string(),
        modules,
        directory,
        moved,
    })
}

/// The version a workspace currently points at, or `None` before the first
/// versioned generation.
pub fn current(root: &Path) -> Option<String> {
    std::fs::read_to_string(root.join("docs").join("versioned").join("current"))
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// Read the redirect map, if it exists. A page that moved from `from` to `to`
/// is answered by the docs server with the destination rather than a 404.
pub fn read_redirects(root: &Path) -> Vec<(String, String)> {
    let path = root.join("docs").join("versioned").join("redirects.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(value) = yashy_common::json_rpc::parse(&text) else {
        return Vec::new();
    };
    let Some(items) = value
        .get("redirects")
        .and_then(yashy_common::json_rpc::Value::as_array)
    else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let from = item.get("from")?.as_str()?.to_string();
            let to = item.get("to")?.as_str()?.to_string();
            Some((from, to))
        })
        .collect()
}

/// Record a moved page. The `from` spelling is the old path; `to` is the new
/// path inside the version tree.
pub fn redirect(root: &Path, from: &str, to: &str) -> Result<(), String> {
    let mut moved = read_redirects(root);
    if moved.iter().any(|(old, _)| old == from) {
        return Ok(());
    }
    moved.push((from.to_string(), to.to_string()));
    moved.sort();
    let versions_root = root.join("docs").join("versioned");
    std::fs::create_dir_all(&versions_root)
        .map_err(|error| format!("{}: {error}", versions_root.display()))?;
    let mut redirects = Json::object();
    redirects.set("schema_version", Json::number(1));
    redirects.set(
        "redirects",
        Json::array(
            moved
                .iter()
                .map(|(from, to)| {
                    let mut entry = Json::object();
                    entry.set("from", Json::string(from));
                    entry.set("to", Json::string(to));
                    entry
                })
                .collect(),
        ),
    );
    std::fs::write(versions_root.join("redirects.json"), redirects.render()).map_err(|error| {
        format!(
            "{}: {error}",
            versions_root.join("redirects.json").display()
        )
    })
}

/// All version directories present, sorted by name.
pub fn versions(root: &Path) -> Vec<String> {
    let directory = root.join("docs").join("versioned");
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(directory) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    out.push(name.to_string());
                }
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_coexist_and_current_points_at_the_last_written() {
        let dir = std::env::temp_dir().join(format!("yashy-versioned-docs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let model = DocModel::default();
        write(&dir, "1.0.0", &model).expect("the first version");
        write(&dir, "2.0.0", &model).expect("the second version");
        assert_eq!(versions(&dir), vec!["1.0.0", "2.0.0"]);
        assert_eq!(current(&dir).as_deref(), Some("2.0.0"));
        assert!(version_dir(&dir, "1.0.0").join("manifest.json").is_file());
    }

    #[test]
    fn a_bad_version_name_is_refused() {
        let dir = std::env::temp_dir().join("yashy-versioned-docs-bad");
        let model = DocModel::default();
        assert!(write(&dir, "../escape", &model).is_err());
    }
}
