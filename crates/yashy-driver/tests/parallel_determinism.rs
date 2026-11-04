//! The parallel front end: thread reporting, and determinism.
//!
//! Parsing runs on a thread pool, so the output must not depend on which
//! worker finished first. These tests drive the CLI rather than the internals:
//! the promise is about what a user sees, and the merge that makes it true
//! happens before anything is printed.
//!
//! Three properties:
//!
//! - a project large enough to be worth parallelizing reports more than one
//!   front-end thread, and `--jobs 1` reports exactly one;
//! - two runs over the same tree with `--jobs 1` and `--jobs 0` produce
//!   byte-identical `--json` envelopes, even with errors in several files;
//! - several errors in one file are reported in span order.

mod common;

use common::{work_dir, write, yashy};
use std::path::Path;
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

/// The value of a `key=value` line in a `--timings` report.
fn timing_value(report: &str, key: &str) -> Option<usize> {
    report.lines().find_map(|line| {
        line.strip_prefix(&format!("{key}="))
            .and_then(|value| value.trim().parse().ok())
    })
}

/// Every `"line":N` value in a JSON document, in the order it appears.
///
/// Diagnostics are emitted in the order the front end merged them, so the
/// sequence of line numbers is the sequence of diagnostics.
fn json_lines(text: &str) -> Vec<i64> {
    let mut lines = Vec::new();
    let mut rest = text;
    while let Some(index) = rest.find("\"line\":") {
        rest = &rest[index + "\"line\":".len()..];
        let rest_trimmed = rest.trim_start();
        let digits: String = rest_trimmed
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        rest = rest_trimmed;
        if let Ok(value) = digits.parse() {
            lines.push(value);
        }
    }
    lines
}

#[test]
fn a_ten_thousand_line_project_parses_on_several_threads() {
    let dir = work_dir("parallel-threads");

    // Ten thousand lines across twenty modules. Each module is a file to parse
    // and a file to import, which is what the pool hands out as separate jobs.
    let modules = 20;
    let per_module = 125;
    for module in 0..modules {
        let mut text = String::new();
        for function in 0..per_module {
            text.push_str(&format!(
                "pub fn f{module}_{function}(int x) -> int {{\n    return x + 1;\n}}\n\n"
            ));
        }
        write(&dir, &format!("m{module}.ya"), &text);
    }

    let mut main = String::new();
    for module in 0..modules {
        main.push_str(&format!("import m{module};\n"));
    }
    main.push_str("\nfn main() {\n");
    for module in 0..modules {
        main.push_str(&format!("    let v{module} = f{module}_0(1);\n"));
    }
    main.push_str("}\n");
    write(&dir, "main.ya", &main);

    let parallel = run(&dir, &["check", "--timings", "--jobs", "4", "main.ya"]);
    assert_eq!(
        parallel.status.code(),
        Some(0),
        "the synthetic project must check:\n{}",
        stderr(&parallel)
    );
    let threads = timing_value(&stderr(&parallel), "frontend_threads")
        .expect("`--timings` must report the front-end thread count");
    assert_eq!(
        threads,
        4,
        "`--jobs 4` must use four front-end threads:\n{}",
        stderr(&parallel)
    );
    assert!(
        threads > 1,
        "the project must parse on more than one thread"
    );

    let serial = run(&dir, &["check", "--timings", "--jobs", "1", "main.ya"]);
    assert_eq!(serial.status.code(), Some(0), "{}", stderr(&serial));
    assert_eq!(
        timing_value(&stderr(&serial), "frontend_threads"),
        Some(1),
        "`--jobs 1` must use one front-end thread:\n{}",
        stderr(&serial)
    );
}

#[test]
fn jobs_one_and_jobs_zero_produce_identical_json() {
    let dir = work_dir("parallel-identical");
    write(
        &dir,
        "good.ya",
        "pub fn good(int x) -> int {\n    return x + 1;\n}\n",
    );
    // Two files fail to parse. The parallel run parses both; the merge must
    // still print the same envelope as the serial run, which stops at the
    // first failing file.
    write(
        &dir,
        "bad.ya",
        "pub fn bad(int x) -> int {\n    return 1 + ;\n}\n",
    );
    write(
        &dir,
        "also_bad.ya",
        "pub fn also_bad(int x) -> int {\n    return 1 + ;\n}\n",
    );
    write(
        &dir,
        "main.ya",
        "import good;\nimport bad;\nimport also_bad;\n\nfn main() {\n    let x = good(1);\n}\n",
    );

    let serial = run(&dir, &["check", "--json", "--jobs", "1", "main.ya"]);
    let parallel = run(&dir, &["check", "--json", "--jobs", "0", "main.ya"]);
    assert_eq!(
        serial.status.code(),
        Some(1),
        "the broken modules must fail the check:\n{}",
        stderr(&serial)
    );
    assert_eq!(parallel.status.code(), Some(1), "{}", stderr(&parallel));
    assert_eq!(
        stdout(&serial),
        stdout(&parallel),
        "the envelope must not depend on the scheduler"
    );
}

#[test]
fn one_files_errors_are_reported_in_span_order() {
    let dir = work_dir("parallel-span-order");
    write(
        &dir,
        "main.ya",
        "fn a() {\n    const int x = ;\n}\nfn b() {\n    const int y = ;\n}\nfn c() {\n    const int z = ;\n}\n",
    );

    let output = run(&dir, &["check", "--json", "--jobs", "4", "main.ya"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "the syntax errors must fail the check:\n{}",
        stderr(&output)
    );
    let lines = json_lines(&stdout(&output));
    assert_eq!(
        lines.len(),
        3,
        "all three syntax errors must be reported:\n{}",
        stdout(&output)
    );
    assert!(
        lines.windows(2).all(|pair| pair[0] <= pair[1]),
        "the errors must be in span order, got {lines:?}:\n{}",
        stdout(&output)
    );
}
