//! The token vocabulary, the tokens that carry it, and the decoding of string
//! literals.
//!
//! A [`Token`] is a [`TokenKind`] and the half-open byte range of the source
//! text it stands for. It deliberately carries *no value*: an `I64` token does
//! not hold the number, a `Str` token does not hold the string. The parser
//! reads the value back out of the span with [`str::get`], which keeps the
//! value and the text it came from impossible to disagree, keeps `TokenKind`
//! `Copy` so a parser can hold a lookahead cheaply, and means a literal that
//! does not fit its type (`i64` overflow) is reported by the stage that gives
//! it a type rather than by the lexer.
//!
//! The cost of that choice is that every stage past the lexer must have the
//! source text available to interpret a token. [`Lexer`] hands out spans into
//! the string it was given, and the parser is given the same string, so that is
//! true by construction rather than by convention.

use otter_common::{Diagnostic, FileId, Span};

use crate::error::{self, LitError};

/// The `i64` an `I64` token stands for.
///
/// `src` is the whole source text the token's span indexes into, `span` is the
/// token's own span -- `token.span`, sliced out of the same string the lexer
/// was given -- and `file` is the [`FileId`] of that text.
///
/// This exists because a [`Token`] carries no value. Without it, every reader
/// of an `I64` token writes `src[span].parse::<i64>().unwrap()`, and that
/// panics on `99999999999999999999999` -- a number a user can type by accident,
/// and the one input this whole crate exists not to crash on. `parse` already
/// returns a `Result`; this turns it into a [`Diagnostic`] so the failure is
/// reportable rather than fatal.
///
/// # Why `file` is not optional
///
/// There is deliberately no two-argument form. A [`FileId`] is an index into one
/// particular [`otter_common::SourceMap`] and means nothing without it, so a
/// default of file zero is right only when the source really is file zero --
/// which a caller reading from a map cannot know. A default that is silently
/// wrong puts the caret in an unrelated file and sends the reader hunting
/// through source that has nothing to do with the mistake.
///
/// This is the same hazard [`Lexer::new`] versus [`Lexer::with_file`] already
/// has, and it is handled the same way: one name, the argument required, and the
/// doc saying so. A second, differently-named way to spell the same call is how
/// the footgun comes back.
///
/// # Errors
///
/// A diagnostic against `file` when the span does not name digits, or names
/// digits too large for an `i64`. The span is the one that was passed in, so
/// the caret covers the literal the user wrote.
pub fn literal_as_i64(src: &str, span: Span, file: FileId) -> Result<i64, Diagnostic> {
    // `usize::try_from` cannot fail for a `u32` on any platform this runs on,
    // and a saturated `usize::MAX` is out of range for `str::get` anyway, so
    // an impossible offset degrades to a diagnostic.
    let start = usize::try_from(span.start).unwrap_or(usize::MAX);
    let end = usize::try_from(span.end).unwrap_or(usize::MAX);
    let text = match src.get(start..end) {
        // An empty span names no text at all, and `parse` would say the same
        // thing about it -- but "expected an integer literal" is the useful
        // half of that answer.
        Some(text) if !text.is_empty() => text,
        _ => return Err(error::invalid_integer_literal(file, span)),
    };
    if !text.bytes().all(|byte| byte.is_ascii_digit()) {
        // The lexer never emits a span covering anything but digits, so this is
        // a caller that sliced the wrong range -- a diagnostic is still better
        // than handing back a number read out of the middle of a string.
        return Err(error::invalid_integer_literal(file, span));
    }
    match text.parse::<i64>() {
        Ok(value) => Ok(value),
        Err(_) => Err(error::integer_literal_out_of_range(file, span)),
    }
}

/// A string literal with its escape sequences decoded.
///
/// The brief calls for both a `String` and a `Vec<u8>` of the literal's text.
/// They are the same text -- `bytes` is `value.as_bytes()` -- and are both
/// present because the stages that consume a literal want different things: the
/// AST and diagnostics want `value`, and code generation wants bytes it can put
/// straight into an LLVM IR string constant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StringLit {
    /// The literal's text with escape sequences decoded.
    pub value: String,
    /// The same decoded text as bytes, for stages that emit byte strings.
    pub bytes: Vec<u8>,
}

impl StringLit {
    /// Decodes the source text of a `Str` token into the literal it stands for.
    ///
    /// `raw` is the token's source text, quotes included, which is exactly
    /// `source[span.start..span.end]` for that token.
    ///
    /// This is the *only* implementation of what an escape sequence means: the
    /// lexer calls it to decide whether a literal is well formed, and the
    /// parser calls it to get the value. Two implementations would be two
    /// chances to disagree about whether `"a\nb"` is four bytes or five.
    ///
    /// # Errors
    ///
    /// Returns a [`LitError`] whose `offset` is relative to `raw`. A token the
    /// lexer has already accepted never fails here; the error path exists so
    /// that a caller which has only raw text -- the parser, holding a `Str`
    /// token from somewhere else -- reports rather than guesses.
    pub fn decode(raw: &str) -> Result<StringLit, LitError> {
        let bytes = raw.as_bytes();
        // The quotes are part of the token's span but not of its text. Accepting
        // a `raw` without them costs nothing and keeps the function usable on
        // hand-written input, but the lexer always passes the full token.
        let mut at = if bytes.first() == Some(&b'"') { 1 } else { 0 };
        let mut out: Vec<u8> = Vec::new();

        loop {
            let current = match bytes.get(at) {
                Some(byte) => *byte,
                None => {
                    return Err(LitError::new(at, "the text ends before the closing quote"));
                }
            };
            match current {
                b'"' => break,
                b'\\' => {
                    let escaped = match bytes.get(at.saturating_add(1)) {
                        Some(byte) => *byte,
                        None => {
                            return Err(LitError::new(at, "the text ends after a backslash"));
                        }
                    };
                    out.push(decode_escape(escaped).ok_or_else(|| {
                        LitError::new(at, format!("unknown escape `\\{}`", printable_byte(escaped)))
                    })?);
                    at = at.saturating_add(2);
                }
                byte => {
                    // A multi-byte character is copied byte by byte, which keeps
                    // it intact: the input was a `&str`, so its bytes are
                    // already valid UTF-8 in valid positions.
                    out.push(byte);
                    at = at.saturating_add(1);
                }
            }
        }

        // Valid by construction -- the bytes came from a `&str` and only ASCII
        // was ever inserted -- but a `Result` here costs nothing and rules out
        // an `unwrap` in a component whose whole job is not crashing.
        let value = String::from_utf8(out).map_err(|e| {
            LitError::new(0, format!("the literal is not valid UTF-8: {}", e.utf8_error()))
        })?;
        let bytes = value.as_bytes().to_vec();
        Ok(StringLit { value, bytes })
    }
}

/// The byte an escape sequence stands for, or `None` if Otter has no such
/// escape.
///
/// The set is deliberately small and closed: a backslash followed by anything
/// else is a diagnostic rather than a guess, because silently passing `\d`
/// through as `\d` is how a language grows a meaning nobody agreed to.
fn decode_escape(escaped: u8) -> Option<u8> {
    match escaped {
        b'n' => Some(b'\n'),
        b'r' => Some(b'\r'),
        b't' => Some(b'\t'),
        b'0' => Some(0),
        b'\\' => Some(b'\\'),
        b'"' => Some(b'"'),
        b'\'' => Some(b'\''),
        _ => None,
    }
}

/// A byte shown the way it would have to be typed, for a diagnostic message.
fn printable_byte(byte: u8) -> String {
    match byte {
        b'\n' => "\\n".to_string(),
        b'\r' => "\\r".to_string(),
        b'\t' => "\\t".to_string(),
        b'\\' => "\\\\".to_string(),
        _ if byte.is_ascii_graphic() || byte == b' ' => (byte as char).to_string(),
        _ => format!("\\x{byte:02x}"),
    }
}

/// One token: what it is, and exactly where in the source it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    /// The half-open byte range `[start, end)` of this token's source text.
    ///
    /// For `Eof` the span is empty and sits at the end of the file, which is
    /// what lets a parser report an unclosed delimiter with a caret under the
    /// end of the last line.
    pub span: Span,
}

/// Every kind of token Otter has.
///
/// The keywords come first, then punctuation, then the classes of value that
/// carry their text in their span rather than in the token. `true` and `false`
/// are keywords, not identifiers: they are part of the grammar, not names the
/// program can shadow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenKind {
    // Keywords.
    Fn,
    Struct,
    Enum,
    Const,
    Var,
    Import,
    Extern,
    Unsafe,
    Defer,
    Return,
    Break,
    Continue,
    If,
    Else,
    For,
    While,
    In,
    Switch,
    Case,
    Default,
    /// `as`, for a cast: `x as f64`.
    ///
    /// Spec §3.1 lists it among the keywords and Task 6 parses casts, so
    /// without it `x as f64` lexes as three identifiers.
    As,
    /// `move`, for an ownership transfer: `move buf`.
    ///
    /// Spec §3.1 lists it too. The memory model it belongs to arrives in M3,
    /// but a keyword added later is a breaking change to every file that uses
    /// `move` as a name, so it is a keyword now.
    Move,
    True,
    False,

    // Punctuation. `ColonEq`, `DotDot`, `Question`, `Bang`, `AndAnd` and `OrOr`
    // are additions to the punctuation the interface first named: `:=` is the
    // `if`-condition binding, `..` the range operator, `?` the postfix
    // try, `!` and `&&`/`||` the boolean operators the expression grammar
    // needs. Every one of them is written somewhere in the language, and a
    // token kind that cannot be produced is worse than one that was not
    // planned -- the parser would have no way to read `for i in 0..10`.
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Semi,
    Colon,
    ColonEq,
    Dot,
    DotDot,
    Arrow,
    At,
    Question,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Eq,
    EqEq,
    Bang,
    BangEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    AndAnd,
    OrOr,

    // Everything whose text is its value.
    Ident,
    I64,
    F64,
    Str,

    /// The end of the file. Always the last token.
    Eof,
}

impl TokenKind {
    /// The keyword `text` names, or `None` if it is an ordinary identifier.
    ///
    /// The comparison is over the whole of `text`. A match on a prefix would
    /// turn `fnx` into `fn` and leave the tail of the identifier unlexed,
    /// which is a bug the tests pin by lexing `fn fnx` and expecting two
    /// different kinds of token.
    pub fn keyword(text: &str) -> Option<TokenKind> {
        Some(match text {
            "fn" => TokenKind::Fn,
            "struct" => TokenKind::Struct,
            "enum" => TokenKind::Enum,
            "const" => TokenKind::Const,
            "var" => TokenKind::Var,
            "import" => TokenKind::Import,
            "extern" => TokenKind::Extern,
            "unsafe" => TokenKind::Unsafe,
            "defer" => TokenKind::Defer,
            "return" => TokenKind::Return,
            "break" => TokenKind::Break,
            "continue" => TokenKind::Continue,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "for" => TokenKind::For,
            "while" => TokenKind::While,
            "in" => TokenKind::In,
            "switch" => TokenKind::Switch,
            "case" => TokenKind::Case,
            "default" => TokenKind::Default,
            "as" => TokenKind::As,
            "move" => TokenKind::Move,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            _ => return None,
        })
    }

    /// True for a keyword, false for punctuation, values and `Eof`.
    pub fn is_keyword(&self) -> bool {
        matches!(
            self,
            TokenKind::Fn
                | TokenKind::Struct
                | TokenKind::Enum
                | TokenKind::Const
                | TokenKind::Var
                | TokenKind::Import
                | TokenKind::Extern
                | TokenKind::Unsafe
                | TokenKind::Defer
                | TokenKind::Return
                | TokenKind::Break
                | TokenKind::Continue
                | TokenKind::If
                | TokenKind::Else
                | TokenKind::For
                | TokenKind::While
                | TokenKind::In
                | TokenKind::Switch
                | TokenKind::Case
                | TokenKind::Default
                | TokenKind::As
                | TokenKind::Move
                | TokenKind::True
                | TokenKind::False
        )
    }

    /// True for `Eof` only.
    pub fn is_eof(&self) -> bool {
        matches!(self, TokenKind::Eof)
    }

    /// How a kind is written, when there is no single spelling to write.
    ///
    /// [`TokenKind::spelling`] and [`TokenKind::describe`] are both views of
    /// one exhaustive table. Keeping them separate meant each had to assume
    /// what the other did -- one carried a fallback for a kind "that cannot
    /// happen", which is an assumption the compiler cannot check and a reader
    /// has to verify by eye. Here there is no assumption: every kind is written
    /// down once, and adding one to the enum is a compile error until it is.
    fn naming(&self) -> Naming {
        match self {
            TokenKind::Fn => Naming::Fixed("fn"),
            TokenKind::Struct => Naming::Fixed("struct"),
            TokenKind::Enum => Naming::Fixed("enum"),
            TokenKind::Const => Naming::Fixed("const"),
            TokenKind::Var => Naming::Fixed("var"),
            TokenKind::Import => Naming::Fixed("import"),
            TokenKind::Extern => Naming::Fixed("extern"),
            TokenKind::Unsafe => Naming::Fixed("unsafe"),
            TokenKind::Defer => Naming::Fixed("defer"),
            TokenKind::Return => Naming::Fixed("return"),
            TokenKind::Break => Naming::Fixed("break"),
            TokenKind::Continue => Naming::Fixed("continue"),
            TokenKind::If => Naming::Fixed("if"),
            TokenKind::Else => Naming::Fixed("else"),
            TokenKind::For => Naming::Fixed("for"),
            TokenKind::While => Naming::Fixed("while"),
            TokenKind::In => Naming::Fixed("in"),
            TokenKind::Switch => Naming::Fixed("switch"),
            TokenKind::Case => Naming::Fixed("case"),
            TokenKind::Default => Naming::Fixed("default"),
            TokenKind::As => Naming::Fixed("as"),
            TokenKind::Move => Naming::Fixed("move"),
            TokenKind::True => Naming::Fixed("true"),
            TokenKind::False => Naming::Fixed("false"),
            TokenKind::LParen => Naming::Fixed("("),
            TokenKind::RParen => Naming::Fixed(")"),
            TokenKind::LBrace => Naming::Fixed("{"),
            TokenKind::RBrace => Naming::Fixed("}"),
            TokenKind::LBracket => Naming::Fixed("["),
            TokenKind::RBracket => Naming::Fixed("]"),
            TokenKind::Comma => Naming::Fixed(","),
            TokenKind::Semi => Naming::Fixed(";"),
            TokenKind::Colon => Naming::Fixed(":"),
            TokenKind::ColonEq => Naming::Fixed(":="),
            TokenKind::Dot => Naming::Fixed("."),
            TokenKind::DotDot => Naming::Fixed(".."),
            TokenKind::Arrow => Naming::Fixed("->"),
            TokenKind::At => Naming::Fixed("@"),
            TokenKind::Question => Naming::Fixed("?"),
            TokenKind::Plus => Naming::Fixed("+"),
            TokenKind::Minus => Naming::Fixed("-"),
            TokenKind::Star => Naming::Fixed("*"),
            TokenKind::Slash => Naming::Fixed("/"),
            TokenKind::Percent => Naming::Fixed("%"),
            TokenKind::Eq => Naming::Fixed("="),
            TokenKind::EqEq => Naming::Fixed("=="),
            TokenKind::Bang => Naming::Fixed("!"),
            TokenKind::BangEq => Naming::Fixed("!="),
            TokenKind::Lt => Naming::Fixed("<"),
            TokenKind::LtEq => Naming::Fixed("<="),
            TokenKind::Gt => Naming::Fixed(">"),
            TokenKind::GtEq => Naming::Fixed(">="),
            TokenKind::PlusEq => Naming::Fixed("+="),
            TokenKind::MinusEq => Naming::Fixed("-="),
            TokenKind::StarEq => Naming::Fixed("*="),
            TokenKind::SlashEq => Naming::Fixed("/="),
            TokenKind::PercentEq => Naming::Fixed("%="),
            TokenKind::AndAnd => Naming::Fixed("&&"),
            TokenKind::OrOr => Naming::Fixed("||"),

            // The five kinds whose text is their value have no spelling: there
            // is no one way to write `an identifier`, only many.
            TokenKind::Ident => Naming::Class("an identifier"),
            TokenKind::I64 => Naming::Class("an integer literal"),
            TokenKind::F64 => Naming::Class("a float literal"),
            TokenKind::Str => Naming::Class("a string literal"),
            TokenKind::Eof => Naming::Class("end of file"),
        }
    }

    /// The source text of a fixed token, or `None` for a kind that stands for
    /// a class of text rather than one spelling.
    ///
    /// This is the raw spelling: no quotes, no backticks. A parser that wants
    /// to say `expected (`, got }` uses this; one that wants a finished
    /// sentence uses [`TokenKind::describe`].
    pub fn spelling(&self) -> Option<&'static str> {
        match self.naming() {
            Naming::Fixed(spelling) => Some(spelling),
            Naming::Class(_) => None,
        }
    }

    /// How to name this token in a diagnostic.
    ///
    /// A fixed token is named by its spelling in backticks, and a class of text
    /// by a noun phrase, so that both halves of `expected {}, found {}` read
    /// as a sentence: `expected `(`, found an identifier`.
    pub fn describe(&self) -> String {
        match self.naming() {
            Naming::Fixed(spelling) => format!("`{spelling}`"),
            Naming::Class(phrase) => phrase.to_string(),
        }
    }
}

/// What a kind is called in a diagnostic: one spelling, or a class of text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Naming {
    /// Written exactly like this in source: the keyword, or the operator.
    Fixed(&'static str),
    /// A class of source text, named by a noun phrase. No spelling exists,
    /// because the text is whatever the user wrote.
    Class(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The keyword table, written out here as well as in the integration tests
    /// so that a keyword added to one list and not the other is a failing test
    /// rather than a difference nobody notices.
    const KEYWORDS: [(&str, TokenKind); 24] = [
        ("fn", TokenKind::Fn),
        ("struct", TokenKind::Struct),
        ("enum", TokenKind::Enum),
        ("const", TokenKind::Const),
        ("var", TokenKind::Var),
        ("import", TokenKind::Import),
        ("extern", TokenKind::Extern),
        ("unsafe", TokenKind::Unsafe),
        ("defer", TokenKind::Defer),
        ("return", TokenKind::Return),
        ("break", TokenKind::Break),
        ("continue", TokenKind::Continue),
        ("if", TokenKind::If),
        ("else", TokenKind::Else),
        ("for", TokenKind::For),
        ("while", TokenKind::While),
        ("in", TokenKind::In),
        ("switch", TokenKind::Switch),
        ("case", TokenKind::Case),
        ("default", TokenKind::Default),
        ("as", TokenKind::As),
        ("move", TokenKind::Move),
        ("true", TokenKind::True),
        ("false", TokenKind::False),
    ];

    #[test]
    fn the_keyword_table_is_in_both_directions() {
        for (text, kind) in KEYWORDS {
            assert_eq!(TokenKind::keyword(text), Some(kind), "{text:?} must be a keyword");
            assert_eq!(kind.spelling(), Some(text), "{kind:?} must be spelled {text:?}");
            assert!(kind.is_keyword(), "{text:?} must report itself as a keyword");
        }

        // Nothing else is a keyword. `fnx` is the interesting one: a prefix
        // match would accept it.
        for text in ["fnx", "Fn", "iffy", "trueish", "", " let", "fn "] {
            assert_eq!(TokenKind::keyword(text), None, "{text:?} must not be a keyword");
        }
    }

    #[test]
    fn is_keyword_and_is_eof_agree_with_the_tables() {
        assert!(!TokenKind::Ident.is_keyword());
        assert!(!TokenKind::I64.is_keyword());
        assert!(!TokenKind::F64.is_keyword());
        assert!(!TokenKind::Str.is_keyword());
        assert!(!TokenKind::Eof.is_keyword());
        assert!(!TokenKind::LParen.is_keyword());
        assert!(TokenKind::Eof.is_eof());
        for (_, kind) in KEYWORDS {
            assert!(!kind.is_eof());
        }
        assert!(!TokenKind::Ident.is_eof());
    }

    #[test]
    fn every_kind_is_named_exactly_once_and_no_other_way() {
        // All sixty-four kinds, not a sample of them. `naming` is a match with
        // no catch-all, so a new variant is a compile error until it is named
        // there -- and this test then checks the naming is *sensible*: a fixed
        // spelling must be backticked, and the five value kinds must be named
        // by what they are rather than by a spelling that does not exist.
        let every_kind = [
            TokenKind::Fn,
            TokenKind::Struct,
            TokenKind::Enum,
            TokenKind::Const,
            TokenKind::Var,
            TokenKind::Import,
            TokenKind::Extern,
            TokenKind::Unsafe,
            TokenKind::Defer,
            TokenKind::Return,
            TokenKind::Break,
            TokenKind::Continue,
            TokenKind::If,
            TokenKind::Else,
            TokenKind::For,
            TokenKind::While,
            TokenKind::In,
            TokenKind::Switch,
            TokenKind::Case,
            TokenKind::Default,
            TokenKind::As,
            TokenKind::Move,
            TokenKind::True,
            TokenKind::False,
            TokenKind::LParen,
            TokenKind::RParen,
            TokenKind::LBrace,
            TokenKind::RBrace,
            TokenKind::LBracket,
            TokenKind::RBracket,
            TokenKind::Comma,
            TokenKind::Semi,
            TokenKind::Colon,
            TokenKind::ColonEq,
            TokenKind::Dot,
            TokenKind::DotDot,
            TokenKind::Arrow,
            TokenKind::At,
            TokenKind::Question,
            TokenKind::Plus,
            TokenKind::Minus,
            TokenKind::Star,
            TokenKind::Slash,
            TokenKind::Percent,
            TokenKind::Eq,
            TokenKind::EqEq,
            TokenKind::Bang,
            TokenKind::BangEq,
            TokenKind::Lt,
            TokenKind::LtEq,
            TokenKind::Gt,
            TokenKind::GtEq,
            TokenKind::PlusEq,
            TokenKind::MinusEq,
            TokenKind::StarEq,
            TokenKind::SlashEq,
            TokenKind::PercentEq,
            TokenKind::AndAnd,
            TokenKind::OrOr,
            TokenKind::Ident,
            TokenKind::I64,
            TokenKind::F64,
            TokenKind::Str,
            TokenKind::Eof,
        ];

        for kind in every_kind {
            let described = kind.describe();
            assert!(!described.is_empty(), "{kind:?} has no description");
            match kind.spelling() {
                Some(spelling) => {
                    assert_eq!(described, format!("`{spelling}`"), "{kind:?} must be named by its spelling");
                    assert!(!spelling.is_empty(), "{kind:?} has an empty spelling");
                }
                None => assert!(
                    matches!(
                        described.as_str(),
                        "an identifier" | "an integer literal" | "a float literal" | "a string literal" | "end of file"
                    ),
                    "{kind:?} has no spelling, so it must be named by what it is: {described}"
                ),
            }
        }
    }

    #[test]
    fn describe_reads_as_a_sentence_in_both_halves() {
        assert_eq!(
            format!(
                "expected {}, found {}",
                TokenKind::LParen.describe(),
                TokenKind::Ident.describe()
            ),
            "expected `(`, found an identifier"
        );
    }

    #[test]
    fn a_decoded_string_keeps_its_text_and_bytes_in_step() {
        let lit = match StringLit::decode("\"hi\\nthere\"") {
            Ok(lit) => lit,
            Err(e) => panic!("must decode: {}", e.message),
        };
        assert_eq!(lit.value, "hi\nthere");
        assert_eq!(lit.bytes, lit.value.as_bytes().to_vec());
    }

    #[test]
    fn an_empty_string_decodes_to_nothing() {
        let lit = match StringLit::decode("\"\"") {
            Ok(lit) => lit,
            Err(e) => panic!("must decode: {}", e.message),
        };
        assert_eq!(lit.value, "");
        assert!(lit.bytes.is_empty());
    }

    #[test]
    fn decoding_reports_where_it_stopped_rather_than_guessing() {
        // The offsets are into the text passed in, so the lexer can turn them
        // into a file offset by adding the token's start.
        let cases: [(&str, usize, &str); 4] = [
            ("\"abc", 4, "closing quote"),
            ("\"abc\\", 4, "backslash"),
            ("\"a\\qb\"", 2, "\\q"),
            ("\"", 1, "closing quote"),
        ];
        for (raw, offset, expected_in_message) in cases {
            let err = match StringLit::decode(raw) {
                Ok(lit) => panic!("{raw:?} must not decode, got {:?}", lit.value),
                Err(e) => e,
            };
            assert_eq!(err.offset, offset, "{raw:?}");
            assert!(err.message.contains(expected_in_message), "{raw:?}: {}", err.message);
        }
    }

    #[test]
    fn a_bad_escape_is_reported_at_the_backslash_rather_than_after_it() {
        // The lexer turns this offset into a one-byte span, so a caret lands on
        // the `\` the user typed instead of on the character after it.
        let err = match StringLit::decode("\"a\\qb\"") {
            Ok(lit) => panic!("must not decode, got {:?}", lit.value),
            Err(e) => e,
        };
        assert_eq!(err.offset, 2);
        assert_eq!(err.message, "unknown escape `\\q`");
    }

    #[test]
    fn an_unknown_escape_names_the_character_after_the_backslash() {
        let err = match StringLit::decode("\"a\\qb\"") {
            Ok(lit) => panic!("must not decode, got {:?}", lit.value),
            Err(e) => e,
        };
        assert_eq!(err.message, "unknown escape `\\q`");
    }

    #[test]
    fn escapes_that_look_like_their_own_byte_are_still_escapes() {
        // `\\n` is a backslash followed by `n`, not a newline. If `\\` were
        // decoded by looking at the character after the backslash without
        // consuming it, this would come out as a newline followed by `n`.
        let lit = match StringLit::decode("\"\\\\n\"") {
            Ok(lit) => lit,
            Err(e) => panic!("must decode: {}", e.message),
        };
        assert_eq!(lit.value, "\\n");
    }

    #[test]
    fn a_quote_inside_a_string_ends_it_and_the_rest_is_not_decoded() {
        // The lexer hands over exactly the token's text, so decoding stops at
        // the closing quote rather than complaining about what follows.
        let lit = match StringLit::decode("\"ab\"cd") {
            Ok(lit) => lit,
            Err(e) => panic!("must decode: {}", e.message),
        };
        assert_eq!(lit.value, "ab");
    }

    #[test]
    fn a_multi_byte_character_survives_decoding() {
        let lit = match StringLit::decode("\"héllo\"") {
            Ok(lit) => lit,
            Err(e) => panic!("must decode: {}", e.message),
        };
        assert_eq!(lit.value, "héllo");
        assert_eq!(lit.bytes.len(), 6, "five characters, six bytes");
    }

    #[test]
    fn an_unprintable_escaped_byte_is_shown_as_hex() {
        let err = match StringLit::decode("\"a\\\u{1}b\"") {
            Ok(lit) => panic!("must not decode, got {:?}", lit.value),
            Err(e) => e,
        };
        assert!(err.message.contains("\\x01"), "{}", err.message);
    }
}
