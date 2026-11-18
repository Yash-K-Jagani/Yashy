//! Conservative impacted-test selection.
//!
//! A test is selected when it changed itself, or when it imports a module
//! whose file changed. This is deliberately file-level: Yashy has no call
//! graph in the CLI yet, so a dependency the selector cannot see widens the
//! selection instead of dropping a test. A selection that skips a test whose
//! dependency changed is a bug, not a speed win.

use std::path::{Path, PathBuf};

/// Which tests run, and why the others were skipped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub selected: Vec<String>,
    pub skipped: Vec<(String, String)>,
}

/// The module names a test imports, in source order.
fn imports(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("import ") else {
            continue;
        };
        let name = rest
            .trim()
            .trim_end_matches(';')
            .split('.')
            .last()
            .unwrap_or("")
            .to_string();
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// Select tests for a set of changed files.
///
/// `root` makes the reported names relative and forward-slash, so a run on
/// Windows and Linux produces the same JSON. An unknown changed file selects
/// every test with a stated reason.
pub fn select(root: &Path, tests: &[PathBuf], changed: &[PathBuf]) -> Selection {
    if changed.is_empty() {
        return Selection {
            selected: Vec::new(),
            skipped: Vec::new(),
        };
    }
    let unknown: Vec<String> = changed
        .iter()
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) != Some("ya"))
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .collect();

    let mut selected = Vec::new();
    let mut skipped = Vec::new();
    for test in tests {
        let name = test
            .strip_prefix(root)
            .unwrap_or(test)
            .to_string_lossy()
            .replace('\\', "/");
        let text = std::fs::read_to_string(test).unwrap_or_default();
        let names = imports(&text);
        let touched = changed.iter().any(|path| {
            let stem = path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            name.contains(&path.to_string_lossy().replace('\\', "/"))
                || (!stem.is_empty() && names.iter().any(|module| module == stem))
        });
        if !unknown.is_empty() || touched {
            selected.push(name);
        } else {
            skipped.push((
                name,
                "no changed module is imported by this test".to_string(),
            ));
        }
    }
    Selection { selected, skipped }
}

/// The JSON form of a selection, for `test --affected --json`.
pub fn to_json(selection: &Selection) -> yashy_common::Json {
    use yashy_common::Json;
    let selected: Vec<Json> = selection.selected.iter().map(|s| Json::string(s)).collect();
    let skipped: Vec<Json> = selection
        .skipped
        .iter()
        .map(|(name, reason)| {
            let mut object = Json::object();
            object.set("name", Json::string(name));
            object.set("reason", Json::string(reason));
            object
        })
        .collect();
    let mut report = Json::object();
    report.set("selected", Json::array(selected));
    report.set("skipped", Json::array(skipped));
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_are_read_from_plain_lines() {
        assert_eq!(
            imports("import io;\n// import fake;\nimport a.b;\n"),
            vec!["io".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn an_unknown_changed_file_selects_every_test() {
        let dir = std::env::temp_dir().join("yashy-affected-test");
        let _ = std::fs::create_dir_all(&dir);
        let test = dir.join("one.ya");
        std::fs::write(&test, "import io;\nfn main() {}\n").unwrap();
        let selection = select(&dir, &[test], &[PathBuf::from("build.rs")]);
        assert_eq!(selection.selected, vec!["one.ya".to_string()]);
        assert!(selection.skipped.is_empty());
    }
}
