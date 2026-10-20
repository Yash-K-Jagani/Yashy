//! The M6 Task 10 builtins at the checking edge: files, the environment, and
//! the substring used by the stdlib's `trim` and `split`.

use otter_common::{Diagnostic, SourceMap};
use otter_lexer::Lexer;
use otter_parser::Parser;
use otter_sema::{check_program, HirProgram};

fn check(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("test.ot", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .expect("the parser must accept the program");
    check_program(&program, &sources, file)
}

fn assert_rejected(src: &str, needle: &str) {
    match check(src) {
        Ok(_) => panic!("expected an error containing `{needle}`"),
        Err(diagnostics) => assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains(needle)),
            "expected an error containing `{needle}`, got {diagnostics:?}"
        ),
    }
}

#[test]
fn read_file_yields_an_owned_string() {
    check("fn main() { var text: string = read_file(\"notes.txt\"); print_str(text); }")
        .expect("read_file takes a path and answers with a string");
}

#[test]
fn write_file_yields_a_status_integer() {
    check("fn main() { var ok: i64 = write_file(\"notes.txt\", \"body\"); print_i64(ok); }")
        .expect("write_file takes a path and a body and answers 1 or 0");
}

#[test]
fn getenv_yields_a_string() {
    check("fn main() { var home: string = getenv(\"HOME\"); print_str(home); }")
        .expect("getenv takes a name and answers with the value or an empty string");
}

#[test]
fn str_slice_takes_two_bounds() {
    check("fn main() { var word: string = str_slice(\"hello\", 1, 3); print_str(word); }")
        .expect("str_slice is (string, i64, i64) -> string");
    assert_rejected(
        "fn main() { var word = str_slice(\"hello\", 1); }",
        "expected 3 arguments",
    );
}

#[test]
fn file_builtins_check_their_argument_types() {
    assert_rejected(
        "fn main() { var text = read_file(1); }",
        "expected `string`",
    );
    assert_rejected(
        "fn main() { var ok = write_file(\"x\", 1); }",
        "expected `string`",
    );
    assert_rejected(
        "fn main() { var value = getenv(true); }",
        "expected `string`",
    );
    assert_rejected(
        "fn main() { var word = str_slice(\"x\", 0.5, 1); }",
        "expected `i64`",
    );
}
