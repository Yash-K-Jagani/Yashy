//! SARIF 2.1.0 rendering of diagnostics.
//!
//! The CLI's schema v1 is the human/tool contract; SARIF is the interchange
//! form CI uploads. Both are rendered from the same `Diagnostic` values.

use yashy_common::Diagnostic;

/// A minimal SARIF 2.1.0 log with rules, results and physical locations.
pub fn to_sarif(diagnostics: &[Diagnostic]) -> yashy_common::Json {
    use yashy_common::Json;

    let mut rules: Vec<yashy_common::Json> = Vec::new();
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

    let results: Vec<yashy_common::Json> = diagnostics
        .iter()
        .map(|diagnostic| {
            let mut result = Json::object();
            let rule_id = diagnostic.code.unwrap_or("E0000");
            result.set("ruleId", Json::string(rule_id));
            result.set(
                "level",
                Json::string(if diagnostic.is_error() { "error" } else { "warning" }),
            );
            let mut message = Json::object();
            message.set("text", Json::string(diagnostic.message.clone()));
            result.set("message", message);
            let mut region = Json::object();
            region.set("startLine", Json::number(i64::from(diagnostic.span.start)));
            region.set("endLine", Json::number(i64::from(diagnostic.span.end)));
            let mut physical = Json::object();
            physical.set("region", region);
            let mut location = Json::object();
            location.set("physicalLocation", physical);
            result.set("locations", Json::array(vec![location]));
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
    log.set("$schema", Json::string("https://json.schemastore.org/sarif-2.1.0.json"));
    log.set("runs", Json::array(vec![run]));
    log
}
