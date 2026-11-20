//! The unsafe allocation spelling is a checked opt-out, not a second allocator.
//!
//! `alloc(n)` stays safe and zeroed. `alloc_uninit(n)` asks for the same block
//! without the zeroing, and the checker only admits the call inside an
//! `unsafe` block (E1405). These tests hold at the type level, so they run
//! without executing a linked binary.

use yashy_common::{codes, Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program_with, CheckOptions, HirProgram};

fn check(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sm = SourceMap::new();
    let file = sm.add_file("test.ya".to_string(), src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser.parse_program().expect("the source must parse");
    check_program_with(&ast, &sm, file, CheckOptions::default())
}

#[test]
fn alloc_uninit_outside_unsafe_is_e1405() {
    let diagnostics = check("fn main() { let p = alloc_uninit(4); p.free(); }")
        .expect_err("safe code cannot ask for uninitialised memory");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == Some(codes::UNINITIALISED_ALLOC)),
        "expected E1405, got {diagnostics:?}"
    );
}

#[test]
fn alloc_uninit_inside_unsafe_checks_like_alloc() {
    let program = check("fn main() { unsafe { let p = alloc_uninit(4); p[0] = 1; p.free(); } }")
        .expect("an unsafe block licenses the call");
    let text = format!("{:?}", program.items);
    assert!(
        text.contains("alloc_uninit"),
        "the builtin call must reach the backend: {text}"
    );
}

#[test]
fn a_nested_unsafe_block_still_licenses_the_call_after_the_inner_one_ends() {
    // Depth, not a boolean: leaving the inner block must not turn the outer
    // one off while its remaining statements are checked.
    let program =
        check("fn main() { unsafe { unsafe { print(1); }; let p = alloc_uninit(4); p.free(); } }")
            .expect("the outer unsafe block is still in force");
    assert!(!format!("{:?}", program.items).is_empty());
}

#[test]
fn safe_alloc_is_still_accepted_without_unsafe() {
    check("fn main() { let p = alloc(4); p.free(); }").expect("safe allocation did not change");
}
