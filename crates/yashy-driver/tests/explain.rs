//! `yashy explain` and the human diagnostic's link to the error catalog.
//!
//! The catalog pages are embedded with `include_str!`, so `explain` is an
//! offline command. These tests assert that every code prints its page, that
//! the JSON form carries the same facts, and that a coded human diagnostic
//! names the same page.

mod common;

use common::{work_dir, write, yashy};
use std::path::Path;
use std::process::{Command, Output};
use yashy_common::{codes, Json};

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

/// The JSON envelope `explain --json` must print, built with the same writer the
/// CLI uses. Comparing against this is a snapshot of the whole object: the
/// field order, the escaping, and every fact.
fn explain_json(code: &codes::Code) -> String {
    let mut object = Json::object();
    object.set("schema_version", Json::number(1));
    object.set("tool", Json::string("explain"));
    object.set("yashy_version", Json::string(env!("CARGO_PKG_VERSION")));
    object.set("success", Json::bool(true));
    object.set("code", Json::string(code.id));
    object.set("title", Json::string(code.title));
    object.set("summary", Json::string(code.summary));
    object.set("page", Json::string(code.page));
    object.set("docs_url", Json::string(codes::docs_url(code.id)));
    object.render()
}

#[test]
fn every_code_has_a_catalog_page_that_explains_cause_example_and_fix() {
    let dir = work_dir("explain-every-code");

    for code in codes::CODES {
        let output = run(&dir, &["explain", code.id]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "`explain {}` failed:\n{}",
            code.id,
            stderr(&output)
        );
        let text = stdout(&output);
        assert_eq!(text, code.page, "`explain {}` must print its page", code.id);
        assert!(
            text.contains("## Cause"),
            "{} has no cause section:\n{text}",
            code.id
        );
        assert!(
            text.contains("## Example"),
            "{} has no example section:\n{text}",
            code.id
        );
        assert!(
            text.contains("## Fix"),
            "{} has no fix section:\n{text}",
            code.id
        );
        assert!(text.contains(code.id), "{} does not name itself", code.id);
        assert!(
            stderr(&output).is_empty(),
            "`explain {}` wrote to stderr:\n{}",
            code.id,
            stderr(&output)
        );
    }
}

#[test]
fn explain_prints_the_catalog_page() {
    let dir = work_dir("explain-human");
    let code = codes::find("E0100").expect("E0100 is in the catalog");

    let output = run(&dir, &["explain", "E0100"]);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), code.page);
    assert!(stdout(&output).contains("## Cause"));
    assert!(stdout(&output).contains("## Example"));
    assert!(stdout(&output).contains("## Fix"));
    assert!(stderr(&output).is_empty());
}

#[test]
fn explain_json_has_the_catalog_entry_and_docs_url() {
    let dir = work_dir("explain-json");
    let code = codes::find("E0100").expect("E0100 is in the catalog");

    let output = run(&dir, &["explain", "--json", "E0100"]);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), format!("{}\n", explain_json(code)));
    assert!(stderr(&output).is_empty());
}

#[test]
fn human_and_json_explain_share_the_facts() {
    let dir = work_dir("explain-shared-facts");
    let code = codes::find("E0100").expect("E0100 is in the catalog");

    let human = run(&dir, &["explain", "E0100"]);
    let json = run(&dir, &["explain", "--json", "E0100"]);

    assert_eq!(human.status.code(), Some(0));
    assert_eq!(json.status.code(), Some(0));
    assert_eq!(stdout(&human), code.page);
    assert_eq!(stdout(&json), format!("{}\n", explain_json(code)));
    assert_eq!(code.title, "type mismatch");
    assert_eq!(
        code.summary,
        "An expression's type does not match its context."
    );
}

#[test]
fn explain_suggests_the_nearest_code() {
    let dir = work_dir("explain-near-miss");

    let output = run(&dir, &["explain", "E010"]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "an unknown code is a usage error"
    );
    assert!(stdout(&output).is_empty(), "no page may be printed");
    let error = stderr(&output);
    assert!(
        error.contains("unknown error code `E010`"),
        "the message must name the bad code:\n{error}"
    );
    assert!(
        error.contains("did you mean `E0100`?"),
        "the message must suggest the near code:\n{error}"
    );
}

#[test]
fn explain_rejects_an_unrelated_code_without_a_suggestion() {
    let dir = work_dir("explain-unrelated");

    let output = run(&dir, &["explain", "E9999"]);

    assert_eq!(output.status.code(), Some(2));
    let error = stderr(&output);
    assert!(
        error.contains("unknown error code `E9999`"),
        "got:\n{error}"
    );
    assert!(
        !error.contains("did you mean"),
        "an unrelated string gets no suggestion:\n{error}"
    );
}

#[test]
fn explain_requires_a_code() {
    let dir = work_dir("explain-no-code");

    let output = run(&dir, &["explain"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("needs an error code"),
        "got:\n{}",
        stderr(&output)
    );
}

#[test]
fn explain_rejects_extra_arguments() {
    let dir = work_dir("explain-extra");

    let output = run(&dir, &["explain", "E0100", "E0200"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("unexpected extra argument `E0200`"),
        "got:\n{}",
        stderr(&output)
    );
}

#[test]
fn explain_rejects_an_unknown_option() {
    let dir = work_dir("explain-option");

    let output = run(&dir, &["explain", "--verbose", "E0100"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("unknown option `--verbose`"),
        "got:\n{}",
        stderr(&output)
    );
}

#[test]
fn explain_json_with_an_unknown_code_still_exits_2() {
    let dir = work_dir("explain-json-unknown");

    let output = run(&dir, &["explain", "--json", "E010"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("did you mean `E0100`?"),
        "got:\n{}",
        stderr(&output)
    );
}

#[test]
fn a_coded_human_diagnostic_links_to_the_catalog_page() {
    let dir = work_dir("explain-human-link");
    write(
        &dir,
        "bad.ya",
        "fn main() {\n    const count: int = \"three\";\n}\n",
    );

    let output = run(&dir, &["check", "bad.ya"]);

    assert_eq!(output.status.code(), Some(1));
    let error = stderr(&output);
    assert!(
        error.contains("error[E0100]: expected `int`, found `string`"),
        "the code is still in the message:\n{error}"
    );
    assert!(
        error.contains("  docs: https://yashy.dev/errors/E0100"),
        "the diagnostic must link the same catalog page:\n{error}"
    );
}

#[test]
fn an_uncoded_human_diagnostic_has_no_docs_line() {
    let dir = work_dir("explain-human-uncoded");
    write(&dir, "bad.ya", "fn main() {\n    const x = 3?;\n}\n");

    let output = run(&dir, &["check", "bad.ya"]);

    assert_eq!(output.status.code(), Some(1));
    let error = stderr(&output);
    assert!(
        error.contains("can only be used on a `Result`"),
        "the fixture must produce the uncoded diagnostic:\n{error}"
    );
    assert!(
        !error.contains("docs:"),
        "an uncoded diagnostic has no catalog page:\n{error}"
    );
}
