//! Exhaustiveness re-check over the HIR.
//!
//! The checker enforces this rule first, so a program that reaches the verifier
//! is already exhaustive: `check_program` returns a diagnostic and no HIR, and
//! the verifier never runs. The pass exists anyway, for the same reason the
//! verifier re-checks ownership: exhaustiveness is a soundness property -- a
//! switch with no arm for a value has nothing to run -- and the verifier is
//! where soundness rules are owned. It is the guard against a checker
//! regression, and it becomes reachable through the driver when phase 8 adds
//! generic enums, whose variant lists the checker will have to compute.
//!
//! `yashy-hir::exhaustive` holds the shape matching; this module walks the
//! program, finds every switch, and asks it. Every [`HirPattern`] has an AST
//! [`Pattern`] counterpart after checking, so the conversion is total.

use yashy_common::{codes, Diagnostic, FileId, Severity, Span};
use yashy_hir::exhaustive::{
    covering_arm, missing_witnesses, reachable_arms, Pattern, TypeExpr, VariantDef, VariantField,
};
use yashy_hir::hir::{
    expr_ty, HirBlock, HirExpr, HirFn, HirItem, HirPattern, HirProgram, HirStmt, HirSwitchArm,
};
use yashy_hir::ty::{Type, TypeKind};

/// Re-check every `switch` in `program`, appending a diagnostic per gap.
pub fn check(program: &HirProgram, out: &mut Vec<Diagnostic>) {
    for item in &program.items {
        if let HirItem::Fn(function) = item {
            check_fn(program, function, out);
        }
    }
}

fn check_fn(program: &HirProgram, function: &HirFn, out: &mut Vec<Diagnostic>) {
    check_block(program, &function.body, function.file, out);
}

fn check_block(program: &HirProgram, block: &HirBlock, file: FileId, out: &mut Vec<Diagnostic>) {
    for stmt in &block.stmts {
        check_stmt(program, stmt, file, out);
    }
}

fn check_stmt(program: &HirProgram, stmt: &HirStmt, file: FileId, out: &mut Vec<Diagnostic>) {
    match stmt {
        HirStmt::Expr(expr, ..) => check_expr(program, expr, file, out),
        HirStmt::Const { init, .. }
        | HirStmt::Var { init, .. }
        | HirStmt::Destructure { init, .. }
        | HirStmt::Assign { value: init, .. }
        | HirStmt::FieldAssign { value: init, .. } => check_expr(program, init, file, out),
        HirStmt::IndexAssign {
            base, index, value, ..
        } => {
            check_expr(program, base, file, out);
            check_expr(program, index, file, out);
            check_expr(program, value, file, out);
        }
        HirStmt::FieldPtrAssign {
            pointer,
            index,
            value,
            ..
        } => {
            check_expr(program, pointer, file, out);
            check_expr(program, index, file, out);
            check_expr(program, value, file, out);
        }
        HirStmt::Return(value, _) => {
            if let Some(value) = value {
                check_expr(program, value, file, out);
            }
        }
        HirStmt::If {
            init,
            cond,
            then_block,
            else_block,
            ..
        } => {
            if let Some(init) = init {
                check_stmt(program, init, file, out);
            }
            check_expr(program, cond, file, out);
            check_block(program, then_block, file, out);
            if let Some(else_block) = else_block {
                check_block(program, else_block, file, out);
            }
        }
        HirStmt::ForRange {
            start, end, body, ..
        } => {
            check_expr(program, start, file, out);
            check_expr(program, end, file, out);
            check_block(program, body, file, out);
        }
        HirStmt::ForIn { iterable, body, .. } => {
            check_expr(program, iterable, file, out);
            check_block(program, body, file, out);
        }
        HirStmt::ForC {
            init,
            cond,
            update,
            body,
            ..
        } => {
            if let Some(init) = init {
                check_stmt(program, init, file, out);
            }
            if let Some(cond) = cond {
                check_expr(program, cond, file, out);
            }
            if let Some(update) = update {
                check_expr(program, update, file, out);
            }
            check_block(program, body, file, out);
        }
        HirStmt::Switch {
            subject,
            arms,
            span,
        } => {
            check_expr(program, subject, file, out);
            check_switch(program, subject, arms, *span, file, out);
            check_arms(program, arms, file, out);
        }
        HirStmt::Defer { body, .. } => check_block(program, body, file, out),
        HirStmt::IfLet {
            subject,
            then_block,
            else_block,
            ..
        } => {
            // The pattern is deliberately allowed to be partial -- that is what
            // `if let` is for -- so only the bodies are walked.
            check_expr(program, subject, file, out);
            check_block(program, then_block, file, out);
            if let Some(else_block) = else_block {
                check_block(program, else_block, file, out);
            }
        }
        HirStmt::LoopInvariant(contract) => check_expr(program, &contract.expr, file, out),
        HirStmt::Break(_) | HirStmt::Continue(_) => {}
    }
}

fn check_arms(
    program: &HirProgram,
    arms: &[HirSwitchArm],
    file: FileId,
    out: &mut Vec<Diagnostic>,
) {
    for arm in arms {
        if let Some(guard) = &arm.guard {
            check_expr(program, guard, file, out);
        }
        check_expr(program, &arm.body, file, out);
    }
}

fn check_expr(program: &HirProgram, expr: &HirExpr, file: FileId, out: &mut Vec<Diagnostic>) {
    match expr {
        HirExpr::Literal(..)
        | HirExpr::Ident(..)
        | HirExpr::AddrOf { .. }
        | HirExpr::Capture { .. } => {}
        HirExpr::Binary { lhs, rhs, .. } => {
            check_expr(program, lhs, file, out);
            check_expr(program, rhs, file, out);
        }
        HirExpr::Unary { operand, .. } | HirExpr::Move(operand, ..) => {
            check_expr(program, operand, file, out);
        }
        HirExpr::Call { args, .. } | HirExpr::CallCapture { args, .. } => {
            for arg in args {
                check_expr(program, arg, file, out);
            }
        }
        HirExpr::StructLit { fields, .. } => {
            for (_, value, _) in fields {
                check_expr(program, value, file, out);
            }
        }
        HirExpr::SliceLit { elements, .. }
        | HirExpr::FixedArrayLit { elements, .. }
        | HirExpr::TupleLit { elements, .. } => {
            for element in elements {
                check_expr(program, element, file, out);
            }
        }
        HirExpr::FieldAccess { base, .. }
        | HirExpr::Cast { expr: base, .. }
        | HirExpr::Try { expr: base, .. } => {
            check_expr(program, base, file, out);
        }
        HirExpr::Index { base, index, .. } => {
            check_expr(program, base, file, out);
            check_expr(program, index, file, out);
        }
        HirExpr::View {
            base, start, end, ..
        } => {
            check_expr(program, base, file, out);
            check_expr(program, start, file, out);
            check_expr(program, end, file, out);
        }
        HirExpr::SwitchExpr {
            subject,
            arms,
            span,
            ..
        } => {
            check_expr(program, subject, file, out);
            check_switch(program, subject, arms, *span, file, out);
            check_arms(program, arms, file, out);
        }
        HirExpr::Lambda { f, .. } => check_fn(program, f, out),
        HirExpr::HigherOrder {
            receiver,
            func,
            init,
            ..
        } => {
            check_expr(program, receiver, file, out);
            check_expr(program, func, file, out);
            if let Some(init) = init {
                check_expr(program, init, file, out);
            }
        }
    }
}

/// Ask the rule whether `arms` cover every variant of `subject`'s type.
fn check_switch(
    program: &HirProgram,
    subject: &HirExpr,
    arms: &[HirSwitchArm],
    span: Span,
    file: FileId,
    out: &mut Vec<Diagnostic>,
) {
    // A guard does not cover the case where it is false, so a guarded arm takes
    // no part in exhaustiveness. A `default` is a wildcard pattern, handled by
    // the matrix itself -- except over a non-enum subject, where the matrix
    // has no variants to search and only the `default` can cover the value.
    let subject_ty = expr_ty(subject);
    let has_default = arms.iter().any(|arm| arm.is_default && arm.guard.is_none());
    let variants = match &subject_ty.kind {
        TypeKind::Enum(name) => match enum_variants(program, name) {
            Some(variants) => variants,
            // The checker resolves the type before the verifier runs; an enum
            // the program does not declare is not a gap this pass can describe.
            None => return,
        },
        TypeKind::Option(inner) => vec![
            variant("None", &[]),
            variant("Some", &[type_expr_of(inner)]),
        ],
        TypeKind::Result(ok, err) => vec![
            variant("Ok", &[type_expr_of(ok)]),
            variant("Err", &[type_expr_of(err)]),
        ],
        _ => {
            if has_default {
                return;
            }
            out.push(
                Diagnostic::new(
                    Severity::Error,
                    file,
                    span,
                    "switch over a non-enum type is not exhaustive: add a `default` arm",
                )
                .with_code(codes::NON_EXHAUSTIVE_MATCH)
                .with_label("this switch does not cover every case"),
            );
            return;
        }
    };

    let patterns: Vec<Pattern> = arms
        .iter()
        .filter(|arm| arm.guard.is_none())
        .map(|arm| to_ast_pattern(&arm.pattern))
        .collect();
    let missing = missing_witnesses("", &variants, &patterns, false);
    if !missing.is_empty() {
        let message = format!(
            "switch is not exhaustive: missing case{} {}",
            if missing.len() == 1 { "" } else { "s" },
            missing
                .iter()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        out.push(
            Diagnostic::new(Severity::Error, file, span, message)
                .with_code(codes::NON_EXHAUSTIVE_MATCH)
                .with_label("this switch does not cover every case"),
        );
    }
    // Unreachable arms: a later arm covered by an earlier unguarded arm.
    // A guarded arm is always reachable (the guard may fail).
    let enum_name = match &subject_ty.kind {
        TypeKind::Enum(name) => name.clone(),
        TypeKind::Option(_) => "Option".to_string(),
        TypeKind::Result(_, _) => "Result".to_string(),
        _ => String::new(),
    };
    let all_patterns: Vec<Pattern> = arms
        .iter()
        .map(|arm| to_ast_pattern(&arm.pattern))
        .collect();
    let guards: Vec<bool> = arms.iter().map(|arm| arm.guard.is_some()).collect();
    let reachable = reachable_arms(&enum_name, &variants, &all_patterns, false, &guards);
    for (index, (arm, is_reachable)) in arms.iter().zip(reachable.iter()).enumerate() {
        if !is_reachable {
            let label =
                match covering_arm(&enum_name, &variants, &all_patterns, false, &guards, index) {
                    Some(earlier) => format!(
                        "this arm can never match; the earlier arm `{}` already covers it",
                        render_pattern(&all_patterns[earlier])
                    ),
                    None => {
                        "this arm can never match; an earlier arm already covers it".to_string()
                    }
                };
            out.push(
                Diagnostic::new(
                    Severity::Error,
                    file,
                    arm.span,
                    "unreachable switch arm; an earlier arm already covers it".to_string(),
                )
                .with_code(codes::UNREACHABLE_ARM)
                .with_label(label),
            );
        }
    }
}

/// A short source spelling of a pattern, for naming the arm that covers a
/// dead one in an `E0601` label. Only the shape is needed, never its span.
fn render_pattern(pattern: &Pattern) -> String {
    match pattern {
        Pattern::Wildcard(_) => "_".to_string(),
        Pattern::Binding { name, .. } => name.clone(),
        Pattern::Literal(literal, _) => format!("{literal:?}"),
        Pattern::UnitVariant { path, .. } => path.last().cloned().unwrap_or_default(),
        Pattern::TupleVariant { path, args, .. } => format!(
            "{}({})",
            path.last().map(String::as_str).unwrap_or_default(),
            args.iter()
                .map(render_pattern)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Pattern::StructVariant { path, .. } => format!(
            "{} {{ .. }}",
            path.last().map(String::as_str).unwrap_or_default()
        ),
        Pattern::Tuple { elements, .. } => format!(
            "({})",
            elements
                .iter()
                .map(render_pattern)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Pattern::Or { alternatives, .. } => alternatives
            .iter()
            .map(render_pattern)
            .collect::<Vec<_>>()
            .join(" | "),
        Pattern::Range {
            lo, hi, inclusive, ..
        } => format!("{lo:?}..{}{hi:?}", if *inclusive { "=" } else { "" }),
    }
}

/// The variants of the enum called `name`, in declaration order, with their
/// payload types preserved: the usefulness matrix recurses into payload
/// columns, so a variant with no field types would look like a nullary
/// constructor and a payload pattern would be searched against nothing.
fn enum_variants(program: &HirProgram, name: &str) -> Option<Vec<VariantDef>> {
    program.items.iter().find_map(|item| match item {
        HirItem::Enum(definition) if definition.name == name => Some(
            definition
                .variants
                .iter()
                .map(|variant| VariantDef {
                    name: variant.name.clone(),
                    fields: variant
                        .fields
                        .iter()
                        .map(|field| match &field.name {
                            Some(field_name) => VariantField::Named {
                                name: field_name.clone(),
                                ty: type_expr_of(&field.ty),
                                span: field.span,
                            },
                            None => VariantField::Positional {
                                ty: type_expr_of(&field.ty),
                                span: field.span,
                            },
                        })
                        .collect(),
                    span: variant.span,
                })
                .collect(),
        ),
        _ => None,
    })
}

/// A synthetic `Option`/`Result` variant: `Some`, `Ok` and `Err` each carry
/// one payload, `None` carries none.
fn variant(name: &str, payload: &[TypeExpr]) -> VariantDef {
    VariantDef {
        name: name.to_string(),
        fields: payload
            .iter()
            .map(|ty| VariantField::Positional {
                ty: ty.clone(),
                span: Span::new(0, 0),
            })
            .collect(),
        span: Span::new(0, 0),
    }
}

/// The AST spelling of a checked type, for the shape matcher: only the kinds
/// the matrix can search (primitives, enums, `Option`, `Result`, tuples) are
/// spelled precisely; everything else becomes an unknown name, which the
/// matcher reads as an opaque leaf.
fn type_expr_of(ty: &Type) -> TypeExpr {
    let span = ty.span;
    match &ty.kind {
        TypeKind::I64 => TypeExpr::Name("int".to_string(), span),
        TypeKind::F64 => TypeExpr::Name("float".to_string(), span),
        TypeKind::Bool => TypeExpr::Name("bool".to_string(), span),
        TypeKind::Char => TypeExpr::Name("char".to_string(), span),
        TypeKind::Str => TypeExpr::Name("string".to_string(), span),
        TypeKind::Struct(name) | TypeKind::Enum(name) => TypeExpr::Name(name.clone(), span),
        TypeKind::Option(inner) => TypeExpr::Option(Box::new(type_expr_of(inner)), span),
        TypeKind::Result(ok, err) => TypeExpr::Result(
            Box::new(type_expr_of(ok)),
            Box::new(type_expr_of(err)),
            span,
        ),
        TypeKind::Tuple(elements) => {
            TypeExpr::Tuple(elements.iter().map(type_expr_of).collect(), span)
        }
        _ => TypeExpr::Name("?".to_string(), span),
    }
}

/// The AST shape of a checked pattern. Total: the checker resolves every form.
fn to_ast_pattern(pattern: &HirPattern) -> Pattern {
    match pattern {
        HirPattern::Wildcard(span) => Pattern::Wildcard(*span),
        HirPattern::UnitVariant { path, span } => Pattern::UnitVariant {
            path: path.clone(),
            span: *span,
        },
        HirPattern::TupleVariant { path, args, span } => Pattern::TupleVariant {
            path: path.clone(),
            args: args.iter().map(to_ast_pattern).collect(),
            span: *span,
        },
        HirPattern::StructVariant {
            path,
            fields,
            rest,
            span,
        } => Pattern::StructVariant {
            path: path.clone(),
            fields: fields
                .iter()
                .map(|(name, pattern, span)| (name.clone(), to_ast_pattern(pattern), *span))
                .collect(),
            rest: *rest,
            span: *span,
        },
        HirPattern::Binding { name, span, .. } => Pattern::Binding {
            name: name.clone(),
            span: *span,
        },
        HirPattern::Literal(literal, span) => Pattern::Literal(literal.clone(), *span),
        HirPattern::Range {
            lo,
            hi,
            inclusive,
            span,
            ..
        } => Pattern::Range {
            lo: lo.clone(),
            hi: hi.clone(),
            inclusive: *inclusive,
            span: *span,
        },
        HirPattern::Or { alternatives, span } => Pattern::Or {
            alternatives: alternatives.iter().map(to_ast_pattern).collect(),
            span: *span,
        },
        HirPattern::Tuple { elements, span } => Pattern::Tuple {
            elements: elements.iter().map(to_ast_pattern).collect(),
            span: *span,
        },
    }
}
