//! `for i, x in xs`.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program, HirProgram, HirStmt};

fn check(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("test.ya", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .expect("the parser must accept the program");
    check_program(&program, &sources, file)
}

fn main_body(program: &HirProgram) -> yashy_sema::HirBlock {
    let yashy_sema::HirItem::Fn(f) = program
        .items
        .iter()
        .find(|item| matches!(item, yashy_sema::HirItem::Fn(f) if f.name == "main"))
        .expect("a main function")
    else {
        panic!("expected a function item");
    };
    f.body.clone()
}

#[test]
fn an_indexed_loop_lowers_to_a_range_loop_with_a_read() {
    let program = check("fn main() { let xs = []int { 7, 8 }; for i, x in xs { print(x); } }")
        .expect("an indexed loop over a slice checks");
    let body = main_body(&program);
    let Some((name, loop_body)) = body.stmts.iter().find_map(|stmt| match stmt {
        HirStmt::ForRange { name, body, .. } => Some((name, body)),
        _ => None,
    }) else {
        panic!("expected a ForRange lowering, got {:?}", body.stmts);
    };
    let (name, loop_body) = (name, loop_body);
    assert_eq!(name, "i");
    assert!(
        matches!(loop_body.stmts[0], HirStmt::Destructure { .. }),
        "the body first reads the element into `x` through its pattern"
    );
}

#[test]
fn an_indexed_loop_over_a_string_is_e0100() {
    match check(r#"fn main() { for i, c in "abc" { print(c); } }"#) {
        Ok(_) => panic!("expected an error"),
        Err(diagnostics) => assert!(
            diagnostics.iter().any(|d| d.code == Some("E0100")),
            "expected E0100, got {diagnostics:?}"
        ),
    }
}

#[test]
fn an_indexed_loop_over_a_view_checks() {
    check(
        "fn main() { let xs = []int { 7, 8, 9 }; let v = xs[0..2]; for i, x in v { print(x); } }",
    )
    .expect("an indexed loop over a view checks");
}

#[test]
fn an_indexed_loop_over_a_temporary_is_rejected() {
    match check("fn main() { for i, x in []int { 1 } { print(x); } }") {
        Ok(_) => panic!("expected an error"),
        Err(diagnostics) => assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("needs an array binding")),
            "expected the binding error, got {diagnostics:?}"
        ),
    }
}
