//! The typestate pass: what state a resource is in.
//!
//! A resource handle carries a state: `Allocated`/`Freed`, `Unlocked`/`Locked`,
//! `Open`/`Closed`. Operations require a particular state and produce another;
//! an operation in the wrong state is `E1400`, and using a resource in a
//! terminal state is `E1401`. The state is tracked per binding across the
//! control-flow graph the same way the leak pass tracks release bits, and the
//! merge is an intersection: a resource must be in the same state on every path
//! that reaches a point, otherwise it is `E1400` with both states named.

use crate::typestate::ResourceState as State;
use yashy_common::{codes, Diagnostic, FileId, Severity, Span};
use yashy_hir::hir::{
    HirBlock, HirExpr, HirFn, HirItem, HirPattern, HirProgram, HirStmt, HirSwitchArm,
};

/// Run the typestate pass over `program`, pushing diagnostics into `out`.
pub fn check(program: &HirProgram, out: &mut Vec<Diagnostic>) {
    check_typestate(program, out);
    check_race(program, out);
    check_async_rules(program, out);
}

/// Structured concurrency (phase 10, task 5): `E1701` (task escape) and
/// `E1702` (hold across await).
///
/// The ruling in `.superpowers/sdd/2026-11-08-phase-10/progress.md` places
/// `E1702` in the async pass rather than the typestate state walk: a suspension
/// is an async fact, and this function computes only the *intersection* of
/// "a lock or linear resource is held here" (the same lock/unlock and
/// alloc/free facts the typestate walk already tracks) with "the next
/// statement suspends". It does not duplicate the merge machinery.
fn check_async_rules(program: &HirProgram, out: &mut Vec<Diagnostic>) {
    for item in &program.items {
        if let HirItem::Fn(function) = item {
            check_escape_fn(function, out);
            if function.is_async {
                check_hold_fn(function, out);
            }
        }
    }
}

/// `E1701`: a `Task[T]` handle returned, stored in a struct field, or pushed
/// into a collection. The handle's defining span answers the liveness
/// question: a handle defined inside a `scope` that is used at a shallower
/// depth, or stored into a field/collection at any depth (storage outlives
/// the scope by construction), escapes.
fn check_escape_fn(function: &HirFn, out: &mut Vec<Diagnostic>) {
    let mut walker = EscapeWalker {
        out,
        file: function.file,
        depth: 0,
        scoped: Vec::new(),
    };
    walker.block(&function.body);
}

struct EscapeWalker<'a> {
    out: &'a mut Vec<Diagnostic>,
    file: FileId,
    depth: usize,
    /// `(name, defining depth)` for `Task` bindings created inside a scope.
    scoped: Vec<(String, usize)>,
}

impl EscapeWalker<'_> {
    fn report(&mut self, name: &str, span: Span) {
        self.out.push(
            Diagnostic::new(
                Severity::Error,
                self.file,
                span,
                format!("task handle `{name}` escapes its scope"),
            )
            .with_code(codes::TASK_ESCAPE),
        );
    }

    fn defines_task(&self, name: &str) -> Option<usize> {
        self.scoped
            .iter()
            .rev()
            .find(|(scoped, _)| scoped == name)
            .map(|(_, depth)| *depth)
    }

    fn block(&mut self, block: &HirBlock) {
        for stmt in &block.stmts {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &HirStmt) {
        match stmt {
            HirStmt::Scope { body, .. } => {
                self.depth += 1;
                self.block(body);
                self.depth -= 1;
            }
            HirStmt::Var { name, init, .. } | HirStmt::Const { name, init, .. } => {
                self.expr(init);
                if self.depth > 0 && is_task_ty(init) {
                    self.scoped.push((name.clone(), self.depth));
                }
            }
            HirStmt::Return(Some(expr), _) => {
                self.expr(expr);
                if let Some((name, span)) = escaping_ident(expr, &self.scoped, self.depth) {
                    self.report(&name, span);
                }
            }
            HirStmt::Assign { name, value, .. } => {
                self.expr(value);
                // Assigning a spawned task into a binding that outlives the
                // scope (one not itself defined inside the scope) escapes.
                if matches!(root_ident(value), HirExpr::Spawn { .. })
                    && self.defines_task(name).is_none()
                {
                    let span = yashy_hir::hir::expr_span(value);
                    self.report(name, span);
                }
            }
            HirStmt::FieldAssign { value, .. } => {
                self.expr(value);
                if matches!(root_ident(value), HirExpr::Spawn { .. }) {
                    let span = yashy_hir::hir::expr_span(value);
                    self.report("spawn(...)", span);
                }
                if let HirExpr::Ident(path, span, _) = root_ident(value) {
                    if path
                        .last()
                        .is_some_and(|name| self.defines_task(name).is_some())
                    {
                        let name = path.last().cloned().unwrap_or_default();
                        self.report(&name, *span);
                    }
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
                    self.stmt(init);
                }
                self.expr(cond);
                self.block(then_block);
                if let Some(else_block) = else_block {
                    self.block(else_block);
                }
            }
            HirStmt::ForRange {
                start, end, body, ..
            } => {
                self.expr(start);
                self.expr(end);
                self.block(body);
            }
            HirStmt::ForIn { iterable, body, .. } => {
                self.expr(iterable);
                self.block(body);
            }
            HirStmt::ForC {
                init,
                cond,
                update,
                body,
                ..
            } => {
                if let Some(init) = init {
                    self.stmt(init);
                }
                if let Some(cond) = cond {
                    self.expr(cond);
                }
                if let Some(update) = update {
                    self.expr(update);
                }
                self.block(body);
            }
            HirStmt::Switch { subject, arms, .. } => {
                self.expr(subject);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.expr(guard);
                    }
                    self.expr(&arm.body);
                }
            }
            HirStmt::IfLet {
                subject,
                then_block,
                else_block,
                ..
            } => {
                self.expr(subject);
                self.block(then_block);
                if let Some(else_block) = else_block {
                    self.block(else_block);
                }
            }
            HirStmt::Defer { body, .. } => self.block(body),
            HirStmt::Expr(expr, ..) => self.expr(expr),
            HirStmt::Destructure { init, .. } => self.expr(init),
            HirStmt::Assign { value, .. } => self.expr(value),
            HirStmt::IndexAssign {
                base, index, value, ..
            } => {
                self.expr(base);
                self.expr(index);
                self.expr(value);
            }
            HirStmt::FieldPtrAssign {
                pointer,
                index,
                value,
                ..
            } => {
                self.expr(pointer);
                self.expr(index);
                self.expr(value);
            }
            HirStmt::LoopInvariant(contract) => self.expr(&contract.expr),
            _ => {}
        }
    }

    fn expr(&mut self, expr: &HirExpr) {
        match expr {
            HirExpr::Call {
                callee, args, span, ..
            } => {
                let name = callee.last().map(|s| s.as_str()).unwrap_or("");
                if name == "push" {
                    for arg in args {
                        if let HirExpr::Ident(path, arg_span, _) = root_ident(arg) {
                            if path
                                .last()
                                .is_some_and(|name| self.defines_task(name).is_some())
                            {
                                let name = path.last().cloned().unwrap_or_default();
                                self.report(&name, *arg_span);
                            }
                        }
                        if matches!(arg, HirExpr::Spawn { .. }) {
                            self.report("spawn(...)", *span);
                        }
                    }
                }
                for arg in args {
                    self.expr(arg);
                }
            }
            HirExpr::StructLit { fields, .. } => {
                for (_, value, _) in fields {
                    if matches!(root_ident(value), HirExpr::Spawn { .. }) {
                        let span = yashy_hir::hir::expr_span(value);
                        self.report("spawn(...)", span);
                    }
                    if let HirExpr::Ident(path, span, _) = root_ident(value) {
                        if path
                            .last()
                            .is_some_and(|name| self.defines_task(name).is_some())
                        {
                            let name = path.last().cloned().unwrap_or_default();
                            self.report(&name, *span);
                        }
                    }
                    self.expr(value);
                }
            }
            HirExpr::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            HirExpr::Unary { operand, .. } | HirExpr::Move(operand, ..) => self.expr(operand),
            HirExpr::SliceLit { elements, .. }
            | HirExpr::FixedArrayLit { elements, .. }
            | HirExpr::TupleLit { elements, .. } => {
                for element in elements {
                    self.expr(element);
                }
            }
            HirExpr::FieldAccess { base, .. } => self.expr(base),
            HirExpr::Index { base, index, .. } => {
                self.expr(base);
                self.expr(index);
            }
            HirExpr::View {
                base, start, end, ..
            } => {
                self.expr(base);
                self.expr(start);
                self.expr(end);
            }
            HirExpr::Cast { expr, .. } | HirExpr::Try { expr, .. } => self.expr(expr),
            HirExpr::Await { expr, .. } => self.expr(expr),
            HirExpr::Spawn { callee, args, .. } => {
                self.expr(callee);
                for arg in args {
                    self.expr(arg);
                }
            }
            HirExpr::SwitchExpr { subject, arms, .. } => {
                self.expr(subject);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.expr(guard);
                    }
                    self.expr(&arm.body);
                }
            }
            HirExpr::Lambda { f, .. } => {
                let saved = self.depth;
                // A lambda is a fresh body; its tasks escape only across the
                // lambda's own scopes.
                self.depth = 0;
                let saved_scoped = std::mem::take(&mut self.scoped);
                self.block(&f.body);
                self.scoped = saved_scoped;
                self.depth = saved;
            }
            HirExpr::CallCapture { args, .. } => {
                for arg in args {
                    self.expr(arg);
                }
            }
            HirExpr::HigherOrder {
                receiver,
                func,
                init,
                ..
            } => {
                self.expr(receiver);
                self.expr(func);
                if let Some(init) = init {
                    self.expr(init);
                }
            }
            _ => {}
        }
    }
}

fn root_ident(expr: &HirExpr) -> &HirExpr {
    match expr {
        HirExpr::Move(inner, ..) => root_ident(inner),
        other => other,
    }
}

/// The escaping ident a `return` carries: the returned expression itself when
/// it names a handle defined in a deeper scope than the `return` sits in.
fn escaping_ident(
    expr: &HirExpr,
    scoped: &[(String, usize)],
    depth: usize,
) -> Option<(String, Span)> {
    let HirExpr::Ident(path, span, _) = root_ident(expr) else {
        return None;
    };
    let name = path.last().cloned().unwrap_or_default();
    let def_depth = scoped
        .iter()
        .rev()
        .find(|(scoped, _)| scoped == &name)
        .map(|(_, depth)| *depth)?;
    if def_depth > depth {
        Some((name, *span))
    } else {
        None
    }
}

fn is_task_ty(expr: &HirExpr) -> bool {
    use yashy_hir::ty::TypeKind;
    matches!(yashy_hir::hir::expr_ty(expr).kind, TypeKind::Task(_))
        || matches!(expr, HirExpr::Spawn { .. })
}

/// `E1702`: a resource held across an `await`. Tracks, in statement order,
/// mutexes in `Locked` state and linear (allocated) resources, and fires when
/// a statement suspends while one is held and no `defer` in scope releases it.
fn check_hold_fn(function: &HirFn, out: &mut Vec<Diagnostic>) {
    let mut held: Vec<(String, &'static str, bool)> = Vec::new();
    let mut walker = HoldWalker {
        out,
        file: function.file,
        held: &mut held,
    };
    walker.block(&function.body);
}

struct HoldWalker<'a> {
    out: &'a mut Vec<Diagnostic>,
    file: FileId,
    held: &'a mut Vec<(String, &'static str, bool)>,
}

impl HoldWalker<'_> {
    fn block(&mut self, block: &HirBlock) {
        // A `defer` anywhere in this block releases at its exit; whether it
        // exempts an `await` in this block is answered per-await by whether
        // the defer was *reached* before it.
        for stmt in &block.stmts {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &HirStmt) {
        // First the await check: the intersection of "held" with "suspends".
        if let Some(await_span) = stmt_await_span(stmt) {
            let held: Vec<String> = self
                .held
                .iter()
                .filter(|(_, _, deferred)| !deferred)
                .map(|(name, _, _)| name.clone())
                .collect();
            for name in held {
                self.out.push(
                    Diagnostic::new(
                        Severity::Error,
                        self.file,
                        await_span,
                        format!("`{name}` is held across an `await`: release it before suspending, or `defer` the release"),
                    )
                    .with_code(codes::HOLD_ACROSS_AWAIT),
                );
            }
        }
        match stmt {
            HirStmt::Var { name, init, .. } | HirStmt::Const { name, init, .. } => {
                self.note_init(name, init);
                self.expr(init);
            }
            HirStmt::Defer { body, .. } => {
                for name in deferred_releases(body) {
                    if let Some(entry) = self.held.iter_mut().find(|(held, ..)| held == &name) {
                        entry.2 = true;
                    }
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
                    self.stmt(init);
                }
                self.expr(cond);
                let saved = self.held.clone();
                self.block(then_block);
                let then_held = self.held.clone();
                *self.held = saved;
                if let Some(else_block) = else_block {
                    self.block(else_block);
                }
                // Conservative join: a held flag set on either branch stays.
                for entry in then_held {
                    if !self.held.iter().any(|(name, _, _)| name == &entry.0) {
                        self.held.push(entry);
                    }
                }
            }
            HirStmt::Scope { body, .. } => self.block(body),
            HirStmt::ForRange {
                start, end, body, ..
            } => {
                self.expr(start);
                self.expr(end);
                let saved = self.held.clone();
                self.block(body);
                let _ = saved;
            }
            HirStmt::ForIn { iterable, body, .. } => {
                self.expr(iterable);
                self.block(body);
            }
            HirStmt::ForC {
                init,
                cond,
                update,
                body,
                ..
            } => {
                if let Some(init) = init {
                    self.stmt(init);
                }
                if let Some(cond) = cond {
                    self.expr(cond);
                }
                self.block(body);
                if let Some(update) = update {
                    self.expr(update);
                }
            }
            HirStmt::Switch { subject, arms, .. } => {
                self.expr(subject);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.expr(guard);
                    }
                    self.expr(&arm.body);
                }
            }
            HirStmt::IfLet {
                subject,
                then_block,
                else_block,
                ..
            } => {
                self.expr(subject);
                self.block(then_block);
                if let Some(else_block) = else_block {
                    self.block(else_block);
                }
            }
            HirStmt::Expr(expr, ..) => self.expr(expr),
            HirStmt::Destructure { init, .. } => self.expr(init),
            HirStmt::Assign { value, .. } => self.expr(value),
            HirStmt::IndexAssign {
                base, index, value, ..
            } => {
                self.expr(base);
                self.expr(index);
                self.expr(value);
            }
            HirStmt::FieldAssign { value, .. } => self.expr(value),
            HirStmt::FieldPtrAssign {
                pointer,
                index,
                value,
                ..
            } => {
                self.expr(pointer);
                self.expr(index);
                self.expr(value);
            }
            HirStmt::Return(Some(expr), _) => self.expr(expr),
            HirStmt::LoopInvariant(contract) => self.expr(&contract.expr),
            _ => {}
        }
    }

    fn note_init(&mut self, name: &str, init: &HirExpr) {
        let HirExpr::Call { callee, .. } = init else {
            return;
        };
        match callee.last().map(|s| s.as_str()).unwrap_or("") {
            // An unlocked mutex is not held yet: the flag starts exempt and
            // the lock transition clears it.
            "mutex_new" => {
                self.held.push((name.to_string(), "mutex", true));
            }
            "alloc" => {
                self.held.push((name.to_string(), "resource", false));
            }
            _ => {}
        }
    }

    fn expr(&mut self, expr: &HirExpr) {
        match expr {
            HirExpr::Call { callee, args, .. } => {
                match callee.last().map(|s| s.as_str()).unwrap_or("") {
                    "mutex_lock" | "lock" => {
                        if let Some(name) = first_arg_name(args) {
                            if let Some(entry) =
                                self.held.iter_mut().find(|(held, ..)| held == &name)
                            {
                                entry.1 = "mutex";
                                entry.2 = false;
                            } else {
                                self.held.push((name, "mutex", false));
                            }
                        }
                    }
                    "mutex_unlock" | "unlock" | "free" | "dealloc" | "mutex_free" => {
                        if let Some(name) = first_arg_name(args) {
                            self.held.retain(|(held, ..)| held != &name);
                        }
                    }
                    _ => {}
                }
                for arg in args {
                    self.expr(arg);
                }
            }
            HirExpr::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            HirExpr::Unary { operand, .. } | HirExpr::Move(operand, ..) => self.expr(operand),
            HirExpr::StructLit { fields, .. } => {
                for (_, value, _) in fields {
                    self.expr(value);
                }
            }
            HirExpr::SliceLit { elements, .. }
            | HirExpr::FixedArrayLit { elements, .. }
            | HirExpr::TupleLit { elements, .. } => {
                for element in elements {
                    self.expr(element);
                }
            }
            HirExpr::FieldAccess { base, .. } => self.expr(base),
            HirExpr::Index { base, index, .. } => {
                self.expr(base);
                self.expr(index);
            }
            HirExpr::View {
                base, start, end, ..
            } => {
                self.expr(base);
                self.expr(start);
                self.expr(end);
            }
            HirExpr::Cast { expr, .. } | HirExpr::Try { expr, .. } => self.expr(expr),
            HirExpr::Await { expr, .. } => self.expr(expr),
            HirExpr::Spawn { callee, args, .. } => {
                self.expr(callee);
                for arg in args {
                    self.expr(arg);
                }
            }
            HirExpr::SwitchExpr { subject, arms, .. } => {
                self.expr(subject);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.expr(guard);
                    }
                    self.expr(&arm.body);
                }
            }
            HirExpr::Lambda { f, .. } => {
                let saved = self.held.clone();
                self.block(&f.body);
                *self.held = saved;
            }
            HirExpr::CallCapture { args, .. } => {
                for arg in args {
                    self.expr(arg);
                }
            }
            HirExpr::HigherOrder {
                receiver,
                func,
                init,
                ..
            } => {
                self.expr(receiver);
                self.expr(func);
                if let Some(init) = init {
                    self.expr(init);
                }
            }
            _ => {}
        }
    }
}

fn first_arg_name(args: &[HirExpr]) -> Option<String> {
    match args.first()? {
        HirExpr::AddrOf { name, .. } => Some(name.clone()),
        HirExpr::Ident(path, ..) if path.len() == 1 => path.first().cloned(),
        _ => None,
    }
}

/// The span of the first `await` a statement suspends at, if any.
fn stmt_await_span(stmt: &HirStmt) -> Option<Span> {
    fn expr_await(expr: &HirExpr) -> Option<Span> {
        match expr {
            HirExpr::Await { span, .. } => Some(*span),
            HirExpr::Binary { lhs, rhs, .. } => expr_await(lhs).or_else(|| expr_await(rhs)),
            HirExpr::Unary { operand, .. } | HirExpr::Move(operand, ..) => expr_await(operand),
            HirExpr::Call { args, .. } | HirExpr::CallCapture { args, .. } => {
                args.iter().find_map(expr_await)
            }
            HirExpr::StructLit { fields, .. } => {
                fields.iter().find_map(|(_, value, _)| expr_await(value))
            }
            HirExpr::SliceLit { elements, .. }
            | HirExpr::FixedArrayLit { elements, .. }
            | HirExpr::TupleLit { elements, .. } => elements.iter().find_map(expr_await),
            HirExpr::FieldAccess { base, .. } => expr_await(base),
            HirExpr::Index { base, index, .. } => expr_await(base).or_else(|| expr_await(index)),
            HirExpr::View {
                base, start, end, ..
            } => expr_await(base)
                .or_else(|| expr_await(start))
                .or_else(|| expr_await(end)),
            HirExpr::Cast { expr, .. } | HirExpr::Try { expr, .. } => expr_await(expr),
            HirExpr::Spawn { callee, args, .. } => {
                expr_await(callee).or_else(|| args.iter().find_map(expr_await))
            }
            HirExpr::SwitchExpr { subject, arms, .. } => expr_await(subject).or_else(|| {
                arms.iter().find_map(|arm| {
                    arm.guard
                        .as_ref()
                        .and_then(expr_await)
                        .or_else(|| expr_await(&arm.body))
                })
            }),
            HirExpr::HigherOrder {
                receiver,
                func,
                init,
                ..
            } => expr_await(receiver)
                .or_else(|| expr_await(func))
                .or_else(|| init.as_ref().and_then(|value| expr_await(value))),
            HirExpr::Lambda { f, .. } => f.body.stmts.iter().find_map(stmt_await_span),
            _ => None,
        }
    }
    match stmt {
        HirStmt::Expr(expr, ..) => expr_await(expr),
        HirStmt::Var { init, .. }
        | HirStmt::Const { init, .. }
        | HirStmt::Destructure { init, .. } => expr_await(init),
        HirStmt::Assign { value, .. } => expr_await(value),
        HirStmt::Return(Some(expr), _) => expr_await(expr),
        HirStmt::If {
            cond,
            then_block,
            else_block,
            ..
        } => expr_await(cond)
            .or_else(|| then_block.stmts.iter().find_map(stmt_await_span))
            .or_else(|| {
                else_block
                    .as_ref()
                    .and_then(|block| block.stmts.iter().find_map(stmt_await_span))
            }),
        HirStmt::ForRange {
            start, end, body, ..
        } => expr_await(start)
            .or_else(|| expr_await(end))
            .or_else(|| body.stmts.iter().find_map(stmt_await_span)),
        HirStmt::Scope { body, .. } | HirStmt::Defer { body, .. } => {
            body.stmts.iter().find_map(stmt_await_span)
        }
        _ => None,
    }
}

/// Names released by `defer` bodies in `block`: a `defer` naming `x` in a
/// release call exempts `x` from `E1702`.
fn deferred_releases(block: &HirBlock) -> Vec<String> {
    fn expr_releases(expr: &HirExpr, out: &mut Vec<String>) {
        match expr {
            HirExpr::Call { callee, args, .. } => {
                match callee.last().map(|s| s.as_str()).unwrap_or("") {
                    "mutex_unlock" | "unlock" | "free" | "dealloc" | "mutex_free" => {
                        if let Some(name) = first_arg_name(args) {
                            out.push(name);
                        }
                    }
                    _ => {}
                }
                for arg in args {
                    expr_releases(arg, out);
                }
            }
            HirExpr::Binary { lhs, rhs, .. } => {
                expr_releases(lhs, out);
                expr_releases(rhs, out);
            }
            HirExpr::Unary { operand, .. } | HirExpr::Move(operand, ..) => {
                expr_releases(operand, out)
            }
            HirExpr::StructLit { fields, .. } => {
                for (_, value, _) in fields {
                    expr_releases(value, out);
                }
            }
            HirExpr::SliceLit { elements, .. }
            | HirExpr::FixedArrayLit { elements, .. }
            | HirExpr::TupleLit { elements, .. } => {
                for element in elements {
                    expr_releases(element, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for stmt in &block.stmts {
        match stmt {
            HirStmt::Expr(expr, ..) => expr_releases(expr, &mut out),
            HirStmt::Var { init, .. } | HirStmt::Const { init, .. } => {
                expr_releases(init, &mut out)
            }
            HirStmt::Assign { value, .. } => expr_releases(value, &mut out),
            _ => {}
        }
    }
    out
}

/// The typestate pass: what state a resource is in.
pub fn check_typestate(program: &HirProgram, out: &mut Vec<Diagnostic>) {
    let mut pass = TypestatePass {
        out,
        file: FileId::from_index(0),
        frames: Vec::new(),
    };
    for item in &program.items {
        if let HirItem::Fn(function) = item {
            pass.check_fn(function);
        }
    }
}

/// One tracked resource binding.
#[derive(Clone)]
struct Binding {
    name: String,
    span: Span,
    state: State,
    /// Whether this binding is a tracked resource. Non-resource bindings are
    /// ignored.
    tracked: bool,
    /// An iterator is `Open` until a `next` that returned `None` is
    /// observed on this path, `Closed` after it. `iter_closed` means
    /// definitely closed: closed on every path that reaches here (the
    /// merge is an intersection, unlike resources where any terminal
    /// path wins).
    iter_closed: bool,
    /// When this binding holds the result of `next()` on the named
    /// iterator, matching it against `None` closes that iterator.
    iter_src: Option<String>,
}

type Scope = Vec<Binding>;
type StateFrames = Vec<Scope>;

struct TypestatePass<'a> {
    out: &'a mut Vec<Diagnostic>,
    file: FileId,
    frames: StateFrames,
}

impl TypestatePass<'_> {
    fn check_fn(&mut self, function: &HirFn) {
        let outer_frames = std::mem::take(&mut self.frames);
        let outer_file = self.file;
        self.file = function.file;
        let _ = self.block(&function.body);
        self.frames = outer_frames;
        self.file = outer_file;
    }

    fn block(&mut self, block: &HirBlock) -> bool {
        // Returns true if execution falls through.
        self.frames.push(Vec::new());
        let mut flow = true;
        for stmt in &block.stmts {
            if !flow {
                break;
            }
            flow = self.stmt(stmt);
        }
        self.frames.pop();
        flow
    }

    fn stmt(&mut self, stmt: &HirStmt) -> bool {
        match stmt {
            HirStmt::Expr(expr, _, _) => {
                self.walk(expr);
                true
            }
            HirStmt::Const {
                name,
                ty,
                init,
                span,
                ..
            }
            | HirStmt::Var {
                name,
                ty,
                init,
                span,
                ..
            } => {
                self.walk(init);
                let state = resource_state_of_init(init);
                if let Some(state) = state {
                    if let Some(frame) = self.frames.last_mut() {
                        frame.push(Binding {
                            name: name.clone(),
                            span: *span,
                            state,
                            tracked: true,
                            iter_closed: false,
                            iter_src: None,
                        });
                    }
                } else if let Some(src) = self.iterator_source(init) {
                    // `let x = it.next()`: matching `x` against `None`
                    // later closes `it`.
                    if let Some(frame) = self.frames.last_mut() {
                        frame.push(Binding {
                            name: name.clone(),
                            span: *span,
                            state: State::Open,
                            tracked: false,
                            iter_closed: false,
                            iter_src: Some(src),
                        });
                    }
                }
                let _ = ty;
                true
            }
            HirStmt::Destructure {
                init,
                pattern,
                span,
                ..
            } => {
                self.walk(init);
                let state = resource_state_of_init(init);
                if let Some(state) = state {
                    if let Some(frame) = self.frames.last_mut() {
                        for (name, _) in pattern.bindings() {
                            frame.push(Binding {
                                name,
                                span: *span,
                                state,
                                tracked: true,
                                iter_closed: false,
                                iter_src: None,
                            });
                        }
                    }
                }
                true
            }
            HirStmt::Assign { name, value, .. } => {
                self.walk(value);
                let new_state = resource_state_of_init(value);
                if let Some(state) = new_state {
                    if let Some(binding) = self.find_mut(name) {
                        binding.state = state;
                        binding.tracked = true;
                    }
                } else {
                    // Rebinding a non-resource value resets tracking.
                    if let Some(binding) = self.find_mut(name) {
                        binding.tracked = false;
                    }
                }
                true
            }
            HirStmt::IndexAssign {
                base, index, value, ..
            } => {
                self.walk(base);
                self.walk(index);
                self.walk(value);
                true
            }
            HirStmt::FieldAssign { value, .. } => {
                self.walk(value);
                true
            }
            HirStmt::FieldPtrAssign {
                pointer,
                index,
                value,
                ..
            } => {
                self.walk(pointer);
                self.walk(index);
                self.walk(value);
                true
            }
            HirStmt::Return(value, _) => {
                if let Some(value) = value {
                    self.walk(value);
                }
                false
            }
            HirStmt::If {
                init,
                cond,
                then_block,
                else_block,
                ..
            } => {
                if let Some(init) = init {
                    if !self.stmt(init) {
                        return false;
                    }
                }
                self.walk(cond);
                self.branch(then_block, else_block.as_ref())
            }
            HirStmt::ForRange {
                start, end, body, ..
            } => {
                self.walk(start);
                self.walk(end);
                self.loop_body(body);
                true
            }
            HirStmt::ForIn { iterable, body, .. } => {
                self.walk(iterable);
                self.loop_body(body);
                true
            }
            HirStmt::ForC {
                init,
                cond,
                update,
                body,
                ..
            } => {
                if let Some(init) = init {
                    if !self.stmt(init) {
                        return false;
                    }
                }
                if let Some(cond) = cond {
                    self.walk(cond);
                }
                let entry = self.frames.clone();
                let _ = self.block(body);
                if let Some(update) = update {
                    self.walk(update);
                }
                self.frames = entry;
                true
            }
            HirStmt::Switch { subject, arms, .. } => {
                self.walk(subject);
                let source = self.iterator_source(subject);
                self.switch_arms_with_source(arms, source.as_deref())
            }
            HirStmt::IfLet {
                subject,
                pattern,
                then_block,
                else_block,
                ..
            } => {
                // A pattern binds a fresh value; a resource it carries is
                // tracked from where it is used, as a `switch` arm's is.
                self.walk(subject);
                // `if let Some(x) = it.next()`: the then path saw `Some`
                // (still open), the else path saw `None` (closed); a
                // `None` pattern is the mirror image.
                if let Some(source) = self.iterator_source(subject) {
                    if pattern_is_some(pattern) {
                        return self.branch_iter(
                            then_block,
                            else_block.as_ref(),
                            false,
                            true,
                            &source,
                        );
                    }
                    if pattern_is_none(pattern) {
                        return self.branch_iter(
                            then_block,
                            else_block.as_ref(),
                            true,
                            false,
                            &source,
                        );
                    }
                }
                self.branch(then_block, else_block.as_ref())
            }
            HirStmt::Break(_) => false,
            HirStmt::Continue(_) => false,
            HirStmt::Defer { body, .. } => {
                let _ = self.block(body);
                true
            }
            HirStmt::Scope { body, .. } => {
                let _ = self.block(body);
                true
            }
            HirStmt::LoopInvariant(contract) => {
                self.walk(&contract.expr);
                true
            }
        }
    }

    fn walk(&mut self, expr: &HirExpr) {
        match expr {
            HirExpr::Literal(..)
            | HirExpr::Ident(..)
            | HirExpr::AddrOf { .. }
            | HirExpr::Capture { .. } => {
                // Check terminal-state use on ident.
                if let HirExpr::Ident(path, span, _) = expr {
                    if path.len() == 1 {
                        if let Some(name) = path.first() {
                            if let Some(binding) = self.find(name) {
                                if binding.tracked && binding.state.is_terminal() {
                                    self.report_use_after_close(name, *span);
                                }
                            }
                        }
                    }
                }
            }
            HirExpr::Move(inner, _, _) => {
                self.walk(inner);
            }
            HirExpr::Binary { lhs, rhs, .. } => {
                self.walk(lhs);
                self.walk(rhs);
            }
            HirExpr::Unary { operand, .. } => self.walk(operand),
            HirExpr::Call {
                callee, args, span, ..
            } => {
                // An `Iterator::next` call: the receiver is `Open` until a
                // `None` is observed, `Closed` after it (terminal: the
                // protocol does not promise resurrection).
                if is_iterator_next(callee) {
                    if let Some(name) = args.first().and_then(receiver_name) {
                        self.ensure_iter_binding(&name, *span);
                        let closed = self
                            .find(&name)
                            .map(|binding| binding.iter_closed)
                            .unwrap_or(false);
                        if closed {
                            self.report_iter_after_done(&name, *span);
                        }
                    }
                    for arg in args {
                        self.walk(arg);
                    }
                    return;
                }
                // Determine if this is a state-changing transition.
                let transition = transition_for(callee);
                // Walk args, but for the resource arg of a transition, handle state.
                for (i, arg) in args.iter().enumerate() {
                    // Check if arg is a tracked binding in terminal state (use).
                    // For transition calls, the resource arg is handled below.
                    let is_resource_arg = transition.is_some() && i == 0;
                    if !is_resource_arg {
                        self.walk(arg);
                    }
                }
                if let Some(t) = transition {
                    // First arg is the resource.
                    if let Some(HirExpr::Ident(path, span, _)) = args.first() {
                        if path.len() == 1 {
                            if let Some(name) = path.first() {
                                if let Some(binding) = self.find_mut(name) {
                                    if binding.tracked {
                                        let (req, next) = t;
                                        // chan_free accepts both Open and Closed.
                                        let callee_name =
                                            callee.last().map(|s| s.as_str()).unwrap_or("");
                                        let ok = if callee_name == "chan_free" {
                                            binding.state == State::Open
                                                || binding.state == State::Closed
                                        } else {
                                            binding.state == req
                                        };
                                        if !ok {
                                            let from = binding.state;
                                            let sp = *span;
                                            drop(binding);
                                            self.report_bad_transition(name, from, req, sp);
                                        } else {
                                            binding.state = next;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Walk remaining args.
                    for arg in args.iter().skip(1) {
                        self.walk(arg);
                    }
                }
            }
            HirExpr::CallCapture { args, .. } => {
                for arg in args {
                    self.walk(arg);
                }
            }
            HirExpr::StructLit { fields, .. } => {
                for (_, value, _) in fields {
                    self.walk(value);
                }
            }
            HirExpr::SliceLit { elements, .. }
            | HirExpr::FixedArrayLit { elements, .. }
            | HirExpr::TupleLit { elements, .. } => {
                for element in elements {
                    self.walk(element);
                }
            }
            HirExpr::FieldAccess { base, .. } => self.walk(base),
            HirExpr::Index { base, index, .. } => {
                self.walk(base);
                self.walk(index);
            }
            HirExpr::View {
                base, start, end, ..
            } => {
                self.walk(base);
                self.walk(start);
                self.walk(end);
            }
            HirExpr::Cast { expr, .. } | HirExpr::Try { expr, .. } => self.walk(expr),
            HirExpr::SwitchExpr { subject, arms, .. } => {
                self.walk(subject);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.walk(guard);
                    }
                    self.walk(&arm.body);
                }
            }
            HirExpr::Lambda { f, .. } => self.check_fn(f),
            HirExpr::HigherOrder {
                receiver,
                func,
                init,
                ..
            } => {
                self.walk(receiver);
                self.walk(func);
                if let Some(init) = init {
                    self.walk(init);
                }
            }
            HirExpr::Move(inner, _, _) => self.walk(inner),
            _ => {}
        }
    }

    fn branch(&mut self, then_block: &HirBlock, else_block: Option<&HirBlock>) -> bool {
        let entry = self.frames.clone();
        let mut falling: Vec<StateFrames> = Vec::new();
        if self.block(then_block) {
            falling.push(self.frames.clone());
        }
        self.frames = entry.clone();
        match else_block {
            Some(block) => {
                if self.block(block) {
                    falling.push(self.frames.clone());
                }
            }
            None => falling.push(entry.clone()),
        }
        if falling.is_empty() {
            self.frames = entry;
            return false;
        }
        // Merge: intersection. A tracked binding must have the same state on
        // every falling path; otherwise E1400 with both states.
        let merged = merge_states(&falling, self.out, self.file);
        self.frames = merged;
        true
    }

    fn loop_body(&mut self, body: &HirBlock) {
        let entry = self.frames.clone();
        let _ = self.block(body);
        self.frames = entry;
    }

    fn switch_arms_with_source(&mut self, arms: &[HirSwitchArm], source: Option<&str>) -> bool {
        if arms.is_empty() {
            return true;
        }
        let entry = self.frames.clone();
        let mut falling: Vec<StateFrames> = Vec::with_capacity(arms.len());
        for arm in arms {
            self.frames = entry.clone();
            // A `None` arm observed the iterator's end on this path.
            if let Some(source) = source {
                if pattern_is_none(&arm.pattern) {
                    self.mark_iter_closed(source);
                }
            }
            if let Some(guard) = &arm.guard {
                self.walk(guard);
            }
            self.walk(&arm.body);
            falling.push(self.frames.clone());
        }
        let merged = merge_states(&falling, self.out, self.file);
        self.frames = merged;
        true
    }

    /// Like [`Self::branch`], but one side observed `it.next()` ending:
    /// `close_then`/`close_else` say which side saw `None`.
    fn branch_iter(
        &mut self,
        then_block: &HirBlock,
        else_block: Option<&HirBlock>,
        close_then: bool,
        close_else: bool,
        source: &str,
    ) -> bool {
        let entry = self.frames.clone();
        let mut falling: Vec<StateFrames> = Vec::new();
        if close_then {
            self.mark_iter_closed(source);
        }
        if self.block(then_block) {
            falling.push(self.frames.clone());
        }
        self.frames = entry.clone();
        if close_else {
            self.mark_iter_closed(source);
        }
        match else_block {
            Some(block) => {
                if self.block(block) {
                    falling.push(self.frames.clone());
                }
            }
            None => falling.push(self.frames.clone()),
        }
        if falling.is_empty() {
            self.frames = entry;
            return false;
        }
        let merged = merge_states(&falling, self.out, self.file);
        self.frames = merged;
        true
    }

    /// The iterator a `next()` result flows from: the receiver of a
    /// direct `it.next()`, or the iterator a binding holding such a
    /// result was bound from.
    fn iterator_source(&self, expr: &HirExpr) -> Option<String> {
        match expr {
            HirExpr::Call { callee, args, .. } if is_iterator_next(callee) => {
                args.first().and_then(receiver_name)
            }
            HirExpr::Ident(path, ..) if path.len() == 1 => self
                .find(path.first().unwrap())
                .and_then(|binding| binding.iter_src.clone()),
            _ => None,
        }
    }

    fn ensure_iter_binding(&mut self, name: &str, span: Span) {
        if self.find(name).is_none() {
            if let Some(frame) = self.frames.last_mut() {
                let _ = span;
                frame.push(Binding {
                    name: name.to_string(),
                    span,
                    state: State::Open,
                    tracked: true,
                    iter_closed: false,
                    iter_src: None,
                });
            }
        }
    }

    fn mark_iter_closed(&mut self, name: &str) {
        if let Some(binding) = self.find_mut(name) {
            binding.iter_closed = true;
        }
    }

    fn report_iter_after_done(&mut self, name: &str, span: Span) {
        self.out.push(
            Diagnostic::new(
                Severity::Error,
                self.file,
                span,
                format!("`{name}` is used after its iterator returned `None`"),
            )
            .with_code(codes::USE_AFTER_CLOSE),
        );
    }

    fn find(&self, name: &str) -> Option<&Binding> {
        self.frames
            .iter()
            .rev()
            .flat_map(|frame| frame.iter().rev())
            .find(|b| b.name == name)
    }

    fn find_mut(&mut self, name: &str) -> Option<&mut Binding> {
        self.frames
            .iter_mut()
            .rev()
            .flat_map(|frame| frame.iter_mut().rev())
            .find(|b| b.name == name)
    }

    fn report_bad_transition(&mut self, name: &str, from: State, to: State, span: Span) {
        self.out.push(
            Diagnostic::new(
                Severity::Error,
                self.file,
                span,
                format!("`{name}` is in state `{from}`, but this operation requires `{to}`"),
            )
            .with_code(codes::DOUBLE_CLOSE),
        );
    }

    fn report_use_after_close(&mut self, name: &str, span: Span) {
        self.out.push(
            Diagnostic::new(
                Severity::Error,
                self.file,
                span,
                format!("`{name}` is used after it has been released"),
            )
            .with_code(codes::USE_AFTER_CLOSE),
        );
    }
}

/// Whether a callee is a concrete `Type.Iterator.next` symbol.
fn is_iterator_next(callee: &[String]) -> bool {
    callee.len() == 1 && callee[0].contains(".Iterator.next")
}

/// The binding a call's receiver names: `&it` for a pointer receiver, or
/// a bare `it` for a by-value one.
fn receiver_name(arg: &HirExpr) -> Option<String> {
    match arg {
        HirExpr::AddrOf { name, .. } => Some(name.clone()),
        HirExpr::Ident(path, ..) if path.len() == 1 => path.first().cloned(),
        _ => None,
    }
}

fn pattern_is_some(pattern: &HirPattern) -> bool {
    matches!(pattern, HirPattern::TupleVariant { path, .. } if path.last().map(|s| s.as_str()) == Some("Some"))
}

fn pattern_is_none(pattern: &HirPattern) -> bool {
    matches!(pattern, HirPattern::UnitVariant { path, .. } if path.last().map(|s| s.as_str()) == Some("None"))
}

fn resource_state_of_init(expr: &HirExpr) -> Option<State> {
    match expr {
        HirExpr::Try { expr, .. } => resource_state_of_init(expr),
        HirExpr::Call { callee, .. } => {
            let name = callee.last().map(|s| s.as_str()).unwrap_or("");
            match name {
                "mutex_new" => Some(State::Unlocked),
                "chan_new" => Some(State::Open),
                "alloc" => Some(State::Allocated),
                "listen" | "accept" | "connect" | "net_listen" | "net_accept" | "net_connect" => {
                    Some(State::Open)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Returns (required_state, next_state) for a state-changing builtin.
fn transition_for(callee: &[String]) -> Option<(State, State)> {
    let name = callee.last().map(|s| s.as_str()).unwrap_or("");
    match name {
        "mutex_lock" => Some((State::Unlocked, State::Locked)),
        "mutex_unlock" => Some((State::Locked, State::Unlocked)),
        "mutex_free" => Some((State::Unlocked, State::Freed)),
        "chan_close" => Some((State::Open, State::Closed)),
        "chan_free" => Some((State::Open, State::Freed)),
        "close" | "net_close" => Some((State::Open, State::Closed)),
        "close_listener" | "net_close_listener" => Some((State::Open, State::Closed)),
        "dealloc" => Some((State::Allocated, State::Freed)),
        _ => {
            // Check for .free() method calls - callee like ["Type", "free"] or ["free"]
            if name == "free" {
                Some((State::Allocated, State::Freed))
            } else {
                None
            }
        }
    }
}

fn merge_states(states: &[StateFrames], out: &mut Vec<Diagnostic>, file: FileId) -> StateFrames {
    if states.is_empty() {
        return Vec::new();
    }
    if states.len() == 1 {
        return states[0].clone();
    }
    // Assume all states have the same structure (same scopes, same bindings).
    let num_scopes = states[0].len();
    let mut merged: StateFrames = Vec::with_capacity(num_scopes);
    for scope_idx in 0..num_scopes {
        let mut merged_scope: Scope = Vec::new();
        let num_bindings = states[0][scope_idx].len();
        for bind_idx in 0..num_bindings {
            // Find the binding across all states.
            let first = &states[0][scope_idx][bind_idx];
            let name = first.name.clone();
            let span = first.span;
            let tracked = first.tracked;
            // Collect states.
            let mut state_set: Vec<State> = Vec::new();
            for s in states {
                if scope_idx < s.len() && bind_idx < s[scope_idx].len() {
                    state_set.push(s[scope_idx][bind_idx].state);
                }
            }
            // If all same, use that. Otherwise report E1400 and use first.
            let merged_state = if state_set.iter().all(|st| *st == state_set[0]) {
                state_set[0]
            } else {
                // States disagree: this is a real possibility the program has
                // to account for, but reporting it here duplicates the leak
                // pass's E1000 on the same facts. Keep the more conservative
                // state (terminal if any path is terminal) and stay silent.
                if state_set.iter().any(|s| s.is_terminal()) {
                    *state_set.iter().find(|s| s.is_terminal()).unwrap()
                } else {
                    state_set[0]
                }
            };
            // An iterator is definitely closed only when every falling
            // path closed it: a `None` on one branch is not a `None` on
            // the other, and diagnosing the join would be a false
            // positive. `iter_src` survives the merge from any path.
            let mut iter_closed = true;
            let mut iter_src: Option<String> = None;
            for s in states {
                if scope_idx < s.len() && bind_idx < s[scope_idx].len() {
                    let other = &s[scope_idx][bind_idx];
                    iter_closed = iter_closed && other.iter_closed;
                    if iter_src.is_none() {
                        iter_src = other.iter_src.clone();
                    }
                } else {
                    iter_closed = false;
                }
            }
            merged_scope.push(Binding {
                name,
                span,
                state: merged_state,
                tracked,
                iter_closed,
                iter_src,
            });
        }
        merged.push(merged_scope);
    }
    merged
}

/// Race freedom: a capture that reaches a spawned thread must be shareable.
fn check_race(program: &HirProgram, out: &mut Vec<Diagnostic>) {
    for item in &program.items {
        if let HirItem::Fn(function) = item {
            check_race_in_block(&function.body, program.uses_threads, out, function.file);
        }
    }
}

fn check_race_in_block(
    block: &HirBlock,
    uses_threads: bool,
    out: &mut Vec<Diagnostic>,
    file: FileId,
) {
    for stmt in &block.stmts {
        check_race_in_stmt(stmt, uses_threads, out, file);
    }
}

fn check_race_in_stmt(stmt: &HirStmt, uses_threads: bool, out: &mut Vec<Diagnostic>, file: FileId) {
    match stmt {
        HirStmt::Expr(expr, ..) => check_race_in_expr(expr, uses_threads, out, file),
        HirStmt::Const { init, .. }
        | HirStmt::Var { init, .. }
        | HirStmt::Destructure { init, .. } => {
            check_race_in_expr(init, uses_threads, out, file);
        }
        HirStmt::Assign { value, .. } => check_race_in_expr(value, uses_threads, out, file),
        HirStmt::IndexAssign {
            base, index, value, ..
        } => {
            check_race_in_expr(base, uses_threads, out, file);
            check_race_in_expr(index, uses_threads, out, file);
            check_race_in_expr(value, uses_threads, out, file);
        }
        HirStmt::FieldAssign { value, .. } => check_race_in_expr(value, uses_threads, out, file),
        HirStmt::FieldPtrAssign {
            pointer,
            index,
            value,
            ..
        } => {
            check_race_in_expr(pointer, uses_threads, out, file);
            check_race_in_expr(index, uses_threads, out, file);
            check_race_in_expr(value, uses_threads, out, file);
        }
        HirStmt::Return(Some(expr), _) => check_race_in_expr(expr, uses_threads, out, file),
        HirStmt::If {
            init,
            cond,
            then_block,
            else_block,
            ..
        } => {
            if let Some(init) = init {
                check_race_in_stmt(init, uses_threads, out, file);
            }
            check_race_in_expr(cond, uses_threads, out, file);
            check_race_in_block(then_block, uses_threads, out, file);
            if let Some(eb) = else_block {
                check_race_in_block(eb, uses_threads, out, file);
            }
        }
        HirStmt::ForRange {
            start, end, body, ..
        } => {
            check_race_in_expr(start, uses_threads, out, file);
            check_race_in_expr(end, uses_threads, out, file);
            check_race_in_block(body, uses_threads, out, file);
        }
        HirStmt::ForIn { iterable, body, .. } => {
            check_race_in_expr(iterable, uses_threads, out, file);
            check_race_in_block(body, uses_threads, out, file);
        }
        HirStmt::ForC {
            init,
            cond,
            update,
            body,
            ..
        } => {
            if let Some(init) = init {
                check_race_in_stmt(init, uses_threads, out, file);
            }
            if let Some(cond) = cond {
                check_race_in_expr(cond, uses_threads, out, file);
            }
            if let Some(update) = update {
                check_race_in_expr(update, uses_threads, out, file);
            }
            check_race_in_block(body, uses_threads, out, file);
        }
        HirStmt::Switch { subject, arms, .. } => {
            check_race_in_expr(subject, uses_threads, out, file);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    check_race_in_expr(guard, uses_threads, out, file);
                }
                check_race_in_expr(&arm.body, uses_threads, out, file);
            }
        }
        HirStmt::Defer { body, .. } => check_race_in_block(body, uses_threads, out, file),
        HirStmt::Scope { body, .. } => check_race_in_block(body, uses_threads, out, file),
        HirStmt::IfLet {
            subject,
            then_block,
            else_block,
            ..
        } => {
            check_race_in_expr(subject, uses_threads, out, file);
            check_race_in_block(then_block, uses_threads, out, file);
            if let Some(else_block) = else_block {
                check_race_in_block(else_block, uses_threads, out, file);
            }
        }
        HirStmt::LoopInvariant(c) => check_race_in_expr(&c.expr, uses_threads, out, file),
        _ => {}
    }
}

fn check_race_in_expr(expr: &HirExpr, uses_threads: bool, out: &mut Vec<Diagnostic>, file: FileId) {
    match expr {
        HirExpr::Call {
            callee, args, span, ..
        } => {
            let name = callee.last().map(|s| s.as_str()).unwrap_or("");
            if name == "thread_spawn" {
                for arg in args {
                    if let HirExpr::Lambda { f, .. } = arg {
                        for capture in &f.captures {
                            if !is_shareable(&capture.ty, uses_threads) {
                                out.push(Diagnostic::new(
                                    Severity::Error,
                                    file,
                                    f.span,
                                    format!("`{}` is shared across a thread boundary without a `Mutex`", capture.name),
                                ).with_code(codes::RACE));
                            }
                        }
                    }
                }
            }
            for arg in args {
                check_race_in_expr(arg, uses_threads, out, file);
            }
        }
        HirExpr::Spawn {
            callee, args, span, ..
        } => {
            // One rule, one implementation: a capture that reaches a
            // concurrent context must be shareable, where a concurrent
            // context is `thread_spawn` or `spawn`. A `spawn` argument is
            // such a capture, so it is checked like a lambda capture.
            check_race_in_expr(callee, uses_threads, out, file);
            for arg in args {
                if !is_shareable_spawn(&yashy_hir::hir::expr_ty(arg)) {
                    out.push(
                        Diagnostic::new(
                            Severity::Error,
                            file,
                            *span,
                            format!(
                                "`{}` is shared across a task boundary without a `Mutex`",
                                yashy_hir::hir::expr_ty(arg)
                            ),
                        )
                        .with_code(codes::RACE),
                    );
                }
                check_race_in_expr(arg, uses_threads, out, file);
            }
        }
        HirExpr::Lambda { f, .. } => check_race_in_block(&f.body, uses_threads, out, file),
        HirExpr::Binary { lhs, rhs, .. } => {
            check_race_in_expr(lhs, uses_threads, out, file);
            check_race_in_expr(rhs, uses_threads, out, file);
        }
        HirExpr::Unary { operand, .. } | HirExpr::Move(operand, ..) => {
            check_race_in_expr(operand, uses_threads, out, file);
        }
        HirExpr::StructLit { fields, .. } => {
            for (_, v, _) in fields {
                check_race_in_expr(v, uses_threads, out, file);
            }
        }
        HirExpr::SliceLit { elements, .. }
        | HirExpr::FixedArrayLit { elements, .. }
        | HirExpr::TupleLit { elements, .. } => {
            for e in elements {
                check_race_in_expr(e, uses_threads, out, file);
            }
        }
        HirExpr::FieldAccess { base, .. } => check_race_in_expr(base, uses_threads, out, file),
        HirExpr::Index { base, index, .. } => {
            check_race_in_expr(base, uses_threads, out, file);
            check_race_in_expr(index, uses_threads, out, file);
        }
        HirExpr::View {
            base, start, end, ..
        } => {
            check_race_in_expr(base, uses_threads, out, file);
            check_race_in_expr(start, uses_threads, out, file);
            check_race_in_expr(end, uses_threads, out, file);
        }
        HirExpr::Cast { expr, .. } | HirExpr::Try { expr, .. } => {
            check_race_in_expr(expr, uses_threads, out, file);
        }
        HirExpr::SwitchExpr { subject, arms, .. } => {
            check_race_in_expr(subject, uses_threads, out, file);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    check_race_in_expr(guard, uses_threads, out, file);
                }
                check_race_in_expr(&arm.body, uses_threads, out, file);
            }
        }
        HirExpr::CallCapture { args, .. } => {
            for arg in args {
                check_race_in_expr(arg, uses_threads, out, file);
            }
        }
        HirExpr::HigherOrder {
            receiver,
            func,
            init,
            ..
        } => {
            check_race_in_expr(receiver, uses_threads, out, file);
            check_race_in_expr(func, uses_threads, out, file);
            if let Some(init) = init {
                check_race_in_expr(init, uses_threads, out, file);
            }
        }
        _ => {}
    }
}

/// Shareability across an async task boundary: the same rule as
/// [`is_shareable`] with `string` accepted, since an owned `string` capture
/// is a value copy like the other `Copy` scalars.
fn is_shareable_spawn(ty: &yashy_hir::ty::Type) -> bool {
    // Reuse the thread rule with `string` accepted: an owned `string`
    // capture is a value copy like the other `Copy` scalars the rule accepts.
    match &ty.kind {
        yashy_hir::ty::TypeKind::Str => true,
        _ => is_shareable(ty, false),
    }
}

fn is_shareable(ty: &yashy_hir::ty::Type, uses_threads: bool) -> bool {
    use yashy_hir::ty::TypeKind;
    match &ty.kind {
        TypeKind::I64
        | TypeKind::F64
        | TypeKind::Bool
        | TypeKind::Char
        | TypeKind::Unit
        | TypeKind::CInt(_)
        | TypeKind::CUInt(_)
        | TypeKind::CFloat(_)
        | TypeKind::CVoid
        | TypeKind::FixedArray(..)
        | TypeKind::View(_) => true,
        TypeKind::Str => uses_threads,
        TypeKind::Mutex => true,
        TypeKind::Channel(_) => true,
        _ => false,
    }
}
