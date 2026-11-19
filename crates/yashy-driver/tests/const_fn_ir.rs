//! The compiled image of a const-fn table: the fold has to reach the IR as
//! literals, with no symbol left for the compile-time-only function.
//!
//! Emission is a front-end operation, so this runs without a linked binary.

use yashy_common::{ProofFacts, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::check_program;

fn ir(src: &str) -> String {
    let mut sources = SourceMap::new();
    let file = sources.add_file("const_fn_table.ya".to_string(), src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let ast = parser.parse_program().expect("the program must parse");
    let program = check_program(&ast, &sources, file).expect("the program must check");
    yashy_codegen::emit_program(&program, "main", Default::default(), &ProofFacts::empty())
}

#[test]
fn a_const_fn_table_is_emitted_as_literals() {
    let text = ir(include_str!("programs/const_fn_table.ya"));
    assert!(
        text.contains("i64 11") && text.contains("i64 12"),
        "the two folded entries must be literals in the IR: {text}"
    );
    assert!(
        !text.contains("call i64 @table"),
        "the compile-time-only function must not be called: {text}"
    );
    assert!(
        !text.contains("define i64 @table"),
        "the compile-time-only function must not be emitted: {text}"
    );
}
