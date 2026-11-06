//! Pattern checking: `Option`, `Result`, and `switch` typing.

use yashy_common::{codes, Span};
use yashy_parser::ast::{Pattern, TypeExpr, VariantDef, VariantField};

use crate::error::TypeError;
use crate::hir::HirPattern;
use crate::ty::{Mutability, Type, TypeKind};

/// Check one pattern against the scrutinee type.
///
/// Bindings are collected into `out_bindings` as
/// `(name, type, mutability, span)` tuples rather than declared directly, so
/// the caller owns scope pushes/pops and duplicate detection stays in one
/// place ([`crate::scope::Scope::declare`]). Patterns bind by value, hence
/// [`Mutability::Const`].
///
/// `variants` resolves an enum name to its declared variants; it returns
/// `None` for `Option`/`Result` (handled structurally here) and for unknown
/// names. `resolve` turns a payload [`TypeExpr`] into a [`Type`]: variant
/// declarations name their payloads as syntax, and only the caller (which
/// owns the type scope) can resolve them.
pub fn check_pattern(
    pat: &Pattern,
    scrutinee: &Type,
    out_bindings: &mut Vec<(String, Type, Mutability, Span)>,
    variants: &dyn Fn(&str) -> Option<Vec<VariantDef>>,
    resolve: &dyn Fn(&TypeExpr) -> Result<Type, TypeError>,
) -> Result<HirPattern, TypeError> {
    match pat {
        Pattern::Wildcard(span) => Ok(HirPattern::Wildcard(*span)),
        Pattern::Binding { name, span } => {
            out_bindings.push((name.clone(), scrutinee.clone(), Mutability::Const, *span));
            Ok(HirPattern::Binding {
                name: name.clone(),
                ty: scrutinee.clone(),
                mutability: Mutability::Const,
                span: *span,
            })
        }
        Pattern::Literal(lit, span) => {
            let lit_ty = Type {
                kind: match lit {
                    yashy_parser::ast::Literal::I64(_) => TypeKind::I64,
                    yashy_parser::ast::Literal::F64(_) => TypeKind::F64,
                    yashy_parser::ast::Literal::Bool(_) => TypeKind::Bool,
                    yashy_parser::ast::Literal::Str(_) => TypeKind::Str,
                    yashy_parser::ast::Literal::Char(_) => TypeKind::Char,
                },
                span: *span,
            };
            if !lit_ty.eq_ignoring_span(scrutinee) {
                return Err(TypeError::new(
                    *span,
                    format!("expected `{scrutinee}`, found `{lit_ty}`"),
                ));
            }
            Ok(HirPattern::Literal(lit.clone(), *span))
        }
        Pattern::Range {
            lo,
            hi,
            inclusive,
            span,
        } => {
            // A range names an interval of `int` or `char`, so the subject must
            // be the same scalar type. `1..2` against a `string` is not a
            // comparison in disguise; E0602 says so at parse time for the
            // bounds and here for the subject.
            let bound_ty = match lo {
                yashy_parser::ast::Literal::I64(_) => TypeKind::I64,
                yashy_parser::ast::Literal::Char(_) => TypeKind::Char,
                _ => {
                    return Err(invalid_range(
                        *span,
                        "a range pattern needs two `int` bounds or two `char` bounds",
                    ));
                }
            };
            if !matches!(scrutinee.kind, TypeKind::I64 | TypeKind::Char)
                || scrutinee.kind != bound_ty
            {
                return Err(invalid_range(
                    *span,
                    format!(
                        "a range pattern is an interval of `int` or `char`, not `{scrutinee}`"
                    ),
                ));
            }
            Ok(HirPattern::Range {
                lo: lo.clone(),
                hi: hi.clone(),
                inclusive: *inclusive,
                ty: scrutinee.clone(),
                span: *span,
            })
        }
        Pattern::Or {
            alternatives,
            span,
        } => {
            if alternatives.is_empty() {
                return Err(TypeError::new(*span, "an or-pattern needs an alternative"));
            }
            let mut first: Option<(HirPattern, Vec<(String, Type, Mutability, Span)>)> = None;
            for alternative in alternatives {
                let mut scratch = Vec::new();
                let checked =
                    check_pattern(alternative, scrutinee, &mut scratch, variants, resolve)?;
                match &first {
                    None => first = Some((checked, scratch)),
                    Some((_, expected)) => {
                        if !same_bindings(expected, &scratch) {
                            return Err(TypeError::new(
                                *span,
                                "every alternative of an or-pattern must bind the same names \
                                 with the same types",
                            )
                            .with_code(codes::PARTIAL_MOVE));
                        }
                    }
                }
            }
            match first {
                Some((_checked, bindings)) => {
                    out_bindings.extend(bindings);
                    Ok(HirPattern::Or {
                        alternatives: alternatives
                            .iter()
                            .map(|alternative| {
                                let mut scratch = Vec::new();
                                check_pattern(
                                    alternative,
                                    scrutinee,
                                    &mut scratch,
                                    variants,
                                    resolve,
                                )
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                        span: *span,
                    })
                }
                None => Err(TypeError::new(*span, "an or-pattern needs an alternative")),
            }
        }
        Pattern::UnitVariant { path, span } => {
            check_unit_variant(path, scrutinee, variants, *span)?;
            Ok(HirPattern::UnitVariant {
                path: path.clone(),
                span: *span,
            })
        }
        Pattern::TupleVariant { path, args, span } => {
            let payloads = tuple_payloads(path, args.len(), scrutinee, variants, resolve, *span)?;
            let mut hir_args = Vec::with_capacity(args.len());
            for (sub, payload_ty) in args.iter().zip(payloads.iter()) {
                hir_args.push(check_pattern(
                    sub,
                    payload_ty,
                    out_bindings,
                    variants,
                    resolve,
                )?);
            }
            Ok(HirPattern::TupleVariant {
                path: path.clone(),
                args: hir_args,
                span: *span,
            })
        }
        Pattern::StructVariant {
            path,
            fields,
            rest,
            span,
        } => check_struct_variant(
            path,
            fields,
            *rest,
            scrutinee,
            out_bindings,
            variants,
            resolve,
            *span,
        ),
    }
}

/// The final path segment, which is the variant name the user sees.
fn variant_name(path: &[String]) -> String {
    path.last().cloned().unwrap_or_default()
}

/// A range-pattern diagnostic carrying E0602.
fn invalid_range(span: Span, message: impl Into<String>) -> TypeError {
    TypeError::new(span, message).with_code(codes::BAD_RANGE_PATTERN)
}

/// Two alternatives bind the same names with the same types.
fn same_bindings(
    left: &[(String, Type, Mutability, Span)],
    right: &[(String, Type, Mutability, Span)],
) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter().all(|(name, ty, _, _)| {
        right
            .iter()
            .any(|(other, other_ty, _, _)| other == name && other_ty.eq_ignoring_span(ty))
    })
}

/// A unit-variant pattern names a payload-less variant of the scrutinee.
fn check_unit_variant(
    path: &[String],
    scrutinee: &Type,
    variants: &dyn Fn(&str) -> Option<Vec<VariantDef>>,
    span: Span,
) -> Result<(), TypeError> {
    let name = variant_name(path);
    match &scrutinee.kind {
        TypeKind::Option(_) if name == "None" => Ok(()),
        TypeKind::Option(_) => Err(TypeError::new(
            span,
            format!("`{name}` is not a variant of `{scrutinee}`"),
        )),
        TypeKind::Result(_, _) => Err(TypeError::new(
            span,
            format!("`{name}` takes a payload: write `{name}(x)`"),
        )),
        TypeKind::Enum(enum_name) => match variants(enum_name) {
            Some(defs) if defs.iter().any(|d| d.name == name && d.fields.is_empty()) => Ok(()),
            Some(_) => Err(TypeError::new(
                span,
                format!("`{name}` is not a unit variant of `{scrutinee}`"),
            )),
            None => Err(
                TypeError::new(span, format!("cannot find type `{enum_name}`"))
                    .with_code(codes::UNKNOWN_TYPE),
            ),
        },
        _ => Err(TypeError::new(
            span,
            format!("`{name}` is not a variant of `{scrutinee}`"),
        )),
    }
}

/// The payload types a tuple-variant pattern must bind positionally.
fn tuple_payloads(
    path: &[String],
    arity: usize,
    scrutinee: &Type,
    variants: &dyn Fn(&str) -> Option<Vec<VariantDef>>,
    resolve: &dyn Fn(&TypeExpr) -> Result<Type, TypeError>,
    span: Span,
) -> Result<Vec<Type>, TypeError> {
    let name = variant_name(path);
    match &scrutinee.kind {
        TypeKind::Option(inner) if name == "Some" => {
            if arity != 1 {
                return Err(TypeError::new(
                    span,
                    format!("expected 1 field, got {arity}"),
                ));
            }
            Ok(vec![(**inner).clone()])
        }
        TypeKind::Result(ok, err) if name == "Ok" || name == "Err" => {
            if arity != 1 {
                return Err(TypeError::new(
                    span,
                    format!("expected 1 field, got {arity}"),
                ));
            }
            let payload = if name == "Ok" { ok } else { err };
            Ok(vec![(**payload).clone()])
        }
        TypeKind::Enum(enum_name) => match variants(enum_name) {
            Some(defs) => match defs.iter().find(|d| d.name == name) {
                Some(def) => {
                    let mut payloads = Vec::with_capacity(def.fields.len());
                    for field in &def.fields {
                        match field {
                            VariantField::Positional { ty, .. } => {
                                payloads.push(resolve(ty)?);
                            }
                            VariantField::Named { .. } => {
                                return Err(TypeError::new(
                                    span,
                                    format!("variant `{name}` has named fields"),
                                ));
                            }
                        }
                    }
                    if payloads.len() != arity {
                        return Err(TypeError::new(
                            span,
                            format!("expected {} fields, got {arity}", payloads.len()),
                        ));
                    }
                    Ok(payloads)
                }
                None => Err(TypeError::new(
                    span,
                    format!("`{name}` is not a variant of `{scrutinee}`"),
                )),
            },
            None => Err(
                TypeError::new(span, format!("cannot find type `{enum_name}`"))
                    .with_code(codes::UNKNOWN_TYPE),
            ),
        },
        _ => Err(TypeError::new(
            span,
            format!("`{name}` is not a variant of `{scrutinee}`"),
        )),
    }
}

/// Check a struct-variant pattern: every field by name, all fields required.
#[allow(clippy::too_many_arguments)]
fn check_struct_variant(
    path: &[String],
    fields: &[(String, Pattern, Span)],
    rest: bool,
    scrutinee: &Type,
    out_bindings: &mut Vec<(String, Type, Mutability, Span)>,
    variants: &dyn Fn(&str) -> Option<Vec<VariantDef>>,
    resolve: &dyn Fn(&TypeExpr) -> Result<Type, TypeError>,
    span: Span,
) -> Result<HirPattern, TypeError> {
    let name = variant_name(path);
    let declared: Vec<(String, Type)> = match &scrutinee.kind {
        TypeKind::Enum(enum_name) => match variants(enum_name) {
            Some(defs) => match defs.iter().find(|d| d.name == name) {
                Some(def) => {
                    let mut out = Vec::with_capacity(def.fields.len());
                    for field in &def.fields {
                        match field {
                            VariantField::Named { name, ty, .. } => {
                                out.push((name.clone(), resolve(ty)?));
                            }
                            VariantField::Positional { .. } => {
                                return Err(TypeError::new(
                                    span,
                                    format!("variant `{name}` has positional fields"),
                                ));
                            }
                        }
                    }
                    out
                }
                None => {
                    return Err(TypeError::new(
                        span,
                        format!("`{name}` is not a variant of `{scrutinee}`"),
                    ));
                }
            },
            None => {
                return Err(
                    TypeError::new(span, format!("cannot find type `{enum_name}`"))
                        .with_code(codes::UNKNOWN_TYPE),
                );
            }
        },
        _ => {
            return Err(TypeError::new(
                span,
                format!("`{name}` is not a variant of `{scrutinee}`"),
            ));
        }
    };
    if fields.len() != declared.len() && !rest {
        let missing: Vec<String> = declared
            .iter()
            .map(|(name, _)| name.clone())
            .filter(|name| !fields.iter().any(|(written, _, _)| written == name))
            .collect();
        let mut message = if missing.is_empty() {
            // More fields than the variant declares is caught by the loop
            // below as an unknown field.
            format!("expected {} fields, got {}", declared.len(), fields.len())
        } else {
            format!(
                "this pattern misses field{} {} of variant `{name}`",
                if missing.len() == 1 { "" } else { "s" },
                missing
                    .iter()
                    .map(|field| format!("`{field}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        if !missing.is_empty() {
            message.push_str("; write it or add `..`");
        }
        return Err(TypeError::new(span, message));
    }
    let mut hir_fields = Vec::with_capacity(fields.len());
    for (fname, sub, fspan) in fields {
        match declared.iter().find(|(n, _)| n == fname) {
            Some((_, fty)) => {
                let checked = check_pattern(sub, fty, out_bindings, variants, resolve)?;
                hir_fields.push((fname.clone(), checked, *fspan));
            }
            None => {
                let mut error = TypeError::new(
                    *fspan,
                    format!("unknown field `{fname}` in variant `{name}`"),
                )
                .with_code(codes::UNKNOWN_FIELD);
                if let Some(suggestion) =
                    yashy_common::closest(fname, declared.iter().map(|(n, _)| n.as_str()))
                {
                    error = error.with_label(format!("did you mean `{suggestion}`?"));
                }
                return Err(error);
            }
        }
    }
    Ok(HirPattern::StructVariant {
        path: path.to_vec(),
        fields: hir_fields,
        rest,
        span,
    })
}

/// The span a checked pattern was written at.
pub fn hir_pattern_span(pattern: &HirPattern) -> Span {
    match pattern {
        HirPattern::Wildcard(span)
        | HirPattern::UnitVariant { span, .. }
        | HirPattern::TupleVariant { span, .. }
        | HirPattern::StructVariant { span, .. }
        | HirPattern::Binding { span, .. }
        | HirPattern::Or { span, .. }
        | HirPattern::Range { span, .. }
        | HirPattern::Literal(_, span) => *span,
    }
}

/// Whether `pattern` matches every value of `subject`'s type.
///
/// This is the question `if let` and `while let` ask before they are built: a
/// pattern that cannot fail makes the `else` branch dead and the loop endless,
/// which is E0603. It is a property of the *form*, so it is answered here next
/// to the pattern checker rather than inferred from the arms later.
///
/// A wildcard and a binding are irrefutable for every type. A literal never is.
/// A variant pattern is irrefutable only when its enum has exactly one variant
/// and every payload pattern is irrefutable; `Option` and `Result` are two
/// variants, so `Some(x)`, `None`, `Ok(v)` and `Err(e)` all fail the test.
///
/// A nested pattern the checker admits but cannot type -- one whose payload
/// type does not resolve -- answers `false`. That is the safe direction: the
/// worst case is a form the checker does not reject, never a program it
/// wrongly accepts as irrefutable.
pub fn is_irrefutable(
    pattern: &HirPattern,
    subject: &Type,
    variants: &dyn Fn(&str) -> Option<Vec<VariantDef>>,
    resolve: &dyn Fn(&TypeExpr) -> Result<Type, TypeError>,
) -> bool {
    match pattern {
        HirPattern::Wildcard(_) | HirPattern::Binding { .. } => true,
        HirPattern::Literal(..) | HirPattern::Range { .. } => false,
        // An or-pattern matches everything when one alternative does: the arm
        // still has a way to accept any value.
        HirPattern::Or { alternatives, .. } => alternatives
            .iter()
            .any(|alternative| is_irrefutable(alternative, subject, variants, resolve)),
        HirPattern::UnitVariant { path, .. }
        | HirPattern::TupleVariant { path, .. }
        | HirPattern::StructVariant { path, .. } => {
            let TypeKind::Enum(enum_name) = &subject.kind else {
                return false;
            };
            let Some(defs) = variants(enum_name) else {
                return false;
            };
            // More than one variant means some value reaches an arm this
            // pattern does not name.
            if defs.len() != 1 {
                return false;
            }
            let def = &defs[0];
            if def.name != variant_name(path) {
                return false;
            }
            let irrefutable_payload = |ty: &TypeExpr, sub: &HirPattern| match resolve(ty) {
                Ok(ty) => is_irrefutable(sub, &ty, variants, resolve),
                Err(_) => false,
            };
            match pattern {
                HirPattern::UnitVariant { .. } => def.fields.is_empty(),
                HirPattern::TupleVariant { args, .. } => {
                    if args.len() != def.fields.len() {
                        return false;
                    }
                    args.iter().zip(def.fields.iter()).all(|(sub, field)| {
                        let VariantField::Positional { ty, .. } = field else {
                            return false;
                        };
                        irrefutable_payload(ty, sub)
                    })
                }
                HirPattern::StructVariant { fields, .. } => {
                    // `check_struct_variant` already required every declared
                    // field, so a field that does not resolve here is a
                    // declaration the checker could not type; treat it as
                    // refutable rather than guess.
                    fields.iter().all(|(name, sub, _)| {
                        def.fields
                            .iter()
                            .find_map(|field| match field {
                                VariantField::Named {
                                    name: declared, ty, ..
                                } if declared == name => Some(ty),
                                _ => None,
                            })
                            .map(|ty| irrefutable_payload(ty, sub))
                            .unwrap_or(false)
                    })
                }
                _ => false,
            }
        }
    }
}
