//! The Otter lexer: source text in, tokens with byte spans out.
//!
//! This crate is the first stage of the compiler and the first one that looks
//! at anything the user wrote, so its contract is narrower than "correct": no
//! input, however malformed, may make it panic. Every way of being wrong --
//! an unknown character, an unterminated string, an unterminated comment, an
//! escape Otter does not have -- is reported as an
//! [`otter_common::Diagnostic`] instead.
//!
//! # Spans
//!
//! Every token carries the half-open byte range of its own text, which is the
//! foundation of every diagnostic the rest of the compiler will ever print: a
//! span that is off by one puts the caret under the wrong character while
//! looking entirely plausible. The golden tests in `tests/` pin those spans
//! over whole files, which is what catches an off-by-one that only appears
//! after skipped comments and whitespace.
//!
//! # Values live in the source, not in the token
//!
//! A token is a [`TokenKind`] and a [`otter_common::Span`], nothing else. The
//! number, the string and the identifier are read back out of the source text
//! by the parser, which keeps a token `Copy` and makes it impossible for a
//! value and the text it came from to disagree. [`StringLit`] is what that
//! reading produces for a string, and is re-exported for the AST.
//!
//! # Example
//!
//! ```
//! use otter_lexer::{Lexer, TokenKind};
//!
//! let tokens = Lexer::new("fn main() {}").tokenize().expect("valid source");
//! assert_eq!(tokens[0].kind, TokenKind::Fn);
//! assert_eq!(tokens[0].span.start, 0);
//! assert_eq!(tokens[0].span.end, 2);
//! // The last token is `Eof`, spanning nothing at the end of the file: that is
//! // what a parser needs in order to report an unclosed delimiter.
//! let eof = tokens.last().expect("the stream always ends with Eof");
//! assert_eq!(eof.kind, TokenKind::Eof);
//! assert_eq!(eof.span.start, 12);
//! assert_eq!(eof.span.end, 12);
//! ```

pub mod lexer;
pub mod token;

/// How the lexer words its own diagnostics.
///
/// Not part of the public API: a downstream crate that can call
/// [`error::unexpected_character`](error) can mint a lexer diagnostic with any
/// message it likes, and the wording then depends on which crate happened to
/// build it. The type it produces, [`LitError`], is public because
/// [`StringLit::decode`] returns one.
pub(crate) mod error;

pub use error::LitError;
pub use lexer::Lexer;
pub use token::{literal_as_i64, StringLit, Token, TokenKind};
