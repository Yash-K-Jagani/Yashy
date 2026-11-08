//! Declarative macro expansion for Yashy.
//!
//! A macro is data: a name, and arms, and each arm is a pattern over
//! [`TokenTree`]s and a replacement. Expansion runs before an item's body is
//! parsed, so the checker and the verifier see ordinary syntax and need no new
//! rule. A metavariable in a replacement is replaced by the trees the pattern
//! bound to it.
//!
//! Hygiene is the one deliberate asymmetry with text substitution: an
//! identifier the template introduces resolves in the template's scope, and an
//! identifier the caller supplies keeps the caller's scope. The rule is stated
//! once, in [`hygiene`] as [`hygiene::ScopeSet`] and [`hygiene::resolves`];
//! [`hygiene::MarkCounter`] hands out dense marks so the same source produces
//! the same scopes on every run and both operating systems.

pub mod expand;
pub mod fragment;
pub mod hygiene;
pub mod r#match;
pub mod tt;

pub use expand::{expand_tokens, Expander, ExpansionMap, ExpansionRecord, MAX_EXPANSION_DEPTH};
pub use r#match::{matches, Bindings, MatchFailure};
pub use tt::{Delim, TokenTree, TreeError, MAX_TREE_DEPTH};
