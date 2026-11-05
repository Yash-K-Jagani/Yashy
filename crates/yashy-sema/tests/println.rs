//! Variadic `println`.

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

#[test]
fn println_accepts_many_values_of_many_types() {
    check(r#"fn main() { println("sum", 1 + 2, 'a', true); }"#)
        .expect("`println` accepts any number of printable values");
    check("fn main() { println(); }").expect("`println` with no values prints an empty line");
    check("fn main() { println(42); }").expect("a single value still works");
}

#[test]
fn println_still_refuses_unprintable_values() {
    match check("fn f(fn() -> int g) { println(g); } fn main() { }") {
        Ok(_) => panic!("expected an error"),
        Err(diagnostics) => assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("cannot print")),
            "got {diagnostics:?}"
        ),
    }
}
