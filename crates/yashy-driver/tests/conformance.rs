//! Conformance: every error page's example is the rule's evidence.
//!
//! The runner checks each `yashy` block in `docs/errors` and reports whether
//! the compiler produces the code in the page's filename. It is a report by
//! default; `--fail-under` turns it into a gate.

mod common;

use std::path::Path;
use std::process::Command;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").leak()
}

fn run(args: &[&str]) -> (bool, String, String) {
    let output = Command::new(common::yashyc())
        .args(args)
        .current_dir(root())
        .output()
        .expect("the CLI must run");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn the_report_covers_every_error_page() {
    let (ok, stdout, stderr) = run(&["conformance", "--json"]);
    assert!(ok, "a report is not a gate by default: {stderr}");
    assert!(stdout.contains("\"cases\""), "{stdout}");
    assert!(stdout.contains("\"passed\""), "{stdout}");
    assert!(stdout.contains("E0100"), "{stdout}");
}

#[test]
fn every_passing_case_names_its_own_code() {
    let (_, stdout, _) = run(&["conformance", "--json"]);
    let value = yashy_common::json_rpc::parse(&stdout).expect("valid JSON report");
    let cases = value
        .get("cases")
        .and_then(yashy_common::json_rpc::Value::as_array)
        .expect("cases array");
    assert!(
        cases.len() > 50,
        "every code page is a case: {}",
        cases.len()
    );
    for case in cases {
        let status = case
            .get("status")
            .and_then(yashy_common::json_rpc::Value::as_str)
            .unwrap_or("");
        let rule = case
            .get("rule")
            .and_then(yashy_common::json_rpc::Value::as_str)
            .unwrap_or("");
        if status == "pass" {
            let actual = case
                .get("actual")
                .and_then(yashy_common::json_rpc::Value::as_array)
                .expect("actual codes");
            assert!(
                actual.iter().any(|code| code.as_str() == Some(rule)),
                "{rule} must produce its own code: {case:?}"
            );
        }
    }
}

#[test]
fn a_parser_stage_code_page_reproduces_its_code() {
    // `repr(C) fn` is refused by the parser, before sema runs. The runner must
    // keep that diagnostic instead of reporting the page as producing nothing.
    let (_, stdout, _) = run(&["conformance", "--json"]);
    let value = yashy_common::json_rpc::parse(&stdout).expect("valid JSON report");
    let cases = value
        .get("cases")
        .and_then(yashy_common::json_rpc::Value::as_array)
        .expect("cases array");
    let case = cases
        .iter()
        .find(|case| {
            case.get("rule")
                .and_then(yashy_common::json_rpc::Value::as_str)
                == Some("E0106")
        })
        .expect("E0106 has a page");
    assert_eq!(
        case.get("status")
            .and_then(yashy_common::json_rpc::Value::as_str),
        Some("pass"),
        "{case:?}"
    );
}

#[test]
fn a_module_page_reproduces_its_code_from_all_its_blocks() {
    // `E0801` and `E0802` are module-level rules: their pages show several
    // `// name.ya` blocks, and the runner has to stage them as separate files.
    let (_, stdout, _) = run(&["conformance", "--json"]);
    let value = yashy_common::json_rpc::parse(&stdout).expect("valid JSON report");
    let cases = value
        .get("cases")
        .and_then(yashy_common::json_rpc::Value::as_array)
        .expect("cases array");
    for rule in ["E0801", "E0802"] {
        let case = cases
            .iter()
            .find(|case| {
                case.get("rule")
                    .and_then(yashy_common::json_rpc::Value::as_str)
                    == Some(rule)
            })
            .unwrap_or_else(|| panic!("{rule} has a page"));
        assert_eq!(
            case.get("status")
                .and_then(yashy_common::json_rpc::Value::as_str),
            Some("pass"),
            "{rule}: {case:?}"
        );
    }
}

#[test]
fn a_floor_can_turn_the_report_into_a_gate() {
    let (ok, stdout, stderr) = run(&["conformance", "--fail-under", "100"]);
    assert!(
        ok,
        "every source page now reproduces its code: {stderr}\n{stdout}"
    );
}

#[test]
fn out_of_scope_codes_name_their_harness() {
    let (_, stdout, _) = run(&["conformance", "--json"]);
    let value = yashy_common::json_rpc::parse(&stdout).expect("valid JSON report");
    let harnesses = value.get("non_source_harnesses").expect("harness map");
    assert!(
        harnesses.get("E1810").is_some(),
        "the non-source codes are listed"
    );
    let cases = value
        .get("cases")
        .and_then(yashy_common::json_rpc::Value::as_array)
        .expect("cases array");
    for case in cases {
        if case
            .get("status")
            .and_then(yashy_common::json_rpc::Value::as_str)
            == Some("out-of-scope")
        {
            let rule = case
                .get("rule")
                .and_then(yashy_common::json_rpc::Value::as_str)
                .unwrap_or("");
            assert!(
                harnesses.get(rule).is_some_and(|value| {
                    value
                        .as_str()
                        .is_some_and(|text| text.starts_with("cargo test"))
                }),
                "{rule} must name a harness: {case:?}"
            );
        }
    }
}

#[test]
fn a_bad_floor_is_a_usage_error() {
    let (ok, _, stderr) = run(&["conformance", "--fail-under", "101"]);
    assert!(!ok, "{stderr}");
    assert!(stderr.contains("0 to 100"), "{stderr}");
}
