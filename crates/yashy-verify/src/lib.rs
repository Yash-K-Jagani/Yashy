//! The verifier: the single owner of Yashy's static rules.
//!
//! Everything that decides whether a program is *sound* -- as opposed to
//! well-typed -- belongs here. In this slice that is linear ownership and the
//! leak pass; later slices add the `defer` rules, contract discharge, effects
//! and typestate. The crate takes no external dependency: the rules are the
//! compiler's, not a library's.

pub mod domain;
pub mod ownership;
pub mod pass;
pub mod render;

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
    pass::leak::check(hir, &mut diagnostics);
    pass::exhaustive::check(hir, &mut diagnostics);
    pass::contracts::check(hir, options.contracts, &mut obligations, &mut diagnostics);
    VerifyReport {
        diagnostics,
        obligations,
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
