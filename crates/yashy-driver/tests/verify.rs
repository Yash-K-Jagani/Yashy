//! `yashy verify`: the command that runs the verifier, its human output, its
//! JSON envelope, and the whole corpus.
//!
//! The corpus test at the bottom is the one that matters. A verifier is only
//! worth having if it is right about the programs the language already ships,
//! so every worked example and every end-to-end program is verified here. When
//! one of them fails, the rule is wrong or the program is: fix the rule, never
//! the test.

mod common;

use common::{work_dir, write, yashy};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(yashy())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("the yashy CLI must be runnable")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn verify_reports_a_leak_in_human_output() {
    let dir = work_dir("verify-leak-human");
    write(
        &dir,
        "leak.ya",
        "fn main() {\n    let buf = alloc(16);\n}\n",
    );

    let output = run(&dir, &["verify", "leak.ya"]);
    let errors = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "a leak must fail:\n{errors}");
    assert!(
        errors.contains("error[E1000]"),
        "the code must be in the header:\n{errors}"
    );
    assert!(
        errors.contains("`buf`"),
        "the binding must be named:\n{errors}"
    );
    assert!(
        errors.contains("defer"),
        "the remedy must be suggested:\n{errors}"
    );
}

#[test]
fn verify_accepts_a_program_that_releases_what_it_allocates() {
    let dir = work_dir("verify-clean");
    write(
        &dir,
        "clean.ya",
        "fn main() {\n    let buf = alloc(16);\n    defer { buf.free(); }\n}\n",
    );

    let output = run(&dir, &["verify", "clean.ya"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "a released value is not a leak:\n{}",
        stderr(&output)
    );
    assert!(
        stderr(&output).is_empty(),
        "a clean program prints nothing:\n{}",
        stderr(&output)
    );
}

#[test]
fn verify_json_carries_the_proof_record() {
    let dir = work_dir("verify-leak-json");
    write(
        &dir,
        "leak.ya",
        "fn main() {\n    let buf = alloc(16);\n}\n",
    );

    let output = run(&dir, &["verify", "--json", "leak.ya"]);
    let json = stdout(&output);
    assert_eq!(output.status.code(), Some(1), "a leak must fail:\n{json}");

    let proof = &json;
    assert!(
        proof.contains("\"obligation\": \"leak\""),
        "the obligation must be named:\n{proof}"
    );
    assert!(
        proof
            .contains("\"missing_fact\": \"`buf` is not released on every path out of its scope\""),
        "the missing fact must be stated:\n{proof}"
    );
    assert!(
        proof.contains(
            "\"remedy\": \"release it with `buf.free()`, `defer { buf.free(); }`, \
             or by returning it\""
        ),
        "the remedy must be actionable:\n{proof}"
    );
    assert!(json.contains("\"tool\": \"verify\""), "{json}");
    assert!(json.contains("\"success\": false"), "{json}");
    assert!(json.contains("\"code\": \"E1000\""), "{json}");
    assert!(json.contains("\"severity\": \"error\""), "{json}");
    assert!(
        json.contains("\"errors\": 1") && json.contains("\"warnings\": 0"),
        "the summary must count the error:\n{json}"
    );
}

#[test]
fn verify_json_is_silent_about_proof_when_the_program_is_clean() {
    let dir = work_dir("verify-clean-json");
    write(
        &dir,
        "clean.ya",
        "fn main() {\n    let buf = alloc(16);\n    buf.free();\n}\n",
    );

    let output = run(&dir, &["verify", "--json", "clean.ya"]);
    let json = stdout(&output);
    assert_eq!(output.status.code(), Some(0), "{json}");
    assert!(json.contains("\"tool\": \"verify\""), "{json}");
    assert!(json.contains("\"success\": true"), "{json}");
    assert!(json.contains("\"diagnostics\": []"), "{json}");
    assert!(!json.contains("\"proof\""), "{json}");
}

/// A timing report is for the person measuring the compiler, never for the
/// program being compiled: stdout must stay byte-identical whether or not the
/// flag is passed, because a program's output is its own.
#[test]
fn timings_go_to_stderr_and_the_json_report_and_never_to_stdout() {
    let dir = work_dir("timings");
    write(
        &dir,
        "hello.ya",
        "fn main() {\n    println(\"hello\");\n}\n",
    );

    let plain = run(&dir, &["run", "hello.ya"]);
    let timed = run(&dir, &["run", "hello.ya", "--timings"]);
    assert_eq!(plain.status.code(), Some(0), "{}", stderr(&plain));
    assert_eq!(timed.status.code(), Some(0), "{}", stderr(&timed));
    assert_eq!(stdout(&plain), "hello\n");
    assert_eq!(
        stdout(&timed),
        stdout(&plain),
        "`--timings` must not change what the program prints"
    );
    assert!(
        stderr(&plain).is_empty(),
        "a run without the flag stays quiet:\n{}",
        stderr(&plain)
    );

    let report = stderr(&timed);
    let phases: Vec<&str> = report
        .lines()
        .filter_map(|line| line.strip_prefix("timing: "))
        .map(|line| line.split(' ').next().unwrap_or_default())
        .collect();
    for phase in ["load", "check", "verify", "emit", "link"] {
        assert!(phases.contains(&phase), "missing `{phase}` in:\n{report}");
    }

    // `check` stops after the checker, so it has no verify phase; `verify` runs
    // the verifier and neither of them links.
    let check = run(&dir, &["check", "--json", "--timings", "hello.ya"]);
    let json = stdout(&check);
    assert_eq!(check.status.code(), Some(0), "{json}");
    assert!(json.contains("\"phase\": \"load\""), "{json}");
    assert!(json.contains("\"phase\": \"check\""), "{json}");
    assert!(!json.contains("\"phase\": \"verify\""), "{json}");
    assert!(!json.contains("\"phase\": \"link\""), "{json}");
    assert!(stderr(&check).is_empty(), "{}", stderr(&check));

    let verify = run(&dir, &["verify", "--json", "--timings", "hello.ya"]);
    let json = stdout(&verify);
    assert_eq!(verify.status.code(), Some(0), "{json}");
    assert!(json.contains("\"phase\": \"load\""), "{json}");
    assert!(json.contains("\"phase\": \"check\""), "{json}");
    assert!(json.contains("\"phase\": \"verify\""), "{json}");
    assert!(!json.contains("\"phase\": \"link\""), "{json}");
}

/// `test --timings` reports the suite once, not once per test: five lines per
/// test would be unreadable at a hundred tests, and the question is where the
/// run's wall time went.
#[test]
fn test_timings_are_one_report_for_the_whole_suite() {
    let dir = work_dir("timings-suite");
    write(&dir, "tests/a.ya", "fn main() { }\n");
    write(&dir, "tests/b.ya", "fn main() { }\n");

    let output = run(&dir, &["test", "--timings"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        stdout(&output).matches("... ok").count(),
        2,
        "{}",
        stdout(&output)
    );

    let report = stderr(&output);
    assert_eq!(
        report.matches("timing: link ").count(),
        1,
        "one link line for the suite, not one per test:\n{report}"
    );
    assert_eq!(report.matches("timing: load ").count(), 1, "{report}");
}

/// Every program the repository ships, entry points only.
fn corpus() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut programs = Vec::new();

    let programs_dir = root.join("tests").join("programs");
    for entry in fs::read_dir(&programs_dir).expect("the programs directory must exist") {
        let path = entry.expect("a readable directory entry").path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("ya") {
            programs.push(path);
        }
    }

    let examples_dir = root.join("..").join("..").join("examples");
    for entry in fs::read_dir(&examples_dir).expect("the examples directory must exist") {
        let main = entry
            .expect("a readable directory entry")
            .path()
            .join("main.ya");
        if main.is_file() {
            programs.push(main);
        }
    }

    programs.sort();
    programs
}

#[test]
fn verify_accepts_every_program_in_the_examples_and_programs_directories() {
    let dir = work_dir("verify-corpus");
    let mut failures = Vec::new();
    let programs = corpus();
    assert!(
        programs.len() > 50,
        "the corpus must not shrink silently: found {}",
        programs.len()
    );

    for program in &programs {
        let output = run(&dir, &["verify", program.to_str().unwrap()]);
        if output.status.code() != Some(0) {
            failures.push(format!("{}\n{}", program.display(), stderr(&output)));
        }
    }

    assert!(
        failures.is_empty(),
        "the verifier must accept the language's own programs; {} of {} failed:\n\n{}",
        failures.len(),
        programs.len(),
        failures.join("\n")
    );
}
