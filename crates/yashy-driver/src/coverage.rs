//! Source-based coverage: parse lcov records and render one report.
//!
//! The driver asks clang for coverage mapping, runs the instrumented binary,
//! then asks `llvm-profdata` and `llvm-cov` for lcov records. This module owns
//! the shape those records are rendered into: one `Json` value for the CLI and
//! agents, and the lcov text itself for a human who wants to feed another
//! tool. Nothing here decides which lines are interesting; a line that ran
//! zero times is still in the report, because "never ran" is the whole point.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use yashy_common::Json;

/// One source file's line counts, keyed by one-based line number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileCoverage {
    pub path: String,
    pub lines: BTreeMap<u64, u64>,
}

/// Parse an lcov record stream into per-file line counts.
///
/// Only `SF`, `DA` and `end_of_record` matter here: function, branch and
/// checksum records carry facts the current report does not use, and a record
/// the parser does not understand must not corrupt the lines around it.
pub fn parse_lcov(text: &str) -> Vec<FileCoverage> {
    let mut files = Vec::new();
    let mut current: Option<FileCoverage> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(path) = line.strip_prefix("SF:") {
            if let Some(file) = current.take() {
                files.push(file);
            }
            current = Some(FileCoverage {
                path: path.to_string(),
                lines: BTreeMap::new(),
            });
        } else if let Some(record) = line.strip_prefix("DA:") {
            let mut parts = record.split(',');
            if let (Some(line), Some(count)) = (parts.next(), parts.next()) {
                if let (Ok(line), Ok(count)) = (line.parse::<u64>(), count.parse::<u64>()) {
                    if let Some(file) = &mut current {
                        file.lines.insert(line, count);
                    }
                }
            }
        } else if line == "end_of_record" {
            if let Some(file) = current.take() {
                files.push(file);
            }
        }
    }
    if let Some(file) = current {
        files.push(file);
    }
    files
}

/// `(covered, total)` lines across every file in the report.
pub fn summary(files: &[FileCoverage]) -> (u64, u64) {
    let covered = files
        .iter()
        .flat_map(|file| file.lines.values())
        .filter(|count| **count > 0)
        .count() as u64;
    let total = files.iter().map(|file| file.lines.len()).sum::<usize>() as u64;
    (covered, total)
}

/// The schema-v1 report. A file with no lines is still listed: it tells the
/// reader which source the instrumented binary actually mapped.
pub fn to_json(files: &[FileCoverage], source: &Path) -> Json {
    let entries: Vec<Json> = files
        .iter()
        .map(|file| {
            let lines: Vec<Json> = file
                .lines
                .iter()
                .map(|(line, count)| {
                    let mut entry = Json::object();
                    entry.set("line", Json::number(*line as i64));
                    entry.set("count", Json::number(*count as i64));
                    entry
                })
                .collect();
            let mut entry = Json::object();
            entry.set("path", Json::string(file.path.as_str()));
            entry.set("lines", Json::array(lines));
            entry
        })
        .collect();
    let (covered, total) = summary(files);
    let percent = if total == 0 {
        0.0
    } else {
        covered as f64 * 100.0 / total as f64
    };
    let mut report = Json::object();
    report.set("schema_version", Json::number(1));
    report.set("tool", Json::string("coverage"));
    report.set(
        "source",
        Json::string(source.to_string_lossy().into_owned()),
    );
    report.set("files", Json::array(entries));
    report.set("covered_lines", Json::number(covered as i64));
    report.set("total_lines", Json::number(total as i64));
    report.set("percent", Json::number(percent.round() as i64));
    report
}

/// A SARIF 2.1.0 log whose results are the uncovered lines.
///
/// Coverage is not a diagnostic, but CI needs to upload it as an artifact the
/// same way it uploads compiler diagnostics. Each never-executed line becomes
/// one `COVERAGE-1` result with the line as its physical location; the rule is
/// stable so a dashboard can count it across releases.
pub fn to_sarif(files: &[FileCoverage]) -> Json {
    let results: Vec<Json> = files
        .iter()
        .flat_map(|file| {
            file.lines
                .iter()
                .filter(|(_, count)| **count == 0)
                .map(move |(line, _)| {
                    let mut result = Json::object();
                    result.set("ruleId", Json::string("COVERAGE-1"));
                    result.set("level", Json::string("warning"));
                    let mut message = Json::object();
                    message.set(
                        "text",
                        Json::string(format!("line {line} was never executed")),
                    );
                    result.set("message", message);
                    let mut region = Json::object();
                    region.set("startLine", Json::number(*line as i64));
                    let mut artifact = Json::object();
                    artifact.set("uri", Json::string(file.path.as_str()));
                    let mut physical = Json::object();
                    physical.set("artifactLocation", artifact);
                    physical.set("region", region);
                    let mut location = Json::object();
                    location.set("physicalLocation", physical);
                    result.set("locations", Json::array(vec![location]));
                    result
                })
        })
        .collect();

    let mut rule = Json::object();
    rule.set("id", Json::string("COVERAGE-1"));
    let mut description = Json::object();
    description.set("text", Json::string("this line was never executed"));
    rule.set("shortDescription", description);
    let mut tool_driver = Json::object();
    tool_driver.set("name", Json::string("yashyc coverage"));
    tool_driver.set("rules", Json::array(vec![rule]));
    let mut tool = Json::object();
    tool.set("driver", tool_driver);
    let mut run = Json::object();
    run.set("tool", tool);
    run.set("results", Json::array(results));
    let mut log = Json::object();
    log.set("version", Json::string("2.1.0"));
    log.set(
        "$schema",
        Json::string("https://json.schemastore.org/sarif-2.1.0.json"),
    );
    log.set("runs", Json::array(vec![run]));
    log
}

/// Locate an LLVM helper, preferring the one beside the clang doing the build.
pub fn find_tool(name: &str, clang: &Path, env_name: &str) -> Option<PathBuf> {
    if let Ok(path) = std::env::var(env_name) {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    if let Some(parent) = clang.parent() {
        for candidate in [parent.join(name), parent.join(format!("{name}.exe"))] {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    if Command::new(name)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
    {
        return Some(PathBuf::from(name));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lcov_records_become_line_counts_in_file_order() {
        let files = parse_lcov(
            "SF:main.ya\nDA:1,1\nDA:2,0\nDA:3,7\nend_of_record\nSF:lib.ya\nDA:4,1\nend_of_record\n",
        );
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "main.ya");
        assert_eq!(files[0].lines.get(&2), Some(&0));
        assert_eq!(files[1].lines.get(&4), Some(&1));
        assert_eq!(summary(&files), (3, 4));
    }

    #[test]
    fn uncovered_lines_become_sarif_results() {
        let files = parse_lcov("SF:main.ya\nDA:1,1\nDA:2,0\nDA:3,0\nend_of_record\n");
        let rendered = to_sarif(&files).render();
        assert!(rendered.contains("\"version\": \"2.1.0\""));
        assert!(rendered.contains("COVERAGE-1"));
        assert!(rendered.contains("line 2 was never executed"));
        assert!(rendered.contains("line 3 was never executed"));
        assert!(!rendered.contains("line 1 was never executed"));
    }
}
