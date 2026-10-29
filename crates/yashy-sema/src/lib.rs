//! Type checking: the untyped AST goes in, the typed HIR comes out.
//!
//! [`check_program`] is the entry point. It runs two passes -- item
//! signatures first (so mutual recursion works), then bodies -- and returns
//! every diagnostic it found, not just the first.

pub mod check_expr;
pub mod check_pat;
pub mod const_eval;
pub mod error;
pub mod scope;

pub use yashy_hir::{exhaustive, hir, ty};
/// The ownership rules are the verifier's; `yashy-sema` only asks them.
pub use yashy_verify::ownership;

pub use check_expr::{check_modules_with, check_program, check_program_with, CheckOptions};
pub use check_pat::check_pattern;
pub use const_eval::{ConstFn, ConstValue};
pub use error::TypeError;
pub use exhaustive::{is_exhaustive, missing_cases};
pub use hir::*;
pub use ownership::{linearity_of, Linearity, Ownership};
pub use scope::{Binding, Scope};
pub use ty::{Mutability, Type, TypeKind};
