//! Type checking: the untyped AST goes in, the typed HIR comes out.
//!
//! [`check_program`] is the entry point. It runs two passes -- item
//! signatures first (so mutual recursion works), then bodies -- and returns
//! every diagnostic it found, not just the first.

pub mod check_expr;
pub mod check_pat;
pub mod error;
pub mod exhaustive;
pub mod hir;
pub mod ownership;
pub mod scope;
pub mod ty;

pub use check_expr::{check_program, check_program_with, CheckOptions};
pub use check_pat::check_pattern;
pub use error::TypeError;
pub use exhaustive::{is_exhaustive, missing_cases};
pub use hir::*;
pub use ownership::{linearity_of, Linearity, Ownership};
pub use scope::{Binding, Scope};
pub use ty::{Mutability, Type, TypeKind};
