//! `yashyc build --stack-report` names every allocation site and the reason
//! escape analysis put it on the stack or left it on the heap.
//!
//! The report is a view of the note the emitter already recorded, so the tests
//! check it against the IR the same emission produced: a promoted site must
//! have no runtime allocator call.

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

fn build(name: &str, source: &str, args: &[&str]) -> String {
    let dir = work_dir(name);
    write(&dir, "main.ya", source);
    run(&dir, args)
}

/// A literal allocation that is released in the same frame never escapes.
const PROMOTED: &str = "\
fn f() -> int {
    let p = alloc(8);
    p[0] = 1;
    let v = p[0];
    p.free();
    return v;
}

fn main() {
}
";

/// Returning the pointer hands it to a caller outside the frame.
const ESCAPING: &str = "\
fn give() -> *mut int {
    let p = alloc(8);
    return p;
}

fn main() {
}
";

#[test]
fn a_promoted_site_is_named_and_emits_no_heap_call() {
    let dir = work_dir("stack-promoted");
    write(&dir, "main.ya", PROMOTED);
    let text = run(&dir, &["build", "main.ya", "--stack-report"]);
    assert!(
        text.contains("promoted to the stack"),
        "the report must name the placement:\n{text}"
    );
    assert!(text.contains("alloc(8)"), "the site must be named:\n{text}");
    assert!(
        text.contains("fn f"),
        "the site is grouped by function:\n{text}"
    );
    let ir = run(&dir, &["build", "main.ya", "--emit-ir"]);
    assert!(
        !ir.contains("call i8* @yashy_alloc("),
        "a promoted site must not call the runtime allocator:\n{ir}"
    );
    let json = run(&dir, &["build", "main.ya", "--stack-report=json"]);
    assert!(json.contains("\"schema_version\": 1"), "{json}");
    assert!(json.contains("\"count\": 1"), "{json}");
    assert!(json.contains("\"placement\": \"stack\""), "{json}");
    assert!(json.contains("\"line\""), "every site has a span:\n{json}");
}

#[test]
fn an_escaping_site_is_reported_as_heap() {
    let text = build(
        "stack-escaping",
        ESCAPING,
        &["build", "main.ya", "--stack-report"],
    );
    assert!(
        text.contains("the pointer escapes the function"),
        "the report must name the proven reason:\n{text}"
    );
    assert!(text.contains("heap"), "the placement is named:\n{text}");
}

#[test]
fn a_report_cannot_be_combined_with_the_ir_or_the_full_report() {
    let dir = work_dir("stack-exclusive");
    write(&dir, "main.ya", PROMOTED);
    for args in [
        vec!["build", "main.ya", "--stack-report", "--emit-ir"],
        vec!["build", "main.ya", "--stack-report", "--opt-report"],
        vec!["build", "main.ya", "--stack-report=xml"],
    ] {
        let output = Command::new(yashyc())
            .args(&args)
            .current_dir(&dir)
            .output()
            .expect("the CLI must start");
        assert_ne!(output.status.code(), Some(0), "refused: {args:?}");
    }
}

#[test]
fn stack_alloc_stays_in_the_frame() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = root.join("crates/yashy-driver/tests/programs/stack_alloc.ya");
    let dir = work_dir("stack-alloc-site");
    let binary = dir.join("program");
    let built = Command::new(yashyc())
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .current_dir(&root)
        .output()
        .expect("the CLI must run");
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let ran = Command::new(&binary)
        .output()
        .expect("the program must run");
    assert_eq!(String::from_utf8_lossy(&ran.stdout).trim(), "16");

    let text = run(&dir, &["build", source.to_str().unwrap(), "--stack-report"]);
    assert!(
        text.contains("explicit stack_alloc"),
        "the site and its reason are named:\n{text}"
    );
    let ir = run(&dir, &["build", source.to_str().unwrap(), "--emit-ir"]);
    assert!(ir.contains("alloca [8 x i64]"), "{ir}");
    assert!(!ir.contains("call i8* @yashy_alloc("), "{ir}");
}

#[test]
fn an_escaping_stack_alloc_is_refused_with_a_span() {
    let dir = work_dir("stack-alloc-escape");
    write(
        &dir,
        "main.ya",
        "fn give() -> *mut int {\n\
             let p = stack_alloc(4);\n\
             return p;\n\
         }\n\
         fn main() { }\n",
    );
    let output = Command::new(yashyc())
        .args(["verify", "--json", "main.ya"])
        .current_dir(&dir)
        .output()
        .expect("the CLI must start");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.contains("cannot escape the function"), "{text}");
    assert!(
        text.contains("\"line\""),
        "the diagnostic has a span: {text}"
    );
}

#[test]
fn the_frame_budget_is_accounted_per_function() {
    let dir = work_dir("stack-alloc-budget");
    write(
        &dir,
        "main.ya",
        "fn main() {\n    let a = stack_alloc(300);\n    let b = stack_alloc(300);\n    a[0] = 1;\n    b[0] = 2;\n    print(a[0] + b[0]);\n}\n",
    );
    let output = Command::new(yashyc())
        .args(["verify", "--json", "main.ya"])
        .current_dir(&dir)
        .output()
        .expect("the CLI must start");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.contains("frame limit is 4096"), "{text}");
}

#[test]
fn a_non_literal_stack_alloc_count_is_refused() {
    let dir = work_dir("stack-alloc-count");
    write(
        &dir,
        "main.ya",
        "fn f(int n) {\n    let p = stack_alloc(n);\n    p.free();\n}\nfn main() { }\n",
    );
    let output = Command::new(yashyc())
        .args(["check", "--json", "main.ya"])
        .current_dir(&dir)
        .output()
        .expect("the CLI must start");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.contains("positive literal element count"), "{text}");
}

#[test]
fn nothing_to_report_has_a_stable_answer() {
    let text = build(
        "stack-empty",
        "fn main() { print(1); }",
        &["build", "main.ya", "--stack-report"],
    );
    assert!(text.contains("no allocation sites"), "{text}");
}
