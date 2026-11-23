//! Built-in derive expansion.
//!
//! A `derive(Name)` clause is declaration data until this pass: it turns the
//! names into ordinary inherent methods, synthesized as AST and checked by the
//! same two passes as everything the user wrote. That matters for two reasons:
//! the generated methods get the same visibility, ownership and codegen rules
//! as hand-written ones, and the expansion order is the declaration order with
//! no hash-map iteration, so the bytes are stable across runs and platforms.
//!
//! `Debug`, `Eq` and `Hash` are expanded today, and only over fields whose
//! types the generated body can express. An unsupported field is reported with
//! its name rather than silently producing a method that cannot work.
//!
//! A struct gets one inherent method per derive. An enum gets a `switch` over
//! `self` with one arm per variant in declaration order, so the body names the
//! same variants the declaration does and has no other control flow to review.

use yashy_common::Span;
use yashy_lexer::StringLit;
use yashy_parser::ast::{
    BinOp, Block, CallArg, Expr, FieldDef, Item, Literal, Param, Pattern, Program, Stmt, SwitchArm,
    TypeExpr, VariantDef, VariantField,
};

/// A field the expansion knows how to render and compare: an `int`.
fn supported(field: &FieldDef) -> bool {
    matches!(&field.ty, TypeExpr::Name(name, _) if name == "int" || name == "i64")
}

/// Whether a variant field's type can be rendered, compared and folded.
fn variant_field_supported(field: &VariantField) -> bool {
    matches!(field.ty(), TypeExpr::Name(name, _) if name == "int" || name == "i64")
}

/// How a diagnostic names a variant field: by name when it has one, by
/// position when it does not.
fn variant_field_label(field: &VariantField, index: usize) -> String {
    match field {
        VariantField::Named { name, .. } => format!("field `{name}`"),
        VariantField::Positional { .. } => format!("field {index}"),
    }
}

fn text(value: String, span: Span) -> Expr {
    Expr::Literal(
        Literal::Str(StringLit {
            bytes: value.as_bytes().to_vec(),
            value,
        }),
        span,
    )
}

fn ident_expr(value: &str, span: Span) -> Expr {
    Expr::Ident(vec![value.to_string()], span)
}

fn field(base: Expr, field_name: &str, span: Span) -> Expr {
    Expr::FieldAccess {
        base: Box::new(base),
        field: field_name.to_string(),
        span,
    }
}

fn call(callee: &str, args: Vec<Expr>, span: Span) -> Expr {
    Expr::Call {
        callee: vec![callee.to_string()],
        type_args: Vec::new(),
        args: args
            .into_iter()
            .map(|value| CallArg {
                name: None,
                value,
                span,
            })
            .collect(),
        unsafe_call: false,
        span,
    }
}

fn binary(op: BinOp, lhs: Expr, rhs: Expr, span: Span) -> Expr {
    Expr::Binary {
        op,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
        span,
    }
}

/// The receiver every generated method takes.
///
/// The parser reads a bare `self` as a parameter whose type is `Self`, and the
/// checker resolves that name to the `impl` block's type. Building the same
/// shape here is what makes a generated method a method rather than an
/// associated function.
fn receiver(span: Span) -> Param {
    Param {
        name: "self".to_string(),
        ty: TypeExpr::Name("Self".to_string(), span),
        span,
        default: None,
    }
}

fn method(method_name: &str, params: Vec<Param>, ret: &str, stmts: Vec<Stmt>, span: Span) -> Item {
    Item::Fn {
        name: method_name.to_string(),
        type_params: Vec::new(),
        params,
        ret: Some(TypeExpr::Name(ret.to_string(), span)),
        requires: Vec::new(),
        ensures: Vec::new(),
        body: Block { stmts, span },
        span,
    }
}

/// Wrap a single expression as the body of a generated method.
fn returning(body: Expr, span: Span) -> Vec<Stmt> {
    vec![Stmt::Return(Some(body), span)]
}

fn boolean(value: bool, span: Span) -> Expr {
    Expr::Literal(Literal::Bool(value), span)
}

fn integer(value: i64, span: Span) -> Expr {
    Expr::Literal(Literal::I64(value), span)
}

/// One pattern that matches `variant`, binding each payload field to the name
/// at the same position in `bindings`.
///
/// A variant with no payload uses no names; a positional payload is a tuple
/// pattern and a named payload is a `field: binding` list in declaration order,
/// which is the only struct-pattern spelling the language has.
fn variant_pattern(variant: &VariantDef, bindings: &[String], span: Span) -> Pattern {
    let path = vec![variant.name.clone()];
    if variant.fields.is_empty() {
        return Pattern::UnitVariant {
            path,
            span: variant.span,
        };
    }
    let named = variant
        .fields
        .iter()
        .any(|field| matches!(field, VariantField::Named { .. }));
    if named {
        let fields = variant
            .fields
            .iter()
            .zip(bindings)
            .map(|(field, binding)| {
                let VariantField::Named { name, .. } = field else {
                    unreachable!("an enum variant is all-named or all-positional")
                };
                (
                    name.clone(),
                    Pattern::Binding {
                        name: binding.clone(),
                        span: field.span(),
                    },
                    field.span(),
                )
            })
            .collect();
        Pattern::StructVariant {
            path,
            fields,
            rest: false,
            span: variant.span,
        }
    } else {
        let args = bindings
            .iter()
            .map(|binding| Pattern::Binding {
                name: binding.clone(),
                span,
            })
            .collect();
        Pattern::TupleVariant {
            path,
            args,
            span: variant.span,
        }
    }
}

/// A pattern that only tests for `variant`, binding nothing.
fn variant_test_pattern(variant: &VariantDef) -> Pattern {
    let path = vec![variant.name.clone()];
    match &variant.fields[..] {
        [] => Pattern::UnitVariant {
            path,
            span: variant.span,
        },
        fields
            if fields
                .iter()
                .all(|field| matches!(field, VariantField::Named { .. })) =>
        {
            Pattern::StructVariant {
                path,
                fields: variant
                    .fields
                    .iter()
                    .map(|field| {
                        (
                            match field {
                                VariantField::Named { name, .. } => name.clone(),
                                VariantField::Positional { .. } => unreachable!(),
                            },
                            Pattern::Wildcard(field.span()),
                            field.span(),
                        )
                    })
                    .collect(),
                rest: false,
                span: variant.span,
            }
        }
        fields => Pattern::TupleVariant {
            path,
            args: fields
                .iter()
                .map(|field| Pattern::Wildcard(field.span()))
                .collect(),
            span: variant.span,
        },
    }
}

/// `left + right` as a string concatenation.
fn concat(left: Expr, right: Expr, span: Span) -> Expr {
    binary(BinOp::Add, left, right, span)
}

/// The `Debug` body of one enum arm: the variant name followed by its payload
/// in the same order the declaration writes it.
fn enum_debug_arm(variant: &VariantDef, bindings: &[String], span: Span) -> Expr {
    let mut body = text(variant.name.clone(), span);
    if variant.fields.is_empty() {
        return body;
    }
    let named = variant
        .fields
        .iter()
        .any(|field| matches!(field, VariantField::Named { .. }));
    body = concat(
        body,
        text(
            if named {
                " { ".to_string()
            } else {
                "(".to_string()
            },
            span,
        ),
        span,
    );
    for (index, (field, binding)) in variant.fields.iter().zip(bindings).enumerate() {
        if index > 0 {
            body = concat(body, text(", ".to_string(), span), span);
        }
        if let VariantField::Named { name, .. } = field {
            body = concat(body, text(format!("{name}: "), span), span);
        }
        body = concat(
            body,
            call("str_from_i64", vec![ident_expr(binding, span)], span),
            span,
        );
    }
    body = concat(
        body,
        text(
            if named {
                " }".to_string()
            } else {
                ")".to_string()
            },
            span,
        ),
        span,
    );
    body
}

/// The `Eq` body of one enum arm: a switch over `other` that compares only the
/// matching variant's payload.
fn enum_eq_arm(variant: &VariantDef, left: &[String], variants: &[VariantDef], span: Span) -> Expr {
    let arms = variants
        .iter()
        .map(|other| {
            let matches = other.name == variant.name;
            let (pattern, body) = if matches {
                let bindings: Vec<String> =
                    (0..other.fields.len()).map(|i| format!("b{i}")).collect();
                let mut compared = boolean(true, span);
                for (index, (left, right)) in left.iter().zip(bindings.iter()).enumerate() {
                    let comparison = binary(
                        BinOp::Eq,
                        ident_expr(left, span),
                        ident_expr(right, span),
                        span,
                    );
                    compared = if index == 0 {
                        comparison
                    } else {
                        binary(BinOp::And, compared, comparison, span)
                    };
                }
                (
                    variant_pattern(other, &bindings, span),
                    if other.fields.is_empty() {
                        boolean(true, span)
                    } else {
                        compared
                    },
                )
            } else {
                (variant_test_pattern(other), boolean(false, span))
            };
            SwitchArm {
                pattern,
                guard: None,
                body,
                is_default: false,
                span,
            }
        })
        .collect();
    Expr::SwitchExpr {
        subject: Box::new(ident_expr("other", span)),
        arms,
        span,
    }
}

/// The `Hash` body of one enum arm: a fixed polynomial over the variant's
/// position and each payload field, in declaration order.
fn enum_hash_arm(bindings: &[String], index: usize, span: Span) -> Expr {
    let mut hash = binary(
        BinOp::Add,
        binary(
            BinOp::Mul,
            integer(17, span),
            integer(1_000_003, span),
            span,
        ),
        integer(index as i64 + 1, span),
        span,
    );
    for binding in bindings {
        hash = binary(
            BinOp::Add,
            binary(BinOp::Mul, hash, integer(1_000_003, span), span),
            ident_expr(binding, span),
            span,
        );
    }
    hash
}

/// A `switch self { case Variant(..) { .. } ... }` with one arm per variant, in
/// declaration order. `arm` builds the body from the variant, its bound names
/// and the variant's position.
fn enum_method_body(
    variants: &[VariantDef],
    span: Span,
    arm: impl Fn(&VariantDef, &[String], usize) -> Expr,
) -> Expr {
    let arms = variants
        .iter()
        .enumerate()
        .map(|(index, variant)| {
            let bindings: Vec<String> = (0..variant.fields.len())
                .map(|field| format!("f{field}"))
                .collect();
            SwitchArm {
                pattern: variant_pattern(variant, &bindings, span),
                guard: None,
                body: arm(variant, &bindings, index),
                is_default: false,
                span,
            }
        })
        .collect();
    Expr::SwitchExpr {
        subject: Box::new(ident_expr("self", span)),
        arms,
        span,
    }
}

/// Expand the derives on one enum, or report the first unsupported field.
fn expand_enum(
    name: &str,
    generic: bool,
    variants: &[VariantDef],
    derives: &[String],
    span: Span,
    errors: &mut Vec<(Span, String)>,
    added: &mut Vec<Item>,
) {
    for derive in derives {
        if generic {
            errors.push((
                span,
                format!(
                    "cannot derive `{derive}` on a generic enum: expand it for one concrete type"
                ),
            ));
            continue;
        }
        let supported_derive = matches!(derive.as_str(), "Debug" | "Eq" | "Hash");
        if !supported_derive {
            errors.push((
                span,
                format!("`{derive}` is not a derive this compiler knows"),
            ));
            continue;
        }
        let mut unsupported = None;
        for variant in variants {
            for (index, field) in variant.fields.iter().enumerate() {
                if !variant_field_supported(field) {
                    unsupported = Some((
                        field.span(),
                        format!(
                            "cannot derive `{derive}`: {} of variant `{}` is not an `int`",
                            variant_field_label(field, index),
                            variant.name
                        ),
                    ));
                    break;
                }
            }
            if unsupported.is_some() {
                break;
            }
        }
        if let Some(error) = unsupported {
            errors.push(error);
            continue;
        }
        let body = match derive.as_str() {
            "Debug" => enum_method_body(variants, span, |variant, bindings, _| {
                enum_debug_arm(variant, bindings, span)
            }),
            "Eq" => enum_method_body(variants, span, |variant, bindings, _| {
                enum_eq_arm(variant, bindings, variants, span)
            }),
            "Hash" => enum_method_body(variants, span, |_, bindings, index| {
                enum_hash_arm(bindings, index, span)
            }),
            _ => unreachable!("the derive name was checked above"),
        };
        let (method_name, params, ret) = match derive.as_str() {
            "Debug" => ("debug", vec![receiver(span)], "string"),
            "Eq" => (
                "eq",
                vec![
                    receiver(span),
                    Param {
                        name: "other".to_string(),
                        ty: TypeExpr::Name(name.to_string(), span),
                        span,
                        default: None,
                    },
                ],
                "bool",
            ),
            "Hash" => ("hash", vec![receiver(span)], "int"),
            _ => unreachable!("the derive name was checked above"),
        };
        added.push(Item::Impl {
            type_name: name.to_string(),
            trait_name: None,
            type_params: Vec::new(),
            methods: vec![method(
                method_name,
                params,
                ret,
                returning(body, span),
                span,
            )],
            span,
        });
    }
}

/// Return the program with one synthesized impl per derive name. Errors name
/// the field that could not be handled; the caller reports them at the clause.
pub fn expand(program: &Program) -> Result<Program, Vec<(Span, String)>> {
    let mut errors = Vec::new();
    let mut added = Vec::new();
    for item in &program.items {
        match item {
            Item::Struct {
                name,
                fields,
                derives,
                span,
                ..
            } => {
                for derive in derives {
                    match derive.as_str() {
                        "Debug" => {
                            if let Some(unsupported) = fields.iter().find(|field| !supported(field))
                            {
                                errors.push((
                                    unsupported.span,
                                    format!(
                                        "cannot derive `Debug`: field `{}` is not an `int`",
                                        unsupported.name
                                    ),
                                ));
                                continue;
                            }
                            let mut body = text(format!("{name} {{ "), *span);
                            for (index, field_def) in fields.iter().enumerate() {
                                body = binary(
                                    BinOp::Add,
                                    body,
                                    text(format!("{}: ", field_def.name), *span),
                                    *span,
                                );
                                body = binary(
                                    BinOp::Add,
                                    body,
                                    call(
                                        "str_from_i64",
                                        vec![field(
                                            ident_expr("self", *span),
                                            &field_def.name,
                                            *span,
                                        )],
                                        *span,
                                    ),
                                    *span,
                                );
                                if index + 1 < fields.len() {
                                    body = binary(
                                        BinOp::Add,
                                        body,
                                        text(", ".to_string(), *span),
                                        *span,
                                    );
                                }
                            }
                            body = binary(BinOp::Add, body, text(" }".to_string(), *span), *span);
                            added.push(Item::Impl {
                                type_name: name.clone(),
                                trait_name: None,
                                type_params: Vec::new(),
                                methods: vec![method(
                                    "debug",
                                    vec![receiver(*span)],
                                    "string",
                                    returning(body, *span),
                                    *span,
                                )],
                                span: *span,
                            });
                        }
                        "Eq" => {
                            if let Some(unsupported) = fields.iter().find(|field| !supported(field))
                            {
                                errors.push((
                                    unsupported.span,
                                    format!(
                                        "cannot derive `Eq`: field `{}` is not an `int`",
                                        unsupported.name
                                    ),
                                ));
                                continue;
                            }
                            let Some(first) = fields.first() else {
                                continue;
                            };
                            let mut body = binary(
                                BinOp::Eq,
                                field(ident_expr("self", *span), &first.name, *span),
                                field(ident_expr("other", *span), &first.name, *span),
                                *span,
                            );
                            for field_def in fields.iter().skip(1) {
                                let comparison = binary(
                                    BinOp::Eq,
                                    field(ident_expr("self", *span), &field_def.name, *span),
                                    field(ident_expr("other", *span), &field_def.name, *span),
                                    *span,
                                );
                                body = binary(BinOp::And, body, comparison, *span);
                            }
                            let other = Param {
                                name: "other".to_string(),
                                // The concrete type, not `Self`: `Self` names a type
                                // only in the receiver position, so a value parameter
                                // has to spell the name the `impl` block is for.
                                ty: TypeExpr::Name(name.clone(), *span),
                                span: *span,
                                default: None,
                            };
                            added.push(Item::Impl {
                                type_name: name.clone(),
                                trait_name: None,
                                type_params: Vec::new(),
                                methods: vec![method(
                                    "eq",
                                    vec![receiver(*span), other],
                                    "bool",
                                    returning(body, *span),
                                    *span,
                                )],
                                span: *span,
                            });
                        }
                        "Hash" => {
                            if let Some(unsupported) = fields.iter().find(|field| !supported(field))
                            {
                                errors.push((
                                    unsupported.span,
                                    format!(
                                        "cannot derive `Hash`: field `{}` is not an `int`",
                                        unsupported.name
                                    ),
                                ));
                                continue;
                            }
                            // A fixed polynomial over the fields in declaration order.
                            // Every step uses the checked integer operations ordinary
                            // code uses, and the only iteration is over source fields,
                            // so two expansions produce the same bytes.
                            let mut stmts = vec![Stmt::Let {
                                name: "hash".to_string(),
                                ty: Some(TypeExpr::Name("int".to_string(), *span)),
                                init: Expr::Literal(Literal::I64(17), *span),
                                span: *span,
                            }];
                            for field_def in fields {
                                stmts.push(Stmt::Assign {
                                    target: ident_expr("hash", *span),
                                    value: binary(
                                        BinOp::Add,
                                        binary(
                                            BinOp::Mul,
                                            ident_expr("hash", *span),
                                            Expr::Literal(Literal::I64(1_000_003), *span),
                                            *span,
                                        ),
                                        field(ident_expr("self", *span), &field_def.name, *span),
                                        *span,
                                    ),
                                    span: *span,
                                });
                            }
                            stmts.push(Stmt::Return(Some(ident_expr("hash", *span)), *span));
                            added.push(Item::Impl {
                                type_name: name.clone(),
                                trait_name: None,
                                type_params: Vec::new(),
                                methods: vec![method(
                                    "hash",
                                    vec![receiver(*span)],
                                    "int",
                                    stmts,
                                    *span,
                                )],
                                span: *span,
                            });
                        }
                        other => errors.push((
                            *span,
                            format!("`{other}` is not a derive this compiler knows"),
                        )),
                    }
                }
            }
            Item::Enum {
                name,
                type_params,
                variants,
                derives,
                span,
                ..
            } => expand_enum(
                name,
                !type_params.is_empty(),
                variants,
                derives,
                *span,
                &mut errors,
                &mut added,
            ),
            _ => {}
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut items = program.items.clone();
    items.extend(added);
    Ok(Program { items })
}
