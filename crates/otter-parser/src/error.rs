//! The diagnostics the parser reports, and the one type every parse failure is.
//!
//! A [`ParseError`] is two fields and no more on purpose: a span to point at
//! and a message to print. A parser that builds half a tree and then has to
//! describe what went wrong is carrying state that no diagnostic can use, and
//! the fields a later stage would want to add -- a label, a suggestion, a note
//! listing the alternatives -- all of them fit in the message or do not fit at
//! all.
//!
//! Every message is built here rather than at the call site, for the reason the
//! lexer does the same thing: the wording is the part that matters to a reader,
//! and it should read the same whether the caller remembered to be consistent.
//! [`expected`] is the one everybody uses, and it is the reason
//! [`otter_lexer::TokenKind::describe`] exists: `expected `(`, found an
//! identifier` names both halves of the mistake.
//!
//! Messages are lowercase and carry no trailing period, because
//! [`otter_common::SourceMap::render`] supplies the `error: ` prefix and the
//! newline.
//!
//! Nothing in this module prints. The parser describes; the driver decides
//! where a description is shown and with what exit code.

use otter_common::{Diagnostic, FileId, Severity, Span};
use otter_lexer::TokenKind;

/// One thing the parser could not do, and where in the source it gave up.
///
/// Deliberately not a [`Diagnostic`]: a [`ParseError`] has no file, because the
/// parser knows its tokens came from one file but the *diagnostic* is minted
/// later by [`ParseError::to_diagnostic`], which is handed the [`FileId`]. One
/// type per layer, rather than a `ParseError` that is a `Diagnostic` with a
/// file field that is wrong until somebody fixes it up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// The source text this is about.
    ///
    /// For [`expected`] that is the offending token, which for `Eof` is the
    /// empty span at the end of the file -- which is what lets "expected `;`"
    /// point at the line where the statement ran out. For [`unclosed`] it is
    /// the *opening* delimiter, because that is the character the reader has to
    /// go and add.
    pub span: Span,
    /// What is wrong, in lowercase and without a trailing period.
    pub message: String,
}

impl ParseError {
    /// A parse failure at `span` saying `message`.
    pub fn new(span: Span, message: impl Into<String>) -> ParseError {
        ParseError {
            span,
            message: message.into(),
        }
    }

    /// The [`Diagnostic`] this failure becomes in `file`.
    ///
    /// Every diagnostic the parser produces is an error: the parser either
    /// understood the source or it did not, and a warning about a source that
    /// did not parse would be a warning about a file nobody will compile.
    pub fn to_diagnostic(&self, file: FileId) -> Diagnostic {
        Diagnostic::new(Severity::Error, file, self.span, self.message.clone())
    }
}

/// The token the parser wanted, and the one it found.
///
/// Both halves are named, always. "Unexpected token" leaves the reader to work
/// out which of the two it was that was wrong, and the answer is usually the
/// other one.
pub fn expected(expected: TokenKind, found: TokenKind, span: Span) -> ParseError {
    ParseError::new(
        span,
        format!(
            "expected {}, found {}",
            expected.describe(),
            found.describe()
        ),
    )
}

/// A delimiter that was opened and never closed, at the end of the file.
///
/// The span is the **opening** delimiter, not the end of the file: the
/// character to go and add is the one that was written, and a caret at `Eof`
/// tells the reader only that something, somewhere earlier, went wrong. The
/// message names the end of the file as the reason, so the reader knows to
/// look for a brace rather than for a stray token.
pub fn unclosed(open: Span, kind: TokenKind) -> ParseError {
    ParseError::new(
        open,
        format!(
            "unclosed {}: the file ended before the matching {}",
            kind.describe(),
            closer_of(kind).describe()
        ),
    )
}

/// `import` with no path after it, or with one that stops early.
///
/// The generic "expected an identifier, found `;`" is true and useless: the
/// reader has to know that the thing missing is a *path*, and that
/// `import std.io;` is what one looks like.
pub fn import_needs_a_path(span: Span) -> ParseError {
    ParseError::new(span, "this `import` has no path: write `import std.io;`")
}

/// A `struct` with nothing in it.
///
/// Named, because the fix is to add a field to *this* struct and a reader who
/// is told only "a struct needs a field" has to go and find which struct.
pub fn struct_needs_a_field(span: Span, name: &str) -> ParseError {
    ParseError::new(
        span,
        format!("struct `{name}` must have at least one named field"),
    )
}

/// An `enum` with no variants at all.
pub fn enum_needs_a_variant(span: Span, name: &str) -> ParseError {
    ParseError::new(
        span,
        format!("enum `{name}` must have at least one variant"),
    )
}

/// An `extern` block, which is a real keyword in M0-M2 and not yet a feature.
///
/// The variant exists in the AST so that M5 adds a case rather than changing a
/// contract four stages code against. Until then the honest answer to an
/// `extern` block is that the parser has not been taught to read one -- and
/// saying *when* it will be is the difference between a user filing a bug and
/// a user moving on to the next item in their file.
pub fn extern_arrives_in_m5(span: Span) -> ParseError {
    ParseError::new(span, "`extern` blocks arrive in M5")
}

/// A token that cannot begin an item.
pub fn item_expected(found: TokenKind, span: Span) -> ParseError {
    ParseError::new(
        span,
        format!(
            "expected an item: `import`, `struct`, `enum`, `fn` or `extern`, found {}",
            found.describe()
        ),
    )
}

/// Nesting past [`crate::parser::MAX_NESTING_DEPTH`] delimiters.
pub fn nested_too_deep(span: Span) -> ParseError {
    ParseError::new(
        span,
        format!(
            "this source is nested more than {} levels deep, which is more than the parser will read",
            crate::parser::MAX_NESTING_DEPTH
        ),
    )
}

/// A span that could not be read back out of the source text.
///
/// Only reachable if a token's span does not index the file the parser was
/// given -- which should be impossible, since the lexer produced both. It is
/// reported rather than unwrapped because "the compiler panicked on a file it
/// had just read" is the one failure mode this crate exists to prevent, and
/// because the alternative to a diagnostic here is a name invented out of
/// nothing.
pub fn unreadable_text(span: Span) -> ParseError {
    ParseError::new(span, "this could not be read as text from the source file")
}

/// The delimiter that closes `kind`, for the unclosed-delimiter message.
///
/// `TokenKind::spelling` answers the same question for the tokens that have a
/// spelling; this exists so [`unclosed`] does not match on closers and drift out
/// of step with [`Parser::close_open`](crate::Parser).
pub fn closer_of(kind: TokenKind) -> TokenKind {
    match kind {
        TokenKind::LBrace => TokenKind::RBrace,
        TokenKind::LParen => TokenKind::RParen,
        TokenKind::LBracket => TokenKind::RBracket,
        // The three openers are the only kinds this is called with; anything
        // else describes itself, which keeps the message readable rather than
        // claiming a closer that does not exist.
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file() -> FileId {
        FileId::from_index(0)
    }

    #[test]
    fn expected_names_both_halves_of_the_mistake() {
        let e = expected(TokenKind::Semi, TokenKind::Ident, Span::new(3, 4));
        assert_eq!(e.span, Span::new(3, 4));
        assert_eq!(e.message, "expected `;`, found an identifier");
    }

    #[test]
    fn an_unclosed_delimiter_points_at_the_opener_and_names_the_end_of_the_file() {
        let open = Span::new(6, 7);
        for (kind, expected_message) in [
            (
                TokenKind::LBrace,
                "unclosed `{`: the file ended before the matching `}`",
            ),
            (
                TokenKind::LParen,
                "unclosed `(`: the file ended before the matching `)`",
            ),
            (
                TokenKind::LBracket,
                "unclosed `[`: the file ended before the matching `]`",
            ),
        ] {
            let e = unclosed(open, kind);
            assert_eq!(e.span, open, "{kind:?} must be reported at the opener");
            assert_eq!(e.message, expected_message);
            assert!(e.message.contains("unclosed"));
        }
    }

    #[test]
    fn every_message_is_an_error_that_renders() {
        let span = Span::new(1, 2);
        let diags = [
            expected(TokenKind::Semi, TokenKind::Eof, span),
            unclosed(span, TokenKind::LBrace),
            import_needs_a_path(span),
            struct_needs_a_field(span, "S"),
            enum_needs_a_variant(span, "E"),
            extern_arrives_in_m5(span),
            item_expected(TokenKind::At, span),
            nested_too_deep(span),
            unreadable_text(span),
        ];
        for e in &diags {
            let d = e.to_diagnostic(file());
            assert!(d.is_error(), "{}", e.message);
            assert_eq!(d.file, file());
            assert_eq!(d.span, span);
            assert!(!e.message.is_empty());
            assert!(
                !e.message.ends_with('.'),
                "the renderer supplies the punctuation: {}",
                e.message
            );
        }
    }

    #[test]
    fn the_messages_that_name_something_name_the_right_something() {
        assert!(struct_needs_a_field(Span::SYNTHETIC, "Point")
            .message
            .contains("Point"));
        assert!(enum_needs_a_variant(Span::SYNTHETIC, "Colour")
            .message
            .contains("Colour"));
        assert!(struct_needs_a_field(Span::SYNTHETIC, "S")
            .message
            .contains("field"));
        assert!(enum_needs_a_variant(Span::SYNTHETIC, "E")
            .message
            .contains("variant"));
        assert!(import_needs_a_path(Span::SYNTHETIC)
            .message
            .contains("import std.io;"));
        assert_eq!(
            extern_arrives_in_m5(Span::SYNTHETIC).message,
            "`extern` blocks arrive in M5"
        );
        assert!(item_expected(TokenKind::At, Span::SYNTHETIC)
            .message
            .contains("@"));
    }

    #[test]
    fn a_closer_matches_its_opener_and_nothing_else() {
        assert_eq!(closer_of(TokenKind::LBrace), TokenKind::RBrace);
        assert_eq!(closer_of(TokenKind::LParen), TokenKind::RParen);
        assert_eq!(closer_of(TokenKind::LBracket), TokenKind::RBracket);
        // The three closers are never openers, so asking is the answer rather
        // than a guess at a default.
        for kind in [
            TokenKind::RBrace,
            TokenKind::RParen,
            TokenKind::RBracket,
            TokenKind::Ident,
        ] {
            assert_eq!(closer_of(kind), kind);
        }
    }
}
