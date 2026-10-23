//! The M6 Task 10 builtins at the checking edge: files, the environment, and
//! the string slice method the stdlib transforms are built on.

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
    check("fn main() { var text: string = read_file(\"notes.txt\"); print(text); }")
        .expect("read_file takes a path and answers with a string");
}

#[test]
fn write_file_yields_a_status_integer() {
    check("fn main() { var ok: int = write_file(\"notes.txt\", \"body\"); print(ok); }")
        .expect("write_file takes a path and a body and answers 1 or 0");
}

#[test]
fn getenv_yields_a_string() {
    check("fn main() { var home: string = getenv(\"HOME\"); print(home); }")
        .expect("getenv takes a name and answers with the value or an empty string");
}

#[test]
fn slice_takes_two_bounds() {
    check(
        "fn main() { var hello = \"hello\"; var word: string = hello.slice(1, 3); print(word); }",
    )
    .expect("`slice` is (start, end) -> string");
    assert_rejected(
        "fn main() { var hello = \"hello\"; var word = hello.slice(1); }",
        "`slice` takes 2 arguments",
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
        "fn main() { var text = \"x\"; var word = text.slice(0.5, 1); }",
        "expected `int`",
    );
}
