//! Linear ownership: which types may be copied, and which have one owner.
//!
//! Most values in Yashy are `Copy`: `int`, `float`, `bool`, `string` (refcounted,
//! so a copy is a cheap retain), and structs and enums built from them. A raw
//! pointer is different. `*mut T` names a heap allocation that must be freed
//! exactly once, so the checker tracks, per binding, whether that binding still
//! owns the allocation.
//!
//! This is a *syntactic, per-binding* check, exactly as the master plan's ledger
//! L5 specifies: it catches the two mistakes that matter -- double moves and reads
//! of a moved binding -- and it deliberately does not model aliasing, which is
//! what `unsafe` marks (design spec §8.1). It is also conservative across control
//! flow: a binding moved in one branch of an `if` counts as moved after the `if`,
//! so a program that frees in both branches is fine but one that frees only in
//! the `then` branch and then uses the value is rejected rather than risked.
//!
//! The rules live here rather than in `yashy-sema` because the verifier is the
//! single owner of them: `yashy-sema` decides *when* to ask, and this module
//! decides *what the answer is*. One rule, one implementation.

use yashy_hir::ty::TypeKind;

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
/// `Ptr` and `CPtr` are linear: the first owns a Yashy allocation, the second
/// points at memory the checker cannot see across the C boundary. `Slice` and
/// `Map` are linear because they own a backing allocation, and `Mutex` and
/// `Channel` because they own a handle. Nothing else is linear.
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

/// A use of a binding whose value has already been spent.
///
/// The name is carried because the diagnostic has to name the binding: "value
/// has already been moved" is not actionable, "value `buf` has already been
/// moved" is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlreadyMoved {
    pub name: String,
}

/// Spend a binding's value. A second spend of the same binding is the error
/// that stops a double free.
pub fn move_out(state: &mut Ownership, name: &str) -> Result<(), AlreadyMoved> {
    match *state {
        Ownership::Moved => Err(AlreadyMoved {
            name: name.to_string(),
        }),
        Ownership::Live => {
            *state = Ownership::Moved;
            Ok(())
        }
    }
}

/// Reject a read of a binding whose value has been spent.
pub fn check_live(state: Ownership, name: &str) -> Result<(), AlreadyMoved> {
    match state {
        Ownership::Moved => Err(AlreadyMoved {
            name: name.to_string(),
        }),
        Ownership::Live => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yashy_common::Span;
    use yashy_hir::ty::Type;

    #[test]
    fn a_second_move_is_rejected_and_names_the_binding() {
        let mut state = Ownership::Live;
        assert_eq!(move_out(&mut state, "buf"), Ok(()));
        assert_eq!(
            move_out(&mut state, "buf"),
            Err(AlreadyMoved {
                name: "buf".to_string()
            })
        );
    }

    #[test]
    fn a_rebound_binding_is_live_again() {
        let mut state = Ownership::Moved;
        assert!(check_live(state, "buf").is_err());
        state = Ownership::Live;
        assert_eq!(check_live(state, "buf"), Ok(()));
    }

    #[test]
    fn a_moved_binding_cannot_be_read() {
        assert_eq!(
            check_live(Ownership::Moved, "buf"),
            Err(AlreadyMoved {
                name: "buf".to_string()
            })
        );
    }

    #[test]
    fn pointers_slices_maps_mutexes_and_channels_are_linear() {
        let i64_ty = || Type {
            kind: TypeKind::I64,
            span: Span::new(0, 0),
        };
        assert_eq!(
            linearity_of(&TypeKind::Ptr(Box::new(i64_ty()))),
            Linearity::Linear
        );
        assert_eq!(
            linearity_of(&TypeKind::Slice(Box::new(i64_ty()))),
            Linearity::Linear
        );
        assert_eq!(linearity_of(&TypeKind::Mutex), Linearity::Linear);
        assert_eq!(linearity_of(&TypeKind::I64), Linearity::Copy);
    }
}
