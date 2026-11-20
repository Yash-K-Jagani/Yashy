//! Or-patterns, range patterns, and the guard that lives on the arm.
//!
//! These are the three pattern-syntax additions of phase 7. The guard was
//! already stored on `SwitchArm`; these tests pin that it stays there rather
//! than being folded into the pattern, and that `|` and `..` parse in pattern
//! position without a new token.

use yashy_common::{Diagnostic, SourceMap};
use yashy_lexer::Lexer;
use yashy_parser::ast::{
    Expr, FixedArrayElement, Item, Literal, Pattern, Program, Stmt, SwitchArm,
};
use yashy_parser::Parser;

fn parse_src(src: &str) -> Result<Program, Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("patterns.ya", src.to_string());
    let tokens = Lexer::new(src).tokenize()?;
    let mut parser = Parser::new(tokens, &sources, file);
    parser.parse_program()
}

fn parse_ok(src: &str) -> Program {
    match parse_src(src) {
        Ok(program) => program,
        Err(diags) => panic!("{src:?} must parse, got {diags:?}"),
    }
}

fn parse_err(src: &str) -> Vec<Diagnostic> {
    match parse_src(src) {
        Ok(program) => panic!("{src:?} must not parse, got {program:?}"),
        Err(diags) => diags,
    }
}

/// The arms of the first `switch` in `main`.
fn arms_of(switch: &str) -> Vec<SwitchArm> {
    let source = format!("fn main() {{ switch n {{ {switch} }} }}");
    let program = parse_ok(&source);
    let Item::Fn { body, .. } = program.items.into_iter().next().unwrap() else {
        panic!("expected a function");
    };
    let Some(Stmt::Switch { arms, .. }) = body.stmts.into_iter().next() else {
        panic!("expected a switch");
    };
    arms
}

#[test]
fn an_or_pattern_is_one_node_with_every_alternative() {
    let arms = arms_of("case 1 | 2 | 3 { 0 } default { 1 }");
    let Pattern::Or { alternatives, .. } = &arms[0].pattern else {
        panic!("expected an or-pattern, got {:?}", arms[0].pattern);
    };
    assert_eq!(alternatives.len(), 3);
    for (alternative, expected) in alternatives.iter().zip([1, 2, 3]) {
        assert!(
            matches!(alternative, Pattern::Literal(Literal::I64(value), _) if *value == expected),
            "expected literal {expected}, got {alternative:?}"
        );
    }
}

#[test]
fn a_range_pattern_parses_both_forms() {
    let arms = arms_of("case 0..10 { 0 } case 'a'..='z' { 1 } default { 2 }");
    match &arms[0].pattern {
        Pattern::Range {
            lo: Literal::I64(0),
            hi: Literal::I64(10),
            inclusive: false,
            ..
        } => {}
        other => panic!("expected an exclusive int range, got {other:?}"),
    }
    match &arms[1].pattern {
        Pattern::Range {
            lo: Literal::Char(b'a'),
            hi: Literal::Char(b'z'),
            inclusive: true,
            ..
        } => {}
        other => panic!("expected an inclusive char range, got {other:?}"),
    }
}

#[test]
fn a_guard_stays_on_the_arm_and_not_in_the_pattern() {
    let arms = arms_of("case x if x > 0 { 1 } default { 0 }");
    assert!(matches!(arms[0].pattern, Pattern::Binding { .. }));
    assert!(matches!(arms[0].guard, Some(Expr::Binary { .. })));
    assert!(arms[1].guard.is_none());
}

#[test]
fn a_float_range_is_rejected_as_e1518() {
    let diags = parse_err("fn main() { switch n { case 1.0..2.0 { 0 } default { 1 } } }");
    assert!(
        diags.iter().any(|d| d.code.as_deref() == Some("E1518")),
        "expected E1518, got {diags:?}"
    );
}

#[test]
fn a_range_with_its_bounds_reversed_is_rejected_as_e0602() {
    let diags = parse_err("fn main() { switch n { case 10..0 { 0 } default { 1 } } }");
    assert!(
        diags.iter().any(|d| d.code.as_deref() == Some("E0602")),
        "expected E0602, got {diags:?}"
    );
}

#[test]
fn a_struct_pattern_may_write_the_rest_marker() {
    let arms = arms_of("case Point { x: 1, .. } { 0 } default { 1 }");
    match &arms[0].pattern {
        Pattern::StructVariant { fields, rest, .. } => {
            assert!(rest);
            assert_eq!(fields.len(), 1);
        }
        other => panic!("expected a struct pattern, got {other:?}"),
    }
}

#[test]
fn at_binding_wraps_a_nested_pattern() {
    let program = parse_ok("enum Shape { Circle(int) } fn main() { switch Shape.Circle(1) { case whole @ Circle(n) { print(n); } default { 0 } } }");
    assert!(
        format!("{program:?}").contains("BindAt"),
        "the AST must keep the @ form"
    );
}

#[test]
fn a_float_range_is_e1518() {
    let errors =
        parse_err("fn main() { switch 1.5 { case 1.0..2.0 { print(\"x\"); } default { } } }");
    assert!(
        errors
            .iter()
            .any(|e| e.code == Some(yashy_common::codes::FLOAT_RANGE)),
        "E1518 expected: {errors:?}"
    );
}

#[test]
fn a_fixed_array_pattern_names_its_elements() {
    let arms = arms_of("case [a, b] { 0 } default { 1 }");
    let Pattern::FixedArray { elements, .. } = &arms[0].pattern else {
        panic!("expected a fixed-array pattern, got {:?}", arms[0].pattern);
    };
    assert_eq!(elements.len(), 2);
    for element in elements {
        assert!(
            matches!(element, FixedArrayElement::Pattern(Pattern::Binding { .. })),
            "expected a binding, got {element:?}"
        );
    }
}

#[test]
fn a_rest_may_be_bare_or_named_and_sit_in_the_middle() {
    let arms = arms_of("case [first, .., last] { 0 } case [head, ..rest] { 1 } default { 2 }");
    let Pattern::FixedArray { elements, .. } = &arms[0].pattern else {
        panic!("expected a fixed-array pattern, got {:?}", arms[0].pattern);
    };
    assert_eq!(elements.len(), 3);
    assert!(
        matches!(&elements[1], FixedArrayElement::Rest { name: None, .. }),
        "a bare `..` binds nothing, got {:?}",
        elements[1]
    );
    let Pattern::FixedArray { elements, .. } = &arms[1].pattern else {
        panic!("expected a fixed-array pattern, got {:?}", arms[1].pattern);
    };
    assert!(
        matches!(&elements[1], FixedArrayElement::Rest { name: Some(name), .. } if name == "rest"),
        "`..rest` keeps its name, got {:?}",
        elements[1]
    );
}

#[test]
fn two_rests_in_one_fixed_array_pattern_are_e0606() {
    let errors = parse_err("fn main() { let [a, .., ..] = xs; }");
    assert!(
        errors
            .iter()
            .any(|e| e.code == Some(yashy_common::codes::BAD_ARRAY_PATTERN)),
        "E0606 expected: {errors:?}"
    );
}

#[test]
fn an_empty_fixed_array_pattern_is_e0606() {
    let errors = parse_err("fn main() { let [] = xs; }");
    assert!(
        errors
            .iter()
            .any(|e| e.code == Some(yashy_common::codes::BAD_ARRAY_PATTERN)),
        "E0606 expected: {errors:?}"
    );
}
