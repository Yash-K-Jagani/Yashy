//! Scope sets: the identity an identifier resolves by.
//!
//! Hygiene has two halves, and a rename alone gets only one of them. A
//! template's `let tmp` must not capture the caller's `tmp`, and a caller's
//! `x` passed as `$x` must keep resolving to the caller's `x`. The first is
//! what renaming buys; the second needs the substituted token to keep the
//! scope it was written in.
//!
//! A scope set is that scope: a bitmask of the contexts an identifier was
//! introduced in. A use resolves against a binding only when the binding's set
//! is a subset of the use's, so an identifier keeps resolving to what it meant
//! where it was written, and never to something the template or the caller
//! introduced from another scope. The set is a single `u64` because the
//! introduction sites a macro body can create are few; the checker mirrors the
//! same rule so resolution stays single-sourced.
//!
//! [`fresh_mark`] is a counter on the expander, not a hash, so expansion is
//! deterministic byte for byte: the same source gives the same marks in the
//! same order on both operating systems.

/// A single scope set: the contexts an identifier carries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ScopeSet(pub u64);

impl ScopeSet {
    /// The empty set: not introduced by any macro.
    pub const EMPTY: ScopeSet = ScopeSet(0);

    /// A set with exactly `mark` in it.
    pub fn single(mark: u64) -> ScopeSet {
        ScopeSet(1u64 << (mark % 64))
    }

    pub fn union(self, other: ScopeSet) -> ScopeSet {
        ScopeSet(self.0 | other.0)
    }
}

impl std::ops::BitOr for ScopeSet {
    type Output = ScopeSet;
    fn bitor(self, rhs: ScopeSet) -> ScopeSet {
        ScopeSet(self.0 | rhs.0)
    }
}

impl std::ops::BitAnd for ScopeSet {
    type Output = ScopeSet;
    fn bitand(self, rhs: ScopeSet) -> ScopeSet {
        ScopeSet(self.0 & rhs.0)
    }
}

impl std::ops::Not for ScopeSet {
    type Output = ScopeSet;
    fn not(self) -> ScopeSet {
        ScopeSet(!self.0)
    }
}

/// The printed form a hygiene mark gives a template identifier.
///
/// The text stays `name` — it is what a diagnostic shows and what a span covers
/// — and resolution reads the mark, not the text, so two `tmp`s from two
/// expansions of one macro are two bindings that the caller never sees.
pub fn fresh_ident(name: &str, mark: u64) -> String {
    format!("{name}#{mark}")
}

/// A counter that hands out one fresh mark per expansion.
///
/// Kept as data on the expander so the order is explicit and reproducible.
#[derive(Clone, Debug, Default)]
pub struct MarkCounter(u64);

impl MarkCounter {
    pub fn new() -> MarkCounter {
        MarkCounter(0)
    }

    /// The next mark. Marks are dense from 0 so the scope-set bitmask stays
    /// meaningful.
    pub fn next(&mut self) -> u64 {
        let mark = self.0;
        self.0 = self.0.wrapping_add(1);
        mark
    }
}

/// True when `binding` may be used by `use_site`: every context the binding
/// was introduced in is also a context the use carries.
pub fn resolves(binding: ScopeSet, use_site: ScopeSet) -> bool {
    (binding.0 & !use_site.0) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_binding_resolves_in_its_own_set_only() {
        let binding = ScopeSet(0b101);
        assert!(resolves(binding, ScopeSet(0b111)));
        assert!(resolves(binding, binding));
        assert!(!resolves(binding, ScopeSet(0b001)));
        assert!(!resolves(binding, ScopeSet::EMPTY));
    }

    #[test]
    fn marks_are_dense_from_zero() {
        let mut counter = MarkCounter::new();
        assert_eq!(counter.next(), 0);
        assert_eq!(counter.next(), 1);
    }

    #[test]
    fn the_mark_is_part_of_the_resolved_form() {
        assert_eq!(fresh_ident("tmp", 7), "tmp#7");
    }

    #[test]
    fn a_template_local_does_not_capture_a_caller_local() {
        // `macro swap(a, b) { let tmp = $a; ... }` called where the caller
        // already has a `tmp`: the template local carries the definition
        // scope, the caller's `tmp` carries the caller's, and neither
        // resolves to the other.
        let definition = ScopeSet::single(0);
        let caller = ScopeSet::single(1);
        let template_tmp = ScopeSet::single(2) | definition;
        let caller_tmp = caller;

        assert!(resolves(caller_tmp, caller));
        assert!(!resolves(caller_tmp, template_tmp));
        assert!(!resolves(template_tmp, caller));
    }

    #[test]
    fn a_caller_expression_keeps_resolving_in_the_caller() {
        // `macro call_it(f) { $f(); }` invoked as `call_it(g)` where the caller
        // has a local `g`: the substituted identifier keeps the caller's
        // scope, so it resolves to the local, not to a top-level `g` in the
        // definition scope.
        let caller = ScopeSet::single(3);
        let definition = ScopeSet::single(4);
        let caller_g = caller;
        let definition_g = definition;

        assert!(resolves(caller_g, caller));
        assert!(!resolves(definition_g, caller));
    }

    #[test]
    fn two_expansions_of_one_macro_get_different_marks() {
        let mut counter = MarkCounter::new();
        let first = ScopeSet::single(counter.next());
        let second = ScopeSet::single(counter.next());

        assert_ne!(first, second);
        assert!(resolves(first, first));
        assert!(!resolves(first, second));
        assert!(!resolves(second, first));
    }
}
