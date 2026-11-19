//! `yashyc profile --sample`: the native sampler's report model.
//!
//! The runtime records image-relative instruction pointers at a fixed
//! interval; this module parses that file, resolves the addresses through the
//! same `llvm-symbolizer` path the event trace uses, and aggregates samples by
//! enclosing function. A sample that cannot be resolved keeps its address as
//! its name rather than disappearing, so the percentages still add up.
//!
//! Nothing here invents a sample: an empty file is an empty report, and the
//! caller falls back to the counter path with a note.

use std::collections::BTreeMap;
use std::path::Path;

use yashy_common::json_rpc::{self, Value};

/// The runtime sample file: image-relative addresses and the overflow count.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SampleFile {
    pub relative: bool,
    pub addresses: Vec<u64>,
    pub dropped: u64,
}

/// Parse the runtime's sample file.
///
/// The shape is one hexadecimal address per line, bracketed by
/// `# relative 0|1` and `# dropped N`. A malformed line is skipped: a sample
/// that cannot be read is not evidence, and guessing one would be worse than
/// a slightly smaller denominator.
pub fn parse(text: &str) -> SampleFile {
    let mut file = SampleFile::default();
    for line in text.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("# relative ") {
            file.relative = value.trim() == "1";
            continue;
        }
        if let Some(value) = line.strip_prefix("# dropped ") {
            file.dropped = value.trim().parse().unwrap_or(file.dropped);
            continue;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Ok(address) = u64::from_str_radix(line, 16) {
            file.addresses.push(address);
        }
    }
    file
}

/// One function's share of the samples.
#[derive(Clone, Debug, PartialEq)]
pub struct SampleRow {
    pub name: String,
    pub samples: u64,
    pub percent: f64,
}

/// A sampling profile, ready to render as a table or schema-v1 JSON.
#[derive(Clone, Debug, PartialEq)]
pub struct SampleReport {
    pub program: String,
    pub total: u64,
    pub dropped: u64,
    pub symbolized: bool,
    pub functions: Vec<SampleRow>,
}

/// Aggregate resolved frames by function name.
///
/// `frames` is parallel to `addresses`; a frame whose function is `??` is
/// named by the address instead, so every sample has an owner. The order is
/// sample count descending, ties by name, which makes the table byte-stable.
pub fn aggregate(
    program: &str,
    addresses: &[u64],
    frames: &[(String, String)],
    dropped: u64,
    symbolized: bool,
) -> SampleReport {
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    for (index, address) in addresses.iter().enumerate() {
        let function = frames
            .get(index)
            .map(|(function, _)| function.as_str())
            .filter(|function| !function.is_empty() && *function != "??")
            .map(str::to_string)
            .unwrap_or_else(|| format!("0x{address:x}"));
        *counts.entry(function).or_default() += 1;
    }
    let total = addresses.len() as u64;
    let mut functions: Vec<SampleRow> = counts
        .into_iter()
        .map(|(name, samples)| SampleRow {
            name,
            samples,
            percent: if total == 0 {
                0.0
            } else {
                samples as f64 * 100.0 / total as f64
            },
        })
        .collect();
    functions.sort_by(|a, b| b.samples.cmp(&a.samples).then_with(|| a.name.cmp(&b.name)));
    SampleReport {
        program: program.to_string(),
        total,
        dropped,
        symbolized,
        functions,
    }
}

impl SampleReport {
    /// The dominant function when it holds at least 80% of the samples.
    ///
    /// This is the plan's attribution bar: a sampler that spreads a hot loop
    /// across everything it touched has not attributed anything.
    pub fn hot(&self) -> Option<&SampleRow> {
        let top = self.functions.first()?;
        if self.total > 0 && top.samples * 5 >= self.total * 4 {
            Some(top)
        } else {
            None
        }
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("program: {}\n", self.program));
        out.push_str(&format!(
            "samples: {} (dropped {})\n",
            self.total, self.dropped
        ));
        for row in &self.functions {
            out.push_str(&format!(
                "{:<32} samples={:<8} {:>6.2}%\n",
                row.name, row.samples, row.percent
            ));
        }
        if !self.symbolized {
            out.push_str(
                "note: llvm-symbolizer was not found; rows are image-relative addresses, not function names.\n",
            );
        }
        out
    }

    pub fn to_json(&self) -> Value {
        let functions: Vec<Value> = self
            .functions
            .iter()
            .map(|row| {
                json_rpc::object([
                    ("name", json_rpc::string(row.name.clone())),
                    ("samples", Value::Num(row.samples as f64)),
                    ("percent", Value::Num(row.percent)),
                ])
            })
            .collect();
        // The attribution verdict travels with the rows: `null` means no
        // function reached 80%, which is a measurement rather than an error.
        let hot = match self.hot() {
            Some(row) => json_rpc::string(row.name.clone()),
            None => Value::Null,
        };
        json_rpc::object([
            ("schema_version", Value::Num(1.0)),
            ("tool", json_rpc::string("yashyc")),
            ("mode", json_rpc::string("sample")),
            ("program", json_rpc::string(self.program.clone())),
            ("total", Value::Num(self.total as f64)),
            ("dropped", Value::Num(self.dropped as f64)),
            ("symbolized", Value::Bool(self.symbolized)),
            ("hot", hot),
            ("functions", Value::Arr(functions)),
        ])
    }
}

/// Resolve addresses and build the report.
///
/// A missing symbolizer is not an error: the report keeps the addresses and
/// says so. A read error is the caller's diagnostic.
pub fn measure(program: &str, file: &SampleFile, binary: &Path) -> SampleReport {
    let addresses: Vec<String> = file.addresses.iter().map(|pc| format!("{pc:x}")).collect();
    match crate::trace::frames(&addresses, binary, file.relative) {
        Some(frames) => aggregate(program, &file.addresses, &frames, file.dropped, true),
        None => aggregate(program, &file.addresses, &[], file.dropped, false),
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn the_runtime_file_is_relative_and_bounded() {
        let file = parse("# relative 1\n1a2b\n3c4d\n# dropped 7\n");
        assert!(file.relative);
        assert_eq!(file.addresses, vec![0x1a2b, 0x3c4d]);
        assert_eq!(file.dropped, 7);
    }

    #[test]
    fn samples_are_attributed_by_function_and_add_up() {
        let frames = vec![
            ("hot".to_string(), "main.ya:2:1".to_string()),
            ("hot".to_string(), "main.ya:2:1".to_string()),
            ("hot".to_string(), "main.ya:2:1".to_string()),
            ("hot".to_string(), "main.ya:2:1".to_string()),
            ("cold".to_string(), "main.ya:8:1".to_string()),
        ];
        let report = aggregate("main.ya", &[1, 2, 3, 4, 5], &frames, 0, true);
        assert_eq!(report.total, 5);
        assert_eq!(report.hot().map(|row| row.name.as_str()), Some("hot"));
        let sum: u64 = report.functions.iter().map(|row| row.samples).sum();
        assert_eq!(sum, report.total);
    }

    #[test]
    fn an_unknown_frame_keeps_its_address_instead_of_vanishing() {
        let report = aggregate("main.ya", &[0x10, 0x20], &[], 0, false);
        assert_eq!(report.total, 2);
        assert_eq!(report.functions.len(), 2);
        assert!(report
            .functions
            .iter()
            .all(|row| row.name.starts_with("0x")));
    }

    #[test]
    fn the_json_has_the_same_rows_as_the_table() {
        let frames = vec![("hot".to_string(), String::new())];
        let report = aggregate("main.ya", &[1], &frames, 0, true);
        let json = json_rpc::to_string(&report.to_json());
        assert!(json.contains("\"mode\":\"sample\""), "{json}");
        assert!(json.contains("\"name\":\"hot\""), "{json}");
        assert!(json.contains("\"hot\":\"hot\""), "{json}");
        assert!(json.contains("\"percent\":100"), "{json}");
    }
}
