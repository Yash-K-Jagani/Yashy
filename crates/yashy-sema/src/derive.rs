//! Built-in derive expansion.
//!
//! A `derive(Name)` clause is declaration data until this pass: it turns the
//! names into ordinary inherent methods, synthesized as AST and checked by the
//! same two passes as everything the user wrote. That matters for two reasons:
//! the generated methods get the same visibility, ownership and codegen rules
//! as hand-written ones, and the expansion order is the declaration order with
//! no hash-map iteration, so the bytes are stable across runs and platforms.
//!
//! Only `Debug` and `Eq` are expanded today, and only over fields whose types
//! the generated body can express. An unsupported field is reported with its
//! name rather than silently producing a method that cannot work.

use yashy_common::Span;
use yashy_lexer::StringLit;
use yashy_parser::ast::{
    BinOp, Block, CallArg, Expr, FieldDef, Item, Literal, Param, Program, Stmt, TypeExpr,
};

/// A field the expansion knows how to render and compare: an `int`.
fn supported(field: &FieldDef) -> bool {
    matches!(&field.ty, TypeExpr::Name(name, _) if name == "int" || name == "i64")
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

fn method(method_name: &str, params: Vec<Param>, ret: &str, body: Expr, span: Span) -> Item {
    Item::Fn {
        name: method_name.to_string(),
        type_params: Vec::new(),
        params,
        ret: Some(TypeExpr::Name(ret.to_string(), span)),
        requires: Vec::new(),
        ensures: Vec::new(),
        body: Block {
            stmts: vec![Stmt::Return(Some(body), span)],
            span,
        },
        span,
    }
}

/// Return the program with one synthesized impl per derive name. Errors name
/// the field that could not be handled; the caller reports them at the clause.
pub fn expand(program: &Program) -> Result<Program, Vec<(Span, String)>> {
    let mut errors = Vec::new();
    let mut added = Vec::new();
    for item in &program.items {
        let Item::Struct {
            name,
            fields,
            derives,
            span,
            ..
        } = item
        else {
            continue;
        };
        for derive in derives {
            match derive.as_str() {
                "Debug" => {
                    if let Some(unsupported) = fields.iter().find(|field| !supported(field)) {
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
                                vec![field(ident_expr("self", *span), &field_def.name, *span)],
                                *span,
                            ),
                            *span,
                        );
                        if index + 1 < fields.len() {
                            body = binary(BinOp::Add, body, text(", ".to_string(), *span), *span);
                        }
                    }
                    body = binary(BinOp::Add, body, text(" }".to_string(), *span), *span);
                    added.push(Item::Impl {
                        type_name: name.clone(),
                        trait_name: None,
                        methods: vec![method("debug", Vec::new(), "string", body, *span)],
                        span: *span,
                    });
                }
                "Eq" => {
                    if let Some(unsupported) = fields.iter().find(|field| !supported(field)) {
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
                        ty: TypeExpr::Name("Self".to_string(), *span),
                        span: *span,
                        default: None,
                    };
                    added.push(Item::Impl {
                        type_name: name.clone(),
                        trait_name: None,
                        methods: vec![method("eq", vec![other], "bool", body, *span)],
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
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut items = program.items.clone();
    items.extend(added);
    Ok(Program { items })
}
