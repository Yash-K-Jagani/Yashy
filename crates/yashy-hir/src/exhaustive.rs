//! Exhaustiveness checking for `switch` arms.
//!
//! Given an enum's variants and the patterns of a `switch`'s arms, this
//! module reports which variants no arm covers. It knows nothing about
//! types, scopes or lifetimes -- it is pure shape matching over [`Pattern`]
//! and [`VariantDef`] -- so it can be unit-tested without the rest of the
//! type checker, and called again once sema resolves variant paths.
//!
//! # No input makes this module panic
//!
//! Every cursor over `variants` and `arms` is bounded by their lengths, and
//! every lookup is a membership test rather than an index. There is no
//! `unwrap`, no `expect`, no `panic!` and no unchecked indexing here.

pub use yashy_parser::ast::{Pattern, VariantDef};

/// The names of the enum variants no arm covers, in declaration order.
/// Empty means the arms are exhaustive.
pub fn missing_cases(
    _enum_name: &str,
    variants: &[VariantDef],
    arms: &[Pattern],
    has_default: bool,
) -> Vec<String> {
    if has_default {
        return Vec::new();
    }
    if arms.iter().any(matches_everything) {
        return Vec::new();
    }
    let mut covered: Vec<String> = Vec::new();
    for pattern in arms {
        if let Some(segment) = variant_segment(pattern) {
            if !covered.contains(&segment) {
                covered.push(segment);
            }
        }
    }
    variants
        .iter()
        .filter(|variant| !covered.contains(&variant.name))
        .map(|variant| variant.name.clone())
        .collect()
}

/// True if a single arm matches every possible value.
fn matches_everything(pattern: &Pattern) -> bool {
    matches!(pattern, Pattern::Wildcard(_) | Pattern::Binding { .. })
}

/// The final segment of a variant path, when the pattern names a variant.
/// `Binding` and `Wildcard` are handled by [`matches_everything`] instead;
/// `Literal` names no variant.
fn variant_segment(pattern: &Pattern) -> Option<String> {
    match pattern {
        Pattern::UnitVariant { path, .. } => path.last().cloned(),
        Pattern::TupleVariant { path, .. } => path.last().cloned(),
        Pattern::StructVariant { path, .. } => path.last().cloned(),
        Pattern::Binding { .. } | Pattern::Wildcard(_) | Pattern::Literal(..) => None,
    }
}

/// True when no variant is missing.
pub fn is_exhaustive(variants: &[VariantDef], arms: &[Pattern], has_default: bool) -> bool {
    missing_cases("", variants, arms, has_default).is_empty()
}
