//! The unsafe inventory at the verifier's own boundary.
//!
//! `yashyc verify --inventory` renders this list; testing it here keeps the
//! rule (ALIAS-5: every trust assumption is reported) next to the analysis
//! that produces it. The null-pointer niche is a separate, still-owed feature
//! and is deliberately not claimed by this file.

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::check_program;
use yashy_verify::{inventory, Assumption, AssumptionKind};

fn assumptions(src: &str) -> Vec<Assumption> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("unsafe.ya".to_string(), src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let ast = parser.parse_program().expect("the fixture must parse");
    let program = check_program(&ast, &sources, file).expect("the fixture must check");
    inventory(&program)
}

#[test]
fn a_safe_program_has_no_assumptions() {
    let found = assumptions("fn main() { let p = alloc(4); print(p[0]); p.free(); }");
    assert!(found.is_empty(), "safe code trusts nothing: {found:?}");
}

#[test]
fn an_extern_declaration_and_an_unsafe_block_are_both_listed() {
    let found = assumptions(
        "extern \"C\" { unsafe fn malloc() -> *mut i64; }\n\
         fn main() { let p = malloc(); unsafe { print(p[0]); } }",
    );
    assert!(
        found
            .iter()
            .any(|a| a.kind == AssumptionKind::ExternDecl && a.name == "malloc"),
        "the C declaration must be listed: {found:?}"
    );
    assert!(
        found.iter().any(|a| a.kind == AssumptionKind::UnsafeBlock),
        "the explicit block must be listed: {found:?}"
    );
}

#[test]
fn every_assumption_names_its_kind_and_its_reason() {
    let found = assumptions(
        "extern \"C\" { unsafe fn malloc() -> *mut i64; }\n\
         fn main() { let p = malloc(); unsafe { print(p[0]); } }",
    );
    assert!(!found.is_empty(), "the fixture has two assumptions");
    for item in &found {
        assert!(!item.kind.as_str().is_empty());
        assert!(
            !item.reason.is_empty(),
            "{} has no reason",
            item.kind.as_str()
        );
    }
}
