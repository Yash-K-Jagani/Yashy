//! `yashyc trace`: the generated rule-to-code-to-test matrix.
//!
//! Rule ids live in `codes::rule_for`; codes live in the catalog; the tests are
//! the files that mention a code. Nothing here is hand-maintained, so an
//! orphan rule or code is visible as data rather than a missing spreadsheet.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

/// One runtime event, before and after symbolisation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub seq: u64,
    pub kind: &'static str,
    pub size: i64,
    pub pc: String,
    pub span: String,
}

/// Parse the runtime's flat event file.
///
/// The runtime writes `seq kind size pc` lines in table order and one trailing
/// `# dropped N` line when the bounded table overflowed. Parsing is strict: a
/// malformed line is skipped rather than becoming a wrong event, because a
/// trace that lies about its events is worse than a short one.
pub fn parse_events(text: &str) -> (Vec<Event>, u64) {
    let mut events = Vec::new();
    let mut dropped = 0;
    for line in text.lines() {
        let line = line.trim();
        if let Some(count) = line.strip_prefix("# dropped ") {
            dropped = count.trim().parse().unwrap_or(dropped);
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() != 4 {
            continue;
        }
        let (Ok(seq), Ok(kind), Ok(size)) = (
            fields[0].parse::<u64>(),
            fields[1].parse::<u8>(),
            fields[2].parse::<i64>(),
        ) else {
            continue;
        };
        let kind = match kind {
            1 => "alloc",
            2 => "free",
            3 => "task_start",
            4 => "task_end",
            5 => "io",
            _ => continue,
        };
        events.push(Event {
            seq,
            kind,
            size,
            pc: fields[3].to_string(),
            span: String::new(),
        });
    }
    (events, dropped)
}

/// Find `llvm-symbolizer`, preferring the one beside the binary being traced.
fn symbolizer_for(binary: &Path) -> Option<PathBuf> {
    if let Ok(path) = std::env::var("YASHY_LLVM_SYMBOLIZER") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    if let Some(parent) = binary.parent() {
        for name in ["llvm-symbolizer", "llvm-symbolizer.exe"] {
            let path = parent.join(name);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    // A bare name is resolved by `Command` against PATH. Keep it as the last
    // resort so a mismatched toolchain beside the binary cannot be chosen by a
    // coincidental file name.
    if Command::new("llvm-symbolizer")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
    {
        return Some(PathBuf::from("llvm-symbolizer"));
    }
    None
}

/// Attach a source span to each event, using the debug info in `binary`.
///
/// `llvm-symbolizer` prints two lines per address: the enclosing function and
/// `file:line:column`. The location is the span; when a location cannot be
/// recovered (a stripped binary, an inlined runtime frame), the function name
/// is still a useful answer. Without a symbolizer the raw address is kept, so
/// a machine with no LLVM tools still gets a truthful trace.
pub fn symbolize(events: &mut [Event], binary: &Path) {
    let Some(tool) = symbolizer_for(binary) else {
        for event in events.iter_mut() {
            event.span = event.pc.clone();
        }
        return;
    };
    let mut child = match Command::new(tool)
        .arg(format!("--obj={}", binary.display()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => {
            for event in events.iter_mut() {
                event.span = event.pc.clone();
            }
            return;
        }
    };
    let mut stdin = child.stdin.take().expect("a piped stdin");
    for event in events.iter() {
        let _ = writeln!(stdin, "0x{}", event.pc);
    }
    drop(stdin);
    let output = child
        .wait_with_output()
        .expect("the symbolizer must finish");
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    for (index, event) in events.iter_mut().enumerate() {
        let function = lines.get(index * 2).map(String::as_str).unwrap_or("??");
        let location = lines.get(index * 2 + 1).map(String::as_str).unwrap_or("??");
        event.span = if location.starts_with("??") {
            function.to_string()
        } else {
            location.to_string()
        };
    }
}

/// The JSON form of a traced run.
pub fn events_json(events: &[Event], dropped: u64, source: &Path) -> Json {
    let entries: Vec<Json> = events
        .iter()
        .map(|event| {
            let mut object = Json::object();
            object.set("seq", Json::number(event.seq as i64));
            object.set("kind", Json::string(event.kind));
            object.set("span", Json::string(event.span.as_str()));
            object.set("size", Json::number(event.size));
            object
        })
        .collect();
    let mut report = Json::object();
    report.set("schema_version", Json::number(1));
    report.set("tool", Json::string("trace"));
    report.set("mode", Json::string("events"));
    report.set(
        "source",
        Json::string(source.to_string_lossy().into_owned()),
    );
    report.set("events", Json::array(entries));
    report.set("dropped", Json::number(dropped as i64));
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

    #[test]
    fn runtime_events_parse_in_table_order() {
        let (events, dropped) =
            parse_events("1 1 16 12ab\n2 5 0 dead\n3 2 0 beef\n# dropped 7\nnot an event\n");
        assert_eq!(dropped, 7);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].kind, "alloc");
        assert_eq!(events[0].size, 16);
        assert_eq!(events[1].kind, "io");
        assert_eq!(events[2].kind, "free");
        assert_eq!(events[2].seq, 3);
    }
}
