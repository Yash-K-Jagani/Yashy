//! Impact analysis: fan a set of changed symbols out to the work that must be
//! re-run. The inputs are the same compiler facts review uses, so an impact
//! report cannot name a test that the graph does not know about.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use yashy_common::json_rpc::{self, Value};
use yashy_common::Json;

use crate::index;
use crate::review::{self, Review};

/// The work to re-run after a change.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Impact {
    pub symbols: Vec<String>,
    pub tests: Vec<String>,
    pub packages: Vec<String>,
    pub examples: Vec<String>,
    pub docs: Vec<String>,
}

impl Impact {
    pub fn to_json(&self) -> Json {
        let array =
            |items: &[String]| Json::array(items.iter().map(|item| Json::string(item)).collect());
        let mut root = Json::object();
        root.set("schema_version", Json::number(1));
        root.set("tool", Json::string("yashyc"));
        root.set("symbols", array(&self.symbols));
        root.set("tests", array(&self.tests));
        root.set("packages", array(&self.packages));
        root.set("examples", array(&self.examples));
        root.set("docs", array(&self.docs));
        root
    }

    pub fn render_human(&self) -> String {
        let mut out = format!(
            "Impact: {} symbols, {} tests, {} packages, {} examples, {} docs\n",
            self.symbols.len(),
            self.tests.len(),
            self.packages.len(),
            self.examples.len(),
            self.docs.len()
        );
        for (label, items) in [
            ("tests", &self.tests),
            ("packages", &self.packages),
            ("examples", &self.examples),
            ("docs", &self.docs),
        ] {
            out.push_str(&format!("{label}: {}\n", items.join(", ")));
        }
        out
    }
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn baseline_test_files(baseline_text: &str) -> Vec<String> {
    let Ok(root) = json_rpc::parse(baseline_text) else {
        return Vec::new();
    };
    root.get("tests")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("file").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn collect_text_files(root: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if path.is_dir() {
            if name == "target" || name == ".git" || name.starts_with('.') {
                continue;
            }
            collect_text_files(&path, out);
        } else {
            let extension = path.extension().and_then(|ext| ext.to_str());
            if extension == Some("ya") || extension == Some("md") {
                out.push(path);
            }
        }
    }
}

fn has_component(path: &Path, component: &str) -> bool {
    path.components().any(|part| part.as_os_str() == component)
}

/// Compute the impact of `review` on the workspace at `root`.
pub fn impact(root: &Path, baseline_text: &str, review: &Review) -> Result<Impact, String> {
    let current = index::build_index(root)?;
    let symbols = review::changed_symbols(review);
    let old_tests: BTreeSet<String> = baseline_test_files(baseline_text).into_iter().collect();
    let mut tests: BTreeSet<String> = BTreeSet::new();
    for test in &current.tests {
        if test.calls.iter().any(|call| symbols.contains(call))
            || symbols.iter().any(|name| {
                current
                    .symbols
                    .iter()
                    .any(|symbol| symbol.name == *name && symbol.file == test.file)
            })
        {
            tests.insert(test.file.clone());
        }
    }
    // A test that disappeared still has to be re-created somewhere; name it.
    for file in &old_tests {
        if !current.tests.iter().any(|test| test.file == *file) {
            tests.insert(file.clone());
        }
    }

    let mut packages = Vec::new();
    if let Ok(text) = std::fs::read_to_string(root.join("yashy.toml")) {
        if let Ok(manifest) = yashy_manifest::Manifest::parse(&text) {
            packages.push(manifest.package.name);
            if let Some(workspace) = manifest.workspace {
                for member in workspace.members {
                    let path = root.join(&member);
                    if path.join("yashy.toml").exists() {
                        let name = Path::new(&member)
                            .components()
                            .next_back()
                            .map(|part| part.as_os_str().to_string_lossy().into_owned())
                            .unwrap_or(member);
                        packages.push(name);
                    }
                }
            }
        }
    }
    packages.sort();
    packages.dedup();

    let mut files = Vec::new();
    collect_text_files(root, &mut files);
    files.sort();
    let mut examples = Vec::new();
    let mut docs = Vec::new();
    if symbols.is_empty() {
        return Ok(Impact {
            symbols,
            tests: tests.into_iter().collect(),
            packages,
            examples,
            docs,
        });
    }
    for path in files {
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let wanted = (has_component(&path, "examples") && relative.ends_with(".ya"))
            || (has_component(&path, "docs") && relative.ends_with(".md"));
        if !wanted {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        if symbols.iter().any(|name| text.contains(name)) {
            if has_component(&path, "examples") {
                examples.push(relative);
            } else {
                docs.push(relative);
            }
        }
    }

    Ok(Impact {
        symbols,
        tests: tests.into_iter().collect(),
        packages,
        examples,
        docs,
    })
}
