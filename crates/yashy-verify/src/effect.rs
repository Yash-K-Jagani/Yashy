//! Effects: what a function does to the world.
//!
//! Re-exports the types from `yashy-common` and provides inference.

pub use yashy_common::{DeclaredEffect, Effect};
pub use yashy_hir::hir::HirProgram;

use std::collections::BTreeMap;

/// Infer effects for all functions in a program.
pub fn infer_effects(hir: &HirProgram) -> BTreeMap<String, Effect> {
    crate::pass::effects::infer_effects(hir)
}
