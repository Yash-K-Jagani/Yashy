//! Pattern checking: `Option`, `Result`, and `switch` typing.

use yashy_common::Span;
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
        Pattern::StructVariant { path, fields, span } => check_struct_variant(
            path,
            fields,
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
            None => Err(TypeError::new(
                span,
                format!("cannot find type `{enum_name}`"),
            )),
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
            None => Err(TypeError::new(
                span,
                format!("cannot find type `{enum_name}`"),
            )),
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
                return Err(TypeError::new(
                    span,
                    format!("cannot find type `{enum_name}`"),
                ));
            }
        },
        _ => {
            return Err(TypeError::new(
                span,
                format!("`{name}` is not a variant of `{scrutinee}`"),
            ));
        }
    };
    if fields.len() != declared.len() {
        return Err(TypeError::new(
            span,
            format!("expected {} fields, got {}", declared.len(), fields.len()),
        ));
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
                );
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
        span,
    })
}
