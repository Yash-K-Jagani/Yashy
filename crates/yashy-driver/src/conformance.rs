//! Conformance: run the examples on error pages through the checker.
//!
//! An error page's `yashy` block is the rule's evidence. This module extracts
//! it, checks it in-process, and records whether the diagnostic the page names
//! is the one the compiler produces. A page whose example does not reproduce
//! its own code is reported as a mismatch, not silently skipped: a reader who
//! copies the example must see the diagnostic the page explains.
//!
//! The runner is deliberately permissive by default: it prints the report and
//! exits 0. `--fail-under` turns it into a gate when a project records a
//! threshold it is willing to hold.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use yashy_common::{Json, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program_with, CheckOptions};
use yashy_verify::{verify, VerifyOptions};

/// Codes whose evidence is not a Yashy source file. Each has its own harness:
/// a package manifest, a sandbox run, a toolchain probe or a CLI flag. They are
/// listed in the report, but they are not part of the source conformance
/// denominator -- a source checker cannot honestly claim to run them.
const NON_SOURCE: &[(&str, &str)] = &[
    ("E1404", "cargo test -p yashy-verify --test unsafe"),
    ("E1800", "cargo test -p yashy-driver --test dependencies"),
    ("E1801", "cargo test -p yashy-driver --test dependencies"),
    ("E1802", "cargo test -p yashy-driver --test dependencies"),
    ("E1803", "cargo test -p yashy-driver --test dependencies"),
    ("E1804", "cargo test -p yashy-driver --test registry"),
    ("E1805", "cargo test -p yashy-driver --test audit"),
    ("E1806", "cargo test -p yashy-driver --test audit"),
    ("E1807", "cargo test -p yashy-driver --test approval"),
    ("E1808", "cargo test -p yashy-driver --test capabilities"),
    ("E1809", "cargo test -p yashy-driver --test registry"),
    ("E1810", "cargo test -p yashy-driver --test dependencies"),
    ("E1811", "cargo test -p yashy-driver --test dependencies"),
    (
        "E1900",
        "cargo test -p yashy-driver --test runtime_capabilities",
    ),
    ("E1901", "cargo test -p yashy-driver --test sandbox_escape"),
    ("E2200", "cargo test -p yashy-driver --test toolchain"),
    ("E2201", "cargo test -p yashy-driver --test toolchain"),
    ("E2204", "cargo test -p yashy-driver --test pack"),
];

/// The harness that owns a code's evidence, or `None` for a source rule.
pub fn non_source_harness(rule: &str) -> Option<&'static str> {
    NON_SOURCE
        .iter()
        .find(|(code, _)| *code == rule)
        .map(|(_, harness)| *harness)
}

/// One rule's evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Case {
    pub rule: String,
    pub page: String,
    pub status: &'static str,
    pub expected: String,
    pub actual: Vec<String>,
    /// The byte span of the first matching diagnostic, when one exists.
    pub span: Option<(u32, u32)>,
}

impl Case {
    pub fn to_json(&self) -> Json {
        let mut root = Json::object();
        root.set("rule", Json::string(&self.rule));
        root.set("page", Json::string(&self.page));
        root.set("status", Json::string(self.status));
        root.set("expected", Json::string(&self.expected));
        root.set(
            "actual",
            Json::array(self.actual.iter().map(|code| Json::string(code)).collect()),
        );
        match self.span {
            Some((start, end)) => {
                let mut span = Json::object();
                span.set("start", Json::number(start as i64));
                span.set("end", Json::number(end as i64));
                root.set("span", span);
            }
            None => {
                root.set("span", Json::Null);
            }
        }
        root
    }
}

/// The whole report: cases in code order, with counts that agree with them.
#[derive(Clone, Debug)]
pub struct Report {
    pub cases: Vec<Case>,
    pub passed: usize,
    pub mismatched: usize,
    pub without_example: usize,
}

impl Report {
    /// Source rules are the denominator. A code whose evidence is a manifest,
    /// sandbox run or toolchain probe has its own harness and is reported but
    /// never counted here: this runner cannot honestly execute it.
    pub fn source_total(&self) -> usize {
        self.cases
            .iter()
            .filter(|case| non_source_harness(&case.rule).is_none())
            .count()
    }

    pub fn out_of_scope(&self) -> usize {
        self.cases
            .iter()
            .filter(|case| non_source_harness(&case.rule).is_some())
            .count()
    }

    pub fn percent(&self) -> u64 {
        let total = self.source_total().max(1);
        (self.passed * 100 / total) as u64
    }

    pub fn to_json(&self) -> Json {
        let mut root = Json::object();
        root.set("schema_version", Json::number(1));
        root.set("tool", Json::string("yashyc"));
        root.set("passed", Json::number(self.passed as i64));
        root.set("mismatched", Json::number(self.mismatched as i64));
        root.set("without_example", Json::number(self.without_example as i64));
        root.set("out_of_scope", Json::number(self.out_of_scope() as i64));
        root.set("source_total", Json::number(self.source_total() as i64));
        root.set("percent", Json::number(self.percent() as i64));
        let mut harnesses = Json::object();
        for (code, harness) in NON_SOURCE {
            harnesses.set(*code, Json::string(*harness));
        }
        root.set("non_source_harnesses", harnesses);
        root.set(
            "cases",
            Json::array(self.cases.iter().map(Case::to_json).collect()),
        );
        root
    }

    pub fn render_human(&self) -> String {
        let mut out = format!(
            "Conformance: {} passed, {} mismatched, {} without examples, {} out of scope\n",
            self.passed,
            self.mismatched,
            self.without_example,
            self.out_of_scope()
        );
        for case in &self.cases {
            if case.status == "pass" {
                continue;
            }
            out.push_str(&format!(
                "  {} {} {} (expected {}, got {})\n",
                case.status,
                case.rule,
                case.page,
                case.expected,
                if case.actual.is_empty() {
                    "nothing".to_string()
                } else {
                    case.actual.join(", ")
                }
            ));
        }
        out
    }
}

/// One fenced source block. `name` comes from a leading `// name.ya` label;
/// a single-block page uses `example.ya` as before.
#[derive(Clone, Debug)]
struct SourceExample {
    name: String,
    source: String,
}

/// Every `yashy` block of a page. A leading `// name.ya` is a file label, not
/// source: a module page needs its blocks written as separate files before the
/// loader can resolve the imports between them.
fn examples(page: &str) -> Vec<SourceExample> {
    let mut out = Vec::new();
    let mut lines = page.lines();
    while let Some(line) = lines.next() {
        if line.trim_start().strip_prefix("```") != Some("yashy") {
            continue;
        }
        let mut body = Vec::new();
        for inner in lines.by_ref() {
            if inner.trim_start().starts_with("```") {
                break;
            }
            body.push(inner);
        }
        let mut name = format!("example-{}.ya", out.len());
        if body.first().is_some_and(|first| {
            let trimmed = first.trim();
            trimmed.starts_with("//")
                && trimmed.ends_with(".ya")
                && !trimmed[2..].trim().contains(' ')
        }) {
            name = body.remove(0).trim()[2..].trim().to_string();
        }
        out.push(SourceExample {
            name,
            source: format!("{}\n", body.join("\n")),
        });
    }
    out
}

/// Check a page's blocks. A module page is staged in a private directory so
/// the same loader the CLI uses can resolve its imports and cycles.
fn check_examples(items: &[SourceExample]) -> Vec<(String, u32, u32)> {
    if items.len() == 1 {
        return check_single(&items[0].name, &items[0].source);
    }
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "yashy-conformance-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    if std::fs::create_dir_all(&dir).is_err() {
        return Vec::new();
    }
    let debug = std::env::var_os("YASHY_CONFORMANCE_DEBUG").is_some();
    let result = (|| {
        for item in items {
            if std::fs::write(dir.join(&item.name), &item.source).is_err() {
                if debug {
                    eprintln!("cannot stage `{}` in {}", item.name, dir.display());
                }
                return None;
            }
        }
        let entry = items
            .iter()
            .find(|item| item.name == "main.ya")
            .unwrap_or(&items[0]);
        let mut sources = SourceMap::new();
        let hir = match crate::pipeline::load_and_check(
            &mut sources,
            &dir.join(&entry.name),
            entry.source.clone(),
            false,
            1,
        ) {
            Ok(hir) => hir,
            Err(diagnostics) => {
                if debug {
                    eprintln!("module diagnostics: {diagnostics:?}");
                }
                return Some(codes_of(&diagnostics));
            }
        };
        Some(finish(&hir))
    })();
    let _ = std::fs::remove_dir_all(&dir);
    let found = result.unwrap_or_default();
    if debug {
        eprintln!("module codes: {found:?}");
    }
    found
}

fn finish(hir: &yashy_hir::hir::HirProgram) -> Vec<(String, u32, u32)> {
    // Warnings run after the checker and before the verifier, and live in the
    // driver rather than in `verify` itself. A page for `E2000` is evidence
    // only if the runner collects that pass too. The module path supplies the
    // real export table; a single-file page uses the conservative empty one.
    let mut found = Vec::new();
    let mut warnings = Vec::new();
    yashy_verify::pass::warnings::check(hir, &Default::default(), &mut warnings);
    found.extend(codes_of(&warnings));
    let report = verify(hir, &VerifyOptions::default());
    found.extend(codes_of(&report.diagnostics));
    found
}

/// Check one single-file example and collect every coded diagnostic it produces.
fn check_single(name: &str, source: &str) -> Vec<(String, u32, u32)> {
    let mut sources = SourceMap::new();
    let file = sources.add_file(name, source.to_string());
    // A page's example may be rejected before sema runs: `repr(C) fn` is a
    // parser diagnostic, and discarding it would report the page as producing
    // nothing. The lexer's and parser's own codes are evidence too.
    let tokens = match Lexer::with_file(source, file).tokenize() {
        Ok(tokens) => tokens,
        Err(diagnostics) => return codes_of(&diagnostics),
    };
    let mut parser = Parser::new(tokens.clone(), &sources, file);
    let program = match parser.parse_program() {
        Ok(program) => program,
        Err(diagnostics) => return codes_of(&diagnostics),
    };
    // Macros expand before parsing, and `cfg` is decided before resolution.
    // A page for `E1600` or `E2202` is only evidence if the runner does what
    // the real pipeline does, so both stages run here too.
    let (expanded, _map) = yashy_expand::expand_tokens(&program.items, &sources, file, &tokens);
    let tokens = match expanded {
        Ok(tokens) => tokens,
        Err(diagnostic) => return codes_of(&[diagnostic]),
    };
    let mut program = match Parser::new(tokens, &sources, file).parse_program() {
        Ok(program) => program,
        Err(diagnostics) => return codes_of(&diagnostics),
    };
    let set = crate::cfg::CfgSet::for_target(yashy_codegen::host_target_triple(), false);
    let cfg_diagnostics = crate::cfg::filter(&mut program, &set, file);
    if !cfg_diagnostics.is_empty() {
        return codes_of(&cfg_diagnostics);
    }
    let hir = match check_program_with(&program, &sources, file, CheckOptions::default()) {
        Ok(hir) => hir,
        Err(diagnostics) => return codes_of(&diagnostics),
    };
    finish(&hir)
}

fn codes_of(diagnostics: &[yashy_common::Diagnostic]) -> Vec<(String, u32, u32)> {
    diagnostics
        .iter()
        .filter_map(|diagnostic| {
            diagnostic
                .code
                .map(|code| (code.to_string(), diagnostic.span.start, diagnostic.span.end))
        })
        .collect()
}

/// Run every error page under `docs/errors` and report which examples
/// reproduce the code in the page's filename.
pub fn run(root: &Path) -> Result<Report, String> {
    let directory = root.join("docs").join("errors");
    let entries = std::fs::read_dir(&directory)
        .map_err(|error| format!("{}: {error}", directory.display()))?;
    let mut pages: BTreeMap<String, PathBuf> = BTreeMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("")
            .to_string();
        if name.starts_with('E') {
            pages.insert(name, path);
        }
    }
    let mut cases = Vec::new();
    let (mut passed, mut mismatched, mut without_example) = (0, 0, 0);
    for (rule, path) in pages {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let blocks = examples(&text);
        if blocks.is_empty() {
            without_example += 1;
            let status = if non_source_harness(&rule).is_some() {
                "out-of-scope"
            } else {
                "without-example"
            };
            cases.push(Case {
                rule: rule.clone(),
                page: relative,
                status,
                expected: rule,
                actual: Vec::new(),
                span: None,
            });
            continue;
        };
        let found = check_examples(&blocks);
        let matching = found.iter().find(|(code, _, _)| code == &rule);
        let actual: Vec<String> = found.iter().map(|(code, _, _)| code.clone()).collect();
        if matching.is_some() {
            passed += 1;
            cases.push(Case {
                rule: rule.clone(),
                page: relative,
                status: "pass",
                expected: rule,
                actual,
                span: matching.map(|(_, start, end)| (*start, *end)),
            });
        } else {
            mismatched += 1;
            cases.push(Case {
                rule: rule.clone(),
                page: relative,
                status: "mismatch",
                expected: rule,
                actual,
                span: None,
            });
        }
    }
    Ok(Report {
        cases,
        passed,
        mismatched,
        without_example,
    })
}
