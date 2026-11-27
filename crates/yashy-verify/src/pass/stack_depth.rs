//! Recursion-depth accounting for `stack_alloc`.
//!
//! A `stack_alloc` reserves bytes in the calling frame. A cycle of calls that
//! contains a reservation can exceed the bounded frame no matter how small the
//! reservation is, because nothing bounds the number of times the cycle runs.
//! This pass builds the call graph from checked HIR and refuses such a cycle at
//! the reservation's span, with `E1202` naming the rule.
//!
//! A recursion that reserves nothing is fine by this rule: it can still fail
//! to terminate, but it cannot exceed the `stack_alloc` frame guard.

use std::collections::{BTreeMap, BTreeSet};

use yashy_common::{codes, Diagnostic, FileId, Severity, Span};
use yashy_hir::hir::{
    HirBlock, HirExpr, HirFn, HirItem, HirProgram, HirStmt, HirSwitchArm, Literal,
};

/// What one function contributes to the call graph and the frame guard.
#[derive(Clone, Debug)]
struct FunctionFacts {
    name: String,
    file: FileId,
    /// The first `stack_alloc` span, where a failure is reported.
    reservation: Option<Span>,
    bytes: u64,
    calls: BTreeSet<String>,
}

/// Run the recursion pass over `program`.
pub fn check(program: &HirProgram, out: &mut Vec<Diagnostic>) {
    let mut facts: BTreeMap<(String, u32), FunctionFacts> = BTreeMap::new();
    for item in &program.items {
        if let HirItem::Fn(function) = item {
            let mut walker = Walker {
                reservation: None,
                bytes: 0,
                calls: BTreeSet::new(),
            };
            walker.block(&function.body);
            facts.insert(
                (function.name.clone(), function.body.span.start),
                FunctionFacts {
                    name: function.name.clone(),
                    file: function.file,
                    reservation: walker.reservation,
                    bytes: walker.bytes,
                    calls: walker.calls,
                },
            );
        }
    }

    // Only calls to functions in the program can close a cycle. Builtins and
    // extern declarations have no body here, so they cannot reserve stack
    // bytes through a Yashy frame.
    for ((_, start), fact) in &facts {
        if fact.bytes == 0 {
            continue;
        }
        if reaches_self(fact, &facts) {
            let span = fact
                .reservation
                .unwrap_or_else(|| Span::new(*start, *start));
            out.push(
                Diagnostic::new(
                    Severity::Error,
                    fact.file,
                    span,
                    format!(
                        "recursive call cycle reserves stack bytes in `{}`",
                        fact.name
                    ),
                )
                .with_code(codes::STACK_OVERFLOW),
            );
        }
    }
}

/// Transitive closure over the call graph: can `start` reach itself?
fn reaches_self(start: &FunctionFacts, facts: &BTreeMap<(String, u32), FunctionFacts>) -> bool {
    let mut seen = BTreeSet::new();
    let mut queue: Vec<String> = start.calls.iter().cloned().collect();
    while let Some(name) = queue.pop() {
        if name == start.name {
            return true;
        }
        if !seen.insert(name.clone()) {
            continue;
        }
        for ((_, _), callee) in facts {
            if callee.name == name {
                queue.extend(callee.calls.iter().cloned());
            }
        }
    }
    false
}

struct Walker {
    reservation: Option<Span>,
    bytes: u64,
    calls: BTreeSet<String>,
}

impl Walker {
    fn expr(&mut self, expr: &HirExpr) {
        match expr {
            HirExpr::Call { callee, args, .. } => {
                if let Some(name) = callee.last() {
                    self.calls.insert(name.clone());
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
            HirExpr::UnsafeBlock { body, .. } => self.block(body),
            HirExpr::StructLit { fields, .. } => {
                for (_, value, _) in fields {
                    self.expr(value);
                }
            }
            HirExpr::SliceLit { elements, .. } => {
                for element in elements {
                    self.expr(element);
                }
            }
            HirExpr::SwitchExpr { subject, arms, .. } => {
                self.expr(subject);
                for arm in arms {
                    self.expr(&arm.body);
                }
            }
            HirExpr::Spawn { callee, args, .. } => {
                self.expr(callee);
                for arg in args {
                    self.expr(arg);
                }
            }
            HirExpr::FixedArrayLit { elements, .. } | HirExpr::TupleLit { elements, .. } => {
                for element in elements {
                    self.expr(element);
                }
            }
            HirExpr::View {
                base, start, end, ..
            } => {
                self.expr(base);
                self.expr(start);
                self.expr(end);
            }
            HirExpr::Index { base, index, .. } => {
                self.expr(base);
                self.expr(index);
            }
            HirExpr::Cast { expr, .. }
            | HirExpr::Try { expr, .. }
            | HirExpr::Await { expr, .. } => self.expr(expr),
            HirExpr::Lambda { f, .. } => self.block(&f.body),
            _ => {}
        }
    }

    fn block(&mut self, block: &HirBlock) {
        for stmt in &block.stmts {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &HirStmt) {
        match stmt {
            HirStmt::Var { name, init, .. } | HirStmt::Const { name, init, .. } => {
                self.expr(init);
                if let HirExpr::Call {
                    callee, args, span, ..
                } = init
                {
                    if callee.last().map(String::as_str) == Some("stack_alloc") {
                        if let Some(HirExpr::Literal(Literal::I64(count), _, _)) = args.first() {
                            if *count > 0 {
                                if self.reservation.is_none() {
                                    self.reservation = Some(*span);
                                }
                                self.bytes = self.bytes.saturating_add(*count as u64 * 8);
                            }
                        }
                    }
                }
                let _ = name;
            }
            HirStmt::Assign { value, .. }
            | HirStmt::IndexAssign { value, .. }
            | HirStmt::FieldAssign { value, .. }
            | HirStmt::FieldPtrAssign { value, .. }
            | HirStmt::Expr(value, _, _) => self.expr(value),
            HirStmt::Return(Some(expr), _) => self.expr(expr),
            HirStmt::Destructure { init, .. } | HirStmt::LetElse { init, .. } => {
                self.expr(init);
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
            HirStmt::ForRange {
                start,
                end,
                step,
                body,
                ..
            } => {
                self.expr(start);
                self.expr(end);
                if let Some(step) = step {
                    self.expr(step);
                }
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
                for HirSwitchArm { guard, body, .. } in arms {
                    if let Some(guard) = guard {
                        self.expr(guard);
                    }
                    self.expr(body);
                }
            }
            HirStmt::Defer { body, .. } | HirStmt::Scope { body, .. } => self.block(body),
            _ => {}
        }
    }
}
