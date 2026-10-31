//! Facts the verifier proved, so code generation may delete the checks that
//! enforce them.
//!
//! A fact is keyed by the span of the expression it is about, which is the span
//! the check was attached to: the index for a bounds check, the shift for a
//! shift-count check. The two are kept in separate sets because they answer
//! different questions and a consumer must ask the right one -- a proven index
//! says nothing about a shift count that happens to share a span.

use std::collections::HashSet;

use crate::Span;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProofFacts {
    proven: HashSet<(u32, u32)>,
    shift_counts: HashSet<(u32, u32)>,
}

impl ProofFacts {
    pub fn empty() -> Self {
        Self {
            proven: HashSet::new(),
            shift_counts: HashSet::new(),
        }
    }

    /// True when the bounds or overflow check at `span` may be deleted.
    pub fn is_proven(&self, span: Span) -> bool {
        self.proven.contains(&(span.start, span.end))
    }

    /// True when the shift count at `span` is provably inside `0..bits`, so the
    /// range check may be deleted.
    pub fn is_shift_proven(&self, span: Span) -> bool {
        self.shift_counts.contains(&(span.start, span.end))
    }

    pub fn insert(&mut self, span: Span) {
        self.proven.insert((span.start, span.end));
    }

    pub fn insert_shift(&mut self, span: Span) {
        self.shift_counts.insert((span.start, span.end));
    }
}
