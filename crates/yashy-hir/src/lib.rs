//! The typed IR and its type representation.
//!
//! These lived inside `yashy-sema` until phase 1a. They are their own crate
//! because `yashy-verify` must read the HIR while `yashy-sema` calls into the
//! verifier's ownership rules, and two crates cannot depend on each other.

pub mod exhaustive;
pub mod hir;
pub mod ty;
