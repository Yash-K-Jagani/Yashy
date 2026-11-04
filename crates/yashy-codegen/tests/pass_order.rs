//! The pass manager's contract: one order, deterministic output, and a switch
//! that changes only what its own pass contributes.
//!
//! Each test turns exactly one pass off and compares the IR with the all-passes
//! build. The difference must be the pass's own work — an attribute, an
//! allocation site, a shared global — and nothing else.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::check_program;
use yashy_verify::{verify, VerifyOptions};

use yashy_codegen::pass;
use yashy_codegen::{emit_program, CodegenOptions};

/// Lex, parse, check and emit `src`, then emit it again; both IRs are returned
/// so the determinism test can compare two independent runs.
fn lower_twice(src: &str, options: CodegenOptions) -> (String, String) {
    let mut sources = SourceMap::new();
    let file = sources.add_file("pass_order.ya", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the fixture's lexer must succeed");
    let mut parser = Parser::new(tokens, &sources, file);
    let ast = parser.parse_program().expect("the fixture must parse");
    let hir = check_program(&ast, &sources, file).expect("the fixture must check");

    let report = verify(&hir, &VerifyOptions::default());
    assert_eq!(
        report.error_count(),
        0,
        "the fixture must verify: {:?}",
        report.diagnostics
    );
    let first = emit_program(&hir, "pass_order.ya", options, &report.facts);
    let second = emit_program(&hir, "pass_order.ya", options, &report.facts);
    (first, second)
}

fn lower(src: &str, options: CodegenOptions) -> String {
    lower_twice(src, options).0
}

/// The `define` line that declares `name`.
fn signature<'a>(ir: &'a str, name: &str) -> &'a str {
    let needle = format!("@{name}(");
    ir.lines()
        .find(|line| line.starts_with("define ") && line.contains(&needle))
        .unwrap_or_else(|| panic!("no `define` for `{name}` in:\n{ir}"))
}

/// The body of `name`: everything after the opening `define` line.
fn body<'a>(ir: &'a str, name: &str) -> &'a str {
    let needle = format!("@{name}(");
    let start = ir
        .lines()
        .position(|line| line.starts_with("define ") && line.contains(&needle))
        .unwrap_or_else(|| panic!("no `define` for `{name}` in:\n{ir}"));
    let offset = ir
        .lines()
        .take(start + 1)
        .map(|line| line.len() + 1)
        .sum::<usize>();
    &ir[offset..]
}

const READ_ONLY: &str = "\
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

const STACK: &str = "\
fn f() -> int {
    let p = alloc(8);
    p[0] = 1;
    let v = p[0];
    p.free();
    return v;
}
fn main() { }
";

const SAME_LITERAL: &str = "\
fn main() {
    print(\"same\");
    print(\"same\");
}
";

#[test]
fn the_pipeline_is_deterministic() {
    let (first, second) = lower_twice(READ_ONLY, CodegenOptions::default());
    assert_eq!(first, second, "two runs of the one order must emit one IR");
}

#[test]
fn the_registry_names_one_order_with_its_rows() {
    let names: Vec<&str> = pass::pipeline().iter().map(|pass| pass.name).collect();
    assert_eq!(names, ["facts", "escape", "intern"]);
    assert_eq!(pass::names(), "facts, escape, intern");
    for name in &names {
        assert!(pass::find(name).is_some(), "`{name}` must be findable");
    }
    assert!(pass::find("inlining").is_none());
}

#[test]
fn disabling_facts_changes_only_the_attributes() {
    let enabled = lower(READ_ONLY, CodegenOptions::default());
    let disabled = lower(
        READ_ONLY,
        CodegenOptions {
            facts: false,
            ..CodegenOptions::default()
        },
    );
    assert!(
        signature(&enabled, "read").contains("noalias"),
        "the verified build may rely on its facts:\n{}",
        signature(&enabled, "read")
    );
    assert!(
        !signature(&disabled, "read").contains("noalias"),
        "without the pass the attribute is gone:\n{}",
        signature(&disabled, "read")
    );
    assert_eq!(
        body(&enabled, "read"),
        body(&disabled, "read"),
        "the facts pass changes the signature, not the body it proved"
    );
}

#[test]
fn disabling_escape_keeps_the_allocator() {
    let promoted = lower(
        STACK,
        CodegenOptions {
            escape: true,
            ..CodegenOptions::default()
        },
    );
    let kept = lower(STACK, CodegenOptions::default());
    assert!(
        !body(&promoted, "f").contains("@yashy_alloc("),
        "a promoted allocation has no allocator call:\n{promoted}"
    );
    assert!(
        body(&kept, "f").contains("@yashy_alloc("),
        "without the pass the allocation stays on the heap:\n{kept}"
    );
}

#[test]
fn disabling_intern_emits_one_global_per_literal() {
    let shared = lower(SAME_LITERAL, CodegenOptions::default());
    let split = lower(
        SAME_LITERAL,
        CodegenOptions {
            intern: false,
            ..CodegenOptions::default()
        },
    );
    assert_eq!(
        shared.matches("c\"same\\00\"").count(),
        1,
        "identical literals share one global:\n{shared}"
    );
    assert_eq!(
        split.matches("c\"same\\00\"").count(),
        2,
        "without the pass each literal gets its own global:\n{split}"
    );
}
