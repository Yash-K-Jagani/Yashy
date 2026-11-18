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

/// One row compared against its baseline.
#[derive(Clone, Debug, PartialEq)]
pub struct Diff {
    pub key: String,
    pub baseline: f64,
    pub actual: f64,
    /// `actual / baseline`, or 0 when the baseline was zero.
    pub ratio: f64,
    /// Allowed movement in percent.
    pub threshold: f64,
    pub regressed: bool,
}

/// The threshold that applies to a row: an exact override, then its kind,
/// then the default from a `default=` line.
pub fn threshold_for(key: &str, thresholds: &[Row]) -> f64 {
    let default = thresholds
        .iter()
        .find(|row| row.key == "default")
        .map(|row| row.value)
        .unwrap_or(20.0);
    if let Some(exact) = thresholds.iter().find(|row| row.key == key) {
        return exact.value;
    }
    let kind = if key.ends_with("_compile_ms") {
        "compile"
    } else if key.ends_with("_binary_bytes") {
        "binary"
    } else if key.ends_with("_run_ms") {
        "runtime"
    } else {
        ""
    };
    thresholds
        .iter()
        .find(|row| row.key == kind)
        .map(|row| row.value)
        .unwrap_or(default)
}

/// Compare current rows with a saved baseline.
///
/// A key missing from the baseline is reported with a zero base and `ratio`
/// `0` rather than skipped: a new row is a new claim and must be read. A
/// regression beyond the row's threshold fails the check.
pub fn compare(current: &[Row], baseline: &[Row], thresholds: &[Row]) -> Vec<Diff> {
    current
        .iter()
        .map(|row| {
            let base = baseline
                .iter()
                .find(|entry| entry.key == row.key)
                .map(|entry| entry.value)
                .unwrap_or(0.0);
            let ratio = if base == 0.0 { 0.0 } else { row.value / base };
            let threshold = threshold_for(&row.key, thresholds);
            Diff {
                key: row.key.clone(),
                baseline: base,
                actual: row.value,
                ratio,
                threshold,
                regressed: base != 0.0 && (ratio - 1.0).abs() * 100.0 > threshold,
            }
        })
        .collect()
}

/// The same comparison in the schema-v1 envelope.
pub fn compare_json(diffs: &[Diff]) -> Json {
    let rows: Vec<Json> = diffs
        .iter()
        .map(|diff| {
            let mut object = Json::object();
            object.set("key", Json::string(&diff.key));
            object.set("baseline", Json::string(diff.baseline.to_string()));
            object.set("actual", Json::string(diff.actual.to_string()));
            object.set("ratio", Json::string(diff.ratio.to_string()));
            object.set("threshold", Json::string(diff.threshold.to_string()));
            object.set("regressed", Json::bool(diff.regressed));
            object
        })
        .collect();
    let mut report = Json::object();
    report.set("schema_version", Json::number(1));
    report.set("tool", Json::string("bench"));
    report.set("yashy_version", Json::string(env!("CARGO_PKG_VERSION")));
    report.set("comparison", Json::array(rows));
    report
}

/// The human page, generated from the same rows as the JSON model.
pub fn render_comparison(diffs: &[Diff]) -> String {
    let mut out = String::from("key baseline actual ratio threshold\n");
    for diff in diffs {
        out.push_str(&format!(
            "{} {} {} {:.3} {}%{}\n",
            diff.key,
            diff.baseline,
            diff.actual,
            diff.ratio,
            diff.threshold,
            if diff.regressed { " REGRESSED" } else { "" }
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_regression_beyond_the_threshold_is_flagged() {
        let current = parse("fib_run_ms=150\nfib_compile_ms=110\n");
        let baseline = parse("fib_run_ms=100\nfib_compile_ms=100\n");
        let thresholds = parse("runtime=20\ncompile=100\n");
        let diffs = compare(&current, &baseline, &thresholds);
        assert!(diffs[0].regressed, "50% over a 20% gate: {:?}", diffs[0]);
        assert!(!diffs[1].regressed, "10% under a 100% gate: {:?}", diffs[1]);
    }

    #[test]
    fn an_exact_override_beats_the_kind_threshold() {
        let thresholds = parse("runtime=20\nfib_run_ms=100\n");
        assert_eq!(threshold_for("fib_run_ms", &thresholds), 100.0);
        assert_eq!(threshold_for("other_run_ms", &thresholds), 20.0);
        assert_eq!(threshold_for("unknown", &thresholds), 20.0);
    }

    #[test]
    fn comments_and_malformed_lines_are_not_rows() {
        let rows = parse("# note\n\nfib_compile_ms=12.5\nbad line\nempty=\n");
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].key, "fib_compile_ms");
        assert_eq!(rows[0].value, 12.5);
    }
}
