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
    pub fn to_json(&self) -> Json {
        let mut root = Json::object();
        root.set("schema_version", Json::number(1));
        root.set("tool", Json::string("yashyc"));
        root.set("passed", Json::number(self.passed as i64));
        root.set("mismatched", Json::number(self.mismatched as i64));
        root.set("without_example", Json::number(self.without_example as i64));
        let total = self.cases.len().max(1);
        root.set("percent", Json::number((self.passed * 100 / total) as i64));
        root.set(
            "cases",
            Json::array(self.cases.iter().map(Case::to_json).collect()),
        );
        root
    }

    pub fn render_human(&self) -> String {
        let mut out = format!(
            "Conformance: {} passed, {} mismatched, {} without examples\n",
            self.passed, self.mismatched, self.without_example
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

/// The first fenced `yashy` block of a page, if it has one.
fn example(page: &str) -> Option<String> {
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
        return Some(format!("{}\n", body.join("\n")));
    }
    None
}

/// Check one example and collect every coded diagnostic it produces.
fn check_example(source: &str) -> Vec<(String, u32, u32)> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("example.ya", source.to_string());
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
    let report = verify(&hir, &VerifyOptions::default());
    codes_of(&report.diagnostics)
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
        let Some(source) = example(&text) else {
            without_example += 1;
            cases.push(Case {
                rule: rule.clone(),
                page: relative,
                status: "without-example",
                expected: rule,
                actual: Vec::new(),
                span: None,
            });
            continue;
        };
        let found = check_example(&source);
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
