//! The unsafe allocation builtin has to reach the backend as the runtime's
//! uninitialised allocator, never as the zeroing `yashy_alloc`.
//!
//! IR emission is a front-end operation, so this test proves the lowering
//! without linking or running a program.

use yashy_common::{ProofFacts, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::check_program;

fn ir(src: &str) -> String {
    let mut sources = SourceMap::new();
    let file = sources.add_file("unsafe_alloc.ya".to_string(), src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let ast = parser.parse_program().expect("the source must parse");
    let program = check_program(&ast, &sources, file).expect("the program must check");
    yashy_codegen::emit_program(&program, "main", Default::default(), &ProofFacts::empty())
}

#[test]
fn alloc_uninit_calls_the_uninitialised_runtime_entry_point() {
    let text = ir("fn main() { unsafe { let p = alloc_uninit(4); p.free(); } }");
    assert!(
        text.contains("declare i8* @yashy_alloc_uninit(i64, i64)"),
        "the runtime symbol must be declared: {text}"
    );
    assert!(
        text.contains("call i8* @yashy_alloc_uninit("),
        "the call must use the unsafe symbol: {text}"
    );
}

#[test]
fn safe_alloc_still_calls_the_zeroing_entry_point() {
    let text = ir("fn main() { let p = alloc(4); p.free(); }");
    assert!(
        text.contains("call i8* @yashy_alloc("),
        "safe allocation kept its entry point: {text}"
    );
    assert!(
        !text.contains("call i8* @yashy_alloc_uninit("),
        "a safe program must not reach the unsafe symbol: {text}"
    );
}
