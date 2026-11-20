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
                    if let Some(unsupported) = fields.iter().find(|field| !supported(field)) {
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
                        methods: vec![method("hash", vec![receiver(*span)], "int", stmts, *span)],
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
