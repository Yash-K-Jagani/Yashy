//! ALIAS-2 in practice: a pointer that came from C may alias anything, so the
//! read through it has to be spelled inside an `unsafe` block. Owning pointers
//! keep their safe, zeroed indexing.

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

const MALLOC: &str = r#"extern "C" { unsafe fn malloc() -> *mut i64; }"#;

#[test]
fn a_c_pointer_cannot_be_dereferenced_in_safe_code() {
    let source = format!("{MALLOC}\nfn main() {{ let p = malloc(); let v = p[0]; print(v); }}");
    let diagnostics = check(&source).expect_err("a C pointer read is not a safe operation");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == Some(codes::FFI_POINTER_DEREF)),
        "expected E1406, got {diagnostics:?}"
    );
}

#[test]
fn an_unsafe_block_licenses_the_c_pointer_read() {
    let source = format!("{MALLOC}\nfn main() {{ let p = malloc(); unsafe {{ print(p[0]); }} }}");
    check(&source).expect("the unsafe block is the explicit promise");
}

#[test]
fn an_owning_pointer_is_still_safe_to_index() {
    check("fn main() { let p = alloc(4); print(p[0]); }")
        .expect("linear owning pointers were never gated");
}
