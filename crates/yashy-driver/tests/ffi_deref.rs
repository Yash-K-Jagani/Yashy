//! A C-pointer read becomes the same address arithmetic an owning-pointer read
//! uses, but only after the checker has seen the `unsafe` block that licenses
//! it. Emission is a front-end operation, so no binary is run here.

use yashy_common::{ProofFacts, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::check_program;

fn ir(src: &str) -> String {
    let mut sources = SourceMap::new();
    let file = sources.add_file("ffi_deref.ya".to_string(), src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let ast = parser.parse_program().expect("the source must parse");
    let program = check_program(&ast, &sources, file).expect("the program must check");
    yashy_codegen::emit_program(&program, "main", Default::default(), &ProofFacts::empty())
}

#[test]
fn a_c_pointer_read_emits_a_typed_load() {
    let source = r#"extern "C" { unsafe fn malloc() -> *mut i64; }
                    fn main() { let p = malloc(); unsafe { print(p[0]); } }"#;
    let text = ir(source);
    assert!(
        text.contains("getelementptr i64, ptr"),
        "the index must use the pointee's type: {text}"
    );
    assert!(
        text.contains("load i64, ptr"),
        "the read must load the pointee's type: {text}"
    );
}
