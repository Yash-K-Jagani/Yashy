//! Facts the verifier proved, so code generation may delete the checks that
//! enforce them.
//!
//! A fact is keyed by the span of the expression it is about, which is the span
//! the check was attached to: the index for a bounds check, the shift for a
//! shift-count check. The two are kept in separate sets because they answer
//! different questions and a consumer must ask the right one -- a proven index
//! says nothing about a shift count that happens to share a span.
//!
//! The second family of facts is about a function's parameters: whether a
//! pointer is the only live reference to its allocation (`noalias`), whether it
//! may be null, whether the body writes through it, and what it is aligned to.
//! Those are keyed by the verifier's function identity -- the file, the name and
//! the start of the body -- because the HIR has no separate function id. An
//! absent entry means "no attribute": a fact that was not proved must never
//! become a promise.

use std::collections::{BTreeMap, HashSet};

use crate::{FileId, Span};

/// The parameter attributes the verifier proved for one parameter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParamAttrs {
    /// The parameter is the only live reference to its allocation for the
    /// duration of the call.
    pub noalias: bool,
    /// The parameter's type has no null value.
    pub nonnull: bool,
    /// The body never writes through the parameter.
    pub readonly: bool,
    /// The alignment of the pointee, when the layout is known.
    pub align: Option<u32>,
}

impl ParamAttrs {
    /// True when there is nothing to emit.
    pub fn is_empty(&self) -> bool {
        !self.noalias && !self.nonnull && !self.readonly && self.align.is_none()
    }
}

/// One function's parameter attributes, in parameter order.
///
/// A parameter the verifier said nothing about keeps a default entry, so an
/// index always names the same parameter. A `get` that does not find the index
/// (a caller with a different parameter list) is `None`, and the consumer emits
/// no attribute rather than guessing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParamFacts {
    attrs: Vec<ParamAttrs>,
}

impl ParamFacts {
    pub fn new(attrs: Vec<ParamAttrs>) -> Self {
        Self { attrs }
    }

    /// The attributes of parameter `index`, when the list is long enough.
    pub fn get(&self, index: usize) -> Option<&ParamAttrs> {
        self.attrs.get(index)
    }

    pub fn len(&self) -> usize {
        self.attrs.len()
    }

    /// True when no parameter carries an attribute.
    pub fn is_empty(&self) -> bool {
        self.attrs.iter().all(ParamAttrs::is_empty)
    }

    pub fn iter(&self) -> impl Iterator<Item = &ParamAttrs> {
        self.attrs.iter()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProofFacts {
    proven: HashSet<(u32, u32)>,
    shift_counts: HashSet<(u32, u32)>,
    /// Parameter facts by `(file index, function name, body start)`.
    ///
    /// A `BTreeMap`, not a `HashMap`, so the table has one iteration order; a
    /// fact may be printed or compared by a consumer without depending on hash
    /// seeding.
    params: BTreeMap<(u32, String, u32), ParamFacts>,
}

impl ProofFacts {
    pub fn empty() -> Self {
        Self {
            proven: HashSet::new(),
            shift_counts: HashSet::new(),
            params: BTreeMap::new(),
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

    /// Record the parameter attributes of one function.
    ///
    /// The identity is the one the verifier already uses to walk a program
    /// once per body: the file, the function's name and the start of its body.
    /// A lambda has an empty name, so its body start is what keeps two lambdas
    /// apart.
    pub fn insert_params(&mut self, file: FileId, name: &str, body_start: u32, facts: ParamFacts) {
        self.params
            .insert((file.index(), name.to_string(), body_start), facts);
    }

    /// The parameter facts of one function, if the verifier has any.
    pub fn params(&self, file: FileId, name: &str, body_start: u32) -> Option<&ParamFacts> {
        self.params
            .get(&(file.index(), name.to_string(), body_start))
    }

    /// Every recorded function's parameter facts, in a stable order.
    pub fn param_entries(&self) -> impl Iterator<Item = (FileId, &str, u32, &ParamFacts)> + '_ {
        self.params.iter().map(|((file, name, body_start), facts)| {
            (FileId::from_index(*file), name.as_str(), *body_start, facts)
        })
    }

    /// Every proven bounds/overflow span.
    pub fn proven_spans(&self) -> impl Iterator<Item = Span> + '_ {
        self.proven
            .iter()
            .map(|&(start, end)| Span::new(start, end))
    }

    /// Every proven shift-count span.
    pub fn shift_spans(&self) -> impl Iterator<Item = Span> + '_ {
        self.shift_counts
            .iter()
            .map(|&(start, end)| Span::new(start, end))
    }
}
