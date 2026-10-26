//! `yashy-bench`: compare a fresh measurement with a baseline, or adopt a new
//! baseline after a deliberate change.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use yashy_bench::{compare, format_table, update, Measurement, Thresholds};

const USAGE: &str = "\
yashy-bench - compare two benchmark measurement files

Usage:
  yashy-bench compare --baseline <file> --current <file> [--thresholds <file>] [--report-only]
  yashy-bench update  --current <file> --baseline <file>

`--thresholds` defaults to benchmarks/thresholds.bench when that file exists.

Exit codes:
  0  every gate row is within its threshold
  1  a row regressed, or the current measurement lost a row
  2  usage or file error
";

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(message) => {
            eprintln!("yashy-bench: {message}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        return Err(format!("missing command\n\n{USAGE}"));
    };
    match command.as_str() {
        "compare" => compare_command(args.collect()),
        "update" => update_command(args.collect()),
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        other => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    }
}

#[derive(Default)]
struct Flags {
    baseline: Option<PathBuf>,
    current: Option<PathBuf>,
    thresholds: Option<PathBuf>,
    report_only: bool,
}

fn parse_flags(args: Vec<String>) -> Result<Flags, String> {
    let mut flags = Flags::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--baseline" => {
                flags.baseline = Some(PathBuf::from(next_value(&mut args, "--baseline")?));
            }
            "--current" => {
                flags.current = Some(PathBuf::from(next_value(&mut args, "--current")?));
            }
            "--thresholds" => {
                flags.thresholds = Some(PathBuf::from(next_value(&mut args, "--thresholds")?));
            }
            "--report-only" => flags.report_only = true,
            other => return Err(format!("unknown argument `{other}`\n\n{USAGE}")),
        }
    }
    Ok(flags)
}

fn next_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("`{flag}` needs a value\n\n{USAGE}"))
}

fn compare_command(args: Vec<String>) -> Result<ExitCode, String> {
    let flags = parse_flags(args)?;
    let baseline_path = flags
        .baseline
        .ok_or_else(|| format!("compare needs --baseline <file>\n\n{USAGE}"))?;
    let current_path = flags
        .current
        .ok_or_else(|| format!("compare needs --current <file>\n\n{USAGE}"))?;

    let baseline = read_measurement(&baseline_path)?;
    let current = read_measurement(&current_path)?;
    let thresholds = read_thresholds(flags.thresholds.as_deref())?;

    let results = compare(&baseline, &current, &thresholds);
    print!("{}", format_table(&results));

    let failures = results
        .iter()
        .filter(|result| result.verdict.is_failure())
        .count();
    if failures == 0 {
        println!("all {} gate rows within threshold", results.len());
        return Ok(ExitCode::SUCCESS);
    }
    if flags.report_only {
        println!("{failures} row(s) outside threshold (report only)");
        return Ok(ExitCode::SUCCESS);
    }
    println!("{failures} row(s) outside threshold");
    Ok(ExitCode::from(1))
}

fn update_command(args: Vec<String>) -> Result<ExitCode, String> {
    let flags = parse_flags(args)?;
    if flags.report_only || flags.thresholds.is_some() {
        return Err(format!(
            "update takes only --current and --baseline\n\n{USAGE}"
        ));
    }
    let current = flags
        .current
        .ok_or_else(|| format!("update needs --current <file>\n\n{USAGE}"))?;
    let baseline = flags
        .baseline
        .ok_or_else(|| format!("update needs --baseline <file>\n\n{USAGE}"))?;
    update(&current, &baseline)?;
    println!("wrote {}", baseline.display());
    Ok(ExitCode::SUCCESS)
}

fn read_measurement(path: &Path) -> Result<Measurement, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    Measurement::parse(&text).map_err(|error| format!("{}: {error}", path.display()))
}

fn read_thresholds(explicit: Option<&Path>) -> Result<Thresholds, String> {
    let default_path = Path::new("benchmarks/thresholds.bench");
    let path = explicit.or_else(|| default_path.exists().then_some(default_path));
    match path {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            Thresholds::parse(&text).map_err(|error| format!("{}: {error}", path.display()))
        }
        None => Ok(Thresholds::default()),
    }
}
