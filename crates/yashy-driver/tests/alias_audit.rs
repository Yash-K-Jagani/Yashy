//! The alias audit: every attribute the backend emits must name a verifier
//! fact in `--opt-report=json`, with the source span the reader can check.
//!
//! The IR and the report come from two separate CLI calls on the same source,
//! so a promise written in one path but not explained in the other fails here.
//! This is the driver-level half of the audit whose unit tests live in
//! `crates/yashy-codegen/tests/attributes.rs`.

mod common;

use common::{work_dir, write, yashyc};
use std::path::Path;
use std::process::Command;

fn run(dir: &Path, args: &[&str]) -> String {
    let output = Command::new(yashyc())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("the yashyc CLI must be runnable");
    assert!(
        output.status.success(),
        "{} failed:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A read-only pointer: noalias, nonnull, readonly and align 8.
const READ: &str = "\
fn read(*mut int p, int n) -> int {
    let sum = 0;
    let i = 0;
    while i < n {
        sum += p[i];
        i += 1;
    }
    return sum;
}
fn main() { }
";

/// A written pointer: noalias and nonnull, but no readonly promise.
const FILL: &str = "\
fn fill(*mut int p, int n) {
    let i = 0;
    while i < n {
        p[i] = i * 2;
        i += 1;
    }
}
fn main() { }
";

/// A returned pointer escapes: it keeps nonnull and align, loses noalias.
const ESCAPE: &str = "\
fn give(*mut int p) -> *mut int { return p; }
fn main() { }
";

/// A pointer inside an aggregate is not a bare pointer parameter.
const AGGREGATE: &str = "\
fn maybe(Option[*mut int] p) -> int { return 0; }
fn main() { }
";

fn audit(name: &str, source: &str) -> (String, String) {
    let dir = work_dir(name);
    write(&dir, "main.ya", source);
    let ir = run(&dir, &["build", "main.ya", "--emit-ir"]);
    let report = run(&dir, &["build", "main.ya", "--opt-report=json"]);
    (ir, report)
}

/// The attribute words emitted on `define` parameters, in IR order.
fn emitted_attributes(ir: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in ir.lines().filter(|line| line.starts_with("define ")) {
        let Some(params) = line.split_once('(').map(|(_, rest)| rest) else {
            continue;
        };
        let params = params
            .split_once(')')
            .map(|(head, _)| head)
            .unwrap_or(params);
        for token in ["noalias", "nonnull", "readonly"] {
            if params.split_whitespace().any(|word| word == token) {
                out.push(token.to_string());
            }
        }
        if let Some(at) = params.find(" align ") {
            let width = params[at + 7..]
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();
            out.push(format!("align {width}"));
        }
    }
    out
}

/// The reasons of the `attribute` entries in an opt report.
fn audited_reasons(report: &str) -> Vec<String> {
    report
        .split("\"kind\": \"attribute\"")
        .skip(1)
        .filter_map(|chunk| {
            let reason = chunk.split_once("\"reason\": \"")?.1.split('"').next()?;
            Some(reason.to_string())
        })
        .collect()
}

#[test]
fn every_emitted_attribute_has_a_fact_and_a_span() {
    for (name, source) in [
        ("alias-audit-read", READ),
        ("alias-audit-fill", FILL),
        ("alias-audit-escape", ESCAPE),
        ("alias-audit-aggregate", AGGREGATE),
    ] {
        let (ir, report) = audit(name, source);
        let emitted = emitted_attributes(&ir);
        let audited = audited_reasons(&report);
        for attribute in &emitted {
            assert!(
                audited.iter().any(|reason| reason.starts_with(attribute)),
                "`{attribute}` is emitted without a fact in {name}:\n{ir}\n{report}"
            );
        }
        // A report entry is a claim about a real promise. It must carry the
        // span a reader can check, not just a free-form explanation.
        for chunk in report.split("\"kind\": \"attribute\"").skip(1) {
            assert!(
                chunk.contains("\"span\"") && chunk.contains("\"line\""),
                "an attribute note has no span in {name}:\n{report}"
            );
        }
        if name == "alias-audit-aggregate" {
            assert!(
                emitted.is_empty(),
                "a pointer inside an aggregate is not a pointer parameter:\n{ir}"
            );
        }
    }
}
