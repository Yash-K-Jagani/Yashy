//! `yashy.toml`: a deliberately small TOML subset, and the manifest built on it.
//!
//! The workspace builds offline, so there is no `toml` crate here. The subset
//! is documented in `docs/cli.md` as a one-screen grammar: tables, bare and
//! quoted keys, basic and literal strings, integers, booleans, arrays, and
//! comments. There are no inline tables, no multi-line strings and no dates.
//!
//! Two properties matter more than coverage:
//!
//! - **It is total.** Every input returns `Result`; no input panics, and the
//!   1 MiB cap turns "a huge file" into a diagnostic instead of a slow parse.
//! - **It fails with a location.** A line and column, and `E1800`, so the
//!   reader can be pointed at the mistake.
//!
//! [`manifest`] adds the rules a plain TOML reader cannot know: which keys
//! exist, that versions are exact pins, and that a git dependency names a
//! commit.

pub mod manifest;
pub mod toml;

pub use manifest::{Dependency, Manifest, Package};
pub use toml::{ParseError, Value};
