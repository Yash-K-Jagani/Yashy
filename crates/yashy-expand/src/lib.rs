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
//! identifier the caller supplies keeps the caller's scope. That is carried by
//! [`hygiene::ScopeSet`] on every identifier token, rather than by renaming,
//! so the printed form stays the one the user wrote.

pub mod hygiene;
pub mod tt;

pub use tt::{Delim, TokenTree, TreeError, MAX_TREE_DEPTH};
