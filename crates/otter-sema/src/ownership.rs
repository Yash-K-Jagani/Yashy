//! Linear ownership: which types may be copied, and which have one owner.
//!
//! Most values in Otter are `Copy`: `i64`, `f64`, `bool`, `string` (refcounted,
//! so a copy is a cheap retain), and structs and enums built from them. A raw
//! pointer is different. `*mut T` names a heap allocation that must be freed
//! exactly once, so the checker tracks, per binding, whether that binding still
//! owns the allocation.
//!
//! This is a *syntactic, per-binding* check, exactly as the master plan's ledger
//! L5 specifies: it catches the two mistakes that matter — double moves and reads
//! of a moved binding — and it deliberately does not model aliasing, which is
//! what `unsafe` marks (design spec §8.1). It is also conservative across control
//! flow: a binding moved in one branch of an `if` counts as moved after the `if`,
//! so a program that frees in both branches is fine but one that frees only in
//! the `then` branch and then uses the value is rejected rather than risked.

use crate::ty::TypeKind;

/// Whether a type's values may be freely duplicated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Linearity {
    /// At most one binding owns a value of this type at a time. Moving it
    /// transfers ownership; reading a moved binding is an error.
    Linear,
    /// Values may be copied as often as the program likes.
    Copy,
}

/// Classify a type by whether it can be copied.
///
/// `Ptr` and `CPtr` are linear: the first owns an Otter allocation, the second
/// points at memory the checker cannot see across the C boundary. M6 adds
/// `Slice` — the one arm the design promises — and nothing else becomes linear
/// after that.
pub fn linearity_of(ty: &TypeKind) -> Linearity {
    match ty {
        TypeKind::Ptr(_) | TypeKind::CPtr(_) | TypeKind::Slice(_) | TypeKind::Map(_, _) => {
            Linearity::Linear
        }
        TypeKind::Mutex | TypeKind::Channel(_) => Linearity::Linear,
        _ => Linearity::Copy,
    }
}

/// What a binding currently holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ownership {
    /// The binding owns its value and may be used.
    #[default]
    Live,
    /// The value has been moved out. Any read is an error until the binding is
    /// re-initialised by an assignment.
    Moved,
}
