//! A small, data-driven linter over source text.
//!
//! Lints are advice, not verifier rules: a lint never changes whether a
//! program builds. Each lint has a stable code, a group and a check over the
//! text; `--list --json` is the machine form docs are rendered from.

use yashy_common::{Applicability, Diagnostic, FileId, Severity, Span, Suggestion};

/// The group a lint belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LintGroup {
    Correctness,
    Style,
    Complexity,
    Perf,
    Security,
    Deprecated,
}

impl LintGroup {
    pub fn as_str(self) -> &'static str {
        match self {
            LintGroup::Correctness => "correctness",
            LintGroup::Style => "style",
            LintGroup::Complexity => "complexity",
            LintGroup::Perf => "perf",
            LintGroup::Security => "security",
            LintGroup::Deprecated => "deprecated",
        }
    }

    /// Parse a group name from the command line.
    pub fn parse(name: &str) -> Option<LintGroup> {
        GROUPS.iter().copied().find(|group| group.as_str() == name)
    }
}

pub const GROUPS: &[LintGroup] = &[
    LintGroup::Correctness,
    LintGroup::Style,
    LintGroup::Complexity,
    LintGroup::Perf,
    LintGroup::Security,
    LintGroup::Deprecated,
];

/// One registered lint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lint {
    pub code: &'static str,
    pub group: LintGroup,
    pub message: &'static str,
    /// The one-line fix shown in the generated page.
    pub hint: &'static str,
}

pub const LINTS: &[Lint] = &[
    Lint {
        code: "LINT-UNUSED",
        group: LintGroup::Correctness,
        message: "a binding is never used",
        hint: "remove the binding, or use `_` if the value is intentionally ignored",
    },
    Lint {
        code: "LINT-SHADOW",
        group: LintGroup::Style,
        message: "a binding shadows an earlier name in the same function",
        hint: "rename one of the bindings so both stay readable",
    },
    Lint {
        code: "LINT-VAR",
        group: LintGroup::Correctness,
        message: "`var` was removed: use `let`",
        hint: "`let` is the only mutable binding keyword",
    },
    Lint {
        code: "LINT-SWITCH",
        group: LintGroup::Style,
        message: "a long `if`/`else if` chain over one value reads better as `switch`",
        hint: "rewrite the chain as `switch value { case ... -> ... }`",
    },
    Lint {
        code: "LINT-WHILE",
        group: LintGroup::Correctness,
        message: "`while true` without a `break` never ends",
        hint: "add a `break` exit, or use a bounded `for` loop",
    },
    Lint {
        code: "LINT-RESULT",
        group: LintGroup::Correctness,
        message: "a fallible call's `Result` is ignored",
        hint: "propagate it with `?` or handle it with `switch`/`match`",
    },
];

/// Run every lint over `source`.
pub fn check(source: &str, file: FileId) -> Vec<Diagnostic> {
    check_filtered(source, file, None)
}

/// Run the lints in `group`, or every lint when `group` is `None`.
pub fn check_filtered(source: &str, file: FileId, group: Option<LintGroup>) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for lint in LINTS {
        if group.is_some_and(|group| group != lint.group) {
            continue;
        }
        match lint.code {
            "LINT-VAR" => {
                for (start, _) in find_word(source, "var") {
                    let edit = Suggestion {
                        span: Span::new(start as u32, (start + 3) as u32),
                        replacement: "let".to_string(),
                        applicability: Applicability::MachineApplicable,
                        message: "replace `var` with `let`".to_string(),
                    };
                    out.push(make(lint, file, start, start + 3).with_suggestions(vec![edit]));
                }
            }
            "LINT-UNUSED" | "LINT-SHADOW" => {
                for (start, name) in let_bindings(source) {
                    let uses = count_word(source, &name);
                    if uses <= 1 && lint.code == "LINT-UNUSED" {
                        out.push(make(lint, file, start, start + name.len()));
                    }
                    if lint.code == "LINT-SHADOW"
                        && let_bindings(source)
                            .iter()
                            .filter(|(_, other)| *other == name)
                            .count()
                            > 1
                    {
                        out.push(make(lint, file, start, start + name.len()));
                    }
                }
            }
            "LINT-SWITCH" => {
                if find_word(source, "else if").len() >= 2 {
                    let start = find_word(source, "else if")[0].0;
                    out.push(make(lint, file, start, start + 7));
                }
            }
            "LINT-WHILE" => {
                for (start, _) in find_word(source, "while true") {
                    if find_word(source, "break").is_empty() {
                        out.push(make(lint, file, start, start + 10));
                    }
                }
            }
            "LINT-RESULT" => {
                for fallible in [
                    "read_file",
                    "write_file",
                    "parse_i64",
                    "parse_f64",
                    "getenv",
                ] {
                    for (start, _) in find_word(source, fallible) {
                        let line_start = source[..start].rfind('\n').map_or(0, |at| at + 1);
                        let line_end = source[start..]
                            .find('\n')
                            .map_or(source.len(), |at| start + at);
                        let line = &source[line_start..line_end];
                        let handled = line.contains('?')
                            || line.contains("switch")
                            || line.contains("match")
                            || line.contains("is_ok")
                            || line.contains("is_err")
                            || line.contains("unwrap");
                        if !handled {
                            out.push(make(lint, file, start, start + fallible.len()));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out.sort_by_key(|diagnostic| diagnostic.span.start);
    out
}

/// The human page, rendered from the registry.
///
/// `docs/lints.md` is this string; `cargo test -p yashy-driver --test lint`
/// compares the committed file with it, so a lint cannot exist only in code.
pub fn render_docs() -> String {
    let mut out = String::from("# Lints\n\n");
    out.push_str(
        "`yashyc lint --list --json` is the machine form of this page. A lint is\n\
         advice: it never changes whether a program builds. A lint can be promoted\n\
         to an error with `yashyc lint --deny <code> <file.ya>`, and filtered to one\n\
         group with `--group <correctness|style|complexity|perf|security|deprecated>`.\n\n",
    );
    out.push_str("| Code | Group | Message | Fix |\n|---|---|---|---|\n");
    for lint in LINTS {
        out.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            lint.code,
            lint.group.as_str(),
            lint.message,
            lint.hint
        ));
    }
    out
}

fn make(lint: &Lint, file: FileId, start: usize, end: usize) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(
        Severity::Warning,
        file,
        Span::new(start as u32, end as u32),
        lint.message,
    );
    diagnostic = diagnostic.with_code(lint.code);
    diagnostic
}

fn find_word(source: &str, word: &str) -> Vec<(usize, String)> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at + word.len() <= bytes.len() {
        let end = at + word.len();
        if &bytes[at..end] == word.as_bytes()
            && (at == 0 || !is_word(bytes[at - 1]))
            && (end == bytes.len() || !is_word(bytes[end]))
        {
            out.push((at, word.to_string()));
            at = end;
        } else {
            at += 1;
        }
    }
    out
}

fn count_word(source: &str, word: &str) -> usize {
    find_word(source, word).len()
}

fn let_bindings(source: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (start, _) in find_word(source, "let") {
        let bytes = source.as_bytes();
        let mut at = start + 3;
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        // Skip the type: `let int name` / `let string name`.
        while at < bytes.len()
            && (bytes[at].is_ascii_alphanumeric()
                || bytes[at] == b'_'
                || bytes[at] == b'['
                || bytes[at] == b']')
        {
            at += 1;
        }
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        let name_start = at;
        while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_') {
            at += 1;
        }
        if at > name_start {
            if let Some(name) = source.get(name_start..at) {
                out.push((name_start, name.to_string()));
            }
        }
    }
    out
}

fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}
