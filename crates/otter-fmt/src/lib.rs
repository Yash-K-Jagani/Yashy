//! The canonical formatter.
//!
//! `format` reprints a source file with one canonical spacing and indentation.
//! It never reorders, adds, or removes a token, and it never rewrites the text
//! inside a literal: every token is printed from its own source span, so a
//! `"${x +  1}"` keeps its interior spacing and an escape keeps its bytes.
//!
//! # Where comments live
//!
//! The lexer drops trivia, so comments are not in the token stream at all.
//! They are recovered from the **gaps** between tokens: because the token spans
//! tile the file, every byte between two tokens is whitespace or a comment, and
//! a comment scanner can read those bytes without ever mistaking a `//` inside
//! a string for a comment. That is why the printer walks tokens and gaps
//! rather than the AST: the AST has no comments, and a formatter that drops
//! them is worse than none.
//!
//! # The shape rules
//!
//! - A `{` ends its line and indents; a `}` starts its own line and dedents.
//! - A `;` ends its line. At the top level a statement or item is followed by
//!   one blank line, so items do not run together.
//! - Blank lines in the source survive, collapsed to one.
//! - A comment that begins on the same line as code stays there; any other
//!   comment gets its own line at the current indent.
//!
//! A file that does not parse is reported and left alone: formatting is only
//! offered for input the compiler can read.

use otter_common::{Diagnostic, SourceMap};
use otter_lexer::{Lexer, Token, TokenKind};
use otter_parser::Parser;

/// One indentation step, in spaces.
const INDENT: &str = "    ";

/// Format `src`, or return the diagnostics that stopped it.
///
/// Leading, trailing, and internal comment text is preserved exactly; only the
/// whitespace around tokens is regenerated.
pub fn format(src: &str) -> Result<String, Vec<Diagnostic>> {
    let tokens = Lexer::new(src).tokenize()?;
    // Parse for validity only. The printer needs the token stream, not the AST,
    // but a file that does not parse must be reported instead of reformatted.
    let mut sources = SourceMap::new();
    let file = sources.add_file("<otter fmt>", src.to_string());
    Parser::new(tokens.clone(), &sources, file).parse_program()?;
    Ok(Printer::new(src).print(&tokens))
}

/// The line break the previous token demands before the next one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Break {
    /// None: the next token continues the current line.
    None,
    /// One newline.
    Line,
    /// Two newlines, which renders as one blank line.
    Blank,
}

impl Break {
    fn newlines(self) -> u8 {
        match self {
            Break::None => 0,
            Break::Line => 1,
            Break::Blank => 2,
        }
    }
}

/// A token-stream printer with an output buffer and the state spacing needs.
struct Printer<'a> {
    src: &'a str,
    out: String,
    indent: usize,
    /// The break owed before the next piece, raised by the source's own
    /// newlines and by the canonical rules, never lowered by either.
    line_break: Break,
    /// The last token printed, for the spacing rule. Comments do not clear it:
    /// a comment is a gap, and the tokens on either side still have to be
    /// spaced as if it were not there.
    prev: Option<TokenKind>,
    /// The `{` just printed is immediately followed by `}`.
    compact_brace: bool,
}

impl<'a> Printer<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            out: String::new(),
            indent: 0,
            line_break: Break::None,
            prev: None,
            compact_brace: false,
        }
    }

    /// Print every token (and everything between them) and end with one
    /// newline.
    fn print(mut self, tokens: &[Token]) -> String {
        let src = self.src;
        let mut end = 0;
        for (index, token) in tokens.iter().enumerate() {
            if token.kind == TokenKind::Eof {
                break;
            }
            let start = token.span.start as usize;
            let gap = &src[end..start];
            // `} else` keeps the source's own choice: with no comment and no
            // newline between them, the brace's break is cancelled so the two
            // words share a line. A source that put them on separate lines
            // keeps that too, through the gap below.
            if token.kind == TokenKind::Else
                && !gap.contains('\n')
                && !gap.contains("//")
                && !gap.contains("/*")
            {
                self.line_break = Break::None;
            }
            self.trivia(gap);
            self.token(token, tokens.get(index + 1).map(|next| next.kind));
            end = token.span.end as usize;
        }
        // Comments after the last token are in the final gap.
        self.trivia(&src[end..]);
        while self.out.ends_with(' ') || self.out.ends_with('\t') {
            self.out.pop();
        }
        if !self.out.ends_with('\n') {
            self.out.push('\n');
        }
        self.out
    }

    /// Read the whitespace and comments between two tokens.
    fn trivia(&mut self, gap: &str) {
        let bytes = gap.as_bytes();
        let mut index = 0;
        let mut newlines: u8 = 0;
        let mut seen_comment = false;
        while index < bytes.len() {
            match bytes[index] {
                b'\n' => {
                    newlines = newlines.saturating_add(1);
                    index += 1;
                }
                b'/' if bytes.get(index + 1) == Some(&b'/') => {
                    let start = index;
                    while index < bytes.len() && bytes[index] != b'\n' {
                        index += 1;
                    }
                    let text = gap[start..index].trim_end();
                    self.comment(text, newlines, seen_comment);
                    seen_comment = true;
                    newlines = 0;
                }
                b'/' if bytes.get(index + 1) == Some(&b'*') => {
                    let start = index;
                    index += 2;
                    let mut depth = 1usize;
                    while index < bytes.len() && depth > 0 {
                        if bytes[index] == b'/'
                            && bytes.get(index + 1) == Some(&b'*')
                        {
                            depth += 1;
                            index += 2;
                        } else if bytes[index] == b'*'
                            && bytes.get(index + 1) == Some(&b'/')
                        {
                            depth -= 1;
                            index += 2;
                        } else {
                            index += 1;
                        }
                    }
                    let text = &gap[start..index];
                    self.comment(text, newlines, seen_comment);
                    seen_comment = true;
                    newlines = 0;
                }
                _ => index += 1,
            }
        }
        // Whitespace after the last comment (or the whole gap) is the break the
        // next token owes.
        if newlines >= 2 {
            self.raise(Break::Blank);
        } else if newlines == 1 {
            self.raise(Break::Line);
        }
    }

    /// Place one comment. `newlines` counts the newlines between the previous
    /// token (or comment) and this one; zero means the comment trails code.
    fn comment(&mut self, text: &str, newlines: u8, seen_comment: bool) {
        let at_line_start = self.out.is_empty() || self.out.ends_with('\n');
        let trailing = newlines == 0 && !seen_comment && !at_line_start;
        if trailing {
            if !self.out.ends_with(' ') {
                self.out.push(' ');
            }
            self.out.push_str(text);
        } else {
            if newlines >= 2 {
                self.raise(Break::Blank);
            } else if newlines == 1 || seen_comment {
                self.raise(Break::Line);
            }
            if self.out.is_empty() {
                self.line_break = Break::None;
            }
            self.write_break();
            self.out.push_str(text);
        }
        // A line comment ends its line whatever follows; a block comment does
        // not, so the next token is spaced as usual. A break that was already
        // owed (a `;` before a trailing block comment) is kept, so the code
        // after it still starts on a fresh line.
        if text.starts_with("//") {
            self.line_break = Break::Line;
        }
    }

    fn raise(&mut self, line_break: Break) {
        if line_break > self.line_break {
            self.line_break = line_break;
        }
    }

    /// Print one token, after the break and spacing rules decide what precedes
    /// it.
    fn token(&mut self, token: &Token, next: Option<TokenKind>) {
        let text = &self.src[token.span.start as usize..token.span.end as usize];
        // `{}` stays on one line: it is a container with nothing in it, and
        // splitting it would put the closing brace under the opening one.
        let compact = token.kind == TokenKind::RBrace && self.compact_brace;
        if compact {
            // `{}` was split in the source; the canonical form joins it.
            self.line_break = Break::None;
        } else if token.kind == TokenKind::Semi {
            // A semicolon attaches to whatever precedes it, even a brace.
            self.line_break = Break::None;
        } else if token.kind == TokenKind::RBrace && !compact {
            self.indent = self.indent.saturating_sub(1);
            self.raise(Break::Line);
        }
        if self.line_break != Break::None {
            self.write_break();
        } else if !self.out.is_empty() && !self.out.ends_with([' ', '\n']) {
            if let Some(prev) = self.prev {
                // `*mut i64` is the pointer spelling; `mut` is an identifier
                // here, not a keyword, so the spacing rule needs the text.
                let pointer_qualifier =
                    token.kind == TokenKind::Ident && text == "mut" && prev == TokenKind::Star;
                if !pointer_qualifier && needs_space(prev, token.kind) {
                    self.out.push(' ');
                }
            }
        }
        self.out.push_str(text);
        match token.kind {
            TokenKind::LBrace if next == Some(TokenKind::RBrace) => {
                self.compact_brace = true;
                self.line_break = Break::None;
            }
            TokenKind::LBrace => {
                self.indent += 1;
                self.line_break = Break::Line;
            }
            TokenKind::Semi => {
                self.line_break = top_level_or_line(self.indent);
            }
            TokenKind::RBrace if compact => {
                self.compact_brace = false;
                self.line_break = Break::None;
            }
            TokenKind::RBrace => {
                self.line_break = top_level_or_line(self.indent);
            }
            _ => {
                self.line_break = Break::None;
            }
        }
        self.prev = Some(token.kind);
    }

    /// Write the owed break: newlines up to the requested count, then the
    /// current indent. Trailing spaces are removed first so a blank line has no
    /// invisible content.
    fn write_break(&mut self) {
        let wanted = self.line_break.newlines();
        if wanted == 0 {
            return;
        }
        while self.out.ends_with(' ') || self.out.ends_with('\t') {
            self.out.pop();
        }
        let mut existing = 0u8;
        for byte in self.out.bytes().rev() {
            if byte == b'\n' {
                existing += 1;
            } else {
                break;
            }
        }
        for _ in existing..wanted {
            self.out.push('\n');
        }
        for _ in 0..self.indent {
            self.out.push_str(INDENT);
        }
        self.line_break = Break::None;
    }
}

/// A top-level item or statement is followed by one blank line; anything
/// nested is followed by one newline.
fn top_level_or_line(indent: usize) -> Break {
    if indent == 0 {
        Break::Blank
    } else {
        Break::Line
    }
}

/// Whether one space belongs between two adjacent tokens.
///
/// The default is a space, because most operator and word pairs read better
/// with one. The exceptions are the pairs that bind: no space before `;`, `,`,
/// `)`, `]`, `.`, `:`, or `..`; none after `(`, `[`, `.`, `..`, or `!`; none
/// before `(` or `[`, so a call and a subscript stay attached to their name;
/// none after `]`, so `[]i64` and `map[K]V` print as written.
fn needs_space(prev: TokenKind, cur: TokenKind) -> bool {
    use TokenKind::*;
    if matches!(prev, Colon | Comma) {
        return true;
    }
    if matches!(cur, Comma | Semi | RParen | RBracket | Dot | DotDot | Colon) {
        return false;
    }
    if matches!(prev, LParen | LBracket | Dot | DotDot | Bang) {
        return false;
    }
    // A call or a subscript binds to the name before it, but an opening
    // bracket after an operator is a parenthesised expression and wants air.
    if matches!(cur, LParen | LBracket) {
        return !matches!(prev, Ident | RParen | RBracket | Question);
    }
    // `[]i64` and `map[K]V` print as written.
    if matches!(prev, RBracket) && matches!(cur, Ident) {
        return false;
    }
    // `{}` is one token pair, not two lines.
    if matches!((prev, cur), (LBrace, RBrace)) {
        return false;
    }
    // A lambda's bars bind to its parameter list: `|x: i64|`. The bar before
    // the name takes no space, and the bar that closes the list takes none
    // either. The arrow after the closing bar still gets one, because it is
    // not part of the list.
    if matches!(prev, Pipe) && matches!(cur, Ident) {
        return false;
    }
    if matches!(cur, Pipe) && matches!(prev, Ident | RBracket) {
        return false;
    }
    true
}
