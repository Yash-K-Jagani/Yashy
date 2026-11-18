//! Conservative impacted-test selection.
//!
//! A test is selected when it changed itself, when it imports a module whose
//! file changed, or when it names a type, trait, macro or `cfg` key that a
//! changed file declares. The last rule is the conservative widening: a test
//! can reach a symbol through another module, so a file-level import walk
//! alone would miss it. A dependency the selector cannot see widens the
//! selection instead of dropping a test. A selection that skips a test whose
//! dependency changed is a bug, not a speed win.

use std::path::{Path, PathBuf};

/// Which tests run, and why the others were skipped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub selected: Vec<String>,
    pub skipped: Vec<(String, String)>,
    /// A reason for every selected test, in the same order as `selected`.
    pub reasons: Vec<(String, String)>,
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

/// The declaration keywords whose name another module can mention.
const DECL_KEYWORDS: &[&str] = &[
    "pub struct ",
    "struct ",
    "pub enum ",
    "enum ",
    "pub trait ",
    "trait ",
    "pub object ",
    "object ",
    "pub type ",
    "type ",
    "pub macro ",
    "macro ",
];

/// Type, trait, macro and alias names a file declares.
///
/// A change to one of these can affect a test that reaches the name through a
/// different module, so every mention is a selection signal.
pub fn declared_symbols(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        for keyword in DECL_KEYWORDS {
            let Some(rest) = line.strip_prefix(keyword) else {
                continue;
            };
            let name: String = rest
                .chars()
                .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
                .collect();
            if !name.is_empty() && !names.contains(&name) {
                names.push(name);
            }
            break;
        }
    }
    names
}

/// The `cfg` keys a file mentions, in source order.
///
/// A key's meaning is decided by the build's target triple, so changing one
/// can change which items exist for every test that asks for that key.
pub fn cfg_keys(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let bytes = text.as_bytes();
    let mut index = 0;
    while index + 4 <= bytes.len() {
        if &bytes[index..index + 4] != b"cfg(" {
            index += 1;
            continue;
        }
        let start = index + 4;
        let mut end = start;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            end += 1;
        }
        let key = &text[start..end];
        if !key.is_empty() && !names.contains(&key.to_string()) {
            names.push(key.to_string());
        }
        index = end;
    }
    names
}

/// A changed file's path is relative to `root` in CI, but tests also pass an
/// absolute path. Read whichever exists so both spellings widen the same way.
fn read_changed(root: &Path, path: &Path) -> String {
    let relative = root.join(path);
    std::fs::read_to_string(&relative)
        .or_else(|_| std::fs::read_to_string(path))
        .unwrap_or_default()
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
            reasons: Vec::new(),
        };
    }
    let unknown: Vec<String> = changed
        .iter()
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) != Some("ya"))
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .collect();

    // Every declared name and cfg key in a changed module is a selection
    // signal. They are gathered before the test walk so a test can be
    // selected by a symbol even when it imports the module through a chain.
    let mut signals: Vec<String> = Vec::new();
    for path in changed
        .iter()
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("ya"))
    {
        let text = read_changed(root, path);
        for name in declared_symbols(&text).into_iter().chain(cfg_keys(&text)) {
            if !signals.contains(&name) {
                signals.push(name);
            }
        }
    }

    let mut selected = Vec::new();
    let mut skipped = Vec::new();
    let mut reasons = Vec::new();
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
        let named = signals.iter().find(|signal| text.contains(*signal));
        if !unknown.is_empty() || touched || named.is_some() {
            let reason = if !unknown.is_empty() {
                format!("changed file {} is not a Yashy module", unknown[0])
            } else if touched {
                "changed module is imported by this test".to_string()
            } else {
                format!("names changed symbol `{}`", named.unwrap())
            };
            selected.push(name.clone());
            reasons.push((name, reason));
        } else {
            skipped.push((
                name,
                "no changed module is imported by this test".to_string(),
            ));
        }
    }
    Selection {
        selected,
        skipped,
        reasons,
    }
}

/// The JSON form of a selection, for `test --affected --json`.
pub fn to_json(selection: &Selection) -> yashy_common::Json {
    use yashy_common::Json;
    let selected: Vec<Json> = selection.selected.iter().map(|s| Json::string(s)).collect();
    let reasons: Vec<Json> = selection
        .reasons
        .iter()
        .map(|(name, reason)| {
            let mut object = Json::object();
            object.set("name", Json::string(name));
            object.set("reason", Json::string(reason));
            object
        })
        .collect();
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
    report.set("reasons", Json::array(reasons));
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
    fn declared_symbols_include_types_traits_and_macros() {
        let text = "pub struct Order { total: int }\ntrait Show {}\nmacro_rules! pair {}\n";
        let names = declared_symbols(text);
        assert!(names.contains(&"Order".to_string()));
        assert!(names.contains(&"Show".to_string()));
    }

    #[test]
    fn cfg_keys_are_read_from_the_key_position() {
        assert_eq!(
            cfg_keys("cfg(windows) fn a() {}\ncfg(unix) fn b() {}\n"),
            vec!["windows".to_string(), "unix".to_string()]
        );
    }

    #[test]
    fn a_changed_type_selects_a_test_that_names_it_without_importing() {
        let dir = std::env::temp_dir().join("yashy-affected-symbol-test");
        let _ = std::fs::create_dir_all(&dir);
        let module = dir.join("model.ya");
        let test = dir.join("uses_model.ya");
        std::fs::write(&module, "pub struct Order { total: int }\n").unwrap();
        std::fs::write(&test, "fn main() { let o = Order { total: 1 }; }\n").unwrap();
        let selection = select(&dir, &[test], &[module]);
        assert_eq!(selection.selected, vec!["uses_model.ya".to_string()]);
        assert!(selection.reasons[0].1.contains("Order"));
    }

    #[test]
    fn a_changed_cfg_key_selects_a_test_that_asks_for_it() {
        let dir = std::env::temp_dir().join("yashy-affected-cfg-test");
        let _ = std::fs::create_dir_all(&dir);
        let module = dir.join("platform.ya");
        let test = dir.join("platform_test.ya");
        std::fs::write(
            &module,
            "cfg(windows) fn platform() -> string { return \"win\"; }\n",
        )
        .unwrap();
        std::fs::write(&test, "cfg(windows) fn main() {}\n").unwrap();
        let selection = select(&dir, &[test], &[module]);
        assert_eq!(selection.selected, vec!["platform_test.ya".to_string()]);
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
        assert_eq!(selection.reasons.len(), 1);
    }
}
