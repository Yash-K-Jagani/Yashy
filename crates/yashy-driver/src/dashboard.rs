//! Benchmark history: rows keyed by compiler, target and toolchain.
//!
//! A single measurement is evidence about one run. A history is evidence about
//! a trend: each row names the compiler hash, target, toolchain, benchmark key
//! and value it came from. The dashboard renders those rows and a check fails
//! when a row regresses past its budget. A key with no earlier row is
//! "new", never "green" -- a first measurement cannot prove stability.

use std::collections::BTreeMap;
use std::path::Path;

use yashy_common::json_rpc::{self, Value};
use yashy_common::Json;

/// One historical measurement.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub compiler: String,
    pub target: String,
    pub toolchain: String,
    pub key: String,
    pub value: f64,
    pub budget_pct: f64,
}

impl Row {
    fn identity(&self) -> (String, String, String, String) {
        (
            self.compiler.clone(),
            self.target.clone(),
            self.toolchain.clone(),
            self.key.clone(),
        )
    }

    pub fn to_json(&self) -> Json {
        let mut root = Json::object();
        root.set("compiler", Json::string(&self.compiler));
        root.set("target", Json::string(&self.target));
        root.set("toolchain", Json::string(&self.toolchain));
        root.set("key", Json::string(&self.key));
        root.set("value", Json::number(self.value as i64));
        root.set("budget_pct", Json::number(self.budget_pct as i64));
        root
    }
}

/// The history document.
#[derive(Clone, Debug, Default)]
pub struct History {
    pub rows: Vec<Row>,
}

impl History {
    pub fn parse(text: &str) -> Result<History, String> {
        let root =
            json_rpc::parse(text).map_err(|error| format!("history is not JSON: {error}"))?;
        let version = root
            .get("schema_version")
            .and_then(Value::as_i64)
            .ok_or("history has no schema_version")?;
        if version != 1 {
            return Err(format!("unknown history schema {version}"));
        }
        let items = root
            .get("rows")
            .and_then(Value::as_array)
            .ok_or("history has no rows array")?;
        let mut rows = Vec::new();
        for item in items {
            let field = |name: &str| {
                item.get(name)
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| format!("a history row needs a string `{name}`"))
            };
            let number = |name: &str| {
                item.get(name)
                    .and_then(Value::as_i64)
                    .map(|value| value as f64)
                    .ok_or_else(|| format!("a history row needs a number `{name}`"))
            };
            rows.push(Row {
                compiler: field("compiler")?,
                target: field("target")?,
                toolchain: field("toolchain")?,
                key: field("key")?,
                value: number("value")?,
                budget_pct: number("budget_pct")?,
            });
        }
        Ok(History { rows })
    }

    /// Append a measurement. Every observation is kept: the baseline is the
    /// first value for an identity and the current value is the last.
    pub fn append(&mut self, row: Row) {
        self.rows.push(row);
    }

    pub fn to_json(&self) -> Json {
        let mut root = Json::object();
        root.set("schema_version", Json::number(1));
        root.set("tool", Json::string("yashyc"));
        root.set(
            "rows",
            Json::array(self.rows.iter().map(Row::to_json).collect()),
        );
        root
    }

    /// Markdown table with the baseline, current value and percentage change
    /// for each key.
    pub fn render(&self) -> String {
        let mut out = String::from(
            "| Key | Baseline | Current | Delta | Status |\n|---|---:|---:|---:|---|\n",
        );
        for (key, baseline, current, delta, status) in self.compare() {
            out.push_str(&format!(
                "| {} | {:.0} | {:.0} | {:+.1}% | {} |\n",
                key, baseline, current, delta, status
            ));
        }
        out
    }

    /// One tuple per key: baseline, current, percentage change and status.
    /// The baseline is the first value recorded for that identity; `new`
    /// means there is no earlier row to compare against.
    pub fn compare(&self) -> Vec<(String, f64, f64, f64, &'static str)> {
        let mut first: BTreeMap<(String, String, String, String), f64> = BTreeMap::new();
        let mut latest: BTreeMap<(String, String, String, String), (f64, f64)> = BTreeMap::new();
        for row in &self.rows {
            let id = row.identity();
            first.entry(id.clone()).or_insert(row.value);
            latest.insert(id, (row.value, row.budget_pct));
        }
        let mut out = Vec::new();
        let mut ids: Vec<_> = latest.keys().cloned().collect();
        ids.sort();
        for id in ids {
            let (current, budget) = latest[&id];
            let baseline = first[&id];
            let delta = if baseline == 0.0 {
                0.0
            } else {
                (current - baseline) / baseline.abs() * 100.0
            };
            let status = if first[&id] == current
                && self.rows.iter().filter(|r| r.identity() == id).count() == 1
            {
                "new"
            } else if delta > budget {
                "regressed"
            } else {
                "ok"
            };
            out.push((id.3, baseline, current, delta, status));
        }
        out
    }

    /// Whether any row regressed beyond its budget. A `new` row never fails.
    pub fn is_green(&self) -> bool {
        self.compare()
            .iter()
            .all(|(_, _, _, _, status)| *status != "regressed")
    }
}

/// Read a history from `path`; a missing file is an empty history, so a
/// project's first dashboard run records rows instead of failing.
pub fn load(path: &Path) -> Result<History, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => History::parse(&text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(History::default()),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

/// Write a history to `path` in schema v1 form.
pub fn save(path: &Path, history: &History) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    std::fs::write(path, history.to_json().render())
        .map_err(|error| format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(key: &str, value: i64, budget: i64) -> Row {
        Row {
            compiler: "c1".to_string(),
            target: "x86_64".to_string(),
            toolchain: "clang".to_string(),
            key: key.to_string(),
            value: value as f64,
            budget_pct: budget as f64,
        }
    }

    #[test]
    fn a_first_row_is_new_not_green() {
        let mut history = History::default();
        history.append(row("fib", 100, 5));
        let rows = history.compare();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].4, "new");
        assert!(history.is_green());
    }

    #[test]
    fn a_regression_past_budget_fails_and_one_inside_passes() {
        let mut history = History::default();
        history.append(row("fib", 100, 5));
        history.append(row("fib", 110, 5));
        assert!(!history.is_green(), "a 10% regression past 5% must fail");
        let mut inside = History::default();
        inside.append(row("fib", 100, 5));
        inside.append(row("fib", 103, 5));
        assert!(inside.is_green(), "a 3% regression inside 5% passes");
    }

    #[test]
    fn a_bad_schema_is_refused() {
        assert!(History::parse("{\"schema_version\":2,\"rows\":[]}").is_err());
        assert!(History::parse("{}").is_err());
    }
}
