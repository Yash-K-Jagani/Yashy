//! Rendering for `yashy build --opt-report`.
//!
//! The code generator records a [`yashy_codegen::OptReport`] while it lowers
//! each body: one entry for every check the verifier could not delete and every
//! allocation escape analysis could not promote. This module turns that record
//! into the two shapes the CLI promises -- a short human report grouped by
//! function, and the schema-v1 JSON an agent acts on.
//!
//! Nothing here decides anything. A renderer that re-ran an analysis could
//! disagree with the compiler, which is worse than no report at all.

use yashy_codegen::{FnOptReport, OptKind, OptReport};
use yashy_common::{FileId, Json, SourceMap, Span};

/// The line printed when a program survived with nothing left to explain.
const NOTHING: &str =
    "no checks or allocations to report: every check was proved and every allocation was promoted";

/// The human report, grouped by function.
///
/// Only functions with at least one entry get a heading, so a program with
/// nothing to say produces exactly one line rather than a list of empty
/// sections.
pub fn render(sources: &SourceMap, report: &OptReport) -> String {
    let mut out = String::new();
    for function in report.functions.iter().filter(|f| !f.items.is_empty()) {
        let location = sources.locate(function.file, function.span);
        out.push_str(&format!(
            "{}:{}  {}\n",
            sources.path(function.file),
            location.line,
            heading(function)
        ));
        for item in &function.items {
            let snippet = one_line(sources.snippet(function.file, item.span));
            out.push_str(&format!(
                "  {:<22}{:<18}{}\n",
                label(item.kind),
                snippet,
                item.reason
            ));
            if let Some(fix) = &item.fix {
                out.push_str(&format!("  {:<22}{:<18}fix: {fix}\n", "", ""));
            }
        }
    }
    if out.is_empty() {
        out.push_str(NOTHING);
        out.push('\n');
    }
    out
}

/// The schema-v1 form: `schema_version`, `tool`, and one object per function.
///
/// `fix` is always present, `null` when nothing would help: a consumer must be
/// able to read the field without checking whether the key exists.
pub fn json(sources: &SourceMap, report: &OptReport) -> Json {
    let mut root = Json::object();
    root.set("schema_version", Json::number(1));
    root.set("tool", Json::string("yashy"));
    let mut functions = Json::array(Vec::new());
    for function in report.functions.iter().filter(|f| !f.items.is_empty()) {
        let mut object = Json::object();
        object.set("name", Json::string(&function.name));
        object.set("file", Json::string(sources.path(function.file)));
        object.set("span", span_json(sources, function.file, function.span));
        let mut items = Json::array(Vec::new());
        for item in &function.items {
            let mut entry = Json::object();
            entry.set("kind", Json::string(item.kind.name()));
            entry.set("span", span_json(sources, function.file, item.span));
            entry.set("reason", Json::string(&item.reason));
            entry.set(
                "fix",
                match &item.fix {
                    Some(fix) => Json::string(fix),
                    None => Json::null(),
                },
            );
            items.push(entry);
        }
        object.set("items", items);
        functions.push(object);
    }
    root.set("functions", functions);
    root
}

/// `src/main.ya:12  fn wordcount`, with `fn <lambda>` for an unnamed body.
fn heading(function: &FnOptReport) -> String {
    if function.name.is_empty() {
        "fn <lambda>".to_string()
    } else {
        format!("fn {}", function.name)
    }
}

/// The short spelling of a decision, in the report's first column.
fn label(kind: OptKind) -> &'static str {
    match kind {
        OptKind::Bounds => "bounds check kept",
        OptKind::Overflow => "overflow check kept",
        OptKind::Alloc => "heap allocation",
        OptKind::NotInlined => "call not inlined",
        OptKind::Promoted => "promoted",
        OptKind::NotPromoted => "not promoted",
    }
}

/// Collapse a source snippet onto one line, truncating an over-long one.
fn one_line(text: &str) -> String {
    let mut line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > 16 {
        line = line.chars().take(13).collect::<String>();
        line.push_str("...");
    }
    line
}

/// A span in the shape every other command uses: byte range plus the line and
/// column the reader sees.
fn span_json(sources: &SourceMap, file: FileId, span: Span) -> Json {
    let location = sources.locate(file, span);
    let mut object = Json::object();
    object.set("start", Json::number(i64::from(span.start)));
    object.set("end", Json::number(i64::from(span.end)));
    object.set("line", Json::number(i64::from(location.line)));
    object.set("column", Json::number(i64::from(location.col)));
    object
}
