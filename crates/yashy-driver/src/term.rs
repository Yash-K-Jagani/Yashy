//! The terminal policy: colour, width, paging and hyperlinks.
//!
//! Colour is a rendering decision, never a fact. The same diagnostic renders as
//! plain text when stdout is a pipe, when `NO_COLOR` is set, when `CLICOLOR=0`,
//! or when `TERM=dumb`; `--color=always` overrides all four. `--json` callers
//! never reach this module, so a machine stream cannot contain an escape.
//!
//! The pager and OSC 8 links follow the same rule: a pipe never receives an
//! escape and never starts a second process, and an explicit flag is a stronger
//! statement than an inherited variable.

use std::io::{IsTerminal, Read, Write};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use yashy_common::{Diagnostic, SourceMap};

/// How colour is chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    Auto,
    Always,
    Never,
}

/// How `explain` chooses a pager.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pager {
    Auto,
    Always,
    Never,
}

/// How a source position is rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hyperlink {
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
    /// Whether long human output goes through `PAGER`.
    pub pager: bool,
    /// Whether a source position is wrapped in an OSC 8 link.
    pub links: bool,
    pub stdout_tty: bool,
    pub stderr_tty: bool,
}

static TERM: OnceLock<Term> = OnceLock::new();

/// Strip the global output flags from `args` and resolve the process state.
///
/// The flags are accepted anywhere on the command line and never reach a verb,
/// so `yashyc check --color=never main.ya` and `yashyc --color=never check
/// main.ya` mean the same thing.
pub fn take_global_flags(args: Vec<String>) -> Result<Vec<String>, String> {
    let mut color = Color::Auto;
    let mut pager = Pager::Auto;
    let mut links = Hyperlink::Auto;
    let mut rest = Vec::new();
    let mut literal = false;
    let mut arguments = args.into_iter();
    while let Some(argument) = arguments.next() {
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
            "--color" => color = Color::Always,
            "--no-color" => color = Color::Never,
            "--pager" => pager = Pager::Always,
            "--no-pager" => pager = Pager::Never,
            "--no-hyperlink" => links = Hyperlink::Never,
            "--hyperlink" => {
                let Some(value) = arguments.next() else {
                    return Err("`--hyperlink` needs a policy: auto, always or never".to_string());
                };
                links = parse_hyperlink(&value)?;
            }
            other if other.starts_with("--color=") => {
                color = parse_color(other.trim_start_matches("--color="))?;
            }
            other if other.starts_with("--hyperlink=") => {
                links = parse_hyperlink(other.trim_start_matches("--hyperlink="))?;
            }
            _ => rest.push(argument),
        }
    }
    let _ = TERM.set(detect(color, pager, links));
    Ok(rest)
}

fn parse_color(value: &str) -> Result<Color, String> {
    match value {
        "always" => Ok(Color::Always),
        "never" => Ok(Color::Never),
        "auto" => Ok(Color::Auto),
        other => Err(format!(
            "unknown colour policy `{other}`; expected auto, always or never"
        )),
    }
}

fn parse_hyperlink(value: &str) -> Result<Hyperlink, String> {
    match value {
        "always" => Ok(Hyperlink::Always),
        "never" => Ok(Hyperlink::Never),
        "auto" => Ok(Hyperlink::Auto),
        other => Err(format!(
            "unknown hyperlink policy `{other}`; expected auto, always or never"
        )),
    }
}

/// The process state, resolved lazily for library callers and tests.
pub fn current() -> &'static Term {
    TERM.get_or_init(|| detect(Color::Auto, Pager::Auto, Hyperlink::Auto))
}

fn detect(color_style: Color, pager_style: Pager, link_style: Hyperlink) -> Term {
    let env_allows = std::env::var_os("NO_COLOR").is_none()
        && std::env::var("CLICOLOR")
            .map(|value| value != "0")
            .unwrap_or(true)
        && std::env::var("TERM")
            .map(|value| value != "dumb")
            .unwrap_or(true);
    let stdout_tty = std::io::stdout().is_terminal();
    let stderr_tty = std::io::stderr().is_terminal();
    let color = match color_style {
        Color::Always => true,
        Color::Never => false,
        Color::Auto => env_allows && stderr_tty,
    };
    let pager = match pager_style {
        Pager::Always => true,
        Pager::Never => false,
        Pager::Auto => env_allows && stdout_tty,
    };
    let links = match link_style {
        Hyperlink::Always => true,
        Hyperlink::Never => false,
        Hyperlink::Auto => env_allows && stdout_tty && terminal_speaks_osc8(),
    };
    let columns = std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value >= 20);
    Term {
        color,
        columns,
        pager,
        links,
        stdout_tty,
        stderr_tty,
    }
}

/// Whether the terminal behind stdout is known to understand OSC 8.
///
/// The list is deliberately a set of positive signals rather than a version
/// parse: a wrong `yes` drops an escape into a stream that shows it as junk,
/// and a wrong `no` only loses a click.
fn terminal_speaks_osc8() -> bool {
    if std::env::var("TERM").is_ok_and(|term| term == "dumb") {
        return false;
    }
    if std::env::var_os("WT_SESSION").is_some() {
        return true;
    }
    match std::env::var("TERM_PROGRAM").as_deref() {
        Ok("iTerm.app" | "WezTerm" | "vscode" | "Hyper" | "Ghostty" | "rio") => return true,
        _ => {}
    }
    if std::env::var_os("KITTY_WINDOW_ID").is_some() {
        return true;
    }
    if std::env::var("VTE_VERSION")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .is_some_and(|version| version >= 5000)
    {
        return true;
    }
    std::env::var("TERM").is_ok_and(|term| {
        ["kitty", "alacritty", "wezterm", "ghostty", "xterm-kitty"]
            .iter()
            .any(|name| term.contains(name))
    })
}

/// Wrap one line in an SGR sequence when colour is on.
fn paint(code: &str, text: &str) -> String {
    if current().color {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// Render `text` as a clickable link to `target` when the policy allows it.
///
/// The visible text is unchanged, so a terminal that ignores OSC 8 shows the
/// same `path:line:col` a pipe receives.
pub fn link(target: &str, text: &str) -> String {
    if current().links {
        format!("\x1b]8;;{target}\x1b\\{text}\x1b]8;;\x1b\\")
    } else {
        text.to_string()
    }
}

/// A `file://` URL for a source position, or `None` when the position is not
/// the shape the renderers produce.
pub fn file_url(path: &str, line: u32, column: u32) -> String {
    let absolute = std::fs::canonicalize(path)
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string());
    let mut normalized = absolute.replace('\\', "/");
    if !normalized.starts_with('/') {
        normalized.insert(0, '/');
    }
    format!("file://{normalized}:{line}:{column}")
}

/// Split a rendered diagnostic header (`path:line:col`) into its parts.
fn header_parts(header: &str) -> Option<(&str, u32, u32)> {
    let (rest, column) = header.rsplit_once(':')?;
    let (path, line) = rest.rsplit_once(':')?;
    let line = line.parse::<u32>().ok()?;
    let column = column.parse::<u32>().ok()?;
    Some((path, line, column))
}

/// Render a diagnostic with the process colour, width and link policy.
///
/// The header is a hyperlink when the terminal supports one, cyan always, and
/// the severity word is red (`error`), yellow (`warning`) or blue (anything
/// else). Every other line is copied verbatim, so the source excerpt and the
/// caret keep their exact columns.
pub fn render_diagnostic(map: &SourceMap, diagnostic: &Diagnostic) -> String {
    let plain = map.render(diagnostic);
    if !current().color && current().columns.is_none() && !current().links {
        return plain;
    }
    let wrapped = wrap(&plain, current().columns);
    if !current().color && !current().links {
        return wrapped;
    }
    let mut out = String::new();
    let mut lines = wrapped.lines();
    if let Some(first) = lines.next() {
        let first = match header_parts(first) {
            Some((path, line, column)) => link(&file_url(path, line, column), first),
            None => first.to_string(),
        };
        out.push_str(&paint("1;36", &first));
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

/// Print long human output through `PAGER` when the policy says to.
///
/// The child inherits the terminal, so its own scrolling and search keys work.
/// A missing, failing or unset `PAGER` writes the text directly instead: the
/// reader must always get the page, with or without a pager.
pub fn page(text: &str) {
    if !current().pager {
        print!("{text}");
        let _ = std::io::stdout().flush();
        return;
    }
    let pager = std::env::var("PAGER")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            if cfg!(windows) {
                "more.com".to_string()
            } else {
                "less -R".to_string()
            }
        });
    let mut parts = pager.split_whitespace();
    let Some(program) = parts.next() else {
        print!("{text}");
        return;
    };
    let child = Command::new(program)
        .args(parts)
        .stdin(Stdio::piped())
        .spawn();
    match child {
        Ok(mut child) => {
            let write = match child.stdin.take() {
                Some(mut stdin) => stdin.write_all(text.as_bytes()),
                None => Ok(()),
            };
            // A pager the reader quit early closes the pipe; that is a normal
            // way to stop reading, not an error.
            let _ = write;
            let _ = child.wait();
        }
        Err(_) => {
            print!("{text}");
            let _ = std::io::stdout().flush();
        }
    }
}

/// Whether stdout is a terminal, for a renderer that redraws in place.
pub fn stdout_is_tty() -> bool {
    current().stdout_tty
}

/// The escape that clears the screen before a redraw, or nothing in a pipe.
pub fn clear_screen() -> &'static str {
    if current().stdout_tty {
        "\x1b[2J\x1b[H"
    } else {
        ""
    }
}

/// Read all of stdin. Kept here so `run --stdin-echo` and the REPL share one
/// spelling of "the caller may be a pipe".
pub fn read_stdin() -> std::io::Result<String> {
    let mut buffer = String::new();
    std::io::stdin().read_to_string(&mut buffer)?;
    Ok(buffer)
}
