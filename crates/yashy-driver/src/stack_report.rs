//! Rendering for `yashyc build --stack-report`.
//!
//! The decisions are made by escape analysis while the program is lowered and
//! recorded in the same `OptReport` `--opt-report` reads. This module only
//! selects the placement rows, so a site cannot be "promoted" in one report and
//! "heap" in the other. It renders both a human page and the schema-v1 JSON an
//! agent can act on; neither re-runs an analysis.

use yashy_codegen::{OptKind, OptReport};
use yashy_common::{FileId, Json, SourceMap, Span};

fn is_placement(kind: OptKind) -> bool {
    matches!(
        kind,
        OptKind::Promoted | OptKind::NotPromoted | OptKind::Alloc
    )
}

/// The human report: one line per allocation site, grouped by function.
pub fn render(sources: &SourceMap, report: &OptReport) -> String {
    let mut out = String::new();
    let mut sites = 0usize;
    for function in report
        .functions
        .iter()
        .filter(|f| f.items.iter().any(|item| is_placement(item.kind)))
    {
        let location = sources.locate(function.file, function.span);
        out.push_str(&format!(
            "{}:{}  fn {}\n",
            sources.path(function.file),
            location.line,
            if function.name.is_empty() {
                "<lambda>".to_string()
            } else {
                function.name.clone()
            }
        ));
        for item in function.items.iter().filter(|item| is_placement(item.kind)) {
            sites += 1;
            let location = sources.locate(function.file, item.span);
            let snippet = sources
                .snippet(function.file, item.span)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            out.push_str(&format!(
                "  {}:{}  {:<8} {:<16} {}\n",
                location.line,
                location.col,
                label(item.kind),
                snippet,
                item.reason
            ));
        }
    }
    if sites == 0 {
        out.push_str("no allocation sites to report\n");
    }
    out
}

/// The schema-v1 form: `schema_version`, `tool`, the site count, and one entry
/// per site with its span, placement and the reason for that placement.
pub fn json(sources: &SourceMap, report: &OptReport) -> Json {
    let mut root = Json::object();
    root.set("schema_version", Json::number(1));
    root.set("tool", Json::string("yashyc"));
    let mut sites = Json::array(Vec::new());
    let mut count = 0usize;
    for function in &report.functions {
        for item in function.items.iter().filter(|item| is_placement(item.kind)) {
            count += 1;
            let mut entry = Json::object();
            entry.set("function", Json::string(&function.name));
            entry.set("file", Json::string(sources.path(function.file)));
            entry.set("span", span_json(sources, function.file, item.span));
            entry.set("placement", Json::string(placement(item.kind)));
            entry.set("reason", Json::string(&item.reason));
            sites.push(entry);
        }
    }
    root.set("count", Json::number(count as i64));
    root.set("sites", sites);
    root
}

fn label(kind: OptKind) -> &'static str {
    match kind {
        OptKind::Promoted => "stack",
        OptKind::NotPromoted => "heap",
        _ => "heap",
    }
}

fn placement(kind: OptKind) -> &'static str {
    match kind {
        OptKind::Promoted => "stack",
        OptKind::NotPromoted => "heap",
        _ => "heap (promotion disabled)",
    }
}

fn span_json(sources: &SourceMap, file: FileId, span: Span) -> Json {
    let location = sources.locate(file, span);
    let mut object = Json::object();
    object.set("start", Json::number(i64::from(span.start)));
    object.set("end", Json::number(i64::from(span.end)));
    object.set("line", Json::number(i64::from(location.line)));
    object.set("column", Json::number(i64::from(location.col)));
    object
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_report_has_a_stable_shape() {
        let sources = SourceMap::new();
        let report = OptReport::default();
        assert_eq!(render(&sources, &report), "no allocation sites to report\n");
        let document = json(&sources, &report);
        assert!(format!("{}", document.render()).contains("\"count\": 0"));
    }
}
