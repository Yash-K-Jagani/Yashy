//! The verifier: the single owner of Yashy's static rules.
//!
//! Everything that decides whether a program is *sound* -- as opposed to
//! well-typed -- belongs here. In this slice that is linear ownership and the
//! leak pass; later slices add the `defer` rules, contract discharge, effects
//! and typestate. The crate takes no external dependency: the rules are the
//! compiler's, not a library's.

pub mod ownership;
pub mod pass;

use yashy_common::Diagnostic;
use yashy_hir::hir::HirProgram;

pub use ownership::{check_live, linearity_of, move_out, AlreadyMoved, Linearity, Ownership};

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

/// What the verifier found.
///
/// The verifier never stops at the first problem: a user fixes a list, not a
/// conversation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VerifyReport {
    pub diagnostics: Vec<Diagnostic>,
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
    // `options.contracts` is consulted by the contract pass, which lands with
    // contract syntax. Until then the field is part of the interface and
    // nothing reads it.
    let _ = options;
    let mut diagnostics = Vec::new();
    pass::leak::check(hir, &mut diagnostics);
    VerifyReport { diagnostics }
}
