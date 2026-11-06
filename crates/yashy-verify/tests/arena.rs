//! The arena pass, end to end.
//!
//! The verifier refuses two uses of an `arena { }` block's values: crossing
//! the boundary (`E1003`) and freeing one value on its own (`E1004`). A value
//! used inside the block is neither.

use yashy_common::{codes, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::check_program;
use yashy_verify::{verify, VerifyOptions, VerifyReport};

/// Parse and check `source`, then verify it under the default options.
fn verify_source(source: &str) -> VerifyReport {
    let mut sources = SourceMap::new();
    let file = sources.add_file("arena.ya", source.to_string());
    let tokens = Lexer::new(source)
        .tokenize()
        .expect("the fixture's lexer must succeed");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser.parse_program().expect("the fixture must parse");
    match check_program(&program, &sources, file) {
        Ok(hir) => verify(&hir, &VerifyOptions::default()),
        Err(diagnostics) => VerifyReport {
            diagnostics,
            obligations: Vec::new(),
            facts: Default::default(),
            effects: Default::default(),
            analyses: 0,
        },
    }
}

fn codes(report: &VerifyReport) -> Vec<&'static str> {
    report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.unwrap_or("<uncoded>"))
        .collect()
}

#[test]
fn a_value_used_inside_the_block_is_accepted() {
    let source = "fn main() { arena { let buf = alloc(16); buf[0] = 7; print(buf[0]); } }";
    assert!(verify_source(source).diagnostics.is_empty());
}

#[test]
fn an_arena_value_returned_is_e1003() {
    // The arena value is returned; the verifier rejects the escape. The
    // function needs a top-level `return` as well, because the checker's
    // missing-return rule does not look through an `arena` block.
    let source =
        "fn f() -> *mut int { arena { let buf = alloc(16); return buf; } return alloc(16); } fn main() { }";
    let report = verify_source(source);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == Some(codes::ARENA_ESCAPE)),
        "expected E1003, got {:?}",
        report
            .diagnostics
            .iter()
            .map(|d| (d.code.unwrap_or("<uncoded>"), &d.message))
            .collect::<Vec<_>>()
    );
}

#[test]
fn an_arena_value_assigned_to_an_outer_binding_is_e1003() {
    let source = "fn main() { let out = alloc(16); arena { let buf = alloc(16); out = move buf; } print(out[0]); }";
    assert_eq!(codes(&verify_source(source)), vec!["E1003"]);
}

#[test]
fn an_arena_value_freed_individually_is_e1004() {
    let source = "fn main() { arena { let buf = alloc(16); buf.free(); } }";
    assert_eq!(codes(&verify_source(source)), vec!["E1004"]);
}
