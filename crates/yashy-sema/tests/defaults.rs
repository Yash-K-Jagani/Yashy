//! Default field values.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program, HirProgram, HirStmt};

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

const CONFIG: &str = "struct Config { retries: int = 3, name: string } ";

#[test]
fn an_omitted_defaulted_field_checks() {
    check(&format!("{CONFIG}fn main() {{ const c = Config {{ name: \"x\" }}; }}"))
        .expect("the missing `retries` is filled by its default");
}

#[test]
fn an_explicit_value_overrides_the_default() {
    check(&format!("{CONFIG}fn main() {{ const c = Config {{ retries: 5, name: \"x\" }}; }}"))
        .expect("an explicit field wins over the default");
}

#[test]
fn a_spread_field_overrides_a_default() {
    check(&format!(
        "{CONFIG}fn main() {{ const base = Config {{ retries: 9, name: \"b\" }}; const c = Config {{ ..base }}; }}"
    ))
    .expect("the spread wins over the default");
}

#[test]
fn a_missing_field_without_a_default_is_e0201() {
    assert_error_code(
        "struct P { x: int, y: int } fn main() { const p = P { x: 1 }; }",
        "E0201",
    );
}

#[test]
fn a_call_default_is_e0102() {
    assert_error_code(
        "fn compute() -> int { return 3; } struct C { x: int = compute() } fn main() { const c = C { x: 1 }; }",
        "E0102",
    );
}

#[test]
fn a_mismatched_default_is_e0100() {
    assert_error_code(
        "struct C { x: string = 1 } fn main() { const c = C { x: \"a\" }; }",
        "E0100",
    );
}

#[test]
fn the_literal_carries_the_default_value() {
    let program = check(&format!("{CONFIG}fn main() {{ const c = Config {{ name: \"x\" }}; }}"))
        .expect("a defaulted field checks");
    let yashy_sema::HirItem::Fn(f) = program
        .items
        .iter()
        .find(|item| matches!(item, yashy_sema::HirItem::Fn(f) if f.name == "main"))
        .expect("main")
    else {
        panic!("expected a function item");
    };
    let Some(HirStmt::Const { init, .. }) = f.body.stmts.first() else {
        panic!("expected a const binding, got {:?}", f.body.stmts);
    };
    let yashy_sema::HirExpr::StructLit { fields, .. } = init else {
        panic!("expected a struct literal, got {init:?}");
    };
    assert_eq!(fields.len(), 2, "the literal carries every field");
    assert_eq!(fields[0].0, "retries");
    match &fields[0].1 {
        yashy_sema::HirExpr::Literal(yashy_sema::hir::Literal::I64(3), _, _) => {}
        other => panic!("expected the default `3`, got {other:?}"),
    }
}
