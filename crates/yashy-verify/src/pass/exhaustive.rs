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
use yashy_hir::exhaustive::{missing_cases, Pattern, VariantDef};
use yashy_hir::hir::{
    expr_ty, HirBlock, HirExpr, HirFn, HirItem, HirPattern, HirProgram, HirStmt, HirSwitchArm,
};
use yashy_hir::ty::TypeKind;

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
    // no part in exhaustiveness. A `default` does, but only when unguarded.
    if arms.iter().any(|arm| arm.is_default && arm.guard.is_none()) {
        return;
    }

    let subject_ty = expr_ty(subject);
    let variants = match &subject_ty.kind {
        TypeKind::Enum(name) => match enum_variants(program, name) {
            Some(variants) => variants,
            // The checker resolves the type before the verifier runs; an enum
            // the program does not declare is not a gap this pass can describe.
            None => return,
        },
        TypeKind::Option(_) => vec![variant("None"), variant("Some")],
        TypeKind::Result(_, _) => vec![variant("Ok"), variant("Err")],
        _ => {
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
    let missing = missing_cases("", &variants, &patterns, false);
    if missing.is_empty() {
        return;
    }
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

/// The variants of the enum called `name`, in declaration order.
fn enum_variants(program: &HirProgram, name: &str) -> Option<Vec<VariantDef>> {
    program.items.iter().find_map(|item| match item {
        HirItem::Enum(definition) if definition.name == name => Some(
            definition
                .variants
                .iter()
                .map(|variant| VariantDef {
                    name: variant.name.clone(),
                    // `missing_cases` matches names only; the payload shape is
                    // the checker's business.
                    fields: Vec::new(),
                    span: variant.span,
                })
                .collect(),
        ),
        _ => None,
    })
}

fn variant(name: &str) -> VariantDef {
    VariantDef {
        name: name.to_string(),
        fields: Vec::new(),
        span: Span::new(0, 0),
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
        HirPattern::StructVariant { path, fields, span } => Pattern::StructVariant {
            path: path.clone(),
            fields: fields
                .iter()
                .map(|(name, pattern, span)| (name.clone(), to_ast_pattern(pattern), *span))
                .collect(),
            span: *span,
        },
        HirPattern::Binding { name, span, .. } => Pattern::Binding {
            name: name.clone(),
            span: *span,
        },
        HirPattern::Literal(literal, span) => Pattern::Literal(literal.clone(), *span),
    }
}
