//! Exhaustiveness checking: pure shape matching over [`Pattern`] and
//! [`VariantDef`], with no type checker or source map involved.
//!
//! Every test builds variant trees at synthetic spans and asserts which
//! variant names `missing_cases` reports, and in what order.

use otter_common::Span;
use otter_parser::ast::{Literal, Pattern, TypeExpr, VariantDef, VariantField};
use otter_sema::{is_exhaustive, missing_cases};

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
            ty: TypeExpr::Name("i64".to_string(), span()),
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

fn struct_variant(path: &str) -> Pattern {
    Pattern::StructVariant {
        path: vec![path.to_string()],
        fields: vec![("w".to_string(), Pattern::Wildcard(span()), span())],
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
