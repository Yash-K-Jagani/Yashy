//! The warning pass: things that are not wrong but are worth saying.
//!
//! Four coded warnings, each a dead-code or reachability check:
//! - E2000: a `const`/`let` binding is never read
//! - E2001: an `import` is never used
//! - E2002: a statement cannot be reached
//! - E2003: a parameter is never read
//!
//! The pass is a single walk over the typed HIR. It collects the set of
//! identifiers that are read, the set of imports that are used, and the
//! positions of unreachable statements, then reports the difference.

use std::collections::HashSet;

use yashy_common::{codes, Diagnostic, FileId, Span};
use yashy_hir::hir::{HirBlock, HirExpr, HirFn, HirItem, HirProgram, HirStmt};

/// The names each imported module declares, keyed by the name it is imported
/// under. Empty is a legal table: an import is then used only when its own name
/// is read.
pub type Exports = std::collections::BTreeMap<String, std::collections::BTreeSet<String>>;

/// Run the warning pass over `program`, pushing diagnostics into `out`.
///
/// `exports` says which names each module declares, so an import counts as used
/// when the program reads one of them without a prefix. A caller that has no
/// table passes an empty one and gets the conservative answer.
pub fn check(program: &HirProgram, exports: &Exports, out: &mut Vec<Diagnostic>) {
    // Collect all identifiers that are read anywhere in the program.
    let mut reads: HashSet<String> = HashSet::new();
    let mut imports: Vec<(String, FileId, Span)> = Vec::new();

    // First pass: collect imports and reads.
    for item in &program.items {
        match item {
            HirItem::Import(import) => {
                // Track the import path's last component as the name.
                if let Some(name) = import.path.last() {
                    imports.push((name.clone(), import.file, import.span));
                }
            }
            HirItem::Fn(function) => {
                collect_reads_in_fn(function, &mut reads);
            }
            _ => {}
        }
    }

    // E2001: unused imports.
    for (name, file, span) in &imports {
        if name.starts_with('_') {
            continue;
        }
        if reads.contains(name) {
            continue;
        }
        // A module may be used through its names rather than through its own
        // name: `import wc_words;` then `count_words(..)`. The import is used
        // when any name that module declares is read.
        if let Some(declared) = exports.get(name) {
            if declared.iter().any(|declared| reads.contains(declared)) {
                continue;
            }
        }
        out.push(Diagnostic::warning(
            codes::UNUSED_IMPORT,
            *file,
            *span,
            format!("`{name}` is never used"),
        ));
    }

    // Second pass: check bindings, parameters, and unreachable code.
    for item in &program.items {
        if let HirItem::Fn(function) = item {
            check_fn(function, &reads, out);
        }
    }
}

/// Collect all identifiers read in a function (including nested lambdas).
fn collect_reads_in_fn(function: &HirFn, reads: &mut HashSet<String>) {
    // A capture is a read of the outer binding: the body says `Capture`, not
    // `Ident`, because the value comes out of the environment. The name is
    // recorded on the function, and it is the name the enclosing scope must
    // still consider used.
    for capture in &function.captures {
        reads.insert(capture.name.clone());
    }
    collect_reads_in_block(&function.body, reads);
    for clause in function.requires.iter().chain(function.ensures.iter()) {
        collect_reads_in_expr(&clause.expr, reads);
    }
}

fn collect_reads_in_block(block: &HirBlock, reads: &mut HashSet<String>) {
    for stmt in &block.stmts {
        collect_reads_in_stmt(stmt, reads);
    }
}

fn collect_reads_in_stmt(stmt: &HirStmt, reads: &mut HashSet<String>) {
    match stmt {
        HirStmt::Expr(expr, ..) => collect_reads_in_expr(expr, reads),
        HirStmt::Const { init, .. } | HirStmt::Var { init, .. } | HirStmt::Destructure { init, .. } => {
            collect_reads_in_expr(init, reads);
        }
        HirStmt::Assign { value, .. } => collect_reads_in_expr(value, reads),
        HirStmt::IndexAssign { base, index, value, .. } => {
            collect_reads_in_expr(base, reads);
            collect_reads_in_expr(index, reads);
            collect_reads_in_expr(value, reads);
        }
        HirStmt::FieldAssign { value, .. } => collect_reads_in_expr(value, reads),
        HirStmt::FieldPtrAssign { pointer, index, value, .. } => {
            collect_reads_in_expr(pointer, reads);
            collect_reads_in_expr(index, reads);
            collect_reads_in_expr(value, reads);
        }
        HirStmt::Return(Some(expr), _) => collect_reads_in_expr(expr, reads),
        HirStmt::If { init, cond, then_block, else_block, .. } => {
            if let Some(init) = init {
                collect_reads_in_stmt(init, reads);
            }
            collect_reads_in_expr(cond, reads);
            collect_reads_in_block(then_block, reads);
            if let Some(else_block) = else_block {
                collect_reads_in_block(else_block, reads);
            }
        }
        HirStmt::ForRange { start, end, body, .. } => {
            collect_reads_in_expr(start, reads);
            collect_reads_in_expr(end, reads);
            collect_reads_in_block(body, reads);
        }
        HirStmt::ForIn { iterable, body, .. } => {
            collect_reads_in_expr(iterable, reads);
            collect_reads_in_block(body, reads);
        }
        HirStmt::ForC { init, cond, update, body, .. } => {
            if let Some(init) = init {
                collect_reads_in_stmt(init, reads);
            }
            if let Some(cond) = cond {
                collect_reads_in_expr(cond, reads);
            }
            if let Some(update) = update {
                collect_reads_in_expr(update, reads);
            }
            collect_reads_in_block(body, reads);
        }
        HirStmt::Switch { subject, arms, .. } => {
            collect_reads_in_expr(subject, reads);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    collect_reads_in_expr(guard, reads);
                }
                collect_reads_in_expr(&arm.body, reads);
            }
        }
        HirStmt::Defer { body, .. } => collect_reads_in_block(body, reads),
        HirStmt::LoopInvariant(contract) => collect_reads_in_expr(&contract.expr, reads),
        _ => {}
    }
}

fn collect_reads_in_expr(expr: &HirExpr, reads: &mut HashSet<String>) {
    match expr {
        HirExpr::Ident(path, _, _) => {
            if let Some(name) = path.last() {
                reads.insert(name.clone());
            }
        }
        HirExpr::Binary { lhs, rhs, .. } => {
            collect_reads_in_expr(lhs, reads);
            collect_reads_in_expr(rhs, reads);
        }
        HirExpr::Unary { operand, .. } | HirExpr::Move(operand, ..) => {
            collect_reads_in_expr(operand, reads);
        }
        // A call reads the thing being called: `f(4)` reads `f`, and
        // `JSON.quote(x)` reads `JSON`. The callee is a path, not an
        // expression, so this is the only place the name can be collected.
        // Without it a function value passed in, bound to a local, or called
        // through a parameter looks unused.
        HirExpr::Call { callee, args, .. } => {
            if let Some(first) = callee.first() {
                reads.insert(first.clone());
            }
            for arg in args {
                collect_reads_in_expr(arg, reads);
            }
        }
        HirExpr::CallCapture { args, .. } => {
            for arg in args {
                collect_reads_in_expr(arg, reads);
            }
        }
        HirExpr::StructLit { path, fields, .. } => {
            if let Some(first) = path.first() {
                reads.insert(first.clone());
            }
            for (_, value, _) in fields {
                collect_reads_in_expr(value, reads);
            }
        }
        HirExpr::SliceLit { elements, .. }
        | HirExpr::FixedArrayLit { elements, .. }
        | HirExpr::TupleLit { elements, .. } => {
            for element in elements {
                collect_reads_in_expr(element, reads);
            }
        }
        HirExpr::FieldAccess { base, .. } => collect_reads_in_expr(base, reads),
        HirExpr::Index { base, index, .. } => {
            collect_reads_in_expr(base, reads);
            collect_reads_in_expr(index, reads);
        }
        HirExpr::View { base, start, end, .. } => {
            collect_reads_in_expr(base, reads);
            collect_reads_in_expr(start, reads);
            collect_reads_in_expr(end, reads);
        }
        HirExpr::Cast { expr, .. } | HirExpr::Try { expr, .. } => {
            collect_reads_in_expr(expr, reads);
        }
        HirExpr::SwitchExpr { subject, arms, .. } => {
            collect_reads_in_expr(subject, reads);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    collect_reads_in_expr(guard, reads);
                }
                collect_reads_in_expr(&arm.body, reads);
            }
        }
        HirExpr::Lambda { f, .. } => collect_reads_in_fn(f, reads),
        HirExpr::HigherOrder { receiver, func, init, .. } => {
            collect_reads_in_expr(receiver, reads);
            collect_reads_in_expr(func, reads);
            if let Some(init) = init {
                collect_reads_in_expr(init, reads);
            }
        }
        _ => {}
    }
}

/// Check a function for warnings.
fn check_fn(function: &HirFn, reads: &HashSet<String>, out: &mut Vec<Diagnostic>) {
    let file = function.file;

    // E2003: unused parameters.
    for param in &function.params {
        if !param.name.starts_with('_') && !reads.contains(&param.name) {
            out.push(Diagnostic::warning(
                codes::UNUSED_PARAMETER,
                file,
                param.span,
                format!("`{}` is never read", param.name),
            ));
        }
    }

    // E2000: unused bindings, and E2002: unreachable code.
    check_block(&function.body, reads, out, file, false);
}

fn check_block(block: &HirBlock, reads: &HashSet<String>, out: &mut Vec<Diagnostic>, file: FileId, _in_loop: bool) {
    let mut terminated = false;
    for stmt in &block.stmts {
        if terminated {
            // E2002: unreachable code.
            let span = stmt_span(stmt);
            out.push(Diagnostic::warning(
                codes::UNREACHABLE_CODE,
                file,
                span,
                "unreachable code",
            ));
            continue;
        }

        match stmt {
            HirStmt::Const { name, span, .. } | HirStmt::Var { name, span, .. } => {
                if !name.starts_with('_') && !reads.contains(name) {
                    out.push(Diagnostic::warning(
                        codes::UNUSED_BINDING,
                        file,
                        *span,
                        format!("`{name}` is never read"),
                    ));
                }
            }
            _ => {}
        }

        if is_terminator(stmt) {
            terminated = true;
        }
    }
}

fn is_terminator(stmt: &HirStmt) -> bool {
    match stmt {
        HirStmt::Return(..) => true,
        HirStmt::Break(_) => true,
        HirStmt::Continue(_) => true,
        // panic and os.exit are function calls, not terminators in the HIR.
        // We check for them by name in the Expr variant.
        HirStmt::Expr(HirExpr::Call { callee, .. }, ..) => {
            let name = callee.last().map(|s| s.as_str()).unwrap_or("");
            name == "panic" || name == "os.exit"
        }
        _ => false,
    }
}

fn stmt_span(stmt: &HirStmt) -> Span {
    match stmt {
        HirStmt::Expr(_, span, _) => *span,
        HirStmt::Const { span, .. } => *span,
        HirStmt::Var { span, .. } => *span,
        HirStmt::Destructure { span, .. } => *span,
        HirStmt::Assign { span, .. } => *span,
        HirStmt::IndexAssign { span, .. } => *span,
        HirStmt::FieldAssign { span, .. } => *span,
        HirStmt::FieldPtrAssign { span, .. } => *span,
        HirStmt::Return(_, span) => *span,
        HirStmt::If { span, .. } => *span,
        HirStmt::ForRange { span, .. } => *span,
        HirStmt::ForIn { span, .. } => *span,
        HirStmt::ForC { span, .. } => *span,
        HirStmt::Switch { span, .. } => *span,
        HirStmt::Break(span) => *span,
        HirStmt::Continue(span) => *span,
        HirStmt::Defer { span, .. } => *span,
        HirStmt::LoopInvariant(_) => Span::SYNTHETIC,
    }
}

