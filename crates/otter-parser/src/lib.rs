//! The Otter parser: source text in, an [`ast::Program`] out.
//!
//! The tree it produces is defined in [`ast`] and re-exported here, so a later
//! stage can name `otter_parser::Program` or `otter_parser::ast::Program` and
//! mean the same thing. [`StringLit`] comes along with it, because a string
//! literal in a tree is a lexer value and there is only one way to spell one.
//!
//! # No input makes this crate panic
//!
//! The parser is the component most likely to meet hostile input -- a user
//! pressing keys, or a fuzzer in Task 14 -- so its contract is narrower than
//! "correct". Every cursor operation is total: [`Parser::peek`] at the end of
//! the stream reports `Eof` forever rather than indexing past it, and
//! [`Parser::advance`] past the end returns `Eof` without moving. There is no
//! `unwrap`, no `expect`, no `panic!` and no unchecked indexing anywhere in
//! `src`; every mistake becomes an [`error::ParseError`], and every
//! `ParseError` becomes an [`otter_common::Diagnostic`].
//!
//! Recursion is bounded too, in the only way that matters before Task 14:
//! item parsing does not recurse at all, and nesting is counted against
//! [`parser::MAX_NESTING_DEPTH`] rather than left to the machine stack.
//!
//! # The entry points Tasks 5 and 6 fill in
//!
//! Rust has no way to call a function that does not exist yet, so a task that
//! needs one has to leave a seam. This crate has exactly one, and it is here to
//! be looked at:
//!
//! | Entry point | Defined in | State |
//! | --- | --- | --- |
//! | [`Parser::parse_program`] | [`parser`] | **complete** -- items, with recovery |
//! | [`Parser::parse_block`] | [`parser`] | **seam** -- consumes a balanced body, reads no statements yet |
//! | [`Parser::parse_stmt`] | Task 5 (`stmts.rs`) | not yet written |
//! | [`Parser::parse_expr`] | Task 6 (`exprs.rs`) | not yet written |
//! | [`Parser::parse_type_expr`] | [`types`] | **complete** -- Task 4 needs it for fields, params and return types |
//!
//! Two decisions in that table are worth stating outright, because they are the
//! ones a later task has to live with.
//!
//! **Only `parse_block` is stubbed.** `parse_stmt` and `parse_expr` are *not*
//! defined here, because a stub for them would either panic -- which this
//! crate's contract forbids -- or return an error nobody can reach, since
//! nothing in item parsing calls them. Declaring them anyway would be an API
//! that lies about what it does. Task 5 adds `parse_stmt` next to `parse_block`
//! and Task 6 adds `parse_expr`; neither has to be told that `parse_block`
//! exists.
//!
//! **`parse_type_expr` is complete here rather than stubbed**, even though the
//! plan lists it under Task 6. A struct field is `name : type` and a parameter
//! is `name : type`: without types, Task 4 cannot parse a struct or a function
//! at all, and the alternative -- inventing a type and letting a later stage
//! trust it -- is the one thing the untyped AST exists to prevent. Task 6
//! inherits this implementation; it does not write a second one.

pub mod ast;
pub mod error;
pub mod items;
pub mod parser;
pub mod types;

pub use ast::*;
pub use error::ParseError;
pub use parser::{Parser, MAX_NESTING_DEPTH};
