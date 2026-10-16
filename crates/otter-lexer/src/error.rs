//! The diagnostics the lexer reports.
//!
//! Every message the lexer can produce is built here rather than inline in the
//! scanning code, so that the scanner reads as scanning and the wording of a
//! diagnostic lives in one place. Nothing in this module writes to a console:
//! the lexer only knows how to *describe* a problem, and the driver decides
//! where it is printed.
//!
//! Messages are lowercase and unpunctuated at the end, because
//! [`otter_common::SourceMap::render`] already supplies the `error: ` prefix
//! and the newline.

use otter_common::{Diagnostic, FileId, Severity, Span};

/// A problem with a string literal that is located *within* the literal.
///
/// [`crate::StringLit::decode`] reports these, and the lexer turns one into a
/// [`Diagnostic`] once it knows where the literal sits in the file, so that a
/// malformed escape is reported against the offending backslash rather than
/// against the whole literal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LitError {
    /// Byte offset of the problem, relative to the start of the text passed to
    /// [`crate::StringLit::decode`].
    pub offset: usize,
    /// What is wrong, in lowercase and without a trailing period.
    pub message: String,
}

impl LitError {
    pub(crate) fn new(offset: usize, message: impl Into<String>) -> LitError {
        LitError { offset, message: message.into() }
    }
}

/// The diagnostics the lexer can report.
///
/// The construction of the message -- backticks around a character, the
/// spelling of a control byte -- happens once here instead of at every call
/// site, so `unexpected character` reads the same whether it is reporting `#`
/// or a NUL byte.
pub fn unexpected_character(file: FileId, span: Span, found: Option<char>) -> Diagnostic {
    let shown = match found {
        Some(ch) => format!("`{}`", escape_char(ch)),
        // The lexer only ever reaches past a character it has just read, so
        // this is unreachable in practice; describing the byte rather than
        // saying nothing beats reporting nothing at all if that ever changes.
        None => "that could not be decoded".to_string(),
    };
    error(
        file,
        span,
        format!("unexpected character {shown}"),
    )
}

/// An escape sequence Otter does not have, such as `\q`.
pub fn invalid_escape(file: FileId, span: Span, detail: &str) -> Diagnostic {
    error(file, span, format!("invalid escape sequence in string literal: {detail}"))
}

/// A string literal with no closing quote. `reason` says where the literal ran
/// out, which is the difference between "you forgot a quote" and "you forgot a
/// quote and it ran over the end of the file".
pub fn unterminated_string(file: FileId, span: Span, reason: &str) -> Diagnostic {
    error(
        file,
        span,
        format!("unterminated string literal: {reason}"),
    )
}

/// A `/*` with no matching `*/`.
pub fn unterminated_block_comment(file: FileId, span: Span) -> Diagnostic {
    error(file, span, "unterminated block comment: no `*/` closes it")
}

fn error(file: FileId, span: Span, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(Severity::Error, file, span, message)
}

/// Prints a character the way it would have to be typed to get it.
///
/// A control byte is shown as its escape (``\t``, ``\u{0}``) rather than as the
/// invisible character itself, because a caret under an invisible character and
/// a message naming it are no help at all.
fn escape_char(ch: char) -> String {
    if ch.is_control() {
        ch.escape_default().to_string()
    } else {
        ch.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file() -> FileId {
        FileId::from_index(0)
    }

    #[test]
    fn a_stray_character_is_named_in_the_message() {
        let d = unexpected_character(file(), Span::new(2, 3), Some('#'));
        assert!(d.is_error());
        assert_eq!(d.message, "unexpected character `#`");
        assert_eq!(d.span, Span::new(2, 3));
    }

    #[test]
    fn a_control_character_is_shown_as_an_escape_rather_than_invisibly() {
        let d = unexpected_character(file(), Span::new(0, 1), Some('\0'));
        assert_eq!(d.message, "unexpected character `\\u{0}`");
        assert_eq!(unexpected_character(file(), Span::new(0, 1), Some('\t')).message, "unexpected character `\\t`");
    }

    #[test]
    fn a_multi_byte_character_is_printed_as_itself() {
        let d = unexpected_character(file(), Span::new(0, 4), Some('\u{20ac}'));
        assert_eq!(d.message, "unexpected character `\u{20ac}`");
    }

    #[test]
    fn an_unreadable_byte_still_produces_a_diagnostic() {
        let d = unexpected_character(file(), Span::new(0, 1), None);
        assert_eq!(d.message, "unexpected character that could not be decoded");
    }

    #[test]
    fn every_message_is_an_error_with_a_span() {
        let diags = [
            unexpected_character(file(), Span::new(0, 1), Some('&')),
            invalid_escape(file(), Span::new(1, 2), "unknown escape `\\q`"),
            unterminated_string(file(), Span::new(0, 4), "the file ends before the closing quote"),
            unterminated_block_comment(file(), Span::new(0, 2)),
        ];
        for d in diags {
            assert!(d.is_error(), "{}", d.message);
            assert!(!d.span.is_synthetic(), "{}", d.message);
            assert!(!d.message.ends_with('.'), "the renderer supplies the punctuation: {}", d.message);
            assert!(d.label.is_none(), "{}", d.message);
        }
    }
}