//! The terminal policy: colour, width, and the global flags that select them.
//!
//! Colour is a rendering decision, never a fact. The same diagnostic renders as
//! plain text when stdout is a pipe, when `NO_COLOR` is set, when `CLICOLOR=0`,
//! or when `TERM=dumb`; `--color=always` overrides all four. `--json` callers
//! never reach this module, so a machine stream cannot contain an escape.

use std::io::IsTerminal;
use std::sync::OnceLock;

use yashy_common::{Diagnostic, SourceMap};

/// How colour is chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    Auto,
    Always,
    Never,
}

/// The resolved terminal state for this process.
#[derive(Clone, Copy, Debug)]
pub struct Term {
    pub color: bool,
    /// `COLUMNS`, when the caller or the shell provides it.
    pub columns: Option<usize>,
}

static TERM: OnceLock<Term> = OnceLock::new();

/// Strip the global output flags from `args` and resolve the process state.
///
/// The flags are accepted anywhere on the command line and never reach a verb,
/// so `yashyc check --color=never main.ya` and `yashyc --color=never check
/// main.ya` mean the same thing.
pub fn take_global_flags(args: Vec<String>) -> Result<Vec<String>, String> {
    let mut style = Color::Auto;
    let mut rest = Vec::new();
    let mut literal = false;
    for argument in args {
        if literal {
            rest.push(argument);
            continue;
        }
        if argument == "--" {
            literal = true;
            rest.push(argument);
            continue;
        }
        match argument.as_str() {
            "--color" => style = Color::Always,
            "--no-color" => style = Color::Never,
            other if other.starts_with("--color=") => {
                style = match other.trim_start_matches("--color=") {
                    "always" => Color::Always,
                    "never" => Color::Never,
                    "auto" => Color::Auto,
                    value => {
                        return Err(format!(
                            "unknown colour policy `{value}`; expected auto, always or never"
                        ));
                    }
                };
            }
            _ => rest.push(argument),
        }
    }
    let _ = TERM.set(detect(style));
    Ok(rest)
}

/// The process state, resolved lazily for library callers and tests.
pub fn current() -> &'static Term {
    TERM.get_or_init(|| detect(Color::Auto))
}

fn detect(style: Color) -> Term {
    let env_allows = std::env::var_os("NO_COLOR").is_none()
        && std::env::var("CLICOLOR")
            .map(|value| value != "0")
            .unwrap_or(true)
        && std::env::var("TERM")
            .map(|value| value != "dumb")
            .unwrap_or(true);
    let terminal = std::io::stderr().is_terminal();
    let color = match style {
        Color::Always => true,
        Color::Never => false,
        Color::Auto => env_allows && terminal,
    };
    let columns = std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value >= 20);
    Term { color, columns }
}

/// Wrap one diagnostic in an SGR sequence when colour is on.
fn paint(code: &str, text: &str) -> String {
    if current().color {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// Render a diagnostic with the process colour and width policy.
///
/// The header is cyan, the severity word is red (`error`), yellow (`warning`)
/// or blue (anything else). Every other line is copied verbatim, so the source
/// excerpt and the caret keep their exact columns.
pub fn render_diagnostic(map: &SourceMap, diagnostic: &Diagnostic) -> String {
    let plain = map.render(diagnostic);
    if !current().color && current().columns.is_none() {
        return plain;
    }
    let wrapped = wrap(&plain, current().columns);
    if !current().color {
        return wrapped;
    }
    let mut out = String::new();
    let mut lines = wrapped.lines();
    if let Some(first) = lines.next() {
        out.push_str(&paint("1;36", first));
        out.push('\n');
    }
    if let Some(second) = lines.next() {
        let code = if second.starts_with("error") {
            "1;31"
        } else if second.starts_with("warning") {
            "1;33"
        } else {
            "1;34"
        };
        out.push_str(&paint(code, second));
        out.push('\n');
    }
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Wrap long lines at word boundaries without touching shorter ones.
fn wrap(text: &str, columns: Option<usize>) -> String {
    let Some(columns) = columns else {
        return text.to_string();
    };
    let mut out = String::new();
    for line in text.lines() {
        // A caret line is already aligned to the source; wrapping it would
        // point at the wrong column.
        if line.trim_start().starts_with('^') {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let mut remaining = line;
        while remaining.chars().count() > columns {
            let limit = remaining
                .char_indices()
                .nth(columns)
                .map(|(at, _)| at)
                .unwrap_or(remaining.len());
            let split = remaining[..limit]
                .rfind(char::is_whitespace)
                .filter(|split| *split > 0)
                .unwrap_or(limit);
            out.push_str(remaining[..split].trim_end());
            out.push('\n');
            remaining = remaining[split..].trim_start();
        }
        out.push_str(remaining);
        out.push('\n');
    }
    out
}
