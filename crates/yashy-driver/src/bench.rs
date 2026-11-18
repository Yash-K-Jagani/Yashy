//! The benchmark row reader.
//!
//! This module deliberately does not measure anything: a number printed
//! without a row in a file is a claim, not evidence. It reads the `key=value`
//! rows the benchmark scripts write and renders them for humans and JSON, so
//! a baseline file is one fact with two renderings. Measurement and comparison
//! belong to the scripts until `yashyc bench` grows its own runner.

use yashy_common::Json;

/// One measured row: a stable key and its numeric value.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub key: String,
    pub value: f64,
}

/// Parse `key=value` rows, ignoring blanks and `#` comments.
///
/// A malformed line is skipped rather than guessed at: a row that lies about
/// its value is worse than a missing row, and the scripts that write these
/// files are the source of truth.
pub fn parse(text: &str) -> Vec<Row> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Ok(value) = value.trim().parse::<f64>() else {
            continue;
        };
        rows.push(Row {
            key: key.trim().to_string(),
            value,
        });
    }
    rows
}

/// The same rows in the schema-v1 envelope shape.
pub fn to_json(rows: &[Row]) -> Json {
    let entries: Vec<Json> = rows
        .iter()
        .map(|row| {
            let mut object = Json::object();
            object.set("key", Json::string(&row.key));
            object.set("value", Json::string(row.value.to_string()));
            object
        })
        .collect();
    let mut report = Json::object();
    report.set("schema_version", Json::number(1));
    report.set("tool", Json::string("bench"));
    report.set("yashy_version", Json::string(env!("CARGO_PKG_VERSION")));
    report.set("rows", Json::array(entries));
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_and_malformed_lines_are_not_rows() {
        let rows = parse("# note\n\nfib_compile_ms=12.5\nbad line\nempty=\n");
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].key, "fib_compile_ms");
        assert_eq!(rows[0].value, 12.5);
    }
}
