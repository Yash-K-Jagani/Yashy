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

/// A malformed `${ ... }` inside a string literal: no closing `}`, or an inner
/// string that runs off the end of the line.
///
/// It gets its own message rather than going through [`invalid_escape`], because
/// `${` is not an escape and `}` is not a backslash problem: the reader has an
/// unterminated interpolation, and saying so is the difference between fixing it
/// and hunting for a stray backslash.
pub fn invalid_interpolation(file: FileId, span: Span, detail: &str) -> Diagnostic {
    error(file, span, format!("invalid string interpolation: {detail}"))
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

/// A span whose text is not an integer literal at all: empty, unreadable, or
/// holding something other than digits.
pub(crate) fn invalid_integer_literal(file: FileId, span: Span) -> Diagnostic {
    error(file, span, "expected an integer literal")
}

/// An integer literal whose value does not fit an `i64`.
pub(crate) fn integer_literal_out_of_range(file: FileId, span: Span) -> Diagnostic {
    error(file, span, "integer literal does not fit an i64")
}

/// A number that runs straight into something that cannot be part of one, such
/// as the `x` in `0x1F` or a second dot in `1.2.3`.
///
/// The label matters here more than the message: the mistake is always the same
/// one -- writing a literal Otter does not have -- and saying so is the
/// difference between a user who fixes it and a user who tries `0b1010` next.
pub(crate) fn invalid_numeric_literal(file: FileId, span: Span) -> Diagnostic {
    error(file, span, "invalid numeric literal")
        .with_label("Otter has no hex, binary, exponent or digit-separating literals")
}

fn error(file: FileId, span: Span, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(Severity::Error, file, span, message)
}

/// Prints a character the way it would have to be typed to get it.
///
/// Two classes of character are escaped rather than printed, because a message
/// naming one of them literally is no message at all:
///
/// * control characters -- ``\t``, ``\u{0}`` -- which take up no width and
///   cannot be found by eye in the source line;
/// * invisible formatting characters, which are the same problem wearing a
///   different hat. `U+FEFF` is category `Cf`, not a control character, so
///   `char::is_control` says `false` and the raw one would be printed between
///   the backticks of `unexpected character `` -- precisely the complaint the
///   byte order mark finding was filed about, relocated to the diagnostic that
///   reports a mark found in the middle of a file.
///
/// So the test is not "is this a control character" but "would a reader be able
/// to see this and type it back". Anything invisible fails that, so
/// [`escape_default`] decides, and the set below is the characters a user can
/// actually paste into a file that this rule misses.
fn escape_char(ch: char) -> String {
    if ch.is_control() || is_invisible_format(ch) {
        ch.escape_default().to_string()
    } else {
        ch.to_string()
    }
}

/// Whether `ch` is one of the format characters that render as nothing.
///
/// The list is closed and small on purpose. A blanket "escape every `Cf`
/// character" would turn an ordinary bidi marker inside a string into `\u{200e}`
/// in a diagnostic, which is noise; these three are the ones that show up in
/// real files, from real tools, and that a reader cannot see at all:
///
/// * `U+FEFF` -- the byte order mark, written by plenty of Windows tooling and
///   reachable here because a mark in the middle of a file is a diagnostic;
/// * `U+200B` -- zero width space, a common invisible-character typo;
/// * `U+00AD` -- soft hyphen, which some editors insert when a word is wrapped.
///
/// A `char` that is not one of these and not a control character is printed
/// as itself.
fn is_invisible_format(ch: char) -> bool {
    matches!(ch, '\u{feff}' | '\u{200b}' | '\u{ad}')
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
    fn an_invisible_format_character_is_shown_as_an_escape_rather_than_invisibly() {
        // The complaint the byte order mark finding was filed about, relocated:
        // `U+FEFF` is category `Cf`, so `char::is_control` says `false` and the
        // raw character used to be printed between the backticks -- a message
        // containing, and naming, nothing a reader could see or type back.
        for (ch, escaped) in [
            ('\u{feff}', "\\u{feff}"), // byte order mark
            ('\u{200b}', "\\u{200b}"), // zero width space
            ('\u{ad}', "\\u{ad}"),     // soft hyphen
        ] {
            let d = unexpected_character(file(), Span::new(0, 1), Some(ch));
            assert_eq!(
                d.message,
                format!("unexpected character `{escaped}`"),
                "u+{:04x} must be spelled out",
                ch as u32
            );
            assert!(
                !d.message.contains(ch),
                "u+{:04x} must not appear literally in {:?}",
                ch as u32,
                d.message
            );
            assert!(escaped.contains(&ch.escape_default().to_string()));
        }
    }

    #[test]
    fn an_ordinary_character_next_to_an_invisible_one_is_not_escaped() {
        // The rule is "would a reader see it", not "is it `Cf`": a printable
        // format character is still shown as itself, and a nearby ordinary
        // character must not be dragged into escaping with it.
        assert_eq!(
            unexpected_character(file(), Span::new(0, 1), Some('\u{20ac}')).message,
            "unexpected character `\u{20ac}`",
            "a printable non-control character stays as itself"
        );
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
            invalid_interpolation(file(), Span::new(1, 2), "the line ends before the `}` that closes this `${`"),
            unterminated_string(file(), Span::new(0, 4), "the file ends before the closing quote"),
            unterminated_block_comment(file(), Span::new(0, 2)),
            invalid_integer_literal(file(), Span::new(0, 3)),
            integer_literal_out_of_range(file(), Span::new(0, 40)),
            invalid_numeric_literal(file(), Span::new(0, 3)),
        ];
        for d in diags {
            assert!(d.is_error(), "{}", d.message);
            assert!(!d.span.is_synthetic(), "{}", d.message);
            assert!(!d.message.ends_with('.'), "the renderer supplies the punctuation: {}", d.message);
        }
    }

    #[test]
    fn the_numeric_messages_say_which_literal_is_wrong() {
        assert_eq!(invalid_integer_literal(file(), Span::new(0, 1)).message, "expected an integer literal");
        let out_of_range = integer_literal_out_of_range(file(), Span::new(0, 40));
        assert_eq!(out_of_range.message, "integer literal does not fit an i64");

        // A label is what turns "invalid numeric literal" into something the
        // user can act on, so its presence is pinned rather than incidental.
        let numeric = invalid_numeric_literal(file(), Span::new(0, 3));
        assert_eq!(numeric.message, "invalid numeric literal");
        assert_eq!(
            numeric.label.as_deref(),
            Some("Otter has no hex, binary, exponent or digit-separating literals")
        );
    }
}
