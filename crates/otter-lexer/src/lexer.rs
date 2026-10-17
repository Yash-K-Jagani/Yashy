//! The scanner: bytes in, tokens out.
//!
//! The lexer walks the source once, holding a byte index, and emits a token for
//! every stretch of text it recognises. Trivia -- whitespace and comments -- is
//! skipped but never counted as an error, and the index it leaves behind is the
//! real byte position of the next token, which is the whole reason spans are
//! recorded here rather than reconstructed later.
//!
//! # The lexer never panics
//!
//! This is the first component that touches arbitrary input, so it is the most
//! likely place for a compiler to die on a file the user needs diagnosed. Every
//! read of the source therefore goes through `get`, every offset into a `Span`
//! saturates rather than wraps, and every way of being wrong -- an unknown
//! character, an unterminated string, an unterminated comment, an escape
//! Otter does not have -- produces a [`Diagnostic`] and an `Err`.
//!
//! # When it stops
//!
//! [`Lexer::tokenize`] stops at the first problem. Recovering from an
//! unterminated string or comment would mean guessing where the author meant to
//! stop, and every token after a wrong guess has a wrong span, which produces a
//! second diagnostic that is confidently wrong. The parser is where recovery
//! belongs: it has a delimiter stack to resynchronise on, and it will only ever
//! be handed a token stream that is either complete or absent.

use otter_common::{Diagnostic, DiagnosticBag, FileId, Span};

use crate::error;
use crate::token::{StringLit, Token, TokenKind};

/// The end of a scan: the lexer has already pushed a diagnostic.
#[derive(Debug)]
struct Failed;

/// Scans source text into tokens.
///
/// One lexer owns one source string for its whole life, so every span it emits
/// indexes into that string and nothing else.
pub struct Lexer<'src> {
    src: &'src str,
    /// The file the source came from, for the diagnostics.
    ///
    /// A [`FileId`] is an index into one particular [`otter_common::SourceMap`]
    /// and means nothing without it, so a lexer built by [`Lexer::new`] rather
    /// than [`Lexer::with_file`] names file zero -- correct only if the source
    /// is file zero. Build the lexer with `with_file` whenever the source came
    /// from a map.
    file: FileId,
    /// The byte offset of the next byte to look at.
    pos: usize,
    /// Diagnostics reported so far, in order.
    diags: DiagnosticBag,
}

impl<'src> Lexer<'src> {
    /// A lexer for `src`, attributing diagnostics to file zero.
    ///
    /// See [`Lexer::with_file`] on why that default is a trap worth avoiding.
    pub fn new(src: &'src str) -> Lexer<'src> {
        Lexer::with_file(src, FileId::from_index(0))
    }

    /// A lexer for `src`, attributing diagnostics to `file`.
    pub fn with_file(src: &'src str, file: FileId) -> Lexer<'src> {
        Lexer { src, file, pos: 0, diags: DiagnosticBag::new() }
    }

    /// The file this lexer reports diagnostics against.
    pub fn file(&self) -> FileId {
        self.file
    }

    /// Scans the whole source into tokens, ending with one `Eof`.
    ///
    /// Each call starts from the beginning of the source, so calling it twice
    /// gives the same answer twice; `&mut self` is for reporting progress, not
    /// for resuming.
    ///
    /// # Errors
    ///
    /// The diagnostics collected so far, in order, when the source cannot be
    /// scanned. The token vector is not returned in that case: a stream with a
    /// hole in it is more dangerous than none, because a parser will happily
    /// build a tree over it and report the hole as something else entirely.
    pub fn tokenize(&mut self) -> Result<Vec<Token>, Vec<Diagnostic>> {
        self.pos = 0;
        self.diags = DiagnosticBag::new();
        let mut tokens: Vec<Token> = Vec::new();

        loop {
            if self.skip_trivia().is_err() {
                return Err(self.take_diags());
            }
            let start = self.pos;
            match self.scan_token() {
                Ok(Some(kind)) => tokens.push(Token { kind, span: self.span_of(start, self.pos) }),
                // Past the end of the source: the stream is finished.
                Ok(None) => {
                    let end = self.src.len();
                    tokens.push(Token { kind: TokenKind::Eof, span: self.span_of(end, end) });
                    return Ok(tokens);
                }
                Err(_) => return Err(self.take_diags()),
            }
        }
    }

    /// The diagnostics reported so far, leaving the bag empty.
    ///
    /// Taken rather than read because the caller gets ownership of them, and a
    /// lexer that keeps handing out the same diagnostics would report the same
    /// problem twice on a second call.
    fn take_diags(&mut self) -> Vec<Diagnostic> {
        std::mem::take(&mut self.diags).into_vec()
    }

    /// Skips whitespace and comments, leaving `pos` on the first byte of the
    /// next token.
    fn skip_trivia(&mut self) -> Result<(), Failed> {
        self.skip_byte_order_mark();
        loop {
            let bytes = self.src.as_bytes();
            let first = match peek(bytes, self.pos) {
                Some(byte) => byte,
                None => return Ok(()),
            };
            match first {
                byte if is_space(byte) => self.pos = self.pos.saturating_add(1),
                b'/' if peek_at(bytes, self.pos, 1) == Some(b'/') => self.skip_line_comment(),
                b'/' if peek_at(bytes, self.pos, 1) == Some(b'*') => self.skip_block_comment()?,
                _ => return Ok(()),
            }
        }
    }

    /// Skips a UTF-8 byte order mark at the very start of the file.
    ///
    /// Plenty of Windows tooling writes one, and without this every file it
    /// touches fails to compile with a diagnostic about a character the user
    /// cannot see in their editor. Only a *leading* mark: one further into the
    /// file is not an encoding marker but a character nobody meant to type, and
    /// swallowing that would hide a real mistake. The token after the mark keeps
    /// the span it would have had in a file saved without one, because the mark
    /// is three bytes that were really there.
    ///
    /// Once per file, not once per trivia boundary: `skip_trivia` runs before
    /// every token, so a skipper that re-armed on each call would drag the scan
    /// back to byte 3 in the middle of a file that opened with a mark, and lex
    /// the first token twice. The `pos == 0` guard is what makes it once.
    fn skip_byte_order_mark(&mut self) {
        const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
        let at_start_of_file = self.pos == 0;
        let bytes = self.src.as_bytes();
        if at_start_of_file && bytes.get(..BOM.len()) == Some(BOM.as_slice()) {
            self.pos = BOM.len();
        }
    }

    /// A `//` comment runs to the end of the line, or to the end of the file.
    ///
    /// Running to the end of the file is not an error: a file whose last line is
    /// a comment is a file someone saved mid-edit, and there is nothing to
    /// report about it.
    fn skip_line_comment(&mut self) {
        let bytes = self.src.as_bytes();
        while let Some(byte) = peek(bytes, self.pos) {
            if byte == b'\n' {
                break;
            }
            self.pos = self.pos.saturating_add(1);
        }
    }

    /// A `/* ... */` comment, nested.
    ///
    /// The nesting depth is counted rather than recursed into, so a file with
    /// fifty thousand open comments costs fifty thousand loop iterations and no
    /// stack.
    fn skip_block_comment(&mut self) -> Result<(), Failed> {
        let bytes = self.src.as_bytes();
        let start = self.pos;
        let mut depth = 1usize;
        self.pos = self.pos.saturating_add(2);

        loop {
            match peek(bytes, self.pos) {
                None => {
                    let span = self.span_of(start, self.pos);
                    return self.report(error::unterminated_block_comment(self.file, span));
                }
                Some(b'*') if peek_at(bytes, self.pos, 1) == Some(b'/') => {
                    depth -= 1;
                    self.pos = self.pos.saturating_add(2);
                    if depth == 0 {
                        return Ok(());
                    }
                }
                Some(b'/') if peek_at(bytes, self.pos, 1) == Some(b'*') => {
                    depth = depth.saturating_add(1);
                    self.pos = self.pos.saturating_add(2);
                }
                _ => self.pos = self.pos.saturating_add(1),
            }
        }
    }

    /// Scans one token, leaving `pos` just past it.
    ///
    /// `Ok(None)` means the end of the source, which is not a token but the end
    /// of the stream.
    fn scan_token(&mut self) -> Result<Option<TokenKind>, Failed> {
        let bytes = self.src.as_bytes();
        let first = match peek(bytes, self.pos) {
            Some(byte) => byte,
            None => return Ok(None),
        };

        match first {
            b'"' => {
                self.scan_string()?;
                Ok(Some(TokenKind::Str))
            }
            b'0'..=b'9' => self.scan_number().map(Some),
            byte if is_ident_start(byte) => Ok(Some(self.scan_identifier())),
            byte => self.scan_punctuation(byte),
        }
    }

    /// Scans `[A-Za-z_][A-Za-z0-9_]*` and decides whether the whole of it is a
    /// keyword.
    fn scan_identifier(&mut self) -> TokenKind {
        let bytes = self.src.as_bytes();
        let start = self.pos;
        while matches!(peek(bytes, self.pos), Some(byte) if is_ident_continue(byte)) {
            self.pos = self.pos.saturating_add(1);
        }

        // `start..pos` is a range this loop just walked over ASCII bytes, so it
        // is always a valid boundary; the `unwrap_or` is belt and braces for a
        // rule change that made that untrue, and it costs one branch.
        let text = self.src.get(start..self.pos).unwrap_or("");
        TokenKind::keyword(text).unwrap_or(TokenKind::Ident)
    }

    /// Scans `[0-9]+` or `[0-9]+ '.' [0-9]+`.
    ///
    /// The dot is only part of the number when a digit follows it, which is
    /// what keeps `0..10` two tokens and a `DotDot` rather than a float and a
    /// stray `10`. A float with no digits before the dot (`.5`) is not in the
    /// language, so it is a `Dot` followed by a number.
    ///
    /// A number may not run straight into a name or a second dot. Splitting
    /// `0x1F` into `0` and `x1F` looks like success and leaves the parser to
    /// complain about an identifier the user never wrote, so those are
    /// diagnosed here, where the mistake is visible.
    fn scan_number(&mut self) -> Result<TokenKind, Failed> {
        let bytes = self.src.as_bytes();
        let start = self.pos;
        self.skip_digits();
        let after_int = self.pos;

        let is_float = peek(bytes, after_int) == Some(b'.')
            && matches!(peek_at(bytes, after_int, 1), Some(b'0'..=b'9'));
        if is_float {
            self.pos = self.pos.saturating_add(1);
            self.skip_digits();
        }

        let malformed = match peek(bytes, self.pos) {
            // `123abc`, `1_000`, `1e10`: a letter or an underscore where the
            // number should have ended.
            Some(byte) if is_ident_continue(byte) => true,
            // `1.2.3`: a second dot after a float. A dot after an *integer* is
            // `DotDot` or a bare `Dot`, which are both real tokens.
            Some(b'.') if is_float => true,
            _ => false,
        };
        if malformed {
            let span = self.span_of(start, self.pos);
            return self.report(error::invalid_numeric_literal(self.file, span));
        }

        Ok(if is_float { TokenKind::F64 } else { TokenKind::I64 })
    }

    fn skip_digits(&mut self) {
        let bytes = self.src.as_bytes();
        while matches!(peek(bytes, self.pos), Some(b'0'..=b'9')) {
            self.pos = self.pos.saturating_add(1);
        }
    }

    /// Scans a `"..."` literal.
    ///
    /// Two passes, deliberately: the first finds where the literal ends, the
    /// second hands that exact text to [`StringLit::decode`] to find out whether
    /// it is well formed. Decoding is the only description of what an escape
    /// means, so there is one implementation of the rules and the lexer cannot
    /// accept something the parser will then reject.
    ///
    /// A literal does not run past the end of its line. A forgotten quote would
    /// otherwise swallow the rest of the file, and every token after it would
    /// carry a span belonging to some later line -- a single typo producing a
    /// hundred confidently wrong diagnostics.
    fn scan_string(&mut self) -> Result<(), Failed> {
        let bytes = self.src.as_bytes();
        let start = self.pos;
        let mut at = self.pos.saturating_add(1); // Past the opening quote.

        let end = loop {
            match peek(bytes, at) {
                None => {
                    let span = self.span_of(start, at);
                    return self.report(error::unterminated_string(
                        self.file,
                        span,
                        "the file ends before the closing quote",
                    ));
                }
                Some(b'\n') => {
                    let span = self.span_of(start, at);
                    return self.report(error::unterminated_string(
                        self.file,
                        span,
                        "the line ends before the closing quote",
                    ));
                }
                Some(b'"') => break at.saturating_add(1),
                Some(b'\\') => match peek_at(bytes, at, 1) {
                    // A backslash swallows the next byte so that `\"` cannot end
                    // the literal -- unless there is no next byte, or the next
                    // byte is the newline that ends the line.
                    None => {
                        let span = self.span_of(start, at);
                        return self.report(error::unterminated_string(
                            self.file,
                            span,
                            "the file ends after a backslash",
                        ));
                    }
                    Some(b'\n') => {
                        let span = self.span_of(start, at);
                        return self.report(error::unterminated_string(
                            self.file,
                            span,
                            "the line ends after a backslash",
                        ));
                    }
                    Some(_) => at = at.saturating_add(2),
                },
                _ => at = at.saturating_add(1),
            }
        };

        let raw = match self.src.get(start..end) {
            Some(raw) => raw,
            // `start..end` was walked a byte at a time from a character
            // boundary, so this cannot happen; if it ever does the literal is
            // still unterminated as far as anything can tell.
            None => {
                let span = self.span_of(start, end);
                return self.report(error::unterminated_string(
                    self.file,
                    span,
                    "the literal could not be read",
                ));
            }
        };

        match StringLit::decode(raw) {
            Ok(_) => {
                self.pos = end;
                Ok(())
            }
            Err(failure) => {
                // The failure is relative to the literal; the diagnostic points
                // at the single byte that is wrong inside it.
                let at = start.saturating_add(failure.offset).min(end);
                let span = self.span_of(at, at.saturating_add(1).min(end));
                self.report(error::invalid_escape(self.file, span, &failure.message))
            }
        }
    }

    /// Scans one operator or delimiter.
    ///
    /// Two-character operators are matched before one-character ones, which is
    /// why this is a table over `(first, second)` rather than a single match on
    /// `first`: `==` must not come out as `=` followed by `=`, and `..` must not
    /// come out as `.` followed by `.`.
    fn scan_punctuation(&mut self, first: u8) -> Result<Option<TokenKind>, Failed> {
        let bytes = self.src.as_bytes();
        let second = peek_at(bytes, self.pos, 1);
        let (width, kind) = match operator(first, second) {
            Some(found) => found,
            None => {
                // The span covers the whole character, not just its first byte:
                // a caret one byte wide under a two-byte character points at
                // half of it.
                let found = self.src.get(self.pos..).and_then(|rest| rest.chars().next());
                let width = found.map_or(1, |ch| ch.len_utf8());
                let span = self.span_of(self.pos, self.pos.saturating_add(width));
                return self.report(error::unexpected_character(self.file, span, found));
            }
        };
        self.pos = self.pos.saturating_add(width);
        Ok(Some(kind))
    }

    /// The span covering `[start, end)`, with offsets that saturate.
    ///
    /// Saturating rather than wrapping: a file too large for `u32` offsets is
    /// not a thing that can be compiled on any machine this runs on, but a
    /// wrapped offset would be a caret in the wrong place rather than a limit
    /// reached.
    fn span_of(&self, start: usize, end: usize) -> Span {
        Span::new(to_offset(start), to_offset(end))
    }

    /// Records a diagnostic and ends the scan.
    fn report<T>(&mut self, diagnostic: Diagnostic) -> Result<T, Failed> {
        self.diags.push(diagnostic);
        Err(Failed)
    }
}

impl std::fmt::Debug for Lexer<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lexer")
            .field("file", &self.file)
            .field("len", &self.src.len())
            .field("pos", &self.pos)
            .field("diags", &self.diags.len())
            .finish()
    }
}

/// The operator or delimiter starting at `first`, and its width in bytes.
///
/// `None` means this byte starts nothing Otter has, which is what turns a
/// character like `#` into a diagnostic instead of a silently dropped byte that
/// shifts every span after it.
fn operator(first: u8, second: Option<u8>) -> Option<(usize, TokenKind)> {
    use TokenKind as K;

    Some(match (first, second) {
        // Two-character operators first: this is the only thing that decides
        // `==` vs `=`.
        (b'=', Some(b'=')) => (2, K::EqEq),
        (b'!', Some(b'=')) => (2, K::BangEq),
        (b'<', Some(b'=')) => (2, K::LtEq),
        (b'>', Some(b'=')) => (2, K::GtEq),
        (b'+', Some(b'=')) => (2, K::PlusEq),
        (b'-', Some(b'=')) => (2, K::MinusEq),
        (b'-', Some(b'>')) => (2, K::Arrow),
        (b'*', Some(b'=')) => (2, K::StarEq),
        (b'/', Some(b'=')) => (2, K::SlashEq),
        (b'%', Some(b'=')) => (2, K::PercentEq),
        (b'.', Some(b'.')) => (2, K::DotDot),
        (b':', Some(b'=')) => (2, K::ColonEq),
        (b'&', Some(b'&')) => (2, K::AndAnd),
        (b'|', Some(b'|')) => (2, K::OrOr),

        // One-character operators. `&` and `|` are absent on purpose: Otter
        // has `&&` and `||` and no bitwise operators, so a lone `&` is a typo
        // to report rather than a token to invent.
        (b'=' , _) => (1, K::Eq),
        (b'!', _) => (1, K::Bang),
        (b'<', _) => (1, K::Lt),
        (b'>', _) => (1, K::Gt),
        (b'+', _) => (1, K::Plus),
        (b'-', _) => (1, K::Minus),
        (b'*', _) => (1, K::Star),
        (b'/', _) => (1, K::Slash),
        (b'%', _) => (1, K::Percent),
        (b'.', _) => (1, K::Dot),
        (b':', _) => (1, K::Colon),
        (b'(', _) => (1, K::LParen),
        (b')', _) => (1, K::RParen),
        (b'{', _) => (1, K::LBrace),
        (b'}', _) => (1, K::RBrace),
        (b'[', _) => (1, K::LBracket),
        (b']', _) => (1, K::RBracket),
        (b',', _) => (1, K::Comma),
        (b';', _) => (1, K::Semi),
        (b'@', _) => (1, K::At),
        (b'?', _) => (1, K::Question),

        _ => return None,
    })
}

/// Whether a byte is whitespace.
///
/// `\r` is whitespace on purpose: a file written on Windows between two token
/// streams must produce the same spans as the same file with `\n` endings, and
/// `\r` is the single most likely byte to be left behind by a tool.
fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c)
}

fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_ident_continue(byte: u8) -> bool {
    is_ident_start(byte) || byte.is_ascii_digit()
}

/// The byte at `at`, or `None` past the end of the source.
fn peek(bytes: &[u8], at: usize) -> Option<u8> {
    bytes.get(at).copied()
}

/// The byte `ahead` past `at`, or `None` past the end of the source.
///
/// The `saturating_add` is what makes this safe to call at the end of the
/// source: `at + ahead` is never computed in the first place, so there is no
/// index to be out of range.
fn peek_at(bytes: &[u8], at: usize, ahead: usize) -> Option<u8> {
    bytes.get(at.saturating_add(ahead)).copied()
}

/// A byte offset as the `u32` a [`Span`] holds, saturating.
fn to_offset(offset: usize) -> u32 {
    u32::try_from(offset).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(src: &str) -> Vec<Token> {
        match Lexer::new(src).tokenize() {
            Ok(tokens) => tokens,
            Err(diags) => panic!("expected {src:?} to lex, got {:?}", diags.iter().map(|d| d.message.clone()).collect::<Vec<_>>()),
        }
    }

    fn first_error(src: &str) -> Diagnostic {
        match Lexer::new(src).tokenize() {
            Ok(tokens) => panic!("expected {src:?} to fail, got {tokens:?}"),
            Err(mut diags) => {
                assert!(!diags.is_empty(), "a failed lex must explain itself");
                diags.remove(0)
            }
        }
    }

    #[test]
    fn an_empty_source_is_one_eof_token() {
        assert_eq!(lex(""), vec![Token { kind: TokenKind::Eof, span: Span::new(0, 0) }]);
        assert_eq!(lex("   \n\t\r\n"), vec![Token { kind: TokenKind::Eof, span: Span::new(7, 7) }]);
    }

    #[test]
    fn every_stream_ends_with_exactly_one_eof_at_the_end_of_the_source() {
        for src in ["fn", "1 2 3", "// c\nx", "/* c */", "\0", "fn f() {}"] {
            if src.contains('\0') {
                continue; // Reported by `an_unknown_byte_is_a_diagnostic`.
            }
            let tokens = lex(src);
            assert!(tokens.last().map(|t| t.kind.is_eof()).unwrap_or(false), "{src:?}");
            assert_eq!(tokens.len(), 1 + tokens.iter().filter(|t| !t.kind.is_eof()).count(), "{src:?}");
            let eof = tokens.last().expect("checked above");
            let end = u32::try_from(src.len()).unwrap_or(u32::MAX);
            assert_eq!(eof.span, Span::new(end, end), "{src:?}");
        }
    }

    #[test]
    fn spans_are_half_open_and_never_overlap() {
        let tokens = lex("fn f(a: i64) -> i64 { return a * 2; }");
        let mut previous_end = 0;
        for token in &tokens {
            assert!(token.span.start <= token.span.end, "{token:?} is not ordered");
            assert!(token.span.start as usize >= previous_end, "{token:?} overlaps the token before it");
            previous_end = token.span.end as usize;
        }
    }

    #[test]
    fn a_diagnostic_stops_the_scan_at_the_first_problem() {
        // Two bad characters, one diagnostic: guessing where the author meant to
        // resume would put every later span in the wrong place.
        let diags = match Lexer::new("# #").tokenize() {
            Ok(tokens) => panic!("must fail, got {tokens:?}"),
            Err(diags) => diags,
        };
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].span, Span::new(0, 1));
    }

    #[test]
    fn an_unknown_byte_is_a_diagnostic() {
        for src in ["#", "$", "`", "~", "^", "&", "|", "\\", "\u{1}"] {
            let d = first_error(src);
            assert!(d.message.starts_with("unexpected character"), "{src:?}: {}", d.message);
            assert_eq!(d.span.start, 0, "{src:?}");
        }
    }

    #[test]
    fn a_non_ascii_character_is_reported_rather_than_swallowed_into_an_identifier() {
        // Identifiers are ASCII. Allowing any byte into them would turn a
        // mistyped operator into a plausible-looking name, which is a worse
        // failure than being told about the character.
        let d = first_error("caf\u{e9}");
        assert!(d.message.contains("unexpected character"), "{}", d.message);
        assert_eq!(d.span, Span::new(3, 5), "the two bytes of `é` are one character");
    }

    #[test]
    fn diagnostics_name_the_file_the_lexer_was_built_with() {
        let file = FileId::from_index(7);
        let mut lexer = Lexer::with_file("#", file);
        assert_eq!(lexer.file(), file);
        let diags = match lexer.tokenize() {
            Ok(tokens) => panic!("must fail, got {tokens:?}"),
            Err(diags) => diags,
        };
        assert_eq!(diags[0].file, file);
        assert_eq!(first_error("#").file, FileId::from_index(0), "`new` defaults to file zero");
    }

    #[test]
    fn a_line_ending_never_reaches_the_stream_as_a_token() {
        // CRLF and LF produce the same *tokens*, at offsets shifted by the
        // extra `\r`. That shift is correct: a span is a byte range, and the
        // bytes are there.
        let crlf = lex("1\r\n2");
        let lf = lex("1\n2");
        assert_eq!(crlf.len(), 3);
        assert_eq!(lf.len(), 3);
        assert_eq!(
            crlf.iter().map(|t| t.kind).collect::<Vec<_>>(),
            lf.iter().map(|t| t.kind).collect::<Vec<_>>()
        );
        assert_eq!(crlf[1].span, Span::new(3, 4), "the `2` is at byte 3 once the `\\r` is counted");
        assert_eq!(lf[1].span, Span::new(2, 3));
    }

    #[test]
    fn a_comment_inside_a_string_is_string_content() {
        let tokens = lex("\"/* not a comment */\"");
        assert_eq!(tokens[0].kind, TokenKind::Str);
        assert_eq!(tokens[0].span, Span::new(0, 21), "the whole literal, quotes included");
    }

    #[test]
    fn a_slash_star_inside_a_string_does_not_open_a_comment() {
        // If the trivia skipper ran inside the string this would unbalance the
        // nesting count and eat the rest of the file. Asserting the whole
        // stream, not just its length: a lexer that emitted a `Slash` and a
        // `Star` from inside the quotes would have the right number of tokens
        // here and be completely wrong.
        //
        // `"/*"`
        //  byte 0 is the opening quote, bytes 1..3 are `/*` -- string content,
        //  not the start of a comment -- and byte 3 is the closing quote.
        assert_eq!(lex("\"/*\""), vec![
            Token { kind: TokenKind::Str, span: Span::new(0, 4) },
            Token { kind: TokenKind::Eof, span: Span::new(4, 4) },
        ]);
        // And the file after it is still lexed: the `1` is at byte 5.
        assert_eq!(lex("\"/*\" 1"), vec![
            Token { kind: TokenKind::Str, span: Span::new(0, 4) },
            Token { kind: TokenKind::I64, span: Span::new(5, 6) },
            Token { kind: TokenKind::Eof, span: Span::new(6, 6) },
        ]);
    }

    #[test]
    fn nested_comments_balance() {
        assert_eq!(lex("/**/").len(), 1, "an empty comment is just trivia");

        // Fifteen openers and fifteen closers, then a number. If the depth
        // counter were wrong in either direction this would either stop at the
        // first `*/` or report the comment as unterminated.
        let nested = format!("{}{}1", "/*".repeat(15), "*/".repeat(15));
        assert_eq!(lex(&nested), vec![
            Token { kind: TokenKind::I64, span: Span::new(60, 61) },
            Token { kind: TokenKind::Eof, span: Span::new(61, 61) },
        ]);

        // Two levels of comment are two levels of trivia: the `1` after
        // `/* /* */ */` is at byte 12, exactly as it would be after `/*`.
        assert_eq!(
            lex("/* /* */ */ 1")[0].kind,
            TokenKind::I64,
            "both levels must be skipped"
        );
        assert_eq!(lex("/* /* */ */ 1")[0].span, Span::new(12, 13));
    }

    #[test]
    fn a_block_comment_diagnostic_points_at_the_delimiter_that_opened_it() {
        // `fn f() {}\n/*`
        //  bytes 0..8 are `fn f() {}`, byte 9 is the newline, bytes 10..12 are
        //  the `/*` that never closes.
        let d = first_error("fn f() {}\n/*");
        assert_eq!(d.span, Span::new(10, 12), "the `/*` at byte 10, through the end of the file");
        assert!(d.message.contains("block comment"), "{}", d.message);
    }

    #[test]
    fn a_string_that_ends_the_file_points_at_the_quote_that_opened_it() {
        let d = first_error("fn f() { \"oops");
        assert_eq!(d.span.start, 9, "the quote at byte 9");
        assert!(d.message.contains("unterminated string"), "{}", d.message);
    }

    #[test]
    fn a_string_says_where_it_ran_out() {
        // Two different mistakes with the same symptom, and the user can only
        // act on one of them if the message says which: a quote that is still
        // open at the end of the *line* is a forgotten quote, while one open at
        // the end of the *file* is usually a broken paste. Without this, a
        // lexer that let the literal run past the newline would report the
        // second as the first.
        let at_end_of_file = first_error("\"abc");
        assert!(at_end_of_file.message.contains("the file ends"), "{}", at_end_of_file.message);

        // `"abc\n42`
        //  byte 0 is the quote, bytes 1..3 are `abc`, byte 4 is the newline
        //  that the literal must not cross.
        let at_end_of_line = first_error("\"abc\n42");
        assert!(at_end_of_line.message.contains("the line ends"), "{}", at_end_of_line.message);
        assert_eq!(at_end_of_line.span, Span::new(0, 4), "the quote through the end of the line");

        // A backslash swallows the next byte so `\"` cannot end the literal --
        // but not the newline after it.
        assert!(first_error("\"abc\\\n").message.contains("after a backslash"), "{}", first_error("\"abc\\\n").message);
    }

    #[test]
    fn an_escape_that_is_not_an_escape_points_at_the_backslash() {
        let d = first_error("\"a\\qb\"");
        assert_eq!(d.span, Span::new(2, 3), "the backslash at byte 2");
        assert!(d.message.contains("escape"), "{}", d.message);
    }

    #[test]
    fn an_out_of_range_offset_saturates_instead_of_wrapping() {
        // Not reachable from a real file, but the conversion is the one place
        // a large offset could become a caret at the start of the file.
        assert_eq!(to_offset(usize::MAX), u32::MAX);
        assert_eq!(to_offset(0), 0);
        assert_eq!(to_offset(u32::MAX as usize), u32::MAX);
    }

    #[test]
    fn the_debug_rendering_does_not_dump_the_source() {
        let text = format!("{:?}", Lexer::new("secret"));
        assert!(!text.contains("secret"), "a Debug for a lexer must not print the program: {text}");
        assert!(text.contains("Lexer"), "{text}");
    }

    #[test]
    fn a_byte_order_mark_is_skipped_once_at_the_start_of_a_file() {
        // The mark is three bytes that are really there, so the token after it
        // keeps the offset it has in a file saved without one.
        assert_eq!(lex("\u{feff}1"), vec![
            Token { kind: TokenKind::I64, span: Span::new(3, 4) },
            Token { kind: TokenKind::Eof, span: Span::new(4, 4) },
        ]);

        // A file that is nothing but a mark is an empty file, not an error.
        assert_eq!(lex("\u{feff}"), vec![Token { kind: TokenKind::Eof, span: Span::new(3, 3) }]);

        // Trivia immediately after the mark is trivia, not a stray token. A
        // skipper that read the byte at the old offset *before* advancing over
        // the mark would report the space as `unexpected character` here.
        assert_eq!(lex("\u{feff} 1"), vec![
            Token { kind: TokenKind::I64, span: Span::new(4, 5) },
            Token { kind: TokenKind::Eof, span: Span::new(5, 5) },
        ]);
        assert_eq!(lex("\u{feff}/* c */1"), vec![
            Token { kind: TokenKind::I64, span: Span::new(10, 11) },
            Token { kind: TokenKind::Eof, span: Span::new(11, 11) },
        ]);

        // Once, not per trivia boundary: `skip_trivia` runs before every token,
        // so a skipper that re-armed would drag the scan back to byte 3 after
        // the first token and lex `1` twice.
        assert_eq!(lex("\u{feff}1 2"), vec![
            Token { kind: TokenKind::I64, span: Span::new(3, 4) },
            Token { kind: TokenKind::I64, span: Span::new(5, 6) },
            Token { kind: TokenKind::Eof, span: Span::new(6, 6) },
        ]);

        // Only a *leading* mark is skipped. One further in is a character
        // nobody typed on purpose, and swallowing it would hide a mistake.
        let d = first_error("\u{feff}1 \u{feff}2");
        assert_eq!(d.span, Span::new(5, 8), "the second mark, not a re-lex of the first token");
    }

    #[test]
    fn the_operator_table_covers_every_punctuation_kind() {
        // Each kind that has a spelling must be produced by its own spelling.
        // A kind missing from the table is a kind the parser can never see.
        for kind in [
            TokenKind::LParen, TokenKind::RParen, TokenKind::LBrace, TokenKind::RBrace,
            TokenKind::LBracket, TokenKind::RBracket, TokenKind::Comma, TokenKind::Semi,
            TokenKind::Colon, TokenKind::ColonEq, TokenKind::Dot, TokenKind::DotDot,
            TokenKind::Arrow, TokenKind::At, TokenKind::Question, TokenKind::Plus,
            TokenKind::Minus, TokenKind::Star, TokenKind::Slash, TokenKind::Percent,
            TokenKind::Eq, TokenKind::EqEq, TokenKind::Bang, TokenKind::BangEq, TokenKind::Lt,
            TokenKind::LtEq, TokenKind::Gt, TokenKind::GtEq, TokenKind::PlusEq,
            TokenKind::MinusEq, TokenKind::StarEq, TokenKind::SlashEq, TokenKind::PercentEq,
            TokenKind::AndAnd, TokenKind::OrOr,
        ] {
            let spelling = match kind.spelling() {
                Some(spelling) => spelling,
                None => panic!("{kind:?} must have a spelling"),
            };
            let tokens = lex(spelling);
            assert_eq!(tokens[0].kind, kind, "{spelling:?} must lex as {kind:?}");
            assert_eq!(tokens[0].span, Span::new(0, spelling.len() as u32), "{spelling:?}");
        }
    }
}
