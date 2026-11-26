//! Conservative impacted-test selection.
//!
//! A test is selected when it changed itself, when it imports a module whose
//! file changed, or when it names a type, trait, macro or `cfg` key that a
//! changed file declares. The last rule is the conservative widening: a test
//! can reach a symbol through another module, so a file-level import walk
//! alone would miss it. A dependency the selector cannot see widens the
//! selection instead of dropping a test. A selection that skips a test whose
//! dependency changed is a bug, not a speed win.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::index::collect_files_from;

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

/// Function names a file declares, including methods and generic functions.
///
/// Functions are selection signals too: a test that calls a changed function by
/// name must not be selected only by accident when it imports some other module.
fn declared_functions(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in text.lines() {
        let mut rest = line.trim();
        loop {
            let stripped = rest
                .strip_prefix("pub ")
                .or_else(|| rest.strip_prefix("async "))
                .or_else(|| rest.strip_prefix("const "));
            match stripped {
                Some(next) => rest = next,
                None => break,
            }
        }
        let Some(rest) = rest.strip_prefix("fn ") else {
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
            .collect();
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// The names a file calls, read from `name(` in source order.
///
/// This is deliberately textual: the selector runs before (and sometimes
/// instead of) a full check, so it must work on a file that does not compile.
/// A name after `.` is a method, not a module-level function, and keywords that
/// take a parenthesised head are not calls.
fn called_names(text: &str) -> Vec<String> {
    const KEYWORDS: &[&str] = &[
        "if", "while", "for", "switch", "case", "default", "return", "defer", "async", "let",
        "const", "fn", "pub", "struct", "enum", "trait", "impl", "object", "import", "cfg",
        "unsafe", "macro", "match", "matches", "move", "is", "and", "or", "not", "in", "else",
    ];
    let bytes = text.as_bytes();
    let mut names: Vec<String> = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' {
            let start = index;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            let name = &text[start..index];
            if KEYWORDS.contains(&name) || (start > 0 && bytes[start - 1] == b'.') {
                continue;
            }
            let mut after = index;
            while after < bytes.len() && bytes[after].is_ascii_whitespace() {
                after += 1;
            }
            if after < bytes.len() && bytes[after] == b'(' && !names.iter().any(|n| n == name) {
                names.push(name.to_string());
            }
        } else {
            index += 1;
        }
    }
    names
}

/// The call graph the selector needs: one file per declared function, plus
/// import edges between files. A name declared in more than one file names all
/// of them, so an ambiguous call widens rather than guesses.
struct CallGraph {
    declarations: BTreeMap<String, Vec<PathBuf>>,
    modules: BTreeMap<String, Vec<PathBuf>>,
    calls: BTreeMap<PathBuf, Vec<String>>,
    imports: BTreeMap<PathBuf, Vec<String>>,
}

impl CallGraph {
    fn build(root: &Path) -> Self {
        let mut files = Vec::new();
        collect_files_from(root, &mut files);
        let mut graph = Self {
            declarations: BTreeMap::new(),
            modules: BTreeMap::new(),
            calls: BTreeMap::new(),
            imports: BTreeMap::new(),
        };
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap_or_default();
            for name in declared_functions(&text) {
                graph
                    .declarations
                    .entry(name)
                    .or_default()
                    .push(file.clone());
            }
            let stem = file
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("")
                .to_string();
            if !stem.is_empty() {
                graph.modules.entry(stem).or_default().push(file.clone());
            }
            graph.calls.insert(file.clone(), called_names(&text));
            graph.imports.insert(file.clone(), imports(&text));
        }
        graph
    }

    /// Every file reachable from `start` through calls and imports.
    fn reachable(&self, start: &Path) -> BTreeSet<PathBuf> {
        let mut seen = BTreeSet::new();
        let mut queue = VecDeque::from([start.to_path_buf()]);
        while let Some(file) = queue.pop_front() {
            if !seen.insert(file.clone()) {
                continue;
            }
            for name in self.calls.get(&file).into_iter().flatten() {
                for target in self.declarations.get(name).into_iter().flatten() {
                    if !seen.contains(target) {
                        queue.push_back(target.clone());
                    }
                }
            }
            for name in self.imports.get(&file).into_iter().flatten() {
                for target in self.modules.get(name).into_iter().flatten() {
                    if !seen.contains(target) {
                        queue.push_back(target.clone());
                    }
                }
            }
        }
        seen
    }
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
        for name in declared_symbols(&text)
            .into_iter()
            .chain(declared_functions(&text))
            .chain(cfg_keys(&text))
        {
            if !signals.contains(&name) {
                signals.push(name);
            }
        }
    }

    // Source-level call and import edges are the last widening rule. A test can
    // reach a changed function through a chain of modules without naming it and
    // without importing the changed file, so a name-only walk would miss it.
    let graph = CallGraph::build(root);
    let normalize = |path: &Path| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        }
    };
    let changed_paths: Vec<PathBuf> = changed
        .iter()
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("ya"))
        .map(|path| normalize(path))
        .collect();

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
        let reachable = graph.reachable(&normalize(test));
        let reached = changed_paths.iter().find(|path| reachable.contains(*path));
        if !unknown.is_empty() || touched || named.is_some() || reached.is_some() {
            let reason = if !unknown.is_empty() {
                format!("changed file {} is not a Yashy module", unknown[0])
            } else if touched {
                "changed module is imported by this test".to_string()
            } else if let Some(signal) = named {
                format!("names changed symbol `{signal}`")
            } else {
                let file = reached
                    .unwrap()
                    .strip_prefix(root)
                    .unwrap_or(reached.unwrap())
                    .to_string_lossy()
                    .replace('\\', "/");
                format!("call graph reaches changed module `{file}`")
            };
            selected.push(name.clone());
            reasons.push((name, reason));
        } else {
            skipped.push((
                name,
                "no changed module is imported or called by this test".to_string(),
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

    #[test]
    fn a_call_chain_reaches_a_changed_function_in_another_file() {
        let dir = std::env::temp_dir().join("yashy-affected-call-graph-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let leaf = dir.join("leaf.ya");
        let middle = dir.join("middle.ya");
        let test = dir.join("test.ya");
        std::fs::write(&leaf, "pub fn leaf() -> int { return 1; }\n").unwrap();
        std::fs::write(&middle, "pub fn middle() -> int { return leaf(); }\n").unwrap();
        std::fs::write(&test, "fn main() { assert(middle() == 1); }\n").unwrap();
        let selection = select(&dir, &[test], &[leaf]);
        assert_eq!(selection.selected, vec!["test.ya".to_string()]);
        assert!(
            selection.reasons[0]
                .1
                .contains("call graph reaches changed module"),
            "{:?}",
            selection.reasons
        );
    }

    #[test]
    fn an_unreachable_call_chain_does_not_select_a_test() {
        let dir = std::env::temp_dir().join("yashy-affected-call-graph-negative");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let leaf = dir.join("leaf.ya");
        let other = dir.join("other.ya");
        let test = dir.join("test.ya");
        std::fs::write(&leaf, "pub fn leaf() -> int { return 1; }\n").unwrap();
        std::fs::write(&other, "pub fn other() -> int { return 2; }\n").unwrap();
        std::fs::write(&test, "fn main() { assert(other() == 2); }\n").unwrap();
        let selection = select(&dir, &[test], &[leaf]);
        assert!(selection.selected.is_empty(), "{:?}", selection.reasons);
        assert_eq!(selection.skipped.len(), 1);
    }
}
