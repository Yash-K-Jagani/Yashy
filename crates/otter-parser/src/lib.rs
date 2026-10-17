//! The Otter parser: source text in, an [`ast::Program`] out.
//!
//! The tree it produces is defined in [`ast`] and re-exported here, so a later
//! stage can name `otter_parser::Program` or `otter_parser::ast::Program` and
//! mean the same thing. [`StringLit`] comes along with it, because a string
//! literal in a tree is a lexer value and there is only one way to spell one.

pub mod ast;

pub use ast::*;
