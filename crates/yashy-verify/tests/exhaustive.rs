//! The verifier's exhaustiveness re-check, over the HIR.
//!
//! The checker already rejects a non-exhaustive `switch`, so the pass cannot be
//! reached by writing a bad source program: `check_program` returns before the
//! verifier runs. These tests therefore ask the pass the question it exists to
//! answer by checking a good program and then removing an arm from the HIR. That
//! is the shape a checker regression would have, and the pass is the guard
//! against one.

use yashy_common::SourceMap;
use yashy_hir::hir::{HirItem, HirPattern, HirProgram, HirStmt};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::check_program;
use yashy_verify::{verify, VerifyOptions, VerifyReport};

/// Parse and check `source`, returning the HIR the verifier reads.
fn checked(source: &str) -> HirProgram {
    let mut sources = SourceMap::new();
    let file = sources.add_file("exhaustive.ya", source.to_string());
    let tokens = Lexer::new(source)
        .tokenize()
        .expect("the fixture's lexer must succeed");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser.parse_program().expect("the fixture must parse");
    check_program(&program, &sources, file).expect("the fixture must check")
}

fn codes(report: &VerifyReport) -> Vec<&'static str> {
    report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.unwrap_or("<uncoded>"))
        .collect()
}

/// Remove every arm that names `variant` from the fixture's first `switch`.
fn drop_arm(hir: &mut HirProgram, variant: &str) {
    let function = hir
        .items
        .iter_mut()
        .find_map(|item| match item {
            HirItem::Fn(function) => Some(function),
            _ => None,
        })
        .expect("the fixture has a function");
    for stmt in &mut function.body.stmts {
        if let HirStmt::Switch { arms, .. } = stmt {
            arms.retain(|arm| !names_variant(&arm.pattern, variant));
            return;
        }
    }
    panic!("the fixture has a switch");
}

fn names_variant(pattern: &HirPattern, name: &str) -> bool {
    match pattern {
        HirPattern::UnitVariant { path, .. }
        | HirPattern::TupleVariant { path, .. }
        | HirPattern::StructVariant { path, .. } => path.last().map(String::as_str) == Some(name),
        HirPattern::Wildcard(_) | HirPattern::Binding { .. } | HirPattern::Literal(..) => false,
    }
}

const THREE_COLORS: &str = "enum Color { Red, Green, Blue } \
     fn main() { let c = Color.Red; \
     switch c { case Red { 0 } case Green { 0 } case Blue { 0 } } }";

#[test]
fn a_switch_that_covers_every_variant_is_silent() {
    let hir = checked(THREE_COLORS);
    assert!(verify(&hir, &VerifyOptions::default())
        .diagnostics
        .is_empty());
}

#[test]
fn removing_an_arm_is_reported_by_the_recheck() {
    let mut hir = checked(THREE_COLORS);
    drop_arm(&mut hir, "Blue");
    let report = verify(&hir, &VerifyOptions::default());
    assert_eq!(codes(&report), vec!["E0600"]);
    assert!(report.diagnostics[0].message.contains("Blue"));
}

#[test]
fn a_default_arm_covers_what_the_named_arms_do_not() {
    let source = "enum Color { Red, Green, Blue } \
         fn main() { let c = Color.Red; \
         switch c { case Red { 0 } default { 0 } } }";
    let hir = checked(source);
    assert!(verify(&hir, &VerifyOptions::default())
        .diagnostics
        .is_empty());
}

#[test]
fn a_guarded_arm_does_not_cover_its_variant() {
    // The checker accepts this only because of the `default`. Removing it leaves
    // a guarded `Red` and an unguarded `Green`: the guard does not cover `Red`,
    // so the re-check must say so. Dropping the guard as well makes the same
    // arms exhaustive, which proves the guard was the reason.
    let source = "enum Color { Red, Green } \
         fn main() { let c = Color.Red; \
         switch c { case Red if 1 > 0 { 0 } case Green { 0 } default { 0 } } }";
    let mut hir = checked(source);
    let arms = switch_arms(&mut hir);
    arms.retain(|arm| !arm.is_default);
    assert_eq!(
        codes(&verify(&hir, &VerifyOptions::default())),
        vec!["E0600"]
    );

    let arms = switch_arms(&mut hir);
    for arm in arms.iter_mut() {
        arm.guard = None;
    }
    assert!(verify(&hir, &VerifyOptions::default())
        .diagnostics
        .is_empty());
}

fn switch_arms(hir: &mut HirProgram) -> &mut Vec<yashy_hir::hir::HirSwitchArm> {
    let function = hir
        .items
        .iter_mut()
        .find_map(|item| match item {
            HirItem::Fn(function) => Some(function),
            _ => None,
        })
        .expect("the fixture has a function");
    function
        .body
        .stmts
        .iter_mut()
        .find_map(|stmt| match stmt {
            HirStmt::Switch { arms, .. } => Some(arms),
            _ => None,
        })
        .expect("the fixture has a switch")
}
