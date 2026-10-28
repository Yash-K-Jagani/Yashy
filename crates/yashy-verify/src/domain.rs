//! The numeric abstract domain contract discharge reasons in.
//!
//! A [`Value`] is an [`Interval`] over the `i64` the language's `int` is, plus
//! a [`Congruence`]: the residue class every value of the variable falls in.
//! The interval says how big the value can get; the congruence says what it can
//! be. A literal `6` is the interval `[6, 6]` with congruence "exactly 6", and
//! `n * 2` is whatever `n`'s interval times two with every value even, which is
//! how `n * 2 == 3` is refused without knowing anything about `n`.
//!
//! The domain is deliberately small. It models addition, subtraction,
//! multiplication and comparison; every other operation answers [`Value::top`],
//! and the contract pass treats "top" as "not proved". That is the whole
//! soundness argument: nothing here claims more than it can justify, so a fact
//! the verifier proves is a fact that holds.

use std::collections::BTreeMap;

use yashy_hir::hir::BinOp;
use yashy_hir::ty::{Type, TypeKind};

/// The smallest and largest `int`, as the domain's bounds.
const MIN: i128 = i64::MIN as i128;
const MAX: i128 = i64::MAX as i128;

/// A closed interval of integers.
///
/// The bounds are `i128` so an operation on two `i64`-bounded intervals cannot
/// overflow before it is clamped; every interval this type hands out is inside
/// the `i64` range. An empty interval -- `lo > hi` -- is the bottom element: no
/// value reaches this point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interval {
    pub lo: i128,
    pub hi: i128,
}

impl Interval {
    /// Every `int`.
    pub const TOP: Interval = Interval { lo: MIN, hi: MAX };

    /// No `int`.
    pub const BOTTOM: Interval = Interval { lo: 1, hi: 0 };

    /// The one-point interval `[n, n]`.
    pub fn exact(n: i64) -> Interval {
        Interval {
            lo: n as i128,
            hi: n as i128,
        }
    }

    /// `[lo, hi]`, clamped to the `i64` range; empty when the two cross.
    pub fn new(lo: i128, hi: i128) -> Interval {
        if lo > hi {
            return Interval::BOTTOM;
        }
        // A range entirely outside `i64` describes no `int`, so it is empty
        // rather than a clamped lie.
        if hi < MIN || lo > MAX {
            return Interval::BOTTOM;
        }
        Interval {
            lo: lo.max(MIN),
            hi: hi.min(MAX),
        }
    }

    pub fn is_empty(self) -> bool {
        self.lo > self.hi
    }

    pub fn contains(self, n: i64) -> bool {
        self.lo <= n as i128 && n as i128 <= self.hi
    }

    /// The one value in the interval, if it holds exactly one.
    pub fn exact_value(self) -> Option<i64> {
        if self.lo == self.hi && (MIN..=MAX).contains(&self.lo) {
            Some(self.lo as i64)
        } else {
            None
        }
    }

    /// The smallest interval containing both.
    pub fn join(self, other: Interval) -> Interval {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        Interval {
            lo: self.lo.min(other.lo),
            hi: self.hi.max(other.hi),
        }
    }

    /// The largest interval contained in both.
    pub fn meet(self, other: Interval) -> Interval {
        Interval::new(self.lo.max(other.lo), self.hi.min(other.hi))
    }

    pub fn add(self, other: Interval) -> Interval {
        if self.is_empty() || other.is_empty() {
            return Interval::BOTTOM;
        }
        arithmetic(self.lo + other.lo, self.hi + other.hi)
    }

    pub fn sub(self, other: Interval) -> Interval {
        if self.is_empty() || other.is_empty() {
            return Interval::BOTTOM;
        }
        arithmetic(self.lo - other.hi, self.hi - other.lo)
    }

    pub fn mul(self, other: Interval) -> Interval {
        if self.is_empty() || other.is_empty() {
            return Interval::BOTTOM;
        }
        // Four products cover both signs without assuming anything about them.
        let products = [
            self.lo * other.lo,
            self.lo * other.hi,
            self.hi * other.lo,
            self.hi * other.hi,
        ];
        let lo = products.iter().copied().min().unwrap_or(0);
        let hi = products.iter().copied().max().unwrap_or(0);
        arithmetic(lo, hi)
    }

    pub fn neg(self) -> Interval {
        if self.is_empty() {
            return Interval::BOTTOM;
        }
        arithmetic(-self.hi, -self.lo)
    }

    /// `self + n`, without building a one-point interval at the call site.
    pub fn add_const(self, n: i64) -> Interval {
        self.add(Interval::exact(n))
    }

    /// `self - n`.
    pub fn sub_const(self, n: i64) -> Interval {
        self.sub(Interval::exact(n))
    }
}

/// The residue class a value falls in: `x ≡ residue (mod modulus)`.
///
/// `modulus == 0` means the value is exactly `residue`; `modulus == 1` means
/// every integer, which is the top element. A residue is kept in `0..modulus`
/// so two equal classes compare equal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Congruence {
    modulus: u64,
    residue: i64,
}

impl Congruence {
    /// Every integer.
    pub const ANY: Congruence = Congruence {
        modulus: 1,
        residue: 0,
    };

    /// Exactly `n`.
    pub fn exact(n: i64) -> Congruence {
        Congruence {
            modulus: 0,
            residue: n,
        }
    }

    /// The class `residue (mod modulus)`, normalised.
    pub fn new(modulus: u64, residue: i64) -> Congruence {
        if modulus == 0 {
            return Congruence::exact(residue);
        }
        if modulus == 1 {
            return Congruence::ANY;
        }
        let m = modulus as i128;
        let residue = ((residue as i128).rem_euclid(m)) as i64;
        Congruence { modulus, residue }
    }

    pub fn is_exact(self) -> bool {
        self.modulus == 0
    }

    pub fn exact_value(self) -> Option<i64> {
        if self.is_exact() {
            Some(self.residue)
        } else {
            None
        }
    }

    /// The period a non-exact class repeats with; an exact value constrains no
    /// period, so it counts as one for the gcd and lcm that follow.
    fn period(self) -> u64 {
        if self.modulus == 0 {
            1
        } else {
            self.modulus
        }
    }

    pub fn contains(self, n: i64) -> bool {
        if self.is_exact() {
            return self.residue == n;
        }
        ((n as i128 - self.residue as i128).rem_euclid(self.modulus as i128)) == 0
    }

    /// True when no integer is in both classes.
    pub fn disjoint(self, other: Congruence) -> bool {
        // An exact value has no period to take a gcd with: it is one point, so
        // the answer is just whether the other class contains it.
        if self.is_exact() {
            return !other.contains(self.residue);
        }
        if other.is_exact() {
            return !self.contains(other.residue);
        }
        let g = gcd(self.modulus as u128, other.modulus as u128) as i128;
        (self.residue as i128 - other.residue as i128).rem_euclid(g) != 0
    }

    /// The smallest congruence class containing both.
    pub fn join(self, other: Congruence) -> Congruence {
        if self == Congruence::ANY || other == Congruence::ANY {
            return Congruence::ANY;
        }
        match (self.is_exact(), other.is_exact()) {
            (true, true) => {
                if self.residue == other.residue {
                    self
                } else {
                    let distance =
                        (self.residue as i128 - other.residue as i128).unsigned_abs() as u64;
                    Congruence::new(distance, self.residue)
                }
            }
            (true, false) => {
                if other.contains(self.residue) {
                    other
                } else {
                    Congruence::ANY
                }
            }
            (false, true) => {
                if self.contains(other.residue) {
                    self
                } else {
                    Congruence::ANY
                }
            }
            (false, false) => {
                // Every value in both classes is a multiple of this gcd away
                // from `self.residue`, and no stronger period is common.
                let mut m = gcd(self.modulus as u128, other.modulus as u128);
                m = gcd(
                    m,
                    (self.residue as i128 - other.residue as i128).unsigned_abs(),
                );
                Congruence::new(u64::try_from(m).unwrap_or(u64::MAX), self.residue)
            }
        }
    }

    /// The largest congruence class contained in both, or `None` when they
    /// share no value.
    pub fn meet(self, other: Congruence) -> Option<Congruence> {
        if self == Congruence::ANY {
            return Some(other);
        }
        if other == Congruence::ANY {
            return Some(self);
        }
        if self.is_exact() {
            return other.contains(self.residue).then_some(self);
        }
        if other.is_exact() {
            return self.contains(other.residue).then_some(other);
        }
        if self == other {
            return Some(self);
        }
        // Two different classes with a common value meet in a class whose
        // period is their least common multiple. That refinement is not needed
        // by anything here, so the answer stays honest and loose.
        Some(Congruence::ANY)
    }

    pub fn add(self, other: Congruence) -> Congruence {
        match (self.exact_value(), other.exact_value()) {
            (Some(a), Some(b)) => match checked(a, b, i64::checked_add) {
                Some(n) => Congruence::exact(n),
                None => Congruence::ANY,
            },
            _ => {
                let m = lcm(self.period(), other.period());
                match checked(self.residue, other.residue, i64::checked_add) {
                    Some(n) => Congruence::new(m, n),
                    None => Congruence::ANY,
                }
            }
        }
    }

    pub fn sub(self, other: Congruence) -> Congruence {
        self.add(other.neg())
    }

    pub fn neg(self) -> Congruence {
        if self.is_exact() {
            return match self.residue.checked_neg() {
                Some(n) => Congruence::exact(n),
                None => Congruence::ANY,
            };
        }
        match self.residue.checked_neg() {
            Some(n) => Congruence::new(self.modulus, n),
            None => Congruence::ANY,
        }
    }

    /// `self * k`: every value is `k` times a value of the class.
    pub fn mul_const(self, k: i64) -> Congruence {
        if k == 0 {
            return Congruence::exact(0);
        }
        if let Some(n) = self.exact_value() {
            return match n.checked_mul(k) {
                Some(n) => Congruence::exact(n),
                None => Congruence::ANY,
            };
        }
        let scale = u128::from(k.unsigned_abs());
        let modulus = u128::from(self.period()) * scale;
        match u64::try_from(modulus) {
            Ok(m) => match self.residue.checked_mul(k) {
                Some(n) => Congruence::new(m, n),
                None => Congruence::ANY,
            },
            Err(_) => Congruence::ANY,
        }
    }
}

/// The bounds of a result computed from two non-empty intervals.
///
/// An operation whose result cannot be an `int` is [`Interval::TOP`], not
/// bottom: overflow is a fact the domain does not model, and "no value" would
/// make every clause about the result vacuously true. Bottom is reserved for a
/// genuine contradiction -- an empty meet, an impossible refinement -- because
/// only that means no execution reaches the point.
fn arithmetic(lo: i128, hi: i128) -> Interval {
    if lo > hi || hi < MIN || lo > MAX {
        return Interval::TOP;
    }
    Interval {
        lo: lo.max(MIN),
        hi: hi.min(MAX),
    }
}

fn checked(a: i64, b: i64, f: fn(i64, i64) -> Option<i64>) -> Option<i64> {
    f(a, b)
}

fn gcd(a: u128, b: u128) -> u128 {
    let (mut a, mut b) = (a, b);
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a
}

fn lcm(a: u64, b: u64) -> u64 {
    let g = gcd(u128::from(a), u128::from(b));
    if g == 0 {
        return 0;
    }
    let l = u128::from(a) / g * u128::from(b);
    u64::try_from(l).unwrap_or(u64::MAX)
}

/// One abstract value: an interval and a congruence that both hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Value {
    pub interval: Interval,
    pub congruence: Congruence,
}

impl Value {
    /// No information.
    pub fn top() -> Value {
        Value {
            interval: Interval::TOP,
            congruence: Congruence::ANY,
        }
    }

    /// No value reaches here.
    pub fn bottom() -> Value {
        Value {
            interval: Interval::BOTTOM,
            congruence: Congruence::ANY,
        }
    }

    /// Exactly `n`.
    pub fn exact(n: i64) -> Value {
        Value {
            interval: Interval::exact(n),
            congruence: Congruence::exact(n),
        }
    }

    /// Any value in `[lo, hi]`.
    pub fn range(lo: i128, hi: i128) -> Value {
        let interval = Interval::new(lo, hi);
        if interval.is_empty() {
            return Value::bottom();
        }
        Value {
            interval,
            congruence: Congruence::ANY,
        }
    }

    /// The range a parameter of this type can hold.
    ///
    /// Only the integer types say anything. A `char` is a byte; a C integer is
    /// the range of its width. Everything else -- `float`, `bool`, strings,
    /// aggregates -- is opaque here, because the domain models integers only.
    pub fn of_type(ty: &Type) -> Value {
        match &ty.kind {
            TypeKind::I64 => Value::top(),
            TypeKind::Char => Value::range(0, 255),
            TypeKind::CInt(bits) => {
                let bits = (*bits).min(63);
                let half = 1i128 << (bits - 1);
                Value::range(-half, half - 1)
            }
            TypeKind::CUInt(bits) => {
                let bits = (*bits).min(63);
                Value::range(0, (1i128 << bits) - 1)
            }
            TypeKind::CSize => Value::range(0, MAX),
            _ => Value::top(),
        }
    }

    pub fn is_empty(self) -> bool {
        self.interval.is_empty()
    }

    /// The one value this can be, if it is a single point.
    pub fn exact_value(self) -> Option<i64> {
        self.interval.exact_value()
    }

    pub fn join(self, other: Value) -> Value {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        Value {
            interval: self.interval.join(other.interval),
            congruence: self.congruence.join(other.congruence),
        }
    }

    pub fn meet(self, other: Value) -> Value {
        let congruence = match self.congruence.meet(other.congruence) {
            Some(congruence) => congruence,
            None => return Value::bottom(),
        };
        let mut interval = self.interval.meet(other.interval);
        if let Some(n) = congruence.exact_value() {
            interval = interval.meet(Interval::exact(n));
        }
        if interval.is_empty() {
            return Value::bottom();
        }
        Value {
            interval,
            congruence,
        }
    }

    pub fn add(self, other: Value) -> Value {
        if self.is_empty() || other.is_empty() {
            return Value::bottom();
        }
        Value {
            interval: self.interval.add(other.interval),
            congruence: self.congruence.add(other.congruence),
        }
    }

    pub fn sub(self, other: Value) -> Value {
        if self.is_empty() || other.is_empty() {
            return Value::bottom();
        }
        Value {
            interval: self.interval.sub(other.interval),
            congruence: self.congruence.sub(other.congruence),
        }
    }

    pub fn mul(self, other: Value) -> Value {
        if self.is_empty() || other.is_empty() {
            return Value::bottom();
        }
        // A congruence survives multiplication by a constant, not by another
        // variable, so the class is kept only when one side is a single value.
        let congruence = match (self.exact_value(), other.exact_value()) {
            (Some(k), None) => other.congruence.mul_const(k),
            (None, Some(k)) => self.congruence.mul_const(k),
            _ => Congruence::ANY,
        };
        Value {
            interval: self.interval.mul(other.interval),
            congruence,
        }
    }

    pub fn neg(self) -> Value {
        if self.is_empty() {
            return Value::bottom();
        }
        Value {
            interval: self.interval.neg(),
            congruence: self.congruence.neg(),
        }
    }

    /// True when the two can never be equal.
    pub fn eq_is_false(self, other: Value) -> bool {
        if self.interval.meet(other.interval).is_empty() {
            return true;
        }
        self.congruence.disjoint(other.congruence)
    }

    /// True when the two are the same value.
    pub fn eq_is_true(self, other: Value) -> bool {
        match (self.exact_value(), other.exact_value()) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    }
}

/// Three-valued truth: what a comparison is known to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tri {
    True,
    False,
    Unknown,
}

impl Tri {
    pub fn not(self) -> Tri {
        match self {
            Tri::True => Tri::False,
            Tri::False => Tri::True,
            Tri::Unknown => Tri::Unknown,
        }
    }

    pub fn and(self, other: Tri) -> Tri {
        match (self, other) {
            (Tri::False, _) | (_, Tri::False) => Tri::False,
            (Tri::True, Tri::True) => Tri::True,
            _ => Tri::Unknown,
        }
    }

    pub fn or(self, other: Tri) -> Tri {
        match (self, other) {
            (Tri::True, _) | (_, Tri::True) => Tri::True,
            (Tri::False, Tri::False) => Tri::False,
            _ => Tri::Unknown,
        }
    }
}

/// What `lhs op rhs` is known to be.
pub fn compare(op: BinOp, lhs: Value, rhs: Value) -> Tri {
    if lhs.is_empty() || rhs.is_empty() {
        return Tri::Unknown;
    }
    let (l, r) = (lhs.interval, rhs.interval);
    match op {
        BinOp::Eq => {
            if lhs.eq_is_true(rhs) {
                Tri::True
            } else if lhs.eq_is_false(rhs) {
                Tri::False
            } else {
                Tri::Unknown
            }
        }
        BinOp::Ne => compare(BinOp::Eq, lhs, rhs).not(),
        BinOp::Lt => {
            if l.hi < r.lo {
                Tri::True
            } else if l.lo >= r.hi {
                Tri::False
            } else {
                Tri::Unknown
            }
        }
        BinOp::Le => {
            if l.hi <= r.lo {
                Tri::True
            } else if l.lo > r.hi {
                Tri::False
            } else {
                Tri::Unknown
            }
        }
        BinOp::Gt => compare(BinOp::Lt, rhs, lhs),
        BinOp::Ge => compare(BinOp::Le, rhs, lhs),
        _ => Tri::Unknown,
    }
}

/// What the contract pass knows about one function at one point.
///
/// `values` holds the integers; `lengths` holds the one property of a slice or
/// view the domain can name, its length, so `len(xs)` is a number rather than
/// an unknown. A call can change a slice's length through the header it shares
/// with the caller, so every length is dropped at a call.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Env {
    values: BTreeMap<String, Value>,
    lengths: BTreeMap<String, i64>,
    /// True when no execution reaches this point: a branch condition and its
    /// refinement contradict each other, or a clause is literally false. Every
    /// property holds on an empty set of states, so walking an impossible
    /// environment must not report anything.
    impossible: bool,
}

impl Env {
    pub fn new() -> Env {
        Env::default()
    }

    /// An environment no execution reaches.
    pub fn impossible() -> Env {
        Env {
            impossible: true,
            ..Env::default()
        }
    }

    pub fn is_impossible(&self) -> bool {
        self.impossible
    }

    /// Record that this path cannot happen.
    pub fn mark_impossible(&mut self) {
        self.impossible = true;
    }

    /// The binding's value, or "no information" when it was never seen.
    pub fn get(&self, name: &str) -> Value {
        self.values.get(name).copied().unwrap_or_else(Value::top)
    }

    pub fn set(&mut self, name: impl Into<String>, value: Value) {
        self.values.insert(name.into(), value);
    }

    /// Forget everything about a binding, keeping it present as unknown.
    pub fn forget(&mut self, name: &str) {
        self.values.insert(name.to_string(), Value::top());
    }

    pub fn length(&self, name: &str) -> Option<i64> {
        self.lengths.get(name).copied()
    }

    pub fn set_length(&mut self, name: impl Into<String>, length: i64) {
        self.lengths.insert(name.into(), length);
    }

    /// Drop the known length of one binding, keeping the others.
    pub fn forget_length(&mut self, name: &str) {
        self.lengths.remove(name);
    }

    /// Drop every known length, because a call may have changed one.
    pub fn forget_lengths(&mut self) {
        self.lengths.clear();
    }

    /// The smallest environment describing both paths.
    ///
    /// An impossible path contributes nothing, which is what makes a branch
    /// whose condition is already decided harmless.
    pub fn join(&self, other: &Env) -> Env {
        if self.impossible {
            return other.clone();
        }
        if other.impossible {
            return self.clone();
        }
        let mut values = BTreeMap::new();
        for name in self.values.keys().chain(other.values.keys()) {
            values.insert(name.clone(), self.get(name).join(other.get(name)));
        }
        let mut lengths = BTreeMap::new();
        for (name, length) in &self.lengths {
            if other.lengths.get(name) == Some(length) {
                lengths.insert(name.clone(), *length);
            }
        }
        Env {
            values,
            lengths,
            impossible: false,
        }
    }

    /// The join, with every binding the loop changed widened to "unknown" so
    /// the fixpoint terminates.
    pub fn widen(&self, next: &Env) -> Env {
        let mut values = BTreeMap::new();
        for name in self.values.keys().chain(next.values.keys()) {
            let value = if self.get(name) == next.get(name) {
                next.get(name)
            } else {
                Value::top()
            };
            values.insert(name.clone(), value);
        }
        let mut lengths = BTreeMap::new();
        for (name, length) in &next.lengths {
            if self.lengths.get(name) == Some(length) {
                lengths.insert(name.clone(), *length);
            }
        }
        Env {
            values,
            lengths,
            impossible: next.impossible,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interval(lo: i64, hi: i64) -> Interval {
        Interval::new(lo as i128, hi as i128)
    }

    #[test]
    fn an_interval_knows_its_one_value_and_its_size() {
        assert_eq!(interval(3, 3).exact_value(), Some(3));
        assert_eq!(interval(3, 4).exact_value(), None);
        assert!(interval(3, 4).contains(4));
        assert!(!interval(3, 4).contains(5));
    }

    #[test]
    fn arithmetic_stays_inside_the_interval() {
        assert_eq!(interval(1, 2).add(interval(10, 20)), interval(11, 22));
        assert_eq!(interval(1, 2).sub(interval(10, 20)), interval(-19, -8));
        assert_eq!(interval(-2, 3).mul(interval(-4, 5)), interval(-12, 15));
        assert_eq!(interval(1, 3).neg(), interval(-3, -1));
    }

    #[test]
    fn arithmetic_that_leaves_the_int_range_is_top_rather_than_a_wrong_bound() {
        // A result entirely outside `int` is unknown.
        assert_eq!(
            interval(i64::MAX, i64::MAX).add(interval(1, 1)),
            Interval::TOP
        );
        assert_eq!(
            interval(i64::MIN, i64::MIN).sub(interval(1, 1)),
            Interval::TOP
        );
        assert_eq!(
            interval(i64::MAX, i64::MAX).mul(interval(2, 2)),
            Interval::TOP
        );
        // A result that crosses one edge is clamped: the executions that do not
        // overflow produce exactly the values left inside the range.
        assert_eq!(
            Interval::TOP.add(Interval::exact(1)),
            interval(i64::MIN + 1, i64::MAX)
        );
    }

    #[test]
    fn a_join_is_the_hull_and_a_meet_is_the_overlap() {
        assert_eq!(interval(0, 2).join(interval(5, 9)), interval(0, 9));
        assert_eq!(interval(0, 5).meet(interval(3, 9)), interval(3, 5));
        assert!(interval(0, 1).meet(interval(2, 3)).is_empty());
    }

    #[test]
    fn a_congruence_join_keeps_the_strongest_period_both_share() {
        // {0, 2} is every even number, so the class survives.
        let joined = Congruence::exact(0).join(Congruence::exact(2));
        assert_eq!(joined, Congruence::new(2, 0));
        assert!(joined.contains(4));
        assert!(!joined.contains(3));
        // An exact value in an unrelated class tells us nothing.
        assert_eq!(
            Congruence::exact(3).join(Congruence::new(2, 0)),
            Congruence::ANY
        );
        // An exact value already in the class adds nothing.
        assert_eq!(
            Congruence::exact(4).join(Congruence::new(2, 0)),
            Congruence::new(2, 0)
        );
    }

    #[test]
    fn multiplying_by_two_makes_a_value_even() {
        let doubled = Congruence::ANY.mul_const(2);
        assert_eq!(doubled, Congruence::new(2, 0));
        assert!(doubled.contains(6));
        assert!(!doubled.contains(7));
        assert_eq!(Congruence::exact(3).mul_const(2), Congruence::exact(6));
    }

    #[test]
    fn two_classes_that_share_no_value_are_disjoint() {
        let even = Congruence::new(2, 0);
        assert!(even.disjoint(Congruence::new(2, 1)));
        assert!(!even.disjoint(Congruence::new(4, 0)));
        assert!(Congruence::exact(1).disjoint(Congruence::exact(2)));
        assert!(!Congruence::exact(1).disjoint(Congruence::new(2, 1)));
    }

    #[test]
    fn equality_is_proved_from_disjoint_intervals_and_classes() {
        assert_eq!(
            compare(BinOp::Eq, Value::exact(1), Value::exact(2)),
            Tri::False
        );
        assert_eq!(
            compare(BinOp::Eq, Value::exact(1), Value::exact(1)),
            Tri::True
        );
        let even = Value {
            interval: Interval::TOP,
            congruence: Congruence::new(2, 0),
        };
        assert_eq!(compare(BinOp::Eq, even, Value::exact(3)), Tri::False);
        assert_eq!(compare(BinOp::Eq, even, Value::exact(4)), Tri::Unknown);
    }

    #[test]
    fn ordering_is_proved_only_when_the_intervals_do_not_touch() {
        fn value(lo: i64, hi: i64) -> Value {
            Value::range(lo as i128, hi as i128)
        }
        assert_eq!(compare(BinOp::Lt, value(1, 2), value(3, 9)), Tri::True);
        assert_eq!(compare(BinOp::Lt, value(3, 9), value(1, 2)), Tri::False);
        assert_eq!(compare(BinOp::Le, value(1, 2), value(2, 9)), Tri::True);
        assert_eq!(compare(BinOp::Lt, value(1, 5), value(3, 9)), Tri::Unknown);
        // `a > b` is `b < a`.
        assert_eq!(compare(BinOp::Gt, value(3, 9), value(1, 2)), Tri::True);
    }

    #[test]
    fn a_meet_with_a_single_value_narrows_both_halves() {
        let wide = Value::range(0, 10);
        let three = wide.meet(Value::exact(3));
        assert_eq!(three.exact_value(), Some(3));
        assert!(wide.meet(Value::exact(11)).is_empty());
        // A value outside the congruence's class is no value at all.
        let even = Value {
            interval: Interval::TOP,
            congruence: Congruence::new(2, 0),
        };
        assert!(even.meet(Value::exact(3)).is_empty());
    }

    #[test]
    fn an_environment_join_keeps_only_what_both_paths_agree_on() {
        let mut a = Env::new();
        a.set("x", Value::exact(1));
        a.set_length("xs", 2);
        let mut b = Env::new();
        b.set("x", Value::exact(2));
        b.set_length("xs", 3);
        let joined = a.join(&b);
        assert_eq!(joined.get("x").interval, interval(1, 2));
        assert_eq!(joined.length("xs"), None);
        // A name only one path has is unknown, not absent.
        assert_eq!(joined.get("y"), Value::top());
    }

    #[test]
    fn widening_drops_every_binding_the_iteration_changed() {
        let mut before = Env::new();
        before.set("i", Value::exact(0));
        before.set("n", Value::exact(5));
        let mut after = before.clone();
        after.set("i", Value::range(0, 1));
        let widened = before.widen(&after);
        assert_eq!(widened.get("i"), Value::top());
        assert_eq!(widened.get("n"), Value::exact(5));
    }

    #[test]
    fn an_impossible_path_contributes_nothing_to_a_join() {
        let mut reachable = Env::new();
        reachable.set("x", Value::exact(1));
        let joined = Env::impossible().join(&reachable);
        assert!(!joined.is_impossible());
        assert_eq!(joined.get("x"), Value::exact(1));
        // Two impossible paths are still impossible.
        assert!(Env::impossible().join(&Env::impossible()).is_impossible());
    }
}
