//! The parser cursor: a token vector, an index, and the diagnostics it can
//! point at.
//!
//! # The shape of the type
//!
//! [`Parser`] holds the tokens it was given and a position in them. Every
//! cursor operation is total: `peek` at the end of the stream reports `Eof`
//! forever rather than indexing past it, so a parser written on top of it can
//! never read out of bounds however malformed the input is. That is the whole
//! reason the stream is a `Vec` the parser owns and indexes by hand rather than
//! an iterator: an iterator that has run out is a `None` every call site has to
//! remember to handle, and one call site that forgets is a panic.
//!
//! # The unclosed-delimiter stack
//!
//! A `{` that was never closed is the single most common way to break a parser,
//! because every symptom of it appears somewhere else: the next item is
//! "unexpected", the last statement is "expected `;`", or -- the failure mode
//! that costs a day -- the parser loops forever consuming nothing. So the
//! parser keeps a stack of the delimiters it is inside:
//!
//! * [`Parser::push_open`] records an opener's span and checks the depth limit;
//! * [`Parser::close_open`] pops the innermost opener a closer matches;
//! * [`Parser::take_unclosed`] reports the innermost one still open and clears
//!   the stack, so the same mistake is reported once rather than once per level.
//!
//! Every later statement and expression parser calls [`Parser::expect`], and
//! every one that opens a block pushes before it and pops after, so this task
//! owns the invariant and Tasks 5 and 6 inherit it.
//!
//! # The block seam
//!
//! [`Parser::parse_block`] is Task 4's part of a function that Task 5 finishes.
//! It is documented where it is defined; the short version is that it consumes a
//! balanced body and reports its span, and it does not yet read the statements
//! in it.

use otter_common::{Diagnostic, DiagnosticBag, FileId, SourceMap, Span};
use otter_lexer::{Token, TokenKind};

use crate::ast::Program;
use crate::error::{self, ParseError};

/// How deeply delimiters may nest before the parser stops reading.
///
/// This is a **stack safety** bound, not a style preference. Recursive descent
/// costs one stack frame per open delimiter, and the number that matters is how
/// many frames fit in the smallest stack the parser can be called on: the
/// program's main thread, which is 1 MiB on Windows and 8 MiB on Linux.
///
/// Measured on this machine, a debug build of the driver (`target/debug/otter.exe`,
/// unoptimized frames are the worst case, and the case the tests exercise):
///
/// | nested `(` | result |
/// |-----------|--------|
/// | 90        | parses |
/// | 110       | `STATUS_STACK_OVERFLOW` |
///
/// So the ceiling is around 100, and a limit of 128 -- as this value was, and as
/// Task 14's plan suggested raising it to 256 -- is *past* the point where the
/// process dies. A stack overflow aborts uncatchably, which is worse than a
/// panic and cannot be reported as a diagnostic, so the limit has to sit below
/// the ceiling rather than at it.
///
/// 48 is half the measured ceiling, which leaves room for the frames above the
/// parser (the driver, `parse_program`) and for the type checker and code
/// generator, which walk the same tree afterwards. Real code nests under ten; a
/// generated file that needs more than 48 levels of parentheses is pathological.
///
/// Raising this safely needs one of: an iterative parser, or running the whole
/// pipeline on a thread with a deliberately large stack. Both are M3+ work.
///
/// Task 4 is where the stack is; Task 14 is where it was measured.
pub const MAX_NESTING_DEPTH: usize = 48;

/// One delimiter that has been opened and not yet closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Open {
    /// Which opener this is, so a closer knows whether it matches.
    kind: TokenKind,
    /// Where the opener was written. This is where an unclosed-delimiter
    /// diagnostic points, because this is the character the reader must add.
    span: Span,
}

/// A cursor over one file's tokens.
///
/// `'src` is the lifetime of the [`SourceMap`] the tokens came from, not of the
/// tokens: a [`Token`] carries no value, so the identifier in `struct Point` is
/// read back out of the source text by span, and that text has to stay reachable
/// for as long as the parser is.
///
/// ```
/// use otter_common::SourceMap;
/// use otter_lexer::Lexer;
/// use otter_parser::Parser;
///
/// let src = "struct Point { x: f64 }";
/// let mut map = SourceMap::new();
/// let file = map.add_file("point.ot", src.to_string());
/// let tokens = Lexer::with_file(src, file).tokenize().expect("valid source");
///
/// let program = Parser::new(tokens, &map, file).parse_program().expect("valid items");
/// assert_eq!(program.items.len(), 1);
/// ```
#[derive(Debug)]
pub struct Parser<'src> {
    tokens: Vec<Token>,
    /// Index of the next token to look at.
    pos: usize,
    source: &'src SourceMap,
    file: FileId,
    /// The delimiters currently open, outermost first. Never contains a closer.
    open: Vec<Open>,
}

impl<'src> Parser<'src> {
    /// A parser over `tokens`, reading names out of `source` and reporting
    /// against `file`.
    ///
    /// # Why `source` is borrowed
    ///
    /// A [`Token`] carries no value -- an `Ident` is a kind and a span, not a
    /// name -- so the parser reads every name back out of the source text by
    /// span. That text therefore has to stay reachable for as long as the parser
    /// is, which a borrow expresses and an owned [`SourceMap`] does not.
    ///
    /// The plan's signature took `SourceMap` by value. That would cost the
    /// caller the map it needs for [`SourceMap::render`] -- the driver has to
    /// keep rendering diagnostics after the parse is over, and a map moved into
    /// a parser is a map the driver no longer has. Borrow it.
    ///
    /// `tokens` is expected to be the output of
    /// [`Lexer::tokenize`](otter_lexer::Lexer::tokenize) over `source`'s text
    /// for `file`. Nothing checks that, and nothing has to: the two properties
    /// that would break -- a span that does not index the text, or a file that
    /// is not the one the tokens came from -- both degrade to a diagnostic
    /// rather than to a wrong answer read out of unrelated memory.
    ///
    /// ```
    /// use otter_common::SourceMap;
    /// use otter_lexer::Lexer;
    /// use otter_parser::Parser;
    ///
    /// let src = "import std.io;";
    /// let mut map = SourceMap::new();
    /// let file = map.add_file("main.ot", src.to_string());
    /// let tokens = Lexer::with_file(src, file).tokenize().expect("valid source");
    ///
    /// // The map is still ours afterwards, which is the point of the borrow:
    /// // the driver renders diagnostics with it once the parse is done.
    /// let program = Parser::new(tokens, &map, file).parse_program();
    /// assert_eq!(map.path(file), "main.ot");
    /// ```
    pub fn new(tokens: Vec<Token>, source: &'src SourceMap, file: FileId) -> Parser<'src> {
        Parser {
            tokens,
            pos: 0,
            source,
            file,
            open: Vec::new(),
        }
    }

    /// The file this parser reports against.
    pub fn file(&self) -> FileId {
        self.file
    }

    /// The source map this parser reads names out of.
    pub fn source(&self) -> &'src SourceMap {
        self.source
    }

    // -- the cursor ---------------------------------------------------------

    /// The kind of the next token, or `Eof` once the stream has run out.
    pub fn peek(&self) -> TokenKind {
        self.peek_token().kind
    }

    /// The next token itself.
    ///
    /// Past the end of the vector this is a synthetic `Eof` at the end of the
    /// last real token, which is where a well-formed `Eof` would have been. A
    /// stream built by the lexer always ends with a real `Eof`, so this path is
    /// for a caller that built one by hand.
    pub fn peek_token(&self) -> Token {
        match self.tokens.get(self.pos) {
            Some(token) => *token,
            None => self.end_of_file(),
        }
    }

    /// The span of the next token. For `Eof` that is the empty span at the end
    /// of the file, which is what a diagnostic about running out points at.
    pub fn peek_span(&self) -> Span {
        self.peek_token().span
    }

    /// True when the next token is of kind `kind`.
    pub fn check(&self, kind: TokenKind) -> bool {
        self.peek() == kind
    }

    /// The kind of the token after the next one.
    pub fn peek_second(&self) -> TokenKind {
        match self.tokens.get(self.pos.saturating_add(1)) {
            Some(t) => t.kind,
            None => TokenKind::Eof,
        }
    }

    /// The kind of the token three positions ahead.
    pub fn peek_third(&self) -> TokenKind {
        match self.tokens.get(self.pos.saturating_add(2)) {
            Some(t) => t.kind,
            None => TokenKind::Eof,
        }
    }

    /// True when the next token is `Eof` or the stream has run out.
    pub fn at_eof(&self) -> bool {
        self.peek().is_eof()
    }

    /// Consumes the next token if it is of kind `kind`.
    ///
    /// Returns whether it did, so a caller writes `while self.eat(TokenKind::Dot)`
    /// rather than an `if` around an `advance` that may not have happened --
    /// which is where "the index moved but nothing was consumed" bugs live.
    pub fn eat(&mut self, kind: TokenKind) -> bool {
        if self.check(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Consumes the next token whatever it is, and returns it.
    ///
    /// At the end of the stream this returns the synthetic `Eof` **without**
    /// moving the position. A cursor that ran off the end must not keep running:
    /// a loop that calls `advance` until it sees `Eof` would then spin forever.
    pub fn advance(&mut self) -> Token {
        match self.tokens.get(self.pos) {
            Some(token) => {
                self.pos = self.pos.saturating_add(1);
                *token
            }
            None => self.end_of_file(),
        }
    }

    /// The span of the last token consumed, or the synthetic span if none has
    /// been.
    pub fn previous_span(&self) -> Span {
        match self
            .pos
            .checked_sub(1)
            .and_then(|before| self.tokens.get(before))
        {
            Some(token) => token.span,
            None => Span::SYNTHETIC,
        }
    }

    /// Consumes the next token, which must be of kind `kind`.
    ///
    /// # Errors
    ///
    /// A [`ParseError`] naming both what was wanted and what was found, at the
    /// found token's span -- with one exception. **At the end of the file, a
    /// delimiter that is still open is reported as unclosed**, rather than as
    /// "expected `}`, found end of file":
    ///
    /// ```text
    /// struct S {
    ///     x: f64
    /// ```
    ///
    /// The reader did not forget a `}` at the end of the file; they forgot one
    /// at the `{`, and only the stack knows where that was. Putting the rule in
    /// `expect` is what makes every later parser -- Task 5's blocks, Task 6's
    /// calls and subscripts -- get it by calling `expect` rather than by
    /// remembering to check.
    ///
    /// The rule is deliberately not conditioned on the wanted closer *matching*
    /// the innermost opener. Given `f( { }` with nothing else open, "expected
    /// `}`, found end of file" is a worse answer than "unclosed `(`": the file
    /// really did run out, and the innermost delimiter still standing is the
    /// thing a reader has to close.
    pub fn expect(&mut self, kind: TokenKind) -> Result<Token, ParseError> {
        if self.check(kind) {
            return Ok(self.advance());
        }
        if self.at_eof() {
            // `take_unclosed` clears the stack, so the same mistake is reported
            // once however many delimiters are still open.
            if let Some(e) = self.take_unclosed() {
                return Err(e);
            }
        }
        Err(self.expected_error(kind))
    }

    /// A failure at `span` saying `message`.
    ///
    /// Named `error` because that is what it builds; the free functions in
    /// [`crate::error`] are the wording, and this is the door into them.
    pub fn error(&self, span: Span, message: impl Into<String>) -> ParseError {
        ParseError::new(span, message)
    }

    /// The failure [`Parser::expect`] would return right now, without consuming
    /// anything.
    pub fn expected_error(&self, kind: TokenKind) -> ParseError {
        error::expected(kind, self.peek(), self.peek_span())
    }

    /// The source text a token's span covers.
    ///
    /// # Errors
    ///
    /// [`error::unreadable_text`] when the span does not index this file's
    /// text. An `Ident` token carries no name, so this is the only way to learn
    /// what it was called, and inventing a name for a span that cannot be read
    /// would put a plausible-looking wrong identifier into the tree.
    pub fn text(&self, span: Span) -> Result<String, ParseError> {
        let start = usize::try_from(span.start).unwrap_or(usize::MAX);
        let end = usize::try_from(span.end).unwrap_or(usize::MAX);
        match self.source.file(self.file).get(start..end) {
            Some(text) => Ok(text.to_string()),
            None => Err(error::unreadable_text(span)),
        }
    }

    /// Consumes an identifier and returns its name and span.
    ///
    /// # Errors
    ///
    /// [`error::unreadable_text`] if the name cannot be read back out of the
    /// source, **or if it reads as nothing**. A zero-width `Ident` cannot come
    /// out of the lexer, so this is the guard for a caller that built its own
    /// token stream -- and it is here because the alternative is a struct called
    /// `""` that looks exactly like a struct the user wrote.
    pub fn identifier(&mut self) -> Result<(String, Span), ParseError> {
        let token = self.expect(TokenKind::Ident)?;
        let name = self.text(token.span)?;
        if name.is_empty() {
            return Err(error::unreadable_text(token.span));
        }
        Ok((name, token.span))
    }

    // -- the unclosed-delimiter stack ---------------------------------------

    /// Records that a delimiter has been opened, and returns its index.
    ///
    /// # Errors
    ///
    /// [`error::nested_too_deep`] past [`MAX_NESTING_DEPTH`]. The opener is
    /// *not* recorded in that case, so the stack and the cursor stay in step and
    /// error recovery has a consistent picture to unwind.
    pub fn push_open(&mut self, kind: TokenKind, span: Span) -> Result<usize, ParseError> {
        if self.open.len() >= MAX_NESTING_DEPTH {
            return Err(error::nested_too_deep(span));
        }
        self.open.push(Open { kind, span });
        Ok(self.open.len().saturating_sub(1))
    }

    /// Consumes a closer and pops the opener it closes.
    ///
    /// A closer that matches nothing, or that matches something other than the
    /// innermost opener -- `fn f() { }` inside a `(`, say -- leaves the stack
    /// alone. Popping on a mismatch would make the stack describe delimiters
    /// that are not the ones actually open, and the unclosed-delimiter
    /// diagnostic would then point at a brace the reader already closed.
    pub fn close_open(&mut self, closer: TokenKind) {
        let matches = self
            .open
            .last()
            .map(|open| error::closer_of(open.kind) == closer)
            .unwrap_or(false);
        if matches {
            self.open.pop();
        }
    }

    /// The innermost delimiter still open, clearing the stack.
    ///
    /// Returns `None` when everything that was opened has been closed. The
    /// stack is cleared rather than popped once because at the end of the file
    /// *every* open delimiter is unclosed, and reporting a thousand of them is
    /// noise where reporting the innermost one is a fix.
    pub fn take_unclosed(&mut self) -> Option<ParseError> {
        let open = self.open.pop()?;
        self.open.clear();
        Some(error::unclosed(open.span, open.kind))
    }

    /// How many delimiters are currently open. Always zero between items.
    pub fn open_depth(&self) -> usize {
        self.open.len()
    }

    // -- items --------------------------------------------------------------

    /// Parses every item in the file.
    ///
    /// # Errors
    ///
    /// Every diagnostic found, in the order they were found. Parsing does not
    /// stop at the first one: a user with three typos should be shown three
    /// typos, not three rounds of one typo each. What it does stop at is the end
    /// of the file, and after every failure it skips forward to something that
    /// can begin an item, so one broken declaration cannot report itself a
    /// hundred times over.
    ///
    /// An unclosed `{` is reported once, at the brace, with the end of the file
    /// named as the reason -- see [`take_unclosed`](Parser::take_unclosed).
    pub fn parse_program(&mut self) -> Result<Program, Vec<Diagnostic>> {
        let mut items = Vec::new();
        let mut diags = DiagnosticBag::new();

        while !self.at_eof() {
            let before = self.pos;
            let depth = self.open_depth();
            match self.parse_item() {
                Ok(item) => {
                    items.push(item);
                    // Nothing in `parse_item` can succeed without consuming the
                    // keyword it dispatched on, so this is belt and braces --
                    // but it is the assertion that makes "this loop cannot spin"
                    // true of the loop rather than of every function in it.
                    if self.pos == before {
                        self.advance();
                    }
                }
                Err(e) => {
                    diags.push(e.to_diagnostic(self.file));
                    self.recover(depth, before);
                }
            }
        }

        if let Some(e) = self.take_unclosed() {
            diags.push(e.to_diagnostic(self.file));
        }

        if diags.error_count() == 0 {
            Ok(Program { items })
        } else {
            Err(diags.into_vec())
        }
    }

    /// Skips forward to the next token that can begin an item, or to `Eof`.
    ///
    /// Three things happen here and all of them are load-bearing.
    ///
    /// The delimiter stack is cut back to `depth`, the depth the item started
    /// at: a declaration that failed half way through has not really opened a
    /// brace, and leaving its opener on the stack would have the end of the file
    /// accuse a brace the reader already closed.
    ///
    /// If the failed item consumed nothing at all, one token is forced forward
    /// before the search begins. Without that, an item whose *keyword* is the
    /// mistake would be reported forever: `extern` on its own is rejected
    /// without reading a single token, the cursor would still be on `extern`,
    /// `extern` is in the set this function resumes on, and nothing would ever
    /// move.
    ///
    /// Otherwise the search starts where the failure left off, so a broken item
    /// followed by a whole valid one -- `import import a;` -- still parses the
    /// valid one rather than discarding it.
    fn recover(&mut self, depth: usize, before: usize) {
        self.open.truncate(depth);
        if self.pos == before {
            let _ = self.advance();
        }
        while !self.at_eof() && !starts_item(self.peek()) {
            let _ = self.advance();
        }
    }

    // parse_block is implemented in stmts.rs

    // -- internals ----------------------------------------------------------

    /// The token a stream that has run out reports: `Eof`, at the end of the
    /// last real token.
    ///
    /// An empty stream has no last token, and a synthetic span is the honest
    /// answer for "there is no source here at all" -- it renders without a
    /// caret rather than pointing at a character that does not exist.
    fn end_of_file(&self) -> Token {
        match self.tokens.last() {
            Some(token) => Token {
                kind: TokenKind::Eof,
                span: Span::new(token.span.end, token.span.end),
            },
            None => Token {
                kind: TokenKind::Eof,
                span: Span::SYNTHETIC,
            },
        }
    }
}

/// Whether `kind` can begin an item, which is where error recovery resumes.
fn starts_item(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Import | TokenKind::Struct | TokenKind::Enum | TokenKind::Fn | TokenKind::Extern
    )
}

/// The span covering both `left` and `right`, whichever way round they are.
///
/// A parser should only ever ask this of spans that are in source order, and
/// almost always is; `min`/`max` rather than "first then second" because a
/// reversed span would otherwise render as an empty range at the wrong end,
/// which looks like a caret on the wrong character rather than like a bug.
pub(crate) fn cover(left: Span, right: Span) -> Span {
    Span::new(left.start.min(right.start), left.end.max(right.end))
}

#[cfg(test)]
mod tests {
    use otter_lexer::Lexer;

    use super::*;

    /// A parser over `src`, with its source map, which must lex.
    fn over_in<'a>(src: &str, map: &'a mut SourceMap) -> Parser<'a> {
        let file = map.add_file("main.ot", src.to_string());
        let tokens = match Lexer::with_file(src, file).tokenize() {
            Ok(tokens) => tokens,
            Err(diags) => panic!("test source must lex, got {diags:?}"),
        };
        Parser::new(tokens, map, file)
    }

    #[test]
    fn peeking_past_the_end_reports_eof_and_never_moves() {
        let mut map = SourceMap::new();
        let mut parser = over_in("", &mut map);
        assert_eq!(
            parser.peek(),
            TokenKind::Eof,
            "an empty stream is already at the end"
        );
        for _ in 0..5 {
            let token = parser.advance();
            assert_eq!(token.kind, TokenKind::Eof);
        }
        assert!(parser.at_eof());

        // A stream with tokens in it stops at its own `Eof` and stays there,
        // which is what makes "loop until Eof" a terminating loop.
        let mut map = SourceMap::new();
        let mut parser = over_in("import a;", &mut map);
        let mut seen: usize = 0;
        while !parser.at_eof() {
            parser.advance();
            seen = seen.saturating_add(1);
            assert!(seen < 100, "the loop must terminate");
        }
        assert_eq!(seen, 3, "import, a, ;");
        assert_eq!(
            parser.advance().kind,
            TokenKind::Eof,
            "and stays at the end"
        );
    }

    #[test]
    fn a_stream_with_no_eof_still_terminates() {
        // Hand-built, without the `Eof` the lexer always appends. Reading past
        // the end is a synthetic `Eof`, not an index off the end of the vector.
        let tokens = vec![Token {
            kind: TokenKind::Ident,
            span: Span::new(0, 1),
        }];
        let map = SourceMap::new();
        let mut parser = Parser::new(tokens, &map, FileId::from_index(0));
        assert_eq!(parser.peek(), TokenKind::Ident);
        assert_eq!(parser.advance().kind, TokenKind::Ident);
        assert_eq!(parser.peek(), TokenKind::Eof);
        assert_eq!(
            parser.peek_span(),
            Span::new(1, 1),
            "at the end of the last token"
        );
        assert_eq!(parser.advance().kind, TokenKind::Eof);
        assert_eq!(parser.advance().kind, TokenKind::Eof, "and never past it");
    }

    #[test]
    fn expect_names_both_halves_and_consumes_nothing_when_it_fails() {
        let mut map = SourceMap::new();
        let mut parser = over_in("struct S", &mut map);
        assert_eq!(parser.peek(), TokenKind::Struct);
        let before = parser.peek_span();
        let e = match parser.expect(TokenKind::Import) {
            Ok(_) => panic!("`struct` is not `import`"),
            Err(e) => e,
        };
        assert_eq!(e.message, "expected `import`, found `struct`");
        assert_eq!(e.span, before, "at the token that was found");
        assert_eq!(
            parser.peek(),
            TokenKind::Struct,
            "a failed expect consumes nothing"
        );

        assert!(parser.expect(TokenKind::Struct).is_ok());
        assert_eq!(parser.peek(), TokenKind::Ident);
    }

    #[test]
    fn eat_reports_whether_it_ate_and_only_eats_what_it_was_asked_for() {
        let mut map = SourceMap::new();
        let mut parser = over_in("a.b", &mut map);
        assert!(
            !parser.eat(TokenKind::Dot),
            "`a` is not a `.`, so nothing is eaten"
        );
        assert_eq!(
            parser.peek(),
            TokenKind::Ident,
            "and the failed eat moved nothing"
        );
        assert!(parser.eat(TokenKind::Ident));
        assert!(parser.eat(TokenKind::Dot));
        assert!(parser.eat(TokenKind::Ident), "`b` is eaten");
        assert!(
            !parser.eat(TokenKind::Ident),
            "there is no third identifier"
        );
    }

    #[test]
    fn previous_span_is_the_last_token_consumed() {
        let mut map = SourceMap::new();
        let mut parser = over_in("struct S", &mut map);
        assert_eq!(
            parser.previous_span(),
            Span::SYNTHETIC,
            "nothing consumed yet"
        );
        parser.advance();
        assert_eq!(parser.previous_span(), Span::new(0, 6));
        parser.advance();
        assert_eq!(parser.previous_span(), Span::new(7, 8));
    }

    #[test]
    fn the_delimiter_stack_pops_only_on_a_matching_closer() {
        let mut map = SourceMap::new();
        let mut parser = over_in("", &mut map);
        let brace = Span::new(0, 1);
        let paren = Span::new(1, 2);
        let brace_index = match parser.push_open(TokenKind::LBrace, brace) {
            Ok(index) => index,
            Err(e) => panic!("must push: {}", e.message),
        };
        let _ = parser.push_open(TokenKind::LParen, paren);
        assert_eq!(parser.open_depth(), 2);

        // A closer that matches nothing open leaves the stack alone.
        parser.close_open(TokenKind::RBracket);
        assert_eq!(
            parser.open_depth(),
            2,
            "a bracket closer closes no brace or paren"
        );
        // A closer that does not match the innermost opener leaves it alone too.
        parser.close_open(TokenKind::RBrace);
        assert_eq!(
            parser.open_depth(),
            2,
            "a brace does not close the paren inside the brace"
        );

        parser.close_open(TokenKind::RParen);
        assert_eq!(parser.open_depth(), 1);
        parser.close_open(TokenKind::RBrace);
        assert_eq!(
            parser.open_depth(),
            0,
            "everything that was opened is closed"
        );
        assert_eq!(brace_index, 0);

        parser.close_open(TokenKind::RBrace);
        assert_eq!(
            parser.open_depth(),
            0,
            "closing what is not open is not a pop"
        );
    }

    #[test]
    fn an_unclosed_delimiter_is_reported_once_at_the_innermost_opener() {
        let mut map = SourceMap::new();
        let mut parser = over_in("", &mut map);
        assert!(
            parser.take_unclosed().is_none(),
            "nothing open, nothing to report"
        );
        let _ = parser.push_open(TokenKind::LBrace, Span::new(0, 1));
        let _ = parser.push_open(TokenKind::LBrace, Span::new(4, 5));
        let e = match parser.take_unclosed() {
            Some(e) => e,
            None => panic!("two braces are open"),
        };
        assert_eq!(
            e.span,
            Span::new(4, 5),
            "the innermost opener is the one to close"
        );
        assert!(e.message.contains("unclosed"), "{}", e.message);
        assert_eq!(
            parser.open_depth(),
            0,
            "the stack is cleared, so it cannot be reported twice"
        );
        assert!(parser.take_unclosed().is_none());
    }

    #[test]
    fn nesting_past_the_limit_is_a_diagnostic_not_a_deeper_stack() {
        let mut map = SourceMap::new();
        let mut parser = over_in("", &mut map);
        for depth in 0..MAX_NESTING_DEPTH {
            let _ = parser.push_open(TokenKind::LBrace, Span::new(0, 1));
            assert_eq!(parser.open_depth(), depth.saturating_add(1));
        }
        let e = match parser.push_open(TokenKind::LBrace, Span::new(9, 10)) {
            Ok(_) => panic!("the limit must be enforced"),
            Err(e) => e,
        };
        assert!(e.message.contains("nested"), "{}", e.message);
        assert_eq!(
            parser.open_depth(),
            MAX_NESTING_DEPTH,
            "the rejected opener is not recorded"
        );
    }

    #[test]
    fn a_span_that_is_not_source_text_is_a_diagnostic_not_a_name() {
        let mut map = SourceMap::new();
        let parser = over_in("a", &mut map);
        assert_eq!(parser.text(Span::new(0, 1)), Ok("a".to_string()));
        assert_eq!(
            parser.text(Span::new(0, 9999)),
            Err(error::unreadable_text(Span::new(0, 9999))),
            "a span past the end of the file reads as nothing, not as a panic"
        );
        // An empty span *is* readable -- it names no text, which is a legitimate
        // reading -- so it is `identifier`, not `text`, that refuses to turn one
        // into an empty identifier.
        assert_eq!(parser.text(Span::SYNTHETIC), Ok(String::new()));
    }

    #[test]
    fn a_block_reports_the_span_of_its_braces_and_stops_at_the_matching_closer() {
        let mut map = SourceMap::new();
        let mut parser = over_in("{ } fn", &mut map);
        let block = match parser.parse_block() {
            Ok(block) => block,
            Err(e) => panic!("must parse a block: {}", e.message),
        };
        assert_eq!(
            block.span,
            Span::new(0, 3),
            "both braces are inside the span"
        );
        assert_eq!(
            parser.open_depth(),
            0,
            "the stack is empty after a closed block"
        );
        assert_eq!(
            parser.peek(),
            TokenKind::Fn,
            "and the tokens after the body are untouched"
        );
    }

    #[test]
    fn a_block_whose_body_holds_delimiters_stops_at_its_own_closer() {
        // The `}` inside the body closes the `(`; the one after closes the block.
        let mut map = SourceMap::new();
        let mut parser = over_in("{ }", &mut map);
        match parser.parse_block() {
            Ok(block) => assert_eq!(block.span, Span::new(0, 3)),
            Err(e) => panic!("must parse: {}", e.message),
        }
        assert!(parser.at_eof(), "the whole thing was the block");

        let mut map = SourceMap::new();
        let mut parser = over_in("{ ( }", &mut map);
        // The `{` after `(` is closed by the first `}`; the second `}` does not
        // close the `(`, so it closes nothing. The file then ends with two
        // delimiters still open, and the innermost one is what to report.
        match parser.parse_block() {
            Ok(_) => panic!("the closing brace was consumed closing the paren"),
            Err(e) => {
                assert_eq!(
                    e.message,
                    "expected an expression"
                );
                assert_eq!(e.span, Span::new(4, 5), "at the `(` that was left open");
            }
        }
    }

    #[test]
    fn an_empty_program_is_a_program_with_no_items() {
        let mut map = SourceMap::new();
        let mut parser = over_in("", &mut map);
        match parser.parse_program() {
            Ok(program) => assert!(program.items.is_empty()),
            Err(diags) => panic!("an empty file is valid: {diags:?}"),
        }
    }

    #[test]
    fn a_construct_that_cannot_start_an_item_says_so() {
        assert!(starts_item(TokenKind::Import));
        assert!(starts_item(TokenKind::Struct));
        assert!(starts_item(TokenKind::Enum));
        assert!(starts_item(TokenKind::Fn));
        assert!(starts_item(TokenKind::Extern));
        // Statement keywords are not item keywords: resuming on one would let a
        // broken function body swallow the rest of the file as statements.
        for kind in [
            TokenKind::Const,
            TokenKind::Var,
            TokenKind::Return,
            TokenKind::If,
            TokenKind::Else,
            TokenKind::For,
            TokenKind::Defer,
        ] {
            assert!(!starts_item(kind), "{kind:?} must not resume recovery");
        }
    }
}
