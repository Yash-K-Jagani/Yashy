use std::collections::HashSet;

use crate::Span;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProofFacts {
    proven: HashSet<(u32, u32)>,
}

impl ProofFacts {
    pub fn empty() -> Self {
        Self { proven: HashSet::new() }
    }

    pub fn is_proven(&self, span: Span) -> bool {
        self.proven.contains(&(span.start, span.end))
    }

    pub fn insert(&mut self, span: Span) {
        self.proven.insert((span.start, span.end));
    }
}
