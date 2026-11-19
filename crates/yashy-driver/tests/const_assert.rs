//! A compile-time assertion that holds must leave no trace in the binary: the
//! checked HIR carries a unit literal, and codegen emits nothing for it.

use yashy_common::{ProofFacts, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::check_program;

fn ir(src: &str) -> String {
    let mut sources = SourceMap::new();
    let file = sources.add_file("const_assert.ya".to_string(), src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let ast = parser.parse_program().expect("the source must parse");
    let program = check_program(&ast, &sources, file).expect("the program must check");
    yashy_codegen::emit_program(&program, "main", Default::default(), &ProofFacts::empty())
}

#[test]
fn a_passing_compile_time_assertion_emits_no_call() {
    let text = ir("fn main() { const_assert(2 + 2 == 4); }");
    assert!(
        text.contains("define i32 @main"),
        "the program still has an entry point: {text}"
    );
    assert!(
        !text.contains("call void @yashy_assert"),
        "a compile-time check is not a runtime call: {text}"
    );
    assert!(
        !text.contains("const_assert"),
        "the builtin has no symbol in the IR: {text}"
    );
}
