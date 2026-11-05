//! Chained comparisons at the checking edge.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program, HirProgram};

fn check(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("test.ya", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .expect("the parser must accept the program");
    check_program(&program, &sources, file)
}

fn assert_error_code(src: &str, code: &str) {
    match check(src) {
        Ok(_) => panic!("expected an error with code {code}"),
        Err(diagnostics) => assert!(
            diagnostics.iter().any(|d| d.code == Some(code)),
            "expected code {code}, got {diagnostics:?}"
        ),
    }
}

#[test]
fn equality_in_a_chain_is_e0101() {
    assert_error_code("fn main() { const b = 1 == 1 == 1; }", "E0101");
    assert_error_code("fn main() { const b = 1 < 2 == true; }", "E0101");
}

#[test]
fn a_chain_with_a_mismatched_operand_is_e0100() {
    assert_error_code(r#"fn main() { const b = 1 < "x" < 3; }"#, "E0100");
}

#[test]
fn a_well_typed_chain_checks() {
    check("fn main() { const b = 0 <= 5 < 10; }").expect("a chain of ints checks");
    check("fn main() { const b = 'a' < 'b' < 'c'; }").expect("a chain of chars checks");
    check("fn main() { const b = 1 < 2 < 3 < 4; }").expect("a chain of any length checks");
}

#[test]
fn a_single_equality_is_not_a_chain() {
    check("fn main() { const b = 1 == 1; }").expect("a single `==` stays legal");
}
