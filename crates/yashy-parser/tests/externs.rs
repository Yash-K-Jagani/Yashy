//! Parsing `extern` blocks and their declarations.
//!
//! The shape is fixed by the AST that predates the milestone: an ABI string, an
//! optional library to link, and one or more declarations. Every declaration
//! must say `unsafe` — the keyword is mandatory at the boundary, so the parser
//! is where a forgotten one is caught, before any type is resolved.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::ast::{Item, PtrKind, TypeExpr};
use yashy_parser::Parser;

fn parse(src: &str) -> Result<Vec<Item>, Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("test.ya", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer must not panic");
    let mut parser = Parser::new(tokens, &sources, file);
    parser.parse_program().map(|program| program.items)
}

fn extern_item(src: &str) -> (Option<String>, Option<String>, Vec<yashy_parser::ast::ExternDecl>) {
    match parse(src)
        .expect("the block must parse")
        .into_iter()
        .next()
        .expect("one item")
    {
        Item::Extern {
            abi, link, decls, ..
        } => (abi, link, decls),
        other => panic!("expected an extern item, got {other:?}"),
    }
}

#[test]
fn an_extern_block_parses_its_abi_library_and_declarations() {
    let (abi, link, decls) = extern_item(
        r#"
        extern "C" link "m" {
            unsafe fn puts(s: *const u8) -> i32;
            unsafe fn strlen(s: *const u8) -> usize;
        }
        "#,
    );
    assert_eq!(abi.as_deref(), Some("C"));
    assert_eq!(link.as_deref(), Some("m"));
    assert_eq!(decls.len(), 2);
    assert_eq!(decls[0].name, "puts");
    assert_eq!(decls[1].name, "strlen");
    assert!(matches!(
        decls[0].ret,
        Some(TypeExpr::Name(ref name, _)) if name == "i32"
    ));
    assert!(matches!(
        decls[0].params.first().map(|p| &p.ty),
        Some(TypeExpr::Ptr {
            kind: PtrKind::Const,
            ..
        })
    ));
}

#[test]
fn a_single_declaration_needs_no_braces() {
    let (abi, link, decls) = extern_item(r#"extern "C" unsafe fn abs(x: i32) -> i32;"#);
    assert_eq!(abi.as_deref(), Some("C"));
    assert_eq!(link, None);
    assert_eq!(decls.len(), 1);
    assert_eq!(decls[0].name, "abs");
}

#[test]
fn an_extern_declaration_must_be_unsafe() {
    let diagnostics = parse(r#"extern "C" { fn puts(s: *const u8) -> i32; }"#)
        .expect_err("a declaration without `unsafe` must be rejected");
    assert!(
        diagnostics.iter().any(|d| d.message.contains("unsafe")),
        "the diagnostic must name the missing marker: {diagnostics:?}"
    );
}

#[test]
fn a_trailing_marker_makes_a_declaration_variadic() {
    let (_, _, decls) =
        extern_item(r#"extern "C" { unsafe fn printf(fmt: *const u8, ...) -> i32; }"#);
    assert_eq!(decls.len(), 1);
    assert!(decls[0].variadic, "the marker must be recorded");
    assert_eq!(decls[0].params.len(), 1, "the fixed parameters stay named");
    assert_eq!(decls[0].params[0].name, "fmt");
}

#[test]
fn a_marker_before_another_parameter_is_e0902() {
    let diagnostics = parse(r#"extern "C" { unsafe fn f(a: i32, ..., b: i32) -> i32; }"#)
        .expect_err("nothing may follow `...`");
    assert!(
        diagnostics.iter().any(|d| d.code == Some("E0902")
            && d.message.contains("must be the last parameter")),
        "the marker must be reported as misplaced: {diagnostics:?}"
    );
}

#[test]
fn a_marker_on_a_yashy_fn_is_e0902() {
    let diagnostics = parse("fn show(...) { }").expect_err("a Yashy `fn` cannot be variadic");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == Some("E0902") && d.message.contains("`extern` block")),
        "the marker must be reported as misplaced: {diagnostics:?}"
    );
}

#[test]
fn a_marker_in_a_function_type_is_e0902() {
    let diagnostics =
        parse("fn apply(f: fn(*const u8, ...) -> i32) { }").expect_err("a function type is fixed");
    assert!(
        diagnostics.iter().any(|d| d.code == Some("E0902")),
        "the marker must be reported as misplaced: {diagnostics:?}"
    );
}

#[test]
fn an_empty_extern_block_is_a_diagnostic() {
    let diagnostics =
        parse(r#"extern "C" { }"#).expect_err("a block with no declarations must be rejected");
    assert!(
        diagnostics.iter().any(|d| d.message.contains("at least one")),
        "the diagnostic must say the block is empty: {diagnostics:?}"
    );
}
