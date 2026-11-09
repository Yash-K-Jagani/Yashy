//! `yashy profile`: a per-function counter report, rendered from JSON.
//!
//! The profiler is a set of static counters — one per function, incremented on
//! entry and exit — not a sampling thread: a sampling profiler changes the
//! scheduling it measures. This module owns the report model only: parsing a
//! recorded report, validating it, and rendering the human table sorted by
//! self time. The instrumentation that fills the counters lives in the
//! codegen; what ships here is the honest part, a report that names the
//! function that dominates and prints its measured overhead next to it.
//!
//! A report is `{"program": ..., "overhead_pct": ..., "functions":
//! [{"name": ..., "calls": ..., "self_ns": ..., "total_ns": ...}]}`. Self
//! time is total minus children, so a parent never double-counts the work a
//! child did.

use yashy_common::json_rpc::{self, Value};

/// One function's counters.
#[derive(Clone, Debug, PartialEq)]
pub struct FunctionRow {
    pub name: String,
    pub calls: u64,
    pub self_ns: u64,
    pub total_ns: u64,
}

/// A parsed profiler report.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub program: String,
    pub overhead_pct: f64,
    pub functions: Vec<FunctionRow>,
}

impl Report {
    /// The rows in report order: highest self time first, ties by name so the
    /// table is byte-identical across runs.
    pub fn sorted(&self) -> Vec<&FunctionRow> {
        let mut rows: Vec<&FunctionRow> = self.functions.iter().collect();
        rows.sort_by(|a, b| b.self_ns.cmp(&a.self_ns).then_with(|| a.name.cmp(&b.name)));
        rows
    }

    /// The dominant function: the first row, when it holds at least half of
    /// all self time.
    pub fn dominant(&self) -> Option<&FunctionRow> {
        let top = self.sorted().into_iter().next()?;
        let total: u64 = self.functions.iter().map(|row| row.self_ns).sum();
        if total > 0 && top.self_ns * 2 >= total {
            Some(top)
        } else {
            None
        }
    }

    /// Render the human table: one line per function, sorted by self time,
    /// with the instrumentation overhead stated as a number.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("program: {}\n", self.program));
        for row in self.sorted() {
            out.push_str(&format!(
                "{:<24} calls={:<8} self_ms={:>10.3} total_ms={:>10.3}\n",
                row.name,
                row.calls,
                row.self_ns as f64 / 1e6,
                row.total_ns as f64 / 1e6,
            ));
        }
        out.push_str(&format!(
            "instrumentation overhead: {:.2}%\n",
            self.overhead_pct
        ));
        out
    }

    /// Render the model back as JSON (the schema-v1 payload).
    pub fn to_json(&self) -> Value {
        let functions: Vec<Value> = self
            .sorted()
            .into_iter()
            .map(|row| {
                json_rpc::object([
                    ("name", json_rpc::string(row.name.clone())),
                    ("calls", Value::Num(row.calls as f64)),
                    ("self_ns", Value::Num(row.self_ns as f64)),
                    ("total_ns", Value::Num(row.total_ns as f64)),
                ])
            })
            .collect();
        json_rpc::object([
            ("schema_version", Value::Num(1.0)),
            ("tool", json_rpc::string("yashy")),
            ("program", json_rpc::string(self.program.clone())),
            ("overhead_pct", Value::Num(self.overhead_pct)),
            ("functions", Value::Arr(functions)),
        ])
    }
}

/// Parse a recorded report. A malformed report is an error, never a table of
/// guesses.
pub fn parse_report(text: &str) -> Result<Report, String> {
    let value = json_rpc::parse(text).map_err(|error| format!("invalid report: {error}"))?;
    let program = value
        .get("program")
        .and_then(Value::as_str)
        .unwrap_or("program")
        .to_string();
    let overhead_pct = match value.get("overhead_pct") {
        Some(Value::Num(number)) => *number,
        _ => 0.0,
    };
    let functions = match value.get("functions").and_then(Value::as_array) {
        Some(items) => items,
        None => return Err("the report has no `functions` array".to_string()),
    };
    let mut rows = Vec::new();
    for item in functions {
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "a function row has no `name`".to_string())?
            .to_string();
        let field = |key: &str| -> Result<u64, String> {
            match item.get(key).and_then(Value::as_i64) {
                Some(value) if value >= 0 => Ok(value as u64),
                _ => Err(format!("function `{}` has no non-negative `{key}`", name)),
            }
        };
        let calls = field("calls")?;
        let self_ns = field("self_ns")?;
        let total_ns = field("total_ns")?;
        rows.push(FunctionRow {
            name,
            calls,
            self_ns,
            total_ns,
        });
    }
    if rows.is_empty() {
        return Err("the report records no functions".to_string());
    }
    Ok(Report {
        program,
        overhead_pct,
        functions: rows,
    })
}

/// The LLDB command file a debug run leaves behind: a breakpoint on the
/// entry function and, on stop, the executor's task table. The file is
/// generated per run and is syntactically valid on its own; launching under
/// LLDB is the user's choice (`lldb -s <file> run`), and the same stop
/// behaviour is available without a debugger through `YASHY_DEBUG=1`.
pub fn lldb_commands(entry: &str) -> String {
    format!(
        "breakpoint set --name {entry}\n\
         breakpoint command add 1\n\
         silent\n\
         process print\n\
         continue\n\
         done\n\
         run\n"
    )
}

#[cfg(test)]
mod unit {
    use super::*;

    const SAMPLE: &str = r#"{
  "program": "workload",
  "overhead_pct": 12.5,
  "functions": [
    {"name": "small", "calls": 10, "self_ns": 1000, "total_ns": 2000},
    {"name": "hot", "calls": 1, "self_ns": 9000, "total_ns": 10000}
  ]
}"#;

    #[test]
    fn a_dominant_function_is_named_and_the_table_is_sorted() {
        let report = parse_report(SAMPLE).expect("the sample parses");
        assert_eq!(report.sorted()[0].name, "hot");
        assert_eq!(report.dominant().map(|row| row.name.as_str()), Some("hot"));
        let text = report.render();
        assert!(
            text.contains("overhead"),
            "the overhead is printed:\n{text}"
        );
    }

    #[test]
    fn a_malformed_report_is_an_error_not_a_table() {
        assert!(parse_report("{}").is_err());
        assert!(parse_report("not json").is_err());
    }

    #[test]
    fn the_lldb_file_is_balanced_and_names_the_entry() {
        let file = lldb_commands("main");
        assert!(file.contains("main"), "the entry breakpoint:\n{file}");
        assert_eq!(file.matches('{').count(), file.matches('}').count());
    }
}
