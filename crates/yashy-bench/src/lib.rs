//! The benchmark comparator: read two `key=value` measurement files, apply the
//! thresholds, and say which rows regressed.
//!
//! The measurement scripts are platform-native (`ci.ps1`, `ci.sh`) but the
//! comparison is one implementation on both, because a threshold that means
//! 20% on Windows and 18% on Linux is not a threshold. The format is small
//! enough that the parser here is hand-written: no dependency, no offline
//! build surprise.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

/// The default percent change allowed for a compile-time row.
pub const DEFAULT_COMPILE_PCT: f64 = 20.0;
/// The default percent change allowed for a binary-size row.
pub const DEFAULT_BINARY_PCT: f64 = 10.0;
/// The default percent change allowed for a run-time row.
pub const DEFAULT_RUNTIME_PCT: f64 = 20.0;

/// A malformed line, with the one-based line number that carried it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

/// One measurement file: metadata and gate rows, in the order they appeared.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Measurement {
    fields: BTreeMap<String, String>,
    order: Vec<String>,
}

impl Measurement {
    /// Parse `key=value` lines. Blank lines and `#` comments are ignored.
    ///
    /// A line without `=` is an error, and so is a duplicate key: a file that
    /// names `fib_run_ms` twice does not have one answer.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        let mut measurement = Measurement::default();
        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let Some((key, value)) = trimmed.split_once('=') else {
                return Err(ParseError {
                    line,
                    message: format!("expected `key=value`, found `{trimmed}`"),
                });
            };
            let key = key.trim();
            let value = value.trim();
            if key.is_empty() {
                return Err(ParseError {
                    line,
                    message: "the key is empty".to_string(),
                });
            }
            if measurement.fields.contains_key(key) {
                return Err(ParseError {
                    line,
                    message: format!("duplicate key `{key}`"),
                });
            }
            measurement
                .fields
                .insert(key.to_string(), value.to_string());
            measurement.order.push(key.to_string());
        }
        Ok(measurement)
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields.get(key).map(String::as_str)
    }

    pub fn number(&self, key: &str) -> Option<f64> {
        self.get(key)?.parse().ok()
    }

    /// The rows the gate understands: compile time, binary size, run time.
    ///
    /// Metadata (`os`, `commit`, ...) and any unrecognised key are not rows.
    pub fn gate_rows(&self) -> Vec<&str> {
        self.order
            .iter()
            .map(String::as_str)
            .filter(|row| kind_of(row).is_some())
            .collect()
    }

    /// The fields in file order, for rewriting a baseline.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &str)> {
        self.order.iter().filter_map(|key| {
            self.fields
                .get(key)
                .map(|value| (key.as_str(), value.as_str()))
        })
    }
}

/// What a row measures, taken from its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Compile,
    Binary,
    Runtime,
    Count,
}

fn kind_of(row: &str) -> Option<Kind> {
    if row.ends_with("_compile_ms") {
        Some(Kind::Compile)
    } else if row.ends_with("_binary_bytes") {
        Some(Kind::Binary)
    } else if row.ends_with("_run_ms") {
        Some(Kind::Runtime)
    } else if row == "instantiation_count" {
        // Generic instantiations are a compile-time cost: a pass that
        // instantiates more is held to the compile threshold.
        Some(Kind::Count)
    } else {
        None
    }
}

/// The percent change each kind of row may move before it fails.
#[derive(Clone, Debug, PartialEq)]
pub struct Thresholds {
    pub compile: f64,
    pub binary: f64,
    pub runtime: f64,
    /// Per-row overrides, in percent, keyed by row name.
    pub per_row: BTreeMap<String, f64>,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds {
            compile: DEFAULT_COMPILE_PCT,
            binary: DEFAULT_BINARY_PCT,
            runtime: DEFAULT_RUNTIME_PCT,
            per_row: BTreeMap::new(),
        }
    }
}

impl Thresholds {
    /// Parse `compile=20`, `binary=10`, `runtime=20`, and any other key as a
    /// per-row override in percent.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        let mut thresholds = Thresholds::default();
        let parsed = Measurement::parse(text)?;
        for (key, value) in parsed.entries() {
            let percent: f64 = value.parse().map_err(|_| ParseError {
                line: 0,
                message: format!("`{key}` needs a number, found `{value}`"),
            })?;
            if percent < 0.0 {
                return Err(ParseError {
                    line: 0,
                    message: format!("`{key}` cannot be negative"),
                });
            }
            match key {
                "compile" => thresholds.compile = percent,
                "binary" => thresholds.binary = percent,
                "runtime" => thresholds.runtime = percent,
                _ => {
                    thresholds.per_row.insert(key.to_string(), percent);
                }
            }
        }
        Ok(thresholds)
    }

    /// The threshold for one row: a per-row override, else its kind's default.
    pub fn for_row(&self, row: &str) -> Option<f64> {
        self.per_row.get(row).copied().or_else(|| {
            kind_of(row).map(|kind| match kind {
                Kind::Compile | Kind::Count => self.compile,
                Kind::Binary => self.binary,
                Kind::Runtime => self.runtime,
            })
        })
    }
}

/// How one row fared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Within its threshold.
    Pass,
    /// Over its threshold.
    Fail,
    /// The baseline has the row and the current measurement does not.
    Missing,
    /// The current measurement has a row the baseline does not; not gated.
    New,
    /// The baseline is zero or negative, so a percentage is meaningless.
    Unmeasurable,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Pass => "pass",
            Verdict::Fail => "FAIL",
            Verdict::Missing => "MISSING",
            Verdict::New => "new",
            Verdict::Unmeasurable => "n/a",
        }
    }

    pub fn is_failure(self) -> bool {
        matches!(self, Verdict::Fail | Verdict::Missing)
    }
}

/// One row's comparison, ready to print.
#[derive(Clone, Debug, PartialEq)]
pub struct RowResult {
    pub row: String,
    pub baseline: Option<f64>,
    pub current: Option<f64>,
    pub change_pct: Option<f64>,
    pub threshold_pct: Option<f64>,
    pub verdict: Verdict,
}

/// Compare the gate rows of two measurements.
///
/// Baseline order comes first, then any row that is new in `current`. A row
/// the baseline has and `current` lost is `Missing`: an incomplete measurement
/// is not a pass.
pub fn compare(
    baseline: &Measurement,
    current: &Measurement,
    thresholds: &Thresholds,
) -> Vec<RowResult> {
    let current_rows = current.gate_rows();
    let mut results = Vec::new();
    for row in baseline.gate_rows() {
        let base = baseline.number(row);
        let threshold = thresholds.for_row(row);
        if !current_rows.contains(&row) {
            results.push(RowResult {
                row: row.to_string(),
                baseline: base,
                current: None,
                change_pct: None,
                threshold_pct: threshold,
                verdict: Verdict::Missing,
            });
            continue;
        }
        let cur = current.number(row);
        let (change_pct, verdict) = match (base, cur) {
            (Some(base), Some(cur)) if base > 0.0 => {
                let change = (cur - base) / base * 100.0;
                let verdict = match threshold {
                    Some(limit) if change > limit => Verdict::Fail,
                    _ => Verdict::Pass,
                };
                (Some(change), verdict)
            }
            _ => (None, Verdict::Unmeasurable),
        };
        results.push(RowResult {
            row: row.to_string(),
            baseline: base,
            current: cur,
            change_pct,
            threshold_pct: threshold,
            verdict,
        });
    }
    for row in current_rows {
        if baseline.gate_rows().contains(&row) {
            continue;
        }
        results.push(RowResult {
            row: row.to_string(),
            baseline: None,
            current: current.number(row),
            change_pct: None,
            threshold_pct: thresholds.for_row(row),
            verdict: Verdict::New,
        });
    }
    results
}

/// Render the comparison as a fixed-width table.
pub fn format_table(results: &[RowResult]) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<24} {:>12} {:>12} {:>10}  {}",
        "row", "baseline", "current", "change", "verdict"
    );
    for result in results {
        let baseline = result
            .baseline
            .map(|value| format!("{value:.1}"))
            .unwrap_or_else(|| "-".to_string());
        let current = result
            .current
            .map(|value| format!("{value:.1}"))
            .unwrap_or_else(|| "-".to_string());
        let change = result
            .change_pct
            .map(|value| format!("{value:+.1}%"))
            .unwrap_or_else(|| "n/a".to_string());
        let _ = writeln!(
            out,
            "{:<24} {:>12} {:>12} {:>10}  {}",
            result.row,
            baseline,
            current,
            change,
            result.verdict.label()
        );
    }
    out
}

/// Rewrite `baseline` from `current` after a deliberate update.
/// The current file must parse and must carry at least one gate row; a
/// half-written measurement is not a baseline.
pub fn update(current_path: &Path, baseline_path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(current_path)
        .map_err(|error| format!("cannot read {}: {error}", current_path.display()))?;
    let measurement = Measurement::parse(&text)
        .map_err(|error| format!("{}: {error}", current_path.display()))?;
    if measurement.gate_rows().is_empty() {
        return Err(format!(
            "{} has no gate rows; refusing to write a baseline",
            current_path.display()
        ));
    }
    let mut out = String::from("# generated by `yashy-bench update`; update deliberately\n");
    for (key, value) in measurement.entries() {
        let _ = writeln!(out, "{key}={value}");
    }
    std::fs::write(baseline_path, out)
        .map_err(|error| format!("cannot write {}: {error}", baseline_path.display()))
}

/// The percent by which disabling a pass may *improve* its own row before the
/// pass fails the gate.
///
/// The gate exists so a pass cannot buy one row by selling another. If turning
/// a pass off makes the row it is held to more than 2% better, the pass is not
/// paying for itself and the per-pass gate says so.
pub const PER_PASS_PCT: f64 = 2.0;

/// One pass's comparison: the enabled measurement against the disabled one.
#[derive(Clone, Debug, PartialEq)]
pub struct PassResult {
    pub pass: String,
    pub row: String,
    pub baseline: Option<f64>,
    pub current: Option<f64>,
    /// `(disabled - enabled) / enabled`, in percent. Positive means disabling
    /// the pass made the row worse, which is the pass earning its place.
    pub delta_pct: Option<f64>,
    pub verdict: Verdict,
}

/// Compare every registered pass's own row with the measurement taken while
/// that pass was disabled.
///
/// `registry` is the compiler's pass table as `(name, row)` pairs. A pass with
/// no disabled measurement, or a row missing from either file, is `Missing`: an
/// unmeasured pass is not a pass.
pub fn per_pass(
    baseline: &Measurement,
    without: &[(String, Measurement)],
    registry: &[(&str, &str)],
) -> Vec<PassResult> {
    let mut results = Vec::new();
    for &(pass, row) in registry {
        let base = baseline.number(row);
        let current = without
            .iter()
            .find(|(name, _)| name == pass)
            .and_then(|(_, measurement)| measurement.number(row));
        let (delta_pct, verdict) = match (base, current) {
            (Some(base), Some(current)) if base > 0.0 => {
                let delta = (current - base) / base * 100.0;
                let verdict = if delta < -PER_PASS_PCT {
                    Verdict::Fail
                } else {
                    Verdict::Pass
                };
                (Some(delta), verdict)
            }
            _ => (None, Verdict::Missing),
        };
        results.push(PassResult {
            pass: pass.to_string(),
            row: row.to_string(),
            baseline: base,
            current,
            delta_pct,
            verdict,
        });
    }
    results
}

/// Render the per-pass comparison as the plan's table.
pub fn format_per_pass(results: &[PassResult]) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<8} | {:<20} | {:>10} | {:>10} | {:>9} | {}",
        "pass", "row", "baseline", "current", "delta", "verdict"
    );
    for result in results {
        let baseline = result
            .baseline
            .map(|value| format!("{value:.1}"))
            .unwrap_or_else(|| "-".to_string());
        let current = result
            .current
            .map(|value| format!("{value:.1}"))
            .unwrap_or_else(|| "-".to_string());
        let delta = result
            .delta_pct
            .map(|value| format!("{value:+.1}%"))
            .unwrap_or_else(|| "n/a".to_string());
        let _ = writeln!(
            out,
            "{:<8} | {:<20} | {:>10} | {:>10} | {:>9} | {}",
            result.pass,
            result.row,
            baseline,
            current,
            delta,
            result.verdict.label()
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    const BASELINE: &str = "\
# baseline
os=linux
generated_at=2026-10-26T00:00:00Z
commit=abc1234
toolchain=clang 18
fib_compile_ms=100.0
fib_binary_bytes=1000
fib_run_ms=50.0
";

    fn measurement(text: &str) -> Measurement {
        Measurement::parse(text).expect("the fixture parses")
    }

    fn compare_with(current: &str) -> Vec<RowResult> {
        compare(
            &measurement(BASELINE),
            &measurement(current),
            &Thresholds::default(),
        )
    }

    fn row<'a>(results: &'a [RowResult], name: &str) -> &'a RowResult {
        results
            .iter()
            .find(|result| result.row == name)
            .unwrap_or_else(|| panic!("no result for `{name}`"))
    }

    #[test]
    fn comments_and_blanks_are_ignored() {
        let parsed = measurement("# comment\n\nos=linux\n\nfib_run_ms=1.0\n");
        assert_eq!(parsed.get("os"), Some("linux"));
        assert_eq!(parsed.number("fib_run_ms"), Some(1.0));
    }

    #[test]
    fn a_malformed_line_is_an_error() {
        let error = Measurement::parse("os=linux\nnot-a-pair\n").expect_err("should fail");
        assert_eq!(error.line, 2);
        assert!(error.message.contains("key=value"), "{error}");
    }

    #[test]
    fn a_duplicate_key_is_an_error() {
        let error = Measurement::parse("fib_run_ms=1\nfib_run_ms=2\n").expect_err("should fail");
        assert_eq!(error.line, 2);
        assert!(error.message.contains("duplicate"), "{error}");
    }

    #[test]
    fn a_row_within_the_threshold_passes() {
        let results =
            compare_with("fib_compile_ms=110.0\nfib_binary_bytes=1050\nfib_run_ms=55.0\n");
        assert_eq!(row(&results, "fib_compile_ms").verdict, Verdict::Pass);
        assert_eq!(row(&results, "fib_binary_bytes").verdict, Verdict::Pass);
        assert_eq!(row(&results, "fib_run_ms").verdict, Verdict::Pass);
        assert!(results.iter().all(|result| !result.verdict.is_failure()));
    }

    #[test]
    fn a_compile_regression_fails() {
        let results =
            compare_with("fib_compile_ms=121.0\nfib_binary_bytes=1000\nfib_run_ms=50.0\n");
        assert_eq!(row(&results, "fib_compile_ms").verdict, Verdict::Fail);
        assert!(row(&results, "fib_compile_ms").verdict.is_failure());
    }

    #[test]
    fn a_binary_size_regression_fails() {
        let results =
            compare_with("fib_compile_ms=100.0\nfib_binary_bytes=1110\nfib_run_ms=50.0\n");
        assert_eq!(row(&results, "fib_binary_bytes").verdict, Verdict::Fail);
    }

    #[test]
    fn a_runtime_regression_fails() {
        let results =
            compare_with("fib_compile_ms=100.0\nfib_binary_bytes=1000\nfib_run_ms=61.0\n");
        assert_eq!(row(&results, "fib_run_ms").verdict, Verdict::Fail);
    }

    #[test]
    fn the_container_workloads_gate_as_runtime_rows() {
        let parsed = measurement("wordcount_run_ms=100.0\nmaps_run_ms=100.0\n");
        assert_eq!(parsed.gate_rows(), vec!["wordcount_run_ms", "maps_run_ms"]);
        assert_eq!(
            Thresholds::default().for_row("wordcount_run_ms"),
            Some(DEFAULT_RUNTIME_PCT)
        );
        assert_eq!(
            Thresholds::default().for_row("maps_run_ms"),
            Some(DEFAULT_RUNTIME_PCT)
        );

        let current = measurement("wordcount_run_ms=121.0\nmaps_run_ms=121.0\n");
        let results = compare(&parsed, &current, &Thresholds::default());
        assert_eq!(row(&results, "wordcount_run_ms").verdict, Verdict::Fail);
        assert_eq!(row(&results, "maps_run_ms").verdict, Verdict::Fail);
    }

    #[test]
    fn the_release_row_gates_as_a_runtime_row() {
        let parsed = measurement("release_run_ms=100.0\n");
        assert_eq!(parsed.gate_rows(), vec!["release_run_ms"]);
        assert_eq!(
            Thresholds::default().for_row("release_run_ms"),
            Some(DEFAULT_RUNTIME_PCT)
        );

        let current = measurement("release_run_ms=121.0\n");
        let results = compare(&parsed, &current, &Thresholds::default());
        assert_eq!(row(&results, "release_run_ms").verdict, Verdict::Fail);
    }

    #[test]
    fn a_missing_row_is_reported() {
        let results = compare_with("fib_compile_ms=100.0\nfib_binary_bytes=1000\n");
        assert_eq!(row(&results, "fib_run_ms").verdict, Verdict::Missing);
        assert!(row(&results, "fib_run_ms").verdict.is_failure());
    }

    #[test]
    fn a_new_row_is_reported_but_not_gated() {
        let results = compare_with(
            "fib_compile_ms=100.0\nfib_binary_bytes=1000\nfib_run_ms=50.0\nhello_compile_ms=80.0\n",
        );
        let hello = row(&results, "hello_compile_ms");
        assert_eq!(hello.verdict, Verdict::New);
        assert!(!hello.verdict.is_failure());
    }

    #[test]
    fn a_zero_baseline_is_not_a_division_by_zero() {
        let baseline = measurement("fib_compile_ms=0\nfib_binary_bytes=1000\nfib_run_ms=50.0\n");
        let current = measurement("fib_compile_ms=10\nfib_binary_bytes=1000\nfib_run_ms=50.0\n");
        let results = compare(&baseline, &current, &Thresholds::default());
        let compile = row(&results, "fib_compile_ms");
        assert_eq!(compile.verdict, Verdict::Unmeasurable);
        assert_eq!(compile.change_pct, None);
        assert!(!compile.verdict.is_failure());
    }

    #[test]
    fn a_negative_baseline_is_unmeasurable() {
        let baseline = measurement("fib_compile_ms=-5\nfib_binary_bytes=1000\nfib_run_ms=50.0\n");
        let current = measurement("fib_compile_ms=10\nfib_binary_bytes=1000\nfib_run_ms=50.0\n");
        let results = compare(&baseline, &current, &Thresholds::default());
        assert_eq!(
            row(&results, "fib_compile_ms").verdict,
            Verdict::Unmeasurable
        );
    }

    #[test]
    fn a_per_row_threshold_overrides_the_default() {
        let baseline = measurement("large_compile_ms=1000\n");
        let current = measurement("large_compile_ms=1350\n");
        let default = compare(&baseline, &current, &Thresholds::default());
        assert_eq!(row(&default, "large_compile_ms").verdict, Verdict::Fail);

        let thresholds = Thresholds::parse("compile=20\nlarge_compile_ms=40\n").expect("parses");
        let overridden = compare(&baseline, &current, &thresholds);
        assert_eq!(row(&overridden, "large_compile_ms").verdict, Verdict::Pass);
    }

    #[test]
    fn the_percent_change_is_signed() {
        let results = compare_with("fib_compile_ms=90.0\nfib_binary_bytes=1000\nfib_run_ms=50.0\n");
        let change = row(&results, "fib_compile_ms")
            .change_pct
            .expect("has a change");
        assert!((change + 10.0).abs() < 1e-9, "got {change}");
    }

    #[test]
    fn the_table_names_every_row() {
        let results =
            compare_with("fib_compile_ms=121.0\nfib_binary_bytes=1000\nfib_run_ms=50.0\n");
        let table = format_table(&results);
        assert!(table.contains("fib_compile_ms"));
        assert!(table.contains("FAIL"));
        assert!(table.contains("+21.0%"));
    }

    #[test]
    fn a_disabled_pass_within_two_percent_passes() {
        let baseline = measurement("fib_run_ms=100.0\nfib_binary_bytes=1000\n");
        let without = vec![
            (
                "facts".to_string(),
                measurement("fib_run_ms=103.0\nfib_binary_bytes=1000\n"),
            ),
            (
                "intern".to_string(),
                measurement("fib_run_ms=100.0\nfib_binary_bytes=1010\n"),
            ),
        ];
        let results = per_pass(
            &baseline,
            &without,
            &[("facts", "fib_run_ms"), ("intern", "fib_binary_bytes")],
        );
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].verdict, Verdict::Pass);
        assert_eq!(results[1].verdict, Verdict::Pass);
        let facts = results[0].delta_pct.expect("has a delta");
        assert!((facts - 3.0).abs() < 1e-9, "got {facts}");
    }

    #[test]
    fn a_disabled_pass_that_improves_its_row_by_more_than_two_percent_fails() {
        let baseline = measurement("maps_run_ms=100.0\n");
        let without = vec![("escape".to_string(), measurement("maps_run_ms=90.0\n"))];
        let results = per_pass(&baseline, &without, &[("escape", "maps_run_ms")]);
        let row = &results[0];
        assert_eq!(row.verdict, Verdict::Fail);
        assert!(row.verdict.is_failure());
        let delta = row.delta_pct.expect("has a delta");
        assert!((delta + 10.0).abs() < 1e-9, "got {delta}");
    }

    #[test]
    fn a_pass_without_a_disabled_measurement_is_missing() {
        let baseline = measurement("fib_run_ms=100.0\n");
        let results = per_pass(&baseline, &[], &[("facts", "fib_run_ms")]);
        assert_eq!(results[0].verdict, Verdict::Missing);
        assert!(results[0].verdict.is_failure());
        assert_eq!(results[0].current, None);
    }

    #[test]
    fn the_per_pass_table_has_one_row_per_registered_pass() {
        let baseline = measurement("fib_run_ms=100.0\nmaps_run_ms=100.0\n");
        let without = vec![
            ("facts".to_string(), measurement("fib_run_ms=101.0\n")),
            ("escape".to_string(), measurement("maps_run_ms=101.0\n")),
        ];
        let registry = [("facts", "fib_run_ms"), ("escape", "maps_run_ms")];
        let results = per_pass(&baseline, &without, &registry);
        assert_eq!(results.len(), registry.len());
        let table = format_per_pass(&results);
        let rows = table
            .lines()
            .filter(|line| line.contains("fib_run_ms") || line.contains("maps_run_ms"))
            .count();
        assert_eq!(rows, registry.len(), "{table}");
        assert!(table.starts_with("pass"), "{table}");
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "yashy-bench-{}-{name}-{n}.bench",
            std::process::id()
        ))
    }

    #[test]
    #[test]
    fn instantiation_count_is_a_gate_row() {
        let parsed = measurement("instantiation_count=10\n");
        assert_eq!(parsed.gate_rows(), vec!["instantiation_count"]);
        assert_eq!(
            Thresholds::default().for_row("instantiation_count"),
            Some(DEFAULT_COMPILE_PCT)
        );
        let current = measurement("instantiation_count=13\n");
        let results = compare(&parsed, &current, &Thresholds::default());
        assert_eq!(row(&results, "instantiation_count").verdict, Verdict::Fail);
    }

    #[test]
    fn update_writes_the_baseline() {
        let current = temp_path("current");
        let baseline = temp_path("baseline");
        std::fs::write(&current, "os=windows\nfib_compile_ms=101.5\n").expect("write current");

        update(&current, &baseline).expect("update succeeds");

        let text = std::fs::read_to_string(&baseline).expect("read baseline");
        let parsed = measurement(&text);
        assert_eq!(parsed.get("os"), Some("windows"));
        assert_eq!(parsed.number("fib_compile_ms"), Some(101.5));
        assert!(text.starts_with('#'), "the baseline keeps a header");

        let _ = std::fs::remove_file(&current);
        let _ = std::fs::remove_file(&baseline);
    }

    #[test]
    fn update_refuses_a_file_without_gate_rows() {
        let current = temp_path("empty");
        let baseline = temp_path("never");
        std::fs::write(&current, "os=linux\n").expect("write current");
        let error = update(&current, &baseline).expect_err("no rows, no baseline");
        assert!(error.contains("no gate rows"), "{error}");
        assert!(!baseline.exists());
        let _ = std::fs::remove_file(&current);
    }
}
