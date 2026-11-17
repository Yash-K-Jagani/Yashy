//! SARIF 2.1.0 rendering of diagnostics.
//!
//! The CLI's schema v1 is the human/tool contract; SARIF is the interchange
//! form CI uploads. Both are rendered from the same `Diagnostic` values, and a
//! machine-applicable suggestion becomes a SARIF `fixes` object with the same
//! replacement the CLI applies.

use yashy_common::Json;
use yashy_common::{Diagnostic, SourceMap};

fn region(sources: &SourceMap, diagnostic: &Diagnostic) -> Json {
    let start = sources.locate(diagnostic.file, diagnostic.span);
    let end_span = yashy_common::Span::new(diagnostic.span.end, diagnostic.span.end);
    let end = sources.locate(diagnostic.file, end_span);
    let mut region = Json::object();
    region.set("startLine", Json::number(i64::from(start.line)));
    region.set("startColumn", Json::number(i64::from(start.col)));
    region.set("endLine", Json::number(i64::from(end.line)));
    region.set("endColumn", Json::number(i64::from(end.col)));
    region
}

/// A minimal SARIF 2.1.0 log with rules, results, physical locations and fixes.
pub fn to_sarif(sources: &SourceMap, diagnostics: &[Diagnostic]) -> Json {
    let mut rules: Vec<Json> = Vec::new();
    let mut seen = Vec::new();
    for diagnostic in diagnostics {
        if let Some(code) = diagnostic.code {
            if !seen.contains(&code) {
                seen.push(code);
                let mut rule = Json::object();
                rule.set("id", Json::string(code));
                let mut name = Json::object();
                name.set("text", Json::string(diagnostic.message.clone()));
                rule.set("shortDescription", name);
                rules.push(rule);
            }
        }
    }

    let results: Vec<Json> = diagnostics
        .iter()
        .map(|diagnostic| {
            let mut result = Json::object();
            let rule_id = diagnostic.code.unwrap_or("E0000");
            result.set("ruleId", Json::string(rule_id));
            result.set(
                "level",
                Json::string(if diagnostic.is_error() {
                    "error"
                } else {
                    "warning"
                }),
            );
            let mut message = Json::object();
            message.set("text", Json::string(diagnostic.message.clone()));
            result.set("message", message);
            let mut physical = Json::object();
            let mut artifact = Json::object();
            artifact.set("uri", Json::string(sources.path(diagnostic.file)));
            physical.set("artifactLocation", artifact);
            physical.set("region", region(sources, diagnostic));
            let mut location = Json::object();
            location.set("physicalLocation", physical);
            result.set("locations", Json::array(vec![location]));

            if !diagnostic.suggestions.is_empty() {
                let mut replacements = Vec::new();
                for suggestion in &diagnostic.suggestions {
                    let mut replacement = Json::object();
                    replacement.set("deletedRegion", region(sources, diagnostic));
                    let mut content = Json::object();
                    content.set("text", Json::string(suggestion.replacement.clone()));
                    replacement.set("insertedContent", content);
                    replacements.push(replacement);
                }
                let mut changes = Json::object();
                let mut artifact = Json::object();
                artifact.set("uri", Json::string(sources.path(diagnostic.file)));
                changes.set("artifactLocation", artifact);
                changes.set("replacements", Json::array(replacements));
                let mut fix = Json::object();
                let mut description = Json::object();
                description.set(
                    "text",
                    Json::string(
                        diagnostic
                            .suggestions
                            .first()
                            .map(|suggestion| suggestion.message.clone())
                            .unwrap_or_else(|| "apply the suggested edit".to_string()),
                    ),
                );
                fix.set("description", description);
                fix.set("artifactChanges", Json::array(vec![changes]));
                result.set("fixes", Json::array(vec![fix]));
            }
            result
        })
        .collect();

    let mut run = Json::object();
    let mut tool_driver = Json::object();
    tool_driver.set("name", Json::string("yashyc"));
    tool_driver.set("rules", Json::array(rules));
    let mut tool = Json::object();
    tool.set("driver", tool_driver);
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

#[cfg(test)]
mod tests {
    use super::*;
    use yashy_common::{Applicability, FileId, Severity, Span, Suggestion};

    #[test]
    fn a_machine_applicable_suggestion_becomes_a_sarif_fix() {
        let mut sources = SourceMap::new();
        let file = sources.add_file(
            "main.ya".to_string(),
            "fn main() { var int x = 1; }\n".to_string(),
        );
        let mut diagnostic = Diagnostic::new(
            Severity::Error,
            file,
            Span::new(12, 15),
            "`var` was removed: use `let`".to_string(),
        );
        diagnostic = diagnostic.with_suggestions(vec![Suggestion {
            span: Span::new(12, 15),
            replacement: "let".to_string(),
            applicability: Applicability::MachineApplicable,
            message: "use `let`".to_string(),
        }]);
        let sarif = to_sarif(&sources, &[diagnostic]).render();
        assert!(sarif.contains("\"fixes\""), "{sarif}");
        assert!(sarif.contains("\"artifactChanges\""), "{sarif}");
        assert!(sarif.contains("\"insertedContent\""), "{sarif}");
        assert!(sarif.contains("\"text\": \"let\""), "{sarif}");
        assert!(sarif.contains("\"startLine\": 1"), "{sarif}");
    }
}
