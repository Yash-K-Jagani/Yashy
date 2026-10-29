//! Effects: what a function does to the world.
//!
//! Re-exports the types from `yashy-common` and provides inference.

pub use yashy_common::{DeclaredEffect, Effect};
pub use yashy_hir::hir::HirProgram;

use std::collections::BTreeMap;
use yashy_hir::hir::HirItem;

/// Infer effects for all functions in a program.
pub fn infer_effects(hir: &HirProgram) -> BTreeMap<String, Effect> {
    // Placeholder: will be implemented in Task 2.
    let mut result = BTreeMap::new();
    for item in &hir.items {
        if let HirItem::Fn(f) = item {
            result.insert(f.name.clone(), Effect::PURE);
        }
    }
    result
}