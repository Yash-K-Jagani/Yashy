//! The verifier: the single owner of Yashy's static rules.
//!
//! Everything that decides whether a program is *sound* -- as opposed to
//! well-typed -- belongs here. In this slice that is linear ownership and the
//! leak pass; later slices add the `defer` rules, contract discharge, effects
//! and typestate. The crate takes no external dependency: the rules are the
//! compiler's, not a library's.

pub mod domain;
pub mod effect;
pub mod ownership;
pub mod pass;
pub mod render;

use std::collections::BTreeMap;

use yashy_common::{Diagnostic, Span};
use yashy_hir::hir::{HirBlock, HirExpr, HirFn, HirItem, HirProgram, HirStmt};

pub use ownership::{check_live, linearity_of, move_out, AlreadyMoved, Linearity, Ownership};
pub use pass::inventory::{inventory, Assumption, AssumptionKind};

/// What to do with a contract the verifier cannot discharge.
///
/// The mode exists before the contract pass does because the two are one
/// interface: a caller chooses how much proof it demands, and the verifier
/// answers with diagnostics whose severity follows that choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContractMode {
    /// Report an undischarged contract as a warning. The build still succeeds.
    #[default]
    Warn,
    /// Report an undischarged contract as an error. The build fails.
    Enforce,
}

/// How the verifier should read the program it is given.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VerifyOptions {
    /// What an undischarged contract means. Read by the contract pass.
    pub contracts: ContractMode,
}

/// What a proof obligation is about.
///
/// The kinds are a closed set, like error codes: a new analysis adds a variant
/// and a consumer that knows what to do with it. Phase 3b adds `ShiftInRange`
/// for the shift-count check; the bounds proof here is the shape the rest copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObligationKind {
    /// An array index is inside the array's length.
    Bounds,
    /// A checked integer operation's result is inside the `int` range.
    Overflow,
}

/// One fact the verifier tried to prove, and whether it could.
///
/// An obligation is not a diagnostic. A fact the verifier could not establish
/// is not an error -- it is a runtime check that stays -- and a fact it did
/// establish is what lets a later phase delete that check. Phase 3 turns the
/// established ones into `ProofFacts` and consults them in codegen; nothing
/// before that reads them, which is why they live beside the diagnostics rather
/// than inside them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofObligation {
    /// Which rule the fact belongs to.
    pub kind: ObligationKind,
    /// The span of the expression the fact is about: the index expression, so
    /// codegen can find the check it may delete.
    pub span: Span,
    /// True when the fact holds on every path that reaches the expression.
    pub established: bool,
}

/// What the verifier found.
///
/// The verifier never stops at the first problem: a user fixes a list, not a
/// conversation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VerifyReport {
    pub diagnostics: Vec<Diagnostic>,
    /// Every fact the verifier attempted to prove, established or not.
    pub obligations: Vec<ProofObligation>,
    /// How many functions the verifier analysed, counting each one once no
    /// matter how many times it was reached. A function reached from fifty
    /// call sites is one analysis, not fifty: the walk is memoised by
    /// `(name, body span)` so the same body is never walked twice.
    pub analyses: usize,
}

impl VerifyReport {
    /// True when the program passed every rule.
    pub fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }

    /// How many of the diagnostics are errors.
    pub fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.is_error())
            .count()
    }
}

/// Verify an already-checked program.
///
/// The program must have come out of `yashy-sema`: the passes read the typed
/// HIR, and a program that did not type-check has no HIR to read. Every pass
/// runs; the report holds their diagnostics in pass order.
pub fn verify(hir: &HirProgram, options: &VerifyOptions) -> VerifyReport {
    let mut diagnostics = Vec::new();
    let mut obligations = Vec::new();
    // A function is walked once per program, no matter how many times it is
    // reached. The memo is keyed by (name, body span): a lambda has an empty
    // name, and two functions with the same name but different bodies are
    // different analyses.
    let mut memo: BTreeMap<(String, u32), ()> = BTreeMap::new();
    let analyses = collect_functions(hir, &mut memo);
    pass::leak::check(hir, &mut diagnostics);
    pass::exhaustive::check(hir, &mut diagnostics);
    pass::contracts::check(hir, options.contracts, &mut obligations, &mut diagnostics);
    VerifyReport {
        diagnostics,
        obligations,
        analyses,
    }
}

/// Walk the program's items (and every lambda nested in them) and count the
/// distinct functions by `(name, body span)`. The count is the number of
/// analyses the verifier performs: each unique body is walked once by every
/// pass, and a body reached from fifty call sites is still one body.
fn collect_functions(hir: &HirProgram, memo: &mut BTreeMap<(String, u32), ()>) -> usize {
    for item in &hir.items {
        if let HirItem::Fn(function) = item {
            memo.insert((function.name.clone(), function.body.span.start), ());
            collect_in_block(&function.body, memo);
        }
    }
    memo.len()
}

fn collect_in_block(block: &HirBlock, memo: &mut BTreeMap<(String, u32), ()>) {
    for stmt in &block.stmts {
        collect_in_stmt(stmt, memo);
    }
}

fn collect_in_stmt(stmt: &HirStmt, memo: &mut BTreeMap<(String, u32), ()>) {
    match stmt {
        HirStmt::Expr(expr, ..)
        | HirStmt::Const { init: expr, .. }
        | HirStmt::Var { init: expr, .. }
        | HirStmt::Destructure { init: expr, .. }
        | HirStmt::Assign { value: expr, .. }
        | HirStmt::IndexAssign { value: expr, .. }
        | HirStmt::FieldAssign { value: expr, .. }
        | HirStmt::FieldPtrAssign { value: expr, .. } => collect_in_expr(expr, memo),
        HirStmt::Return(Some(expr), _) => collect_in_expr(expr, memo),
        HirStmt::If {
            init,
            cond,
            then_block,
            else_block,
            ..
        } => {
            if let Some(init) = init {
                collect_in_stmt(init, memo);
            }
            collect_in_expr(cond, memo);
            collect_in_block(then_block, memo);
            if let Some(else_block) = else_block {
                collect_in_block(else_block, memo);
            }
        }
        HirStmt::ForRange {
            start, end, body, ..
        } => {
            collect_in_expr(start, memo);
            collect_in_expr(end, memo);
            collect_in_block(body, memo);
        }
        HirStmt::ForIn { iterable, body, .. } => {
            collect_in_expr(iterable, memo);
            collect_in_block(body, memo);
        }
        HirStmt::ForC {
            init,
            cond,
            update,
            body,
            ..
        } => {
            if let Some(init) = init {
                collect_in_stmt(init, memo);
            }
            if let Some(cond) = cond {
                collect_in_expr(cond, memo);
            }
            if let Some(update) = update {
                collect_in_expr(update, memo);
            }
            collect_in_block(body, memo);
        }
        HirStmt::Switch { subject, arms, .. } => {
            collect_in_expr(subject, memo);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    collect_in_expr(guard, memo);
                }
                collect_in_expr(&arm.body, memo);
            }
        }
        HirStmt::Defer { body, .. } => collect_in_block(body, memo),
        HirStmt::LoopInvariant(contract) => collect_in_expr(&contract.expr, memo),
        HirStmt::Return(None, _)
        | HirStmt::Break(_)
        | HirStmt::Continue(_)
        | HirStmt::Assign { .. }
        | HirStmt::IndexAssign { .. }
        | HirStmt::FieldAssign { .. }
        | HirStmt::FieldPtrAssign { .. } => {}
    }
}

fn collect_in_expr(expr: &HirExpr, memo: &mut BTreeMap<(String, u32), ()>) {
    match expr {
        HirExpr::Lambda { f, .. } => {
            memo.insert((f.name.clone(), f.body.span.start), ());
            collect_in_block(&f.body, memo);
        }
        HirExpr::Binary { lhs, rhs, .. } => {
            collect_in_expr(lhs, memo);
            collect_in_expr(rhs, memo);
        }
        HirExpr::Unary { operand, .. } | HirExpr::Move(operand, ..) => {
            collect_in_expr(operand, memo);
        }
        HirExpr::Call { args, .. } | HirExpr::CallCapture { args, .. } => {
            for arg in args {
                collect_in_expr(arg, memo);
            }
        }
        HirExpr::StructLit { fields, .. } => {
            for (_, value, _) in fields {
                collect_in_expr(value, memo);
            }
        }
        HirExpr::SliceLit { elements, .. }
        | HirExpr::FixedArrayLit { elements, .. }
        | HirExpr::TupleLit { elements, .. } => {
            for element in elements {
                collect_in_expr(element, memo);
            }
        }
        HirExpr::FieldAccess { base, .. } => collect_in_expr(base, memo),
        HirExpr::Index { base, index, .. } => {
            collect_in_expr(base, memo);
            collect_in_expr(index, memo);
        }
        HirExpr::View {
            base, start, end, ..
        } => {
            collect_in_expr(base, memo);
            collect_in_expr(start, memo);
            collect_in_expr(end, memo);
        }
        HirExpr::Cast { expr, .. } | HirExpr::Try { expr, .. } => collect_in_expr(expr, memo),
        HirExpr::SwitchExpr { subject, arms, .. } => {
            collect_in_expr(subject, memo);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    collect_in_expr(guard, memo);
                }
                collect_in_expr(&arm.body, memo);
            }
        }
        HirExpr::HigherOrder {
            receiver,
            func,
            init,
            ..
        } => {
            collect_in_expr(receiver, memo);
            collect_in_expr(func, memo);
            if let Some(init) = init {
                collect_in_expr(init, memo);
            }
        }
        HirExpr::Literal(..)
        | HirExpr::Ident(..)
        | HirExpr::AddrOf { .. }
        | HirExpr::Capture { .. } => {}
    }
}

/// The name of the first function that declares a contract, if any.
///
/// A contract is a promise only the verifier can examine: `requires` and
/// `ensures` are discharged at call sites and returns, and a loop `invariant`
/// is re-established at every back edge. A build that skips the verifier
/// therefore cannot honour any of them, which is why `--no-verify` refuses a
/// program that has one. Answering that question is the verifier's job because
/// it is a fact about contract semantics, and the walk has to reach the places
/// a clause can hide: a loop inside a lambda inside a call argument.
///
/// Returns the *first* name in item order so the message is deterministic.
pub fn first_contract(hir: &HirProgram) -> Option<&str> {
    hir.items.iter().find_map(|item| match item {
        HirItem::Fn(function) if function_has_contract(function) => Some(function.name.as_str()),
        _ => None,
    })
}

/// True when a function declares a clause or has a loop invariant in its body.
fn function_has_contract(function: &HirFn) -> bool {
    !function.requires.is_empty()
        || !function.ensures.is_empty()
        || block_has_contract(&function.body)
}

fn block_has_contract(block: &HirBlock) -> bool {
    block.stmts.iter().any(stmt_has_contract)
}

fn stmt_has_contract(stmt: &HirStmt) -> bool {
    match stmt {
        HirStmt::LoopInvariant(_) => true,
        HirStmt::Expr(expr, ..) => expr_has_contract(expr),
        HirStmt::Const { init, .. }
        | HirStmt::Var { init, .. }
        | HirStmt::Destructure { init, .. }
        | HirStmt::Assign { value: init, .. }
        | HirStmt::FieldAssign { value: init, .. }
        | HirStmt::Return(Some(init), _) => expr_has_contract(init),
        HirStmt::IndexAssign {
            base, index, value, ..
        } => expr_has_contract(base) || expr_has_contract(index) || expr_has_contract(value),
        HirStmt::FieldPtrAssign {
            pointer,
            index,
            value,
            ..
        } => expr_has_contract(pointer) || expr_has_contract(index) || expr_has_contract(value),
        HirStmt::Return(None, _) | HirStmt::Break(_) | HirStmt::Continue(_) => false,
        HirStmt::If {
            init,
            cond,
            then_block,
            else_block,
            ..
        } => {
            init.as_ref().is_some_and(|stmt| stmt_has_contract(stmt))
                || expr_has_contract(cond)
                || block_has_contract(then_block)
                || else_block.as_ref().is_some_and(block_has_contract)
        }
        HirStmt::ForRange {
            start, end, body, ..
        } => expr_has_contract(start) || expr_has_contract(end) || block_has_contract(body),
        HirStmt::ForIn { iterable, body, .. } => {
            expr_has_contract(iterable) || block_has_contract(body)
        }
        HirStmt::ForC {
            init,
            cond,
            update,
            body,
            ..
        } => {
            init.as_ref().is_some_and(|stmt| stmt_has_contract(stmt))
                || cond.as_ref().is_some_and(expr_has_contract)
                || update.as_ref().is_some_and(expr_has_contract)
                || block_has_contract(body)
        }
        HirStmt::Switch { subject, arms, .. } => {
            expr_has_contract(subject)
                || arms.iter().any(|arm| {
                    arm.guard.as_ref().is_some_and(expr_has_contract)
                        || expr_has_contract(&arm.body)
                })
        }
        HirStmt::Defer { body, .. } => block_has_contract(body),
    }
}

fn expr_has_contract(expr: &HirExpr) -> bool {
    match expr {
        // A lambda's body is a function: its clauses and its loops count.
        HirExpr::Lambda { f, .. } => function_has_contract(f),
        HirExpr::Binary { lhs, rhs, .. } => expr_has_contract(lhs) || expr_has_contract(rhs),
        HirExpr::Unary { operand, .. } | HirExpr::Move(operand, ..) => expr_has_contract(operand),
        HirExpr::Call { args, .. } | HirExpr::CallCapture { args, .. } => {
            args.iter().any(expr_has_contract)
        }
        HirExpr::StructLit { fields, .. } => {
            fields.iter().any(|(_, expr, _)| expr_has_contract(expr))
        }
        HirExpr::SliceLit { elements, .. }
        | HirExpr::FixedArrayLit { elements, .. }
        | HirExpr::TupleLit { elements, .. } => elements.iter().any(expr_has_contract),
        HirExpr::FieldAccess { base, .. } | HirExpr::Cast { expr: base, .. } => {
            expr_has_contract(base)
        }
        HirExpr::Index { base, index, .. } => expr_has_contract(base) || expr_has_contract(index),
        HirExpr::View {
            base, start, end, ..
        } => expr_has_contract(base) || expr_has_contract(start) || expr_has_contract(end),
        HirExpr::Try { expr, .. } => expr_has_contract(expr),
        HirExpr::SwitchExpr { subject, arms, .. } => {
            expr_has_contract(subject)
                || arms.iter().any(|arm| {
                    arm.guard.as_ref().is_some_and(expr_has_contract)
                        || expr_has_contract(&arm.body)
                })
        }
        HirExpr::HigherOrder {
            receiver,
            func,
            init,
            ..
        } => {
            expr_has_contract(receiver)
                || expr_has_contract(func)
                || init.as_ref().is_some_and(|expr| expr_has_contract(expr))
        }
        HirExpr::Literal(..)
        | HirExpr::Ident(..)
        | HirExpr::AddrOf { .. }
        | HirExpr::Capture { .. } => false,
    }
}
