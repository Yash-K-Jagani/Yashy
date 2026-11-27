//! `Encode`/`Decode` derives: a struct gets self-contained codec methods.
//!
//! The wire form is the field values in declaration order, separated by `:`.
//! A decode answers `Result[Self, string]`; a malformed wire is an error value,
//! not a fault. The generated methods are ordinary checked functions, so the
//! same ownership and verification rules apply as to hand-written ones.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program_with, CheckOptions, HirItem, HirProgram};

fn check(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sm = SourceMap::new();
    let file = sm.add_file("codec_derive.ya".to_string(), src.to_string());
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
        .filter(|name| name.contains("encode") || name.contains("decode"))
        .collect()
}

#[test]
fn codec_derives_generate_methods() {
    let program = check(
        "struct Point derive(Encode, Decode) { x: int, y: int }\n\
         fn main() { let p = Point { x: 1, y: 2 }; let wire = p.encode(); \
                     let back = Point.decode(wire); }",
    )
    .expect("the generated codec methods must type-check");
    let names = generated_names(&program);
    assert!(
        names.iter().any(|name| name.contains("encode")),
        "an encode method was generated: {names:?}"
    );
    assert!(
        names.iter().any(|name| name.contains("decode")),
        "a decode method was generated: {names:?}"
    );
}

#[test]
fn codec_derives_are_deterministic() {
    let source = "struct Pair derive(Encode, Decode) { a: int, b: int }\nfn main() { }\n";
    let first = check(source).expect("first expansion");
    let second = check(source).expect("second expansion");
    assert_eq!(generated_names(&first), generated_names(&second));
    assert_eq!(first.items.len(), second.items.len());
}

#[test]
fn an_unsupported_field_is_named() {
    let diagnostics = check("struct Bad derive(Encode) { flag: bool }\nfn main() { }\n")
        .expect_err("a bool field cannot be encoded by this derive");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("field `flag`")),
        "the field is named: {:?}",
        diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn codec_derives_on_enums_are_refused_with_a_clear_message() {
    let diagnostics = check("enum Shape derive(Encode) { Circle }\nfn main() { }\n")
        .expect_err("codec derives are struct-only");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("codec derives are struct-only")),
        "a clear message is given: {:?}",
        diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}
