//! Proving that checked integer arithmetic cannot overflow.
//!
//! Integer `+`, `-`, `*` and unary `-` are **checked**: an operation that
//! overflows stops the program with a message and a position instead of
//! wrapping to a wrong number. A provable overflow is therefore a certain
//! fault, and rejecting it is sound -- unlike a provably out-of-range index,
//! which the language defines as `None` (see [`crate::pass::bounds`]).
//!
//! The rule is narrow on purpose. It reports only when *every* result the
//! operands can produce is outside `i64`:
//!
//! - an operation whose whole result range fits is [`Overflow::InRange`], and
//!   phase 3 may use that fact to delete the runtime check;
//! - one whose range may or may not fit is [`Overflow::Unknown`], which is a
//!   runtime check that stays;
//! - one whose whole range is outside is [`Overflow::OutOfRange`], which is a
//!   certain fault and is reported.
//!
//! The range is computed in `i128` *before* any clamping. [`Interval`] clamps
//! every result to `i64` and answers [`Interval::TOP`] when an operation leaves
//! it, so reading the clamped interval would lose exactly the fact this rule
//! needs. Operand bounds are already inside `i64`, so their sum, difference and
//! product are inside `i128`.
//!
//! Division and remainder are not modelled here. The domain does not model them
//! either, and their faults -- a zero divisor, and `i64::MIN / -1` -- belong to
//! the division rule rather than to this one.

use yashy_hir::hir::BinOp;

use crate::domain::Interval;

/// The smallest and largest `int`, as this rule's bounds.
const MIN: i128 = i64::MIN as i128;
const MAX: i128 = i64::MAX as i128;

/// What the domain can say about one operation's result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overflow {
    /// Every value the operation can produce is an `int`.
    InRange,
    /// No value the operation can produce is an `int`: it always faults.
    OutOfRange,
    /// Some values fit and some do not, or the domain knows nothing.
    Unknown,
}

/// Decide a binary `+`, `-` or `*`.
///
/// A different operator, or an empty operand, answers [`Overflow::InRange`]:
/// the former is not this rule's business, and the latter means no execution
/// reaches the operation, so nothing can overflow.
pub fn check(op: BinOp, left: Interval, right: Interval) -> Overflow {
    if left.is_empty() || right.is_empty() {
        return Overflow::InRange;
    }
    let Some((lo, hi)) = math_range(op, left, right) else {
        return Overflow::InRange;
    };
    classify(lo, hi)
}

/// Decide a unary `-`.
pub fn check_neg(operand: Interval) -> Overflow {
    if operand.is_empty() {
        return Overflow::InRange;
    }
    classify(-operand.hi, -operand.lo)
}

/// The mathematical result range of `op`, unclamped, or `None` for an operator
/// this rule does not model.
fn math_range(op: BinOp, left: Interval, right: Interval) -> Option<(i128, i128)> {
    match op {
        BinOp::Add => Some((left.lo + right.lo, left.hi + right.hi)),
        BinOp::Sub => Some((left.lo - right.hi, left.hi - right.lo)),
        BinOp::Mul => {
            // Four products cover both signs without assuming anything about
            // them, exactly as `Interval::mul` does.
            let products = [
                left.lo * right.lo,
                left.lo * right.hi,
                left.hi * right.lo,
                left.hi * right.hi,
            ];
            Some((
                products.iter().copied().min().unwrap_or(0),
                products.iter().copied().max().unwrap_or(0),
            ))
        }
        _ => None,
    }
}

fn classify(lo: i128, hi: i128) -> Overflow {
    if hi < MIN || lo > MAX {
        Overflow::OutOfRange
    } else if MIN <= lo && hi <= MAX {
        Overflow::InRange
    } else {
        Overflow::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interval(lo: i64, hi: i64) -> Interval {
        Interval::new(i128::from(lo), i128::from(hi))
    }

    fn exact(n: i64) -> Interval {
        Interval::exact(n)
    }

    #[test]
    fn a_sum_that_fits_is_in_range() {
        assert_eq!(check(BinOp::Add, exact(1), exact(2)), Overflow::InRange);
        assert_eq!(
            check(BinOp::Add, interval(-10, 10), interval(-5, 5)),
            Overflow::InRange
        );
    }

    #[test]
    fn a_sum_past_the_largest_int_is_out_of_range() {
        assert_eq!(
            check(BinOp::Add, exact(i64::MAX), exact(1)),
            Overflow::OutOfRange
        );
        assert_eq!(
            check(BinOp::Add, exact(i64::MAX), exact(i64::MAX)),
            Overflow::OutOfRange
        );
    }

    #[test]
    fn a_difference_past_the_smallest_int_is_out_of_range() {
        assert_eq!(
            check(BinOp::Sub, exact(i64::MIN), exact(1)),
            Overflow::OutOfRange
        );
        assert_eq!(
            check(BinOp::Sub, exact(0), exact(i64::MIN)),
            Overflow::OutOfRange
        );
    }

    #[test]
    fn a_product_past_the_range_is_out_of_range() {
        assert_eq!(
            check(BinOp::Mul, exact(i64::MAX), exact(2)),
            Overflow::OutOfRange
        );
        assert_eq!(
            check(BinOp::Mul, exact(i64::MIN), exact(i64::MIN)),
            Overflow::OutOfRange
        );
    }

    #[test]
    fn a_range_that_straddles_the_boundary_is_unknown() {
        assert_eq!(
            check(BinOp::Add, Interval::TOP, exact(1)),
            Overflow::Unknown
        );
        assert_eq!(
            check(BinOp::Mul, exact(i64::MAX), interval(0, 2)),
            Overflow::Unknown
        );
    }

    #[test]
    fn negating_the_smallest_int_is_out_of_range() {
        assert_eq!(check_neg(exact(i64::MIN)), Overflow::OutOfRange);
        assert_eq!(check_neg(exact(i64::MAX)), Overflow::InRange);
        assert_eq!(check_neg(exact(0)), Overflow::InRange);
    }

    #[test]
    fn an_operator_this_rule_does_not_model_is_in_range() {
        assert_eq!(
            check(BinOp::Div, exact(i64::MIN), exact(-1)),
            Overflow::InRange
        );
        assert_eq!(check(BinOp::Eq, exact(1), exact(2)), Overflow::InRange);
    }

    #[test]
    fn an_empty_operand_cannot_overflow() {
        assert_eq!(
            check(BinOp::Add, Interval::BOTTOM, exact(i64::MAX)),
            Overflow::InRange
        );
        assert_eq!(check_neg(Interval::BOTTOM), Overflow::InRange);
    }
}
