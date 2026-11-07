//! Exhaustiveness checking: pure shape matching over [`Pattern`] and
//! [`VariantDef`], with no type checker or source map involved.
//!
//! Every test builds variant trees at synthetic spans and asserts which
//! variant names `missing_cases` reports, and in what order.

use yashy_common::Span;
use yashy_parser::ast::{Literal, Pattern, TypeExpr, VariantDef, VariantField};
use yashy_sema::exhaustive::{missing_witnesses, reachable_arms};
use yashy_sema::{is_exhaustive, missing_cases};

fn span() -> Span {
    Span::new(0, 0)
}

fn unit(name: &str) -> VariantDef {
    VariantDef {
        name: name.to_string(),
        fields: Vec::new(),
        span: span(),
    }
}

fn positional(name: &str) -> VariantDef {
    VariantDef {
        name: name.to_string(),
        fields: vec![VariantField::Positional {
            ty: TypeExpr::Name("int".to_string(), span()),
            span: span(),
        }],
        span: span(),
    }
}

fn wildcard() -> Pattern {
    Pattern::Wildcard(span())
}

fn binding(name: &str) -> Pattern {
    Pattern::Binding {
        name: name.to_string(),
        span: span(),
    }
}

fn unit_variant(path: &str) -> Pattern {
    Pattern::UnitVariant {
        path: vec![path.to_string()],
        span: span(),
    }
}

fn tuple_variant(path: &str) -> Pattern {
    Pattern::TupleVariant {
        path: vec![path.to_string()],
        args: Vec::new(),
        span: span(),
    }
}

fn tuple_variant_with(path: &str, args: Vec<Pattern>) -> Pattern {
    Pattern::TupleVariant {
        path: vec![path.to_string()],
        args,
        span: span(),
    }
}

fn or(alternatives: Vec<Pattern>) -> Pattern {
    Pattern::Or {
        alternatives,
        span: span(),
    }
}

fn range(lo: Literal, hi: Literal, inclusive: bool) -> Pattern {
    Pattern::Range {
        lo,
        hi,
        inclusive,
        span: span(),
    }
}

/// A variant whose payload is `Option[int]`, so a nested column can be checked.
fn option_variant(name: &str) -> VariantDef {
    VariantDef {
        name: name.to_string(),
        fields: vec![VariantField::Positional {
            ty: TypeExpr::Option(Box::new(TypeExpr::Name("int".to_string(), span())), span()),
            span: span(),
        }],
        span: span(),
    }
}

fn struct_variant(path: &str) -> Pattern {
    Pattern::StructVariant {
        path: vec![path.to_string()],
        fields: vec![("w".to_string(), Pattern::Wildcard(span()), span())],
        rest: false,
        span: span(),
    }
}

fn variants(names: &[&str]) -> Vec<VariantDef> {
    names.iter().map(|n| unit(n)).collect()
}

#[test]
fn wildcard_covers_everything() {
    let variants = variants(&["Red", "Green", "Blue"]);
    let arms = vec![wildcard()];
    assert!(missing_cases("Color", &variants, &arms, false).is_empty());
    assert!(is_exhaustive(&variants, &arms, false));
}

#[test]
fn default_covers_everything() {
    let variants = variants(&["Red", "Green", "Blue"]);
    let arms = vec![unit_variant("Red")];
    assert!(missing_cases("Color", &variants, &arms, true).is_empty());
    assert!(is_exhaustive(&variants, &arms, true));
}

#[test]
fn all_unit_variants_covered_is_exhaustive() {
    let variants = variants(&["Red", "Green", "Blue"]);
    let arms = vec![
        unit_variant("Red"),
        unit_variant("Green"),
        unit_variant("Blue"),
    ];
    assert!(missing_cases("Color", &variants, &arms, false).is_empty());
    assert!(is_exhaustive(&variants, &arms, false));
}

#[test]
fn missing_variants_are_reported_in_declaration_order() {
    let variants = variants(&["Red", "Green", "Blue"]);
    let arms = vec![unit_variant("Blue")];
    assert_eq!(
        missing_cases("Color", &variants, &arms, false),
        vec!["Red".to_string(), "Green".to_string()]
    );
    assert!(!is_exhaustive(&variants, &arms, false));
}

#[test]
fn binding_covers_everything_like_a_wildcard() {
    let variants = variants(&["Red", "Green"]);
    let arms = vec![binding("x")];
    assert!(missing_cases("Color", &variants, &arms, false).is_empty());
    assert!(is_exhaustive(&variants, &arms, false));
}

#[test]
fn tuple_variant_covers_its_variant() {
    let variants = vec![positional("Point")];
    let arms = vec![tuple_variant("Point")];
    assert!(missing_cases("Point", &variants, &arms, false).is_empty());
    assert!(is_exhaustive(&variants, &arms, false));
}

#[test]
fn struct_variant_covers_its_variant_even_with_partial_fields() {
    let variants = variants(&["Rect", "Circle"]);
    let arms = vec![struct_variant("Rect")];
    assert_eq!(
        missing_cases("Shape", &variants, &arms, false),
        vec!["Circle".to_string()]
    );
}

#[test]
fn literal_patterns_do_not_cover_enum_variants() {
    let variants = variants(&["Red", "Green"]);
    let arms = vec![Pattern::Literal(Literal::I64(1), span())];
    assert_eq!(
        missing_cases("Color", &variants, &arms, false),
        vec!["Red".to_string(), "Green".to_string()]
    );
}

#[test]
fn empty_variant_list_is_exhaustive() {
    let variants: Vec<VariantDef> = Vec::new();
    let arms: Vec<Pattern> = Vec::new();
    assert!(missing_cases("Empty", &variants, &arms, false).is_empty());
    assert!(is_exhaustive(&variants, &arms, false));
}

#[test]
fn duplicate_arms_do_not_double_report() {
    let variants = variants(&["Red", "Green", "Blue"]);
    let arms = vec![unit_variant("Red"), unit_variant("Red")];
    assert_eq!(
        missing_cases("Color", &variants, &arms, false),
        vec!["Green".to_string(), "Blue".to_string()]
    );
}

#[test]
fn an_or_pattern_covers_every_alternative() {
    // `Red | Green` is two rows in the usefulness matrix, so the switch is
    // exhaustive once `Blue` is covered too.
    let variants = variants(&["Red", "Green", "Blue"]);
    let arms = vec![
        or(vec![unit_variant("Red"), unit_variant("Green")]),
        unit_variant("Blue"),
    ];
    assert!(missing_cases("Color", &variants, &arms, false).is_empty());
    assert!(is_exhaustive(&variants, &arms, false));

    // Removing the `Blue` arm says which alternative is uncovered, rather than
    // treating the whole or-pattern as covering nothing or everything.
    let arms = vec![or(vec![unit_variant("Red"), unit_variant("Green")])];
    assert_eq!(
        missing_cases("Color", &variants, &arms, false),
        vec!["Blue".to_string()]
    );
}

#[test]
fn a_range_pattern_covers_only_its_interval() {
    // `0..=9` is a finite interval in an unbounded column, so `10` is missing;
    // only a wildcard or a `default` closes the column.
    let variants = vec![positional("N")];
    let arms = vec![tuple_variant_with(
        "N",
        vec![range(Literal::I64(0), Literal::I64(9), true)],
    )];
    assert_eq!(
        missing_cases("Number", &variants, &arms, false),
        vec!["N".to_string()]
    );
    assert_eq!(
        missing_witnesses("Number", &variants, &arms, false),
        vec!["N(_)".to_string()],
        "the witness names the uncovered payload shape"
    );
    assert!(missing_cases("Number", &variants, &arms, true).is_empty());

    // The upper bound is part of an inclusive range and not of an exclusive
    // one: a following `N(9)` arm is dead after `0..=9`, and reachable after
    // `0..9`.
    let nine = tuple_variant_with("N", vec![Pattern::Literal(Literal::I64(9), span())]);
    let inclusive = vec![
        tuple_variant_with("N", vec![range(Literal::I64(0), Literal::I64(9), true)]),
        nine.clone(),
    ];
    let exclusive = vec![
        tuple_variant_with("N", vec![range(Literal::I64(0), Literal::I64(9), false)]),
        nine,
    ];
    assert_eq!(
        reachable_arms("Number", &variants, &inclusive, false, &[false, false]),
        vec![true, false]
    );
    assert_eq!(
        reachable_arms("Number", &variants, &exclusive, false, &[false, false]),
        vec![true, true]
    );
}

#[test]
fn nested_payloads_are_checked_column_by_column() {
    // `A(Some(1))` leaves `A(None)` uncovered even though it names `A`; the
    // matrix descends into the payload rather than stopping at the variant.
    let variants = vec![option_variant("A"), unit("B")];
    let arms = vec![
        tuple_variant_with(
            "A",
            vec![tuple_variant_with(
                "Some",
                vec![Pattern::Literal(Literal::I64(1), span())],
            )],
        ),
        unit_variant("B"),
    ];
    assert_eq!(
        missing_cases("E", &variants, &arms, false),
        vec!["A".to_string()]
    );
    assert_eq!(
        missing_witnesses("E", &variants, &arms, false),
        vec!["A(None)".to_string()]
    );

    // `A(None)` alone leaves `A(Some(_))` uncovered.
    let none_only = vec![
        tuple_variant_with("A", vec![unit_variant("None")]),
        unit_variant("B"),
    ];
    assert_eq!(
        missing_witnesses("E", &variants, &none_only, false),
        vec!["A(Some(_))".to_string()]
    );

    // `None` plus a wildcard inside `Some` covers the whole payload column,
    // and `B` was already covered: the switch is exhaustive without a
    // wildcard at the top level.
    let covered = vec![
        tuple_variant_with("A", vec![unit_variant("None")]),
        tuple_variant_with("A", vec![tuple_variant_with("Some", vec![wildcard()])]),
        unit_variant("B"),
    ];
    assert!(missing_cases("E", &variants, &covered, false).is_empty());
}

#[test]
fn a_guarded_arm_is_reachable_even_when_an_earlier_arm_covers_it() {
    // Guards are the caller's concern: a guarded arm never enters the matrix,
    // so the second `Red` here is reachable even though the first covers it.
    let variants = variants(&["Red", "Green", "Blue"]);
    let arms = vec![
        unit_variant("Red"),
        unit_variant("Red"),
        unit_variant("Green"),
        unit_variant("Blue"),
    ];
    assert_eq!(
        reachable_arms(
            "Color",
            &variants,
            &arms,
            false,
            &[false, true, false, false]
        ),
        vec![true, true, true, true]
    );
    // Without the guard the second arm is dead.
    assert_eq!(
        reachable_arms(
            "Color",
            &variants,
            &arms,
            false,
            &[false, false, false, false]
        ),
        vec![true, false, true, true]
    );
}
