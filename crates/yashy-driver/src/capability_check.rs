//! Capability checks over checked HIR.
//!
//! The call-site check is deliberately coarse but honest: it names the builtin
//! families that touch the world and the capability each requires. It runs
//! only when a manifest declares `[capabilities]`, so programs written before
//! capabilities existed keep building. A manifest that declares capabilities
//! asks for enforcement.

use yashy_common::{codes, Diagnostic, FileId, Severity, Span};
use yashy_hir::hir::{HirExpr, HirItem, HirProgram, HirStmt};
use yashy_manifest::Manifest;

/// The builtins that need a grant, and the capability name they need.
pub const REQUIRED: &[(&str, &str)] = &[
    ("read_file", "fs.read"),
    ("write_file", "fs.write"),
    ("getenv", "env"),
    ("now", "clock"),
    ("sleep", "clock"),
    ("spawn", "proc"),
    ("dlopen", "ffi"),
];

fn callee_name(expr: &HirExpr) -> String {
    match expr {
        HirExpr::Call { callee, .. } => callee.last().cloned().unwrap_or_default(),
        _ => String::new(),
    }
}

fn required_capability(name: &str) -> Option<&'static str> {
    REQUIRED
        .iter()
        .find(|(builtin, _)| name == *builtin)
        .map(|(_, capability)| *capability)
}

fn call_span(expr: &HirExpr) -> Span {
    match expr {
        HirExpr::Call { span, .. } => *span,
        _ => yashy_common::Span::SYNTHETIC,
    }
}

/// Diagnose calls that exceed the declared grant.
pub fn check(hir: &HirProgram, manifest: &Manifest) -> Vec<Diagnostic> {
    let Some(declared) = &manifest.capabilities else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in &hir.items {
        let HirItem::Fn(function) = item else {
            continue;
        };
        for statement in &function.body.stmts {
            visit_stmt(statement, function.file, declared, &mut out);
        }
    }
    out.sort_by_key(|diagnostic| (diagnostic.file.index(), diagnostic.span.start));
    out
}


fn visit_stmt(
    statement: &HirStmt,
    file: FileId,
    declared: &yashy_manifest::CapabilitySet,
    out: &mut Vec<Diagnostic>,
) {
    match statement {
        HirStmt::Expr(expr, _, _) => visit_expr(expr, file, declared, out),
        HirStmt::Const { init, .. } | HirStmt::Var { init, .. } => {
            visit_expr(init, file, declared, out)
        }
        HirStmt::Assign { value, .. } => visit_expr(value, file, declared, out),
        HirStmt::Return(Some(expr), _) => visit_expr(expr, file, declared, out),
        HirStmt::If {
            init,
            cond,
            then_block,
            else_block,
            ..
        } => {
            if let Some(init) = init {
                visit_stmt(init, file, declared, out);
            }
            visit_expr(cond, file, declared, out);
            visit_block(then_block, file, declared, out);
            if let Some(block) = else_block {
                visit_block(block, file, declared, out);
            }
        }
        HirStmt::ForRange { body, .. }
        | HirStmt::ForIn { body, .. }
        | HirStmt::ForC { body, .. } => visit_block(body, file, declared, out),
        _ => {}
    }
}

fn visit_block(
    block: &yashy_hir::hir::HirBlock,
    file: FileId,
    declared: &yashy_manifest::CapabilitySet,
    out: &mut Vec<Diagnostic>,
) {
    for statement in &block.stmts {
        visit_stmt(statement, file, declared, out);
    }
}

fn visit_expr(
    expr: &HirExpr,
    file: FileId,
    declared: &yashy_manifest::CapabilitySet,
    out: &mut Vec<Diagnostic>,
) {
    let name = callee_name(expr);
    if let Some(capability) = required_capability(&name) {
        if !declared.has(capability) {
            out.push(
                Diagnostic::new(
                    Severity::Error,
                    file,
                    call_span(expr),
                    format!(
                        "`{name}` needs the `{capability}` capability, which this package does not grant"
                    ),
                )
                .with_code(codes::CAPABILITY_DENIED)
                .with_note(format!(
                    "add `{capability} = []` to `[capabilities]` in yashy.toml to grant it"
                )),
            );
        }
    }
    match expr {
        HirExpr::Binary { lhs, rhs, .. } => {
            visit_expr(lhs, file, declared, out);
            visit_expr(rhs, file, declared, out);
        }
        HirExpr::Unary { operand, .. }
        | HirExpr::Try { expr: operand, .. }
        | HirExpr::Cast { expr: operand, .. }
        | HirExpr::Await { expr: operand, .. } => visit_expr(operand, file, declared, out),
        HirExpr::Move(operand, ..) => visit_expr(operand, file, declared, out),
        HirExpr::Call { args, .. } => {
            for arg in args {
                visit_expr(arg, file, declared, out);
            }
        }
        HirExpr::StructLit { fields, .. } => {
            for (_, field, _) in fields {
                visit_expr(field, file, declared, out);
            }
        }
        HirExpr::FieldAccess { base, .. } | HirExpr::Index { base, .. } => {
            visit_expr(base, file, declared, out)
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_capabilities_are_named_once() {
        let mut names: Vec<&str> = REQUIRED.iter().map(|(_, name)| *name).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count);
        assert_eq!(required_capability("read_file"), Some("fs.read"));
        assert_eq!(required_capability("print"), None);
    }
}
