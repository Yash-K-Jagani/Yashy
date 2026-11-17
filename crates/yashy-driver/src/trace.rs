//! `yashyc trace`: the generated rule-to-code-to-test matrix.
//!
//! Rule ids live in `codes::rule_for`; codes live in the catalog; the tests are
//! the files that mention a code. Nothing here is hand-maintained, so an
//! orphan rule or code is visible as data rather than a missing spreadsheet.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use yashy_common::{codes, Json};

/// One row of the matrix: a rule, the codes it names and the tests that
/// mention those codes.
#[derive(Clone, Debug)]
pub struct RuleRow {
    pub rule: String,
    pub codes: Vec<String>,
    pub tests: Vec<String>,
}

fn collect_rust_tests(root: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
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
            collect_rust_tests(&path, files);
        } else if name.ends_with(".rs")
            && path
                .parent()
                .is_some_and(|parent| parent.ends_with("tests"))
        {
            files.push(path);
        }
    }
    files.sort();
}

/// Build the matrix for a workspace root.
pub fn matrix(root: &Path) -> Result<Vec<RuleRow>, String> {
    let mut files = Vec::new();
    collect_rust_tests(&root.join("crates"), &mut files);
    let mut contents = Vec::new();
    for path in &files {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        contents.push((path.clone(), text));
    }
    let mut by_rule: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut rule_for_code: BTreeMap<String, String> = BTreeMap::new();
    for entry in codes::CODES {
        let rule = codes::rule_for(entry.id);
        if rule.is_empty() {
            return Err(format!("{} has no rule id", entry.id));
        }
        rule_for_code.insert(entry.id.to_string(), rule.to_string());
        by_rule
            .entry(rule.to_string())
            .or_default()
            .push(entry.id.to_string());
    }
    let mut rows = Vec::new();
    for (rule, entries) in by_rule {
        let mut tests = Vec::new();
        for (path, text) in &contents {
            if entries.iter().any(|code| text.contains(code)) {
                tests.push(path.to_string_lossy().replace('\\', "/"));
            }
        }
        rows.push(RuleRow {
            rule,
            codes: entries,
            tests,
        });
    }
    Ok(rows)
}

pub fn to_json(rows: &[RuleRow]) -> Json {
    let mut entries = Vec::new();
    let mut orphans = Vec::new();
    for row in rows {
        let mut object = Json::object();
        object.set("rule", Json::string(&row.rule));
        object.set(
            "codes",
            Json::array(row.codes.iter().map(|code| Json::string(code)).collect()),
        );
        object.set(
            "tests",
            Json::array(row.tests.iter().map(|test| Json::string(test)).collect()),
        );
        if row.tests.is_empty() {
            orphans.push(row.rule.clone());
        }
        entries.push(object);
    }
    let mut report = Json::object();
    report.set("schema_version", Json::number(1));
    report.set("tool", Json::string("trace"));
    report.set(
        "note",
        Json::string("runtime --events tracing is not implemented; this is the static rule matrix"),
    );
    report.set("rules", Json::array(entries));
    report.set(
        "orphan_rules",
        Json::array(orphans.iter().map(|rule| Json::string(rule)).collect()),
    );
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_has_a_rule_and_every_rule_has_a_row() {
        let rows = matrix(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .as_path(),
        )
        .expect("the matrix must build");
        let rules: Vec<&str> = rows.iter().map(|row| row.rule.as_str()).collect();
        let expected: std::collections::BTreeSet<&str> = codes::CODES
            .iter()
            .map(|entry| codes::rule_for(entry.id))
            .collect();
        for rule in expected {
            assert!(rules.contains(&rule), "no row for rule {rule}");
        }
        let codes_in_rows: usize = rows.iter().map(|row| row.codes.len()).sum();
        assert_eq!(codes_in_rows, codes::CODES.len());
    }
}
