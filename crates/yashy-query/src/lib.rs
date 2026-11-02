//! The memoised query database.
//!
//! Everything the compiler learns about a program is keyed by a 64-bit FNV-1a
//! content hash, not by time: the driver asks to parse, check, verify or emit,
//! and the database answers from the memo unless an input it read changed.
//!
//! The red-green invariant is hand-written on a single-threaded database: the
//! first execution records every read, and any changed read re-runs the query.
//! `&mut self` is the only interior mutability; task 2 shards per file and
//! therefore per instance.
//!
//! See `docs/superpowers/plans/2026-10-27-phase-04-optimization.md` Task 1.

#![forbid(unsafe_code)]

mod db;
mod query;

pub use db::{Db, QueryStats};
pub use query::{fnv1a, hash_str};

use std::sync::Arc;

/// A function's stable identity: file plus name. The name is enough inside a
/// file, but the pair survives a cross-file merge, which is what the driver
/// needs to route the cached IR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FnId {
    pub file: yashy_common::FileId,
    pub name: String,
}

impl std::hash::Hash for FnId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // `FileId` is a bare `u32` wrapper without `Hash`; the index is the
        // identity inside the map that handed it out.
        self.file.index().hash(state);
        self.name.hash(state);
    }
}

impl FnId {
    pub fn new(file: yashy_common::FileId, name: impl Into<String>) -> Self {
        Self {
            file,
            name: name.into(),
        }
    }
}

/// Checked results for one file.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckedFile {
    pub file: yashy_common::FileId,
    pub functions: Vec<FnId>,
    pub program: Arc<yashy_hir::hir::HirProgram>,
}

/// The verified facts for one function, with function-relative spans so the
/// cache survives a sibling edit that shifts the file's spans.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FnFacts {
    pub id: FnId,
    pub proven: Vec<(u32, u32)>,
    pub shift_proven: Vec<(u32, u32)>,
}

/// The query's result type: zero diagnostics on hit, or a list of diagnostics
/// on miss/failure. The verifier diagnostic also carries a `proof` object
/// naming the obligation, the missing fact and the remedy; that shape comes
/// from `yashy_common::Diagnostic` and is preserved by every layer.
pub type QueryResult<T> = Result<T, Vec<yashy_common::Diagnostic>>;
