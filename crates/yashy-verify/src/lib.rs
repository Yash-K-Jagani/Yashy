//! The verifier: the single owner of Yashy's static rules.
//!
//! Everything that decides whether a program is *sound* -- as opposed to
//! well-typed -- belongs here. In this slice that is linear ownership; later
//! slices add the leak and `defer` passes, contract discharge, effects and
//! typestate. The crate takes no external dependency: the rules are the
//! compiler's, not a library's.

pub mod ownership;

pub use ownership::{check_live, linearity_of, move_out, AlreadyMoved, Linearity, Ownership};
