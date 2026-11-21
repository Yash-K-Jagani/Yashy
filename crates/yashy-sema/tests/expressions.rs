//! Range values check into the existing struct, field and comparison forms.

use yashy_common::{codes, Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program_with, CheckOptions, HirProgram};

fn check(src: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sm = SourceMap::new();
    let file = sm.add_file("expressions.ya".to_string(), src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the lexer must accept it");
    let mut parser = Parser::new(tokens, &sm, file);
    let ast = parser.parse_program().expect("the source must parse");
    check_program_with(&ast, &sm, file, CheckOptions { release: false })
}

#[test]
fn a_range_value_has_len_start_end_and_contains() {
    let program = check(
        "fn main() {\n\
             const r = 0..10;\n\
             print(r.start());\n\
             print(r.end());\n\
             print(r.len());\n\
             print(r.is_empty());\n\
             print(r.contains(3));\n\
             print(r.contains(10));\n\
         }",
    )
    .expect("every range method lowers to existing HIR");
    let text = format!("{:?}", program.items);
    assert!(text.contains("Struct(\"Range\")"), "{text}");
}

#[test]
fn membership_uses_the_range_contains_method() {
    check("fn main() { const r = 0..10; if 3 in r { } }")
        .expect("`in` desugars to the range's `contains`");
}

#[test]
fn an_inclusive_char_range_checks() {
    check("fn main() { const r = 'a'..='z'; print(r.contains('m')); }")
        .expect("char bounds convert to their integer codes");
}

#[test]
fn a_float_range_is_e1518() {
    let diagnostics =
        check("fn main() { const r = 1.0..2.0; }").expect_err("a float range has no total length");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == Some(codes::FLOAT_RANGE)),
        "E1518 expected: {diagnostics:?}"
    );
}

#[test]
fn mixed_range_bounds_are_a_type_error() {
    let diagnostics =
        check("fn main() { const r = 0..'z'; }").expect_err("both bounds must have the same type");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("expected") || d.code == Some("E0100")),
        "a type mismatch expected: {diagnostics:?}"
    );
}

#[test]
fn a_for_range_is_still_a_counted_loop() {
    let program = check("fn main() { for n in 0..10 { print(n); } }")
        .expect("the for head is not a range value");
    let text = format!("{:?}", program.items);
    assert!(text.contains("ForRange"), "{text}");
    assert!(!text.contains("Struct(\"Range\")"), "{text}");
}

#[test]
fn a_for_over_a_range_value_is_a_counted_loop() {
    let program = check("fn main() { const r = 0..4; for n in r { print(n); } }")
        .expect("a range value iterates its fields");
    let text = format!("{:?}", program.items);
    assert!(text.contains("ForRange"), "{text}");
}

#[test]
fn step_by_becomes_a_counted_loop_with_a_stride() {
    let program = check("fn main() { for n in (0..10).step_by(2) { print(n); } }")
        .expect("step_by lowers into the counted loop");
    let text = format!("{:?}", program.items);
    assert!(text.contains("ForRange"), "{text}");
    assert!(
        text.contains("Literal(I64(2)"),
        "the stride is the literal 2: {text}"
    );
}

#[test]
fn a_non_positive_step_is_refused() {
    let diagnostics = check("fn main() { for n in (0..10).step_by(0) { } }")
        .expect_err("a zero step would not terminate");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("positive step")),
        "a diagnostic is expected: {diagnostics:?}"
    );
}
