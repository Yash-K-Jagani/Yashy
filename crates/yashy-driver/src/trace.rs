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

/// The whole matrix: the rows plus the codes no test file mentions.
#[derive(Clone, Debug)]
pub struct Matrix {
    pub rows: Vec<RuleRow>,
    pub orphan_codes: Vec<String>,
    /// Rules the design names but no catalog code yet enforces.
    pub orphan_rules_without_code: Vec<String>,
    /// Public stdlib functions no program under `tests/programs` mentions.
    pub orphan_stdlib_functions: Vec<String>,
    /// Conformance cases whose rule id no diagnostic code enforces.
    pub orphan_conformance_cases: Vec<String>,
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
pub fn matrix(root: &Path) -> Result<Matrix, String> {
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
    let mut orphan_codes = Vec::new();
    for (rule, entries) in by_rule {
        let mut tests = Vec::new();
        for (path, text) in &contents {
            if entries.iter().any(|code| text.contains(code)) {
                // A relative, forward-slash path keeps the matrix byte-stable
                // across machines and operating systems: an absolute path
                // would make the generated page differ on every checkout.
                let relative = path.strip_prefix(root).unwrap_or(path);
                tests.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
        for code in &entries {
            // A shared test file counts for every code it mentions, so a code
            // that appears in at least one test is not an orphan.
            if !contents.iter().any(|(_, text)| text.contains(code)) {
                orphan_codes.push(code.clone());
            }
        }
        rows.push(RuleRow {
            rule,
            codes: entries,
            tests,
        });
    }
    let enforced: std::collections::BTreeSet<&str> = codes::CODES
        .iter()
        .map(|entry| codes::rule_for(entry.id))
        .collect();
    let orphan_rules_without_code = codes::PLANNED_RULES
        .iter()
        .filter(|rule| !enforced.contains(**rule))
        .map(|rule| rule.to_string())
        .collect();
    let orphan_stdlib_functions = stdlib_without_program(root)?;
    let orphan_conformance_cases = conformance_without_rule(root, &enforced)?;
    Ok(Matrix {
        rows,
        orphan_codes,
        orphan_rules_without_code,
        orphan_stdlib_functions,
        orphan_conformance_cases,
    })
}

/// Conformance cases whose rule id is not enforced by any catalog code.
///
/// The registry is `docs/conformance.md`, a Markdown table with `| case |
/// rule | test |` rows. A case that names an unknown rule is a gap in the
/// traceability chain: it points at a rule no diagnostic can report.
pub fn conformance_without_rule(
    root: &Path,
    enforced: &std::collections::BTreeSet<&str>,
) -> Result<Vec<String>, String> {
    let path = root.join("docs/conformance.md");
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let mut missing = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with('|') {
            continue;
        }
        let fields: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        if fields.len() < 2 || fields[0] == "Case" || fields[0].starts_with('-') {
            continue;
        }
        let rule = fields[1].trim_matches('`');
        if !enforced.contains(rule) {
            missing.push(format!("{} ({rule})", fields[0]));
        }
    }
    Ok(missing)
}

/// Public stdlib functions no test program names, as `module.function`.
///
/// A function is covered when its bare name appears in any program under
/// `crates/yashy-driver/tests/programs`. The module prefix is kept in the
/// report so a reader knows which module to look at.
pub fn stdlib_without_program(root: &Path) -> Result<Vec<String>, String> {
    fn collect_yas(dir: &Path, files: &mut Vec<PathBuf>) {
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
                collect_yas(&path, files);
            } else if name.ends_with(".ya") {
                files.push(path);
            }
        }
        files.sort();
    }

    let mut programs = Vec::new();
    collect_yas(
        &root.join("crates/yashy-driver/tests/programs"),
        &mut programs,
    );
    let mut program_text = String::new();
    for path in &programs {
        program_text.push_str(
            &std::fs::read_to_string(path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?,
        );
    }

    let mut modules = Vec::new();
    collect_yas(&root.join("crates/yashy-stdlib/yashy"), &mut modules);
    let mut missing = Vec::new();
    for path in &modules {
        let module = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_default();
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        for line in text.lines() {
            let Some(rest) = line.trim().strip_prefix("pub fn ") else {
                continue;
            };
            let name = rest
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .next()
                .unwrap_or("");
            if !name.is_empty() && !program_text.contains(name) {
                missing.push(format!("{module}.{name}"));
            }
        }
    }
    Ok(missing)
}

/// The human page, generated from the same rows as the JSON model.
///
/// One section per rule, with the codes it names and the test files that
/// mention them. Nothing is hand-maintained: a new code with no test shows up
/// as `Tests: none`, which is the page's way of naming an orphan.
pub fn to_markdown(rows: &[RuleRow]) -> String {
    let mut out = String::new();
    out.push_str("# Traceability matrix\n\n");
    out.push_str(
        "Generated from `codes::rule_for`, the catalog and the test files that mention each code.\n",
    );
    out.push_str("Edit the registry, never this page. `UPDATE_GOLDEN=1` regenerates it.\n\n");
    for row in rows {
        out.push_str(&format!("## `{}`\n\n", row.rule));
        let codes = row
            .codes
            .iter()
            .map(|code| format!("`{code}`"))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("Codes: {codes}\n\nTests:\n\n"));
        if row.tests.is_empty() {
            out.push_str("- none\n");
        } else {
            for test in &row.tests {
                out.push_str(&format!("- `{test}`\n"));
            }
        }
        out.push('\n');
    }
    out
}

pub fn to_json(matrix: &Matrix) -> Json {
    let rows = &matrix.rows;
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
    report.set(
        "orphan_codes",
        Json::array(
            matrix
                .orphan_codes
                .iter()
                .map(|code| Json::string(code))
                .collect(),
        ),
    );
    report.set(
        "orphan_rules_without_code",
        Json::array(
            matrix
                .orphan_rules_without_code
                .iter()
                .map(|rule| Json::string(rule))
                .collect(),
        ),
    );
    report.set(
        "orphan_stdlib_functions",
        Json::array(
            matrix
                .orphan_stdlib_functions
                .iter()
                .map(|entry| Json::string(entry))
                .collect(),
        ),
    );
    report.set(
        "orphan_conformance_cases",
        Json::array(
            matrix
                .orphan_conformance_cases
                .iter()
                .map(|entry| Json::string(entry))
                .collect(),
        ),
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
    pub ptr: u64,
    pub stack: Vec<String>,
    pub span: String,
}

/// Parse the runtime's flat event file.
///
/// The runtime writes `seq kind size pc ptr s1,s2,s3` lines in table order and
/// one trailing `# dropped N` line when the bounded table overflowed. The
/// pointer is what lets a free be matched to its allocation, and the three
/// callers are the bounded stack. Parsing is strict: a malformed line is
/// skipped rather than becoming a wrong event, because a trace that lies about
/// its events is worse than a short one.
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
        if !(5..=6).contains(&fields.len()) {
            continue;
        }
        let (Ok(seq), Ok(kind), Ok(size)) = (
            fields[0].parse::<u64>(),
            fields[1].parse::<u8>(),
            fields[2].parse::<i64>(),
        ) else {
            continue;
        };
        let Ok(ptr) = u64::from_str_radix(fields[4], 16) else {
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
        let stack = fields
            .get(5)
            .map(|text| text.split(',').map(str::to_string).collect())
            .unwrap_or_default();
        events.push(Event {
            seq,
            kind,
            size,
            pc: fields[3].to_string(),
            ptr,
            stack,
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
        let rules: Vec<&str> = rows.rows.iter().map(|row| row.rule.as_str()).collect();
        let expected: std::collections::BTreeSet<&str> = codes::CODES
            .iter()
            .map(|entry| codes::rule_for(entry.id))
            .collect();
        for rule in expected {
            assert!(rules.contains(&rule), "no row for rule {rule}");
        }
        let codes_in_rows: usize = rows.rows.iter().map(|row| row.codes.len()).sum();
        assert_eq!(codes_in_rows, codes::CODES.len());
        assert!(to_json(&rows).render().contains("orphan_codes"));
        assert!(rows
            .orphan_rules_without_code
            .contains(&"ALIAS-1".to_string()));
        assert!(rows
            .orphan_rules_without_code
            .contains(&"DET-1".to_string()));
        assert!(rows
            .orphan_stdlib_functions
            .contains(&"convert.float_to_string".to_string()));
        assert!(rows.orphan_conformance_cases.is_empty());
    }

    #[test]
    fn runtime_events_parse_in_table_order() {
        let (events, dropped) =
            parse_events("1 1 16 12ab 1000 a,b,c\n2 5 0 dead 0 d,e,f\n3 2 0 beef 1000 g,h,i\n# dropped 7\nnot an event\n");
        assert_eq!(dropped, 7);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].kind, "alloc");
        assert_eq!(events[0].size, 16);
        assert_eq!(events[1].kind, "io");
        assert_eq!(events[2].kind, "free");
        assert_eq!(events[2].seq, 3);
    }
}
