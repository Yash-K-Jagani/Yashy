//! The verifier's passes.
//!
//! A pass reads the checked HIR and pushes diagnostics; it never rewrites the
//! program. One module per rule keeps the entry point from growing a branch for
//! every rule, which is what lets contracts, effects and typestate land later
//! without touching [`crate::verify`].

pub mod bounds;
pub mod contracts;
pub mod exhaustive;
pub mod leak;
pub mod overflow;
