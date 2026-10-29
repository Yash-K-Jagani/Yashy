//! Proving that an array index is inside its array.
//!
//! Indexing in Yashy never traps: `xs[i]` is an `Option`, and an index that is
//! negative or past the end answers `None`. The tutorial promises exactly that,
//! and `tests/programs/fixed_arrays.ya` demonstrates it with `grid[3]` on a
//! three-element array. So a bounds rule cannot be a soundness rule -- there is
//! nothing to be unsound about -- and this pass reports no diagnostic.
//!
//! What it produces is a proof. For each index expression, [`check`] answers
//! whether the domain can show `0 <= i < len` on every path that reaches it.
//! Phase 3 deletes the runtime check exactly when that proof exists, and keeps
//! it otherwise; an unproved index is a check that stays, never an error.
//!
//! Two shapes are deliberately not obligations:
//!
//! - a view, `a[start..end]`, because a view *clamps* by definition (`0..99` is
//!   the whole array, `2..1` is empty), so there is no check to prove;
//! - a pointer index, `p[i]`, because a pointer carries no length to compare
//!   against. Pointer access is the unsafe rules' business, not this pass's.

use crate::domain::Interval;

/// What the domain can say about one index against one length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bounds {
    /// Every value the index can take is in `0..length`.
    InRange,
    /// No value the index can take is in `0..length`.
    OutOfRange,
    /// Some values are in range and some are not, or the domain knows nothing.
    Unknown,
}

/// Decide one index against a known length.
///
/// The three answers are exhaustive and the split is exact: an interval that
/// overlaps the valid range but is not contained in it is [`Bounds::Unknown`],
/// because one execution may be in range and another may not.
pub fn check(length: i64, index: Interval) -> Bounds {
    if index.is_empty() {
        // No execution reaches this index, so nothing can be out of range. The
        // caller does not walk impossible environments, but the rule does not
        // depend on that: an empty set of values is vacuously in range.
        return Bounds::InRange;
    }
    let length = i128::from(length);
    if index.hi < 0 || index.lo >= length {
        return Bounds::OutOfRange;
    }
    if index.lo >= 0 && index.hi < length {
        return Bounds::InRange;
    }
    Bounds::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interval(lo: i64, hi: i64) -> Interval {
        Interval::new(i128::from(lo), i128::from(hi))
    }

    #[test]
    fn a_constant_inside_the_length_is_in_range() {
        assert_eq!(check(3, interval(0, 0)), Bounds::InRange);
        assert_eq!(check(3, interval(2, 2)), Bounds::InRange);
        assert_eq!(check(3, interval(0, 2)), Bounds::InRange);
    }

    #[test]
    fn an_interval_that_never_reaches_the_range_is_out_of_range() {
        assert_eq!(check(3, interval(3, 3)), Bounds::OutOfRange);
        assert_eq!(check(3, interval(9, 99)), Bounds::OutOfRange);
        assert_eq!(check(3, interval(-9, -1)), Bounds::OutOfRange);
    }

    #[test]
    fn an_interval_that_straddles_the_range_is_unknown() {
        assert_eq!(check(3, interval(-1, 1)), Bounds::Unknown);
        assert_eq!(check(3, interval(2, 9)), Bounds::Unknown);
        assert_eq!(check(3, Interval::TOP), Bounds::Unknown);
    }

    #[test]
    fn an_empty_length_has_no_valid_index() {
        assert_eq!(check(0, interval(0, 0)), Bounds::OutOfRange);
        assert_eq!(check(0, interval(-1, -1)), Bounds::OutOfRange);
    }
}
