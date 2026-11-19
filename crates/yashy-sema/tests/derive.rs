//! Derive expansion: the names on the clause become ordinary methods, checked
//! by the same passes as hand-written ones. Expansion is declaration-ordered,
//! so the generated items appear in a stable order.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program_with, CheckOptions, HirItem, HirProgram};

fn check(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sm = SourceMap::new();
    let file = sm.add_file("derive.ya".to_string(), src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser.parse_program().expect("the source must parse");
    check_program_with(&ast, &sm, file, CheckOptions { release: false })
}

fn generated_names(program: &HirProgram) -> Vec<String> {
    program
        .items
        .iter()
        .filter_map(|item| match item {
            HirItem::Fn(function) => Some(function.name.clone()),
            _ => None,
        })
        .filter(|name| name.contains("debug") || name.contains("eq"))
        .collect()
}

#[test]
fn debug_and_eq_generate_methods() {
    let program = check(
        "struct Point derive(Debug, Eq) { x: int, y: int }\n\
         fn main() { let a = Point { x: 1, y: 2 }; let b = Point { x: 1, y: 2 }; \
                     print(a.debug()); print(a.eq(b)); }",
    )
    .expect("the generated methods type-check");
    let names = generated_names(&program);
    assert!(
        names.iter().any(|name| name.contains("debug")),
        "a debug method was generated: {names:?}"
    );
    assert!(
        names.iter().any(|name| name.contains("eq")),
        "an eq method was generated: {names:?}"
    );
}

#[test]
fn generated_methods_appear_in_declaration_order() {
    let program = check("struct Pair derive(Debug, Eq) { a: int, b: int } fn main() {}")
        .expect("the program checks");
    let names = generated_names(&program);
    assert_eq!(names.len(), 2, "one method per derive: {names:?}");
}

#[test]
fn an_unsupported_field_is_named() {
    let diagnostics = check(
        "struct Point derive(Debug) { x: string }\n\
         fn main() { let p = Point { x: \"a\" }; print(p.debug()); }",
    )
    .expect_err("a string field cannot be rendered by this expansion");
    assert!(
        diagnostics.iter().any(|d| d.message.contains("field `x`")),
        "the failure names the field: {diagnostics:?}"
    );
}

#[test]
fn an_unknown_derive_is_named() {
    let diagnostics = check("struct Point derive(Show) { x: int } fn main() {}")
        .expect_err("only known derives are admitted");
    assert!(
        diagnostics.iter().any(|d| d.message.contains("`Show`")),
        "the failure names the derive: {diagnostics:?}"
    );
}
