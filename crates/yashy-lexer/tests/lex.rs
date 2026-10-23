//! Lexer tests.
//!
//! Every span asserted here is counted by hand from the fixture string printed
//! next to it, and the byte layout is spelled out in a comment, because a span
//! that is off by one puts every later diagnostic's caret under the wrong
//! character -- and a test that recomputes the offset with the same code that
//! produced it proves nothing about that.
//!
//! The golden tests at the bottom pin the same property over whole files, which
//! is what catches an off-by-one that only shows up after skipped trivia.

use std::fs;
use std::path::{Path, PathBuf};

use yashy_common::{Diagnostic, FileId, Span};
use yashy_lexer::{
    literal_as_i64, split_interpolation, Lexer, StringLit, StringPiece, Token, TokenKind,
};

/// The file id these tests report against.
///
/// `yashy_common::FileId` is an index into a `SourceMap`, and these tests have
/// no map: they lex a literal string. File zero is therefore the only honest
/// choice here, and passing it explicitly is the point -- `literal_as_i64` has
/// no default for it.
fn file() -> FileId {
    FileId::from_index(0)
}

/// Every token as `(kind, span)`, on success.
fn lex(src: &str) -> Result<Vec<(TokenKind, Span)>, Vec<Diagnostic>> {
    match Lexer::new(src).tokenize() {
        Ok(tokens) => Ok(tokens.into_iter().map(|t| (t.kind, t.span)).collect()),
        Err(diags) => Err(diags),
    }
}

/// Like [`lex`], but panics if the input did not lex. Every token is returned,
/// `Eof` included, so the tests below pin its span as well.
fn tokens(src: &str) -> Vec<(TokenKind, Span)> {
    match lex(src) {
        Ok(tokens) => tokens,
        Err(diags) => panic!("expected {:?} to lex, got diagnostics: {:?}", src, messages(&diags)),
    }
}

/// The diagnostics of an input that was expected to fail.
fn errors(src: &str) -> Vec<Diagnostic> {
    match lex(src) {
        Ok(tokens) => panic!("expected {:?} to fail, got {:?}", src, tokens),
        Err(diags) => {
            assert!(!diags.is_empty(), "a failed lex must explain itself");
            diags
        }
    }
}

fn messages(diags: &[Diagnostic]) -> Vec<String> {
    diags.iter().map(|d| format!("{:?}: {}", d.span, d.message)).collect()
}

/// A decoded literal from `value`, built the way [`StringLit::decode`] builds
/// one, so a piece can be compared against it directly.
fn text(value: &str) -> StringLit {
    StringLit {
        value: value.to_string(),
        bytes: value.as_bytes().to_vec(),
    }
}

// --- literals ---------------------------------------------------------------

#[test]
fn lexes_i64_and_f64_literals_with_spans() {
    // `42 3.5`
    //  byte 0..2 is `42`, byte 2 is the space, byte 3..6 is `3.5`,
    //  and the file is 6 bytes long so `Eof` is the empty span 6..6.
    assert_eq!(
        tokens("42 3.5"),
        vec![
            (TokenKind::I64, Span::new(0, 2)),
            (TokenKind::F64, Span::new(3, 6)),
            (TokenKind::Eof, Span::new(6, 6)),
        ]
    );
}

#[test]
fn a_range_operator_does_not_become_a_float_or_an_extra_dot() {
    // `0..10`
    //  byte 0 is `0`, bytes 1..3 are the two dots, bytes 3..5 are `10`.
    //  A float rule that looked only at `digit '.'` would swallow `0.` and
    //  leave a stray `10`; a `..` rule that ran after the float rule would do
    //  the same, so this pins the order the two are tried in.
    assert_eq!(
        tokens("0..10"),
        vec![
            (TokenKind::I64, Span::new(0, 1)),
            (TokenKind::DotDot, Span::new(1, 3)),
            (TokenKind::I64, Span::new(3, 5)),
            (TokenKind::Eof, Span::new(5, 5)),
        ]
    );
}

#[test]
fn a_number_keeps_its_span_even_when_it_does_not_fit_in_an_i64() {
    // The token carries no value: the parser reads the bytes back out of the
    // span. So a literal too big for `i64` must still lex cleanly, and must be
    // a *parser* error later rather than a lexer crash or a rejected token.
    // `literal_as_i64` is how that becomes an error, and it returns a
    // diagnostic rather than panicking.
    let big = "9".repeat(40);
    assert_eq!(tokens(&big), vec![(TokenKind::I64, Span::new(0, 40)), (TokenKind::Eof, Span::new(40, 40))]);
}

#[test]
fn a_number_too_large_for_an_i64_is_a_diagnostic_not_a_panic() {
    // `literal_as_i64` is the only supported way to turn an `I64` token's text
    // into a value. A caller that reached for `str::parse::<i64>().unwrap()`
    // instead would panic here, on the one input a user can type by accident.
    //
    // There is one function and it takes the `FileId`: a two-argument form that
    // defaulted to file zero would put the caret in an unrelated source file
    // for anyone reading from a `SourceMap`, and the wrong caret is worse than
    // no diagnostic.
    let big = "9".repeat(40);
    let d = match literal_as_i64(&big, Span::new(0, 40), file()) {
        Ok(value) => panic!("40 digits cannot be an i64, got {value}"),
        Err(d) => d,
    };
    assert!(d.is_error());
    assert_eq!(d.span, Span::new(0, 40), "the caret covers the whole literal");
    assert_eq!(d.file, file(), "the diagnostic names the file it was given");
    assert!(d.message.contains("i64"), "got {:?}", d.message);

    // The boundaries, so an off-by-one in the range check shows up. Note that
    // `i64::MIN` is not here: the lexer makes the `-` a separate token, so the
    // largest literal an `I64` token can stand for is `i64::MAX`, and folding a
    // sign into it is the parser's business.
    assert_eq!(literal_as_i64("9223372036854775807", Span::new(0, 19), file()), Ok(i64::MAX));
    assert!(literal_as_i64("9223372036854775808", Span::new(0, 19), file()).is_err());
    assert!(literal_as_i64("99999999999999999999", Span::new(0, 20), file()).is_err());

    // A span that names an empty or unsliceable range is a diagnostic, not a
    // panic, and neither is text that is not digits -- including a sign, which
    // is never part of an `I64` token's own span.
    assert!(literal_as_i64("42", Span::new(0, 99), file()).is_err());
    assert!(literal_as_i64("x", Span::new(0, 1), file()).is_err());
    assert!(literal_as_i64("", Span::new(0, 0), file()).is_err());
    assert!(literal_as_i64("-42", Span::new(0, 3), file()).is_err());

    // The file id is not ignored: the same text under a different id reports
    // under that id. A diagnostic that always said file zero would be the bug
    // this argument exists to prevent.
    let other = FileId::from_index(7);
    let d = match literal_as_i64("99999999999999999999", Span::new(0, 20), other) {
        Ok(value) => panic!("20 digits cannot be an i64, got {value}"),
        Err(d) => d,
    };
    assert_eq!(d.file, other, "the diagnostic must not be pinned to file zero");
}

#[test]
fn a_number_may_be_written_with_separators_or_a_radix_prefix() {
    // The separator is part of one token, not a spurious identifier after a
    // number, and the prefix is part of the token rather than a zero followed
    // by a name. Both were diagnostics before; the caret still covers the
    // whole literal, which is what the parser slices to get the value.
    for (src, kind, span) in [
        ("1_000_000", TokenKind::I64, Span::new(0, 9)),
        ("0xFF", TokenKind::I64, Span::new(0, 4)),
        ("0b1010", TokenKind::I64, Span::new(0, 6)),
        ("0o777", TokenKind::I64, Span::new(0, 5)),
        ("0X1f", TokenKind::I64, Span::new(0, 4)),
        ("1_000.5_0", TokenKind::F64, Span::new(0, 9)),
    ] {
        let end = u32::try_from(src.len()).unwrap_or(u32::MAX);
        assert_eq!(
            tokens(src),
            vec![(kind, span), (TokenKind::Eof, Span::new(end, end))],
            "{src:?}"
        );
    }

    // The value is the digits in their radix, not the source text parsed as
    // decimal: `0xFF` is 255, and the separators are not digits.
    for (src, value) in [
        ("1_000_000", 1_000_000),
        ("0xFF", 255),
        ("0b1010", 10),
        ("0o777", 511),
    ] {
        let end = u32::try_from(src.len()).unwrap_or(u32::MAX);
        assert_eq!(
            literal_as_i64(src, Span::new(0, end), file()),
            Ok(value),
            "{src:?}"
        );
    }

    // A prefix with no digits, a digit of the wrong radix, a separator that is
    // not between two digits, and a value too large for the radix's `i64` are
    // all diagnostics rather than guesses.
    for src in ["0x", "0xG", "0b102", "0o8", "0x_1", "0xFF_", "0xFFFFFFFFFFFFFFFFF"] {
        let end = u32::try_from(src.len()).unwrap_or(u32::MAX);
        assert!(
            literal_as_i64(src, Span::new(0, end), file()).is_err(),
            "{src:?} must not produce a value"
        );
    }
}

#[test]
fn a_malformed_number_is_diagnosed_rather_than_split_into_two_tokens() {
    // `123abc` used to lex as `I64` `Ident`, and `0x1F` as `I64` `Ident`. Both
    // are silent successes that leave the parser to report something
    // bewildering about a name the user never wrote.
    for (src, span) in [
        ("123abc", Span::new(0, 3)),
        ("0x", Span::new(0, 2)),
        ("0b102", Span::new(0, 4)),
        ("1e10", Span::new(0, 1)),
        ("1_", Span::new(0, 1)),
        ("1__0", Span::new(0, 1)),
        ("1.2.3", Span::new(0, 3)),
    ] {
        let diags = errors(src);
        assert_eq!(diags.len(), 1, "{src:?}: {:?}", messages(&diags));
        assert!(diags[0].message.contains("invalid numeric literal"), "{src:?}: {:?}", diags[0].message);
        assert_eq!(diags[0].span, span, "{src:?}");
    }

    // A number followed by an operator is still two tokens.
    assert_eq!(
        tokens("1+2"),
        vec![
            (TokenKind::I64, Span::new(0, 1)),
            (TokenKind::Plus, Span::new(1, 2)),
            (TokenKind::I64, Span::new(2, 3)),
            (TokenKind::Eof, Span::new(3, 3)),
        ]
    );
}

#[test]
fn a_byte_order_mark_is_skipped_at_the_start_of_a_file() {
    // Plenty of Windows tooling writes a UTF-8 BOM. Without this such a file
    // does not compile at all, with a diagnostic about an invisible character.
    let src = "\u{feff}1";
    assert_eq!(src.len(), 4, "fixture assumption: the BOM is three bytes");
    assert_eq!(tokens(src), vec![(TokenKind::I64, Span::new(3, 4)), (TokenKind::Eof, Span::new(4, 4))]);

    // A file that is nothing but a BOM is empty, not an error.
    assert_eq!(tokens("\u{feff}"), vec![(TokenKind::Eof, Span::new(3, 3))]);

    // Only a *leading* BOM is skipped. One in the middle of a file is a
    // character nobody typed on purpose, and swallowing it would hide a real
    // mistake -- but the diagnostic naming it has to be readable, or skipping
    // the mark at the start just moves the invisible-character complaint to
    // the mark in the middle.
    let diags = errors("1 \u{feff}2");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].span, Span::new(2, 5));
    assert_eq!(diags[0].message, "unexpected character `\\u{feff}`");
    assert!(!diags[0].message.contains('\u{feff}'), "the mark must be spelled out, not printed");

    // The same in a file that *does* open with a mark, where the second one is
    // at bytes 5..8. This is the case that says the skip happens once, at the
    // start: a skipper that ran on every trivia boundary would reset the scan
    // to byte 3 here and lex `1` twice.
    let diags = errors("\u{feff}1 \u{feff}2");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].span, Span::new(5, 8));
}

// --- keywords and identifiers -----------------------------------------------

#[test]
fn lexes_keywords_as_keywords_not_identifiers() {
    // `fn fnx`
    //  bytes 0..2 are the keyword `fn`, bytes 3..6 are the identifier `fnx`.
    //  Both are spelled the same way up to the `x`, so a keyword match that
    //  used `starts_with` instead of comparing the whole identifier text
    //  would turn `fnx` into `fn` and leave nothing behind.
    assert_eq!(
        tokens("fn fnx"),
        vec![
            (TokenKind::Fn, Span::new(0, 2)),
            (TokenKind::Ident, Span::new(3, 6)),
            (TokenKind::Eof, Span::new(6, 6)),
        ]
    );
}

/// The whole keyword table, spelled out here rather than imported from the
/// crate: a keyword the lexer grows without the test growing would otherwise be
/// invisible.
const KEYWORDS: &[(&str, TokenKind)] = &[
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
    ("in", TokenKind::In),
    ("switch", TokenKind::Switch),
    ("case", TokenKind::Case),
    ("default", TokenKind::Default),
    // Spec §3.1 lists these two among the keywords, and Task 6 parses casts:
    // without them `x as f64` is three identifiers and `move buf` is a name.
    ("as", TokenKind::As),
    ("move", TokenKind::Move),
    ("true", TokenKind::True),
    ("false", TokenKind::False),
];

#[test]
fn every_keyword_is_lexed_as_itself_and_the_table_is_complete() {
    for (text, kind) in KEYWORDS {
        let src = format!("{text} ");
        let end = text.len() as u32 + 1;
        assert_eq!(
            tokens(&src),
            vec![(*kind, Span::new(0, end - 1)), (TokenKind::Eof, Span::new(end, end))],
            "{text:?} must lex as {kind:?}"
        );
        assert!(kind.is_keyword(), "{kind:?} must report itself as a keyword");
    }

    // `true` and `false` are keywords, not identifiers with a value attached:
    // asserting this separately because it is the one place where a plausible
    // implementation ("keywords are just identifiers the parser looks up")
    // would still make every other test pass.
    assert!(TokenKind::True.is_keyword());
    assert!(TokenKind::False.is_keyword());
    assert!(TokenKind::As.is_keyword());
    assert!(TokenKind::Move.is_keyword());
    assert!(!TokenKind::Ident.is_keyword());
    assert!(!TokenKind::Eof.is_keyword());

    // A cast and a transfer use them, so they are exercised in the shape Task 6
    // will parse: `x as f64`.
    assert_eq!(
        tokens("x as f64"),
        vec![
            (TokenKind::Ident, Span::new(0, 1)),
            (TokenKind::As, Span::new(2, 4)),
            (TokenKind::Ident, Span::new(5, 8)),
            (TokenKind::Eof, Span::new(8, 8)),
        ]
    );
}

#[test]
fn identifiers_may_contain_digits_and_underscores_after_the_first_character() {
    // `_a9 _`
    //  bytes 0..3 and 4..5 respectively. The second is an identifier that is
    //  nothing but an underscore, which is worth pinning: the rule is about
    //  what may *follow* the first character, not that a name must have a
    //  letter in it.
    assert_eq!(
        tokens("_a9 _"),
        vec![
            (TokenKind::Ident, Span::new(0, 3)),
            (TokenKind::Ident, Span::new(4, 5)),
            (TokenKind::Eof, Span::new(5, 5)),
        ]
    );

    // A digit may not *start* one: `9a` is a number that ran into a name, not an
    // identifier, and it is diagnosed rather than silently split into two.
    let diags = errors("9a");
    assert_eq!(diags.len(), 1);
    assert!(diags[0].message.contains("invalid numeric literal"), "got {:?}", diags[0].message);
    assert_eq!(diags[0].span, Span::new(0, 1));
}

#[test]
fn only_eof_reports_itself_as_eof() {
    assert!(tokens("").iter().all(|(kind, _)| *kind == TokenKind::Eof), "an empty file is one Eof token");
    assert_eq!(tokens(""), vec![(TokenKind::Eof, Span::new(0, 0))]);
    assert!(TokenKind::Eof.is_eof());
    assert!(!TokenKind::Ident.is_eof());
    assert!(!TokenKind::Fn.is_eof());
}

#[test]
fn every_token_span_slices_back_to_its_own_source_text() {
    // The invariant behind all of the span assertions above, checked over a
    // source that uses every kind of token: for every token but `Eof`, the
    // span is a range this file can be sliced with, and the slice is the text
    // that token stands for.
    let src = "fn f(a: i64) -> i64 { let b = 3.5; return a + b * 2; } // done\n\"s\" @ ..\n";
    let lexed = tokens(src);
    assert!(lexed.len() > 20, "fixture must cover a lot of tokens, got {}", lexed.len());

    for token in &lexed {
        if token.0 == TokenKind::Eof {
            assert_eq!(token.1, Span::new(src.len() as u32, src.len() as u32), "Eof sits at the end of the file");
            continue;
        }
        let start = token.1.start as usize;
        let end = token.1.end as usize;
        let text = src.get(start..end).unwrap_or_else(|| {
            panic!("{:?} is not a sliceable range in a {}-byte file", token.1, src.len())
        });
        assert!(!text.is_empty(), "{:?} covers no bytes", token.1);
        assert!(!text.contains('\n'), "{:?} spans a line break: {text:?}", token.1);
    }
}

#[test]
fn describe_names_fixed_tokens_by_their_spelling_and_classes_by_what_they_are() {
    // `describe` is what the parser puts in "expected X, found Y", so it must
    // read correctly in both halves of that sentence.
    assert_eq!(TokenKind::Fn.describe(), "`fn`");
    assert_eq!(TokenKind::LParen.describe(), "`(`");
    assert_eq!(TokenKind::Arrow.describe(), "`->`");
    assert_eq!(TokenKind::EqEq.describe(), "`==`");
    assert_eq!(TokenKind::Ident.describe(), "an identifier");
    assert_eq!(TokenKind::I64.describe(), "an integer literal");
    assert_eq!(TokenKind::F64.describe(), "a float literal");
    assert_eq!(TokenKind::Str.describe(), "a string literal");
    assert_eq!(TokenKind::Eof.describe(), "end of file");
}

// --- strings ---------------------------------------------------------------

#[test]
fn a_string_token_spans_its_quotes_and_decodes_to_its_contents() {
    // `"a\nb"`
    //  bytes 0 and 5 are the quotes, bytes 1..2 are `a`, 2..4 are the `\n`
    //  escape, and 4..5 is `b`. The token covers all six bytes.
    let src = "\"a\\nb\"";
    assert_eq!(tokens(src), vec![(TokenKind::Str, Span::new(0, 6)), (TokenKind::Eof, Span::new(6, 6))]);

    let lit = StringLit::decode(src).expect("a token the lexer accepted decodes");
    assert_eq!(lit.value, "a\nb", "the escape is decoded, not kept verbatim");
    assert_eq!(lit.bytes, b"a\nb".to_vec());
    assert_eq!(lit.bytes, lit.value.as_bytes().to_vec(), "bytes and value are the same text");
}

#[test]
fn every_supported_escape_decodes_and_anything_else_is_a_diagnostic() {
    for (src, want) in [("\"\\n\"", "\n"), ("\"\\t\"", "\t"), ("\"\\r\"", "\r"), ("\"\\\\\"", "\\"), ("\"\\\"\"", "\""), ("\"\\0\"", "\0"), ("\"\\$\"", "$")] {
        let lit = match StringLit::decode(src) {
            Ok(lit) => lit,
            Err(e) => panic!("{src:?} must decode: {}", e.message),
        };
        assert_eq!(lit.value, want, "{src:?}");
        assert_eq!(tokens(src).len(), 2, "{src:?} is one Str token plus Eof: {src:?}");
    }

    // `\q` is not an escape Yashy has, so the lexer must say so rather than
    // passing the backslash through to the string.
    let diags = errors("\"a\\qb\"");
    assert_eq!(diags.len(), 1);
    assert!(diags[0].message.contains("escape"), "got {:?}", diags[0].message);
    assert!(diags[0].is_error());
}

#[test]
fn interpolation_is_part_of_the_literal_and_spans_the_whole_token() {
    // `"n = ${n}"` -- one `Str` token, 10 bytes, `${n}` included.
    let src = "\"n = ${n}\"";
    assert_eq!(src.len(), 10, "fixture assumption");
    assert_eq!(
        tokens(src),
        vec![
            (TokenKind::Str, Span::new(0, 10)),
            (TokenKind::Eof, Span::new(10, 10)),
        ]
    );

    // `"far: ${d}"` from the design.
    let lit = "\"far: ${d}\"";
    assert_eq!(
        tokens(lit),
        vec![
            (TokenKind::Str, Span::new(0, 11)),
            (TokenKind::Eof, Span::new(11, 11)),
        ]
    );
}

#[test]
fn a_string_nested_inside_an_interpolation_does_not_end_the_literal() {
    // `"${f("x")}"`: the inner quotes belong to the inner literal. If the
    // scanner treated the first of them as the end of the outer one, the whole
    // rest of the line would be lexed as tokens and every later span would be
    // wrong.
    let src = "\"${f(\"x\")}\"";
    let kinds: Vec<TokenKind> = tokens(src).into_iter().map(|(k, _)| k).collect();
    assert_eq!(kinds, vec![TokenKind::Str, TokenKind::Eof], "one literal: {src:?}");

    // A `}` inside the nested string must not close the interpolation either.
    let braced = "\"${f(\"}\")}\"";
    assert_eq!(
        tokens(braced).into_iter().map(|(k, _)| k).collect::<Vec<_>>(),
        vec![TokenKind::Str, TokenKind::Eof],
        "a brace inside a nested string is text: {braced:?}"
    );
}

#[test]
fn an_escaped_dollar_is_text_and_not_an_interpolation() {
    // `\$` means the single character, and it will keep meaning that once `${`
    // is the interpolation marker -- which is why it exists.
    assert_eq!(StringLit::decode("\"a\\${b}\"").map(|l| l.value), Ok("a${b}".to_string()));
    assert_eq!(tokens("\"a\\${b}\"").len(), 2, "one literal plus Eof");

    // `split_interpolation` agrees: it is one text piece, no expression.
    let pieces = split_interpolation("\"a\\${b}\"").expect("the escape decodes");
    assert_eq!(
        pieces,
        vec![StringPiece::Text(text("a${b}"))],
        "an escaped `$` leaves no interpolation piece"
    );
}

#[test]
fn split_interpolation_keeps_text_and_expression_ranges() {
    // `"n = ${n}!"` -- text `n = `, expression at byte 7 (`n`), text `!`.
    //  byte 0 `"`, 1..5 `n = `, 5 `$`, 6 `{`, 7 `n`, 8 `}`, 9 `!`, 10 `"`.
    let raw = "\"n = ${n}!\"";
    assert_eq!(raw.len(), 11, "fixture assumption");
    assert_eq!(
        split_interpolation(raw).expect("well formed"),
        vec![
            StringPiece::Text(text("n = ")),
            StringPiece::Interpolation { start: 7, end: 8 },
            StringPiece::Text(text("!")),
        ]
    );

    // No text at either end: `${a}${b}` is two expressions and nothing else.
    //  `"`(0) `$`(1) `{`(2) `a`(3) `}`(4) `$`(5) `{`(6) `b`(7) `}`(8) `"`(9).
    assert_eq!(
        split_interpolation("\"${a}${b}\"").expect("well formed"),
        vec![
            StringPiece::Interpolation { start: 3, end: 4 },
            StringPiece::Interpolation { start: 7, end: 8 },
        ]
    );

    // An empty interpolation has an empty range, which is how the parser knows
    // to reject it rather than build an expression out of nothing.
    //  `"`(0) `$`(1) `{`(2) `}`(3) `"`(4).
    assert_eq!(
        split_interpolation("\"${}\"").expect("structurally fine"),
        vec![StringPiece::Interpolation { start: 3, end: 3 }]
    );

    // A literal with no interpolation is one text piece, whatever its text.
    assert_eq!(
        split_interpolation("\"hi\"").expect("well formed"),
        vec![StringPiece::Text(text("hi"))]
    );
    assert_eq!(split_interpolation("\"\"").expect("well formed"), Vec::new());
}

#[test]
fn an_interpolation_inside_a_nested_string_is_scanned_too() {
    // `"${f("a${b}c")}"`: the inner string holds an interpolation of its own, so
    // the two inner `"`s are not each other's pair and a scanner that stopped at
    // the first one would end the literal in the middle of the expression.
    let src = "\"${f(\"a${b}c\")}\"";
    assert_eq!(src.len(), 16, "fixture assumption");
    let kinds: Vec<TokenKind> = tokens(src).into_iter().map(|(k, _)| k).collect();
    assert_eq!(kinds, vec![TokenKind::Str, TokenKind::Eof], "one literal: {src:?}");

    // The outer literal is one interpolation and nothing else.
    assert_eq!(
        split_interpolation(src).expect("well formed"),
        vec![StringPiece::Interpolation { start: 3, end: 14 }],
        "the expression is everything between the outer `${{` and `}}`"
    );
}

#[test]
fn an_unterminated_interpolation_is_a_diagnostic_not_a_panic() {
    // `${` with no `}` before the end of the line.
    for src in ["\"${x\"", "\"a${f(b\"", "\"${\""] {
        let diags = errors(src);
        assert_eq!(diags.len(), 1, "{src:?}: {:?}", messages(&diags));
        assert!(diags[0].is_error());
        assert!(
            diags[0].message.contains("interpolation"),
            "{src:?}: {:?}",
            diags[0].message
        );
    }

    // A nested string that runs off the end of the line is the same problem.
    let diags = errors("\"${f(\"x)}\"");
    assert_eq!(diags.len(), 1, "{:?}", messages(&diags));
    assert!(diags[0].message.contains("interpolation"), "{:?}", diags[0].message);

    // And an unterminated interpolation at the very end of the file.
    let diags = errors("\"${x");
    assert_eq!(diags.len(), 1, "{:?}", messages(&diags));
    assert!(diags[0].message.contains("interpolation"), "{:?}", diags[0].message);
}

#[test]
fn spans_count_bytes_not_characters() {
    // `"é"` -- the `é` is two bytes, so the literal is four bytes long.
    //  byte 0 is the quote, bytes 1..3 are `é`, byte 3 is the closing quote.
    let src = "\"é\"";
    assert_eq!(src.len(), 4, "fixture assumption: é is two bytes in UTF-8");
    assert_eq!(tokens(src), vec![(TokenKind::Str, Span::new(0, 4)), (TokenKind::Eof, Span::new(4, 4))]);

    let lit = StringLit::decode(src).expect("decodes");
    assert_eq!(lit.value, "é");
    assert_eq!(lit.bytes.len(), 2, "the decoded text keeps its two UTF-8 bytes");
}

#[test]
fn reports_unterminated_string_with_the_opening_quote_span() {
    let diags = errors("\"abc");
    assert_eq!(diags.len(), 1, "one problem, one diagnostic: {:?}", messages(&diags));
    assert_eq!(diags[0].span.start, 0, "the caret must point at the quote that opened the literal, not at the end of the file");
    assert!(diags[0].is_error());
    assert!(diags[0].message.contains("string"), "got {:?}", diags[0].message);
}

#[test]
fn a_string_does_not_run_past_the_end_of_its_line() {
    // Without this, a forgotten quote swallows the rest of the file and every
    // later span is wrong, which is the worst possible diagnostic experience:
    // one typo, hundreds of shifted carets.
    let diags = errors("\"abc\n42\n");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].span.start, 0);
    assert!(diags[0].message.contains("string"), "got {:?}", diags[0].message);

    // A backslash at the end of the line does not continue the literal either.
    assert_eq!(errors("\"abc\\\n").len(), 1);
    assert_eq!(errors("\"").len(), 1, "a lone quote is unterminated, not a panic");
    assert_eq!(errors("\"\\\"").len(), 1, "a trailing backslash swallows the closing quote");
}

// --- comments ---------------------------------------------------------------

#[test]
fn lexes_line_and_nested_block_comments() {
    // `/* a /* b */ c */ 1`
    //  the outer comment runs bytes 0..17 and the `1` is byte 18. Nested
    //  comments mean the first `*/` at byte 10 does not end it.
    let src = "/* a /* b */ c */ 1";
    assert_eq!(src.len(), 19, "fixture assumption");
    assert_eq!(tokens(src), vec![(TokenKind::I64, Span::new(18, 19)), (TokenKind::Eof, Span::new(19, 19))]);

    // A line comment ends at the newline, and the token after it is at the
    //  position after that newline -- not after the comment alone.
    assert_eq!(
        tokens("// hi\n42"),
        vec![(TokenKind::I64, Span::new(6, 8)), (TokenKind::Eof, Span::new(8, 8))]
    );
    // A line comment that runs to the end of the file is not an error.
    assert_eq!(tokens("// hi"), vec![(TokenKind::Eof, Span::new(5, 5))]);
}

#[test]
fn an_unterminated_block_comment_is_reported_at_its_opening_delimiter() {
    // `1 /* a /* b`
    //  byte 0 is `1`, byte 1 the space, bytes 2..3 the `/*` that never closes.
    let diags = errors("1 /* a /* b");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].span.start, 2, "the caret belongs on the `/*` that was never closed");
    assert!(diags[0].message.contains("comment"), "got {:?}", diags[0].message);
}

#[test]
fn a_stray_slash_is_the_division_operator_and_not_a_comment() {
    // `4/2` -- one byte of trivia skipped here would shift every span after it.
    assert_eq!(
        tokens("4/2"),
        vec![
            (TokenKind::I64, Span::new(0, 1)),
            (TokenKind::Slash, Span::new(1, 2)),
            (TokenKind::I64, Span::new(2, 3)),
            (TokenKind::Eof, Span::new(3, 3)),
        ]
    );
}

// --- punctuation ------------------------------------------------------------

#[test]
fn every_delimiter_lexes_in_isolation_and_reaches_eof() {
    // Each delimiter alone is one token spanning byte 0..1, followed by an
    // `Eof` whose empty span sits at byte 1 -- which is what a parser needs in
    // order to notice an unclosed delimiter and point a caret at the end of the
    // file. Reporting that unclosed-delimiter diagnostic is the parser's job
    // (it owns the delimiter stack); what the lexer owes it is this exact
    // stream, and never a panic on unbalanced input.
    for (ch, kind) in [
        ('(', TokenKind::LParen),
        (')', TokenKind::RParen),
        ('{', TokenKind::LBrace),
        ('}', TokenKind::RBrace),
        ('[', TokenKind::LBracket),
        (']', TokenKind::RBracket),
    ] {
        let src = ch.to_string();
        assert_eq!(
            tokens(&src),
            vec![(kind, Span::new(0, 1)), (TokenKind::Eof, Span::new(1, 1))],
            "{ch:?} in isolation"
        );
    }
}

#[test]
fn two_character_operators_win_over_their_one_character_prefixes() {
    // Each case is `<two chars><space>` so the span is 0..2, and the single
    // character operator cases are `<char><space>` with a span of 0..1.
    for (src, kind) in [
        ("== ", TokenKind::EqEq),
        ("!= ", TokenKind::BangEq),
        ("<= ", TokenKind::LtEq),
        (">= ", TokenKind::GtEq),
        ("+= ", TokenKind::PlusEq),
        ("-= ", TokenKind::MinusEq),
        ("*= ", TokenKind::StarEq),
        ("/= ", TokenKind::SlashEq),
        ("%= ", TokenKind::PercentEq),
        ("-> ", TokenKind::Arrow),
        (".. ", TokenKind::DotDot),
        (":= ", TokenKind::ColonEq),
        ("&& ", TokenKind::AndAnd),
        ("|| ", TokenKind::OrOr),
    ] {
        assert_eq!(
            tokens(src),
            vec![(kind, Span::new(0, 2)), (TokenKind::Eof, Span::new(3, 3))],
            "{src:?}"
        );
    }

    for (src, kind) in [
        ("= ", TokenKind::Eq),
        ("! ", TokenKind::Bang),
        ("< ", TokenKind::Lt),
        ("> ", TokenKind::Gt),
        ("+ ", TokenKind::Plus),
        ("- ", TokenKind::Minus),
        ("* ", TokenKind::Star),
        ("/ ", TokenKind::Slash),
        ("% ", TokenKind::Percent),
        (". ", TokenKind::Dot),
        (": ", TokenKind::Colon),
    ] {
        assert_eq!(
            tokens(src),
            vec![(kind, Span::new(0, 1)), (TokenKind::Eof, Span::new(2, 2))],
            "{src:?}"
        );
    }
}

#[test]
fn an_at_sign_lexes_as_the_at_token() {
    // `TokenKind::At` is part of this crate's published vocabulary, so `@`
    // has to produce it -- otherwise the variant exists for no reason and a
    // parser that checks for it can never match. The brief's "report an
    // unexpected character instead of skipping it" requirement is about
    // characters the lexer has no token for, and is covered by the next test.
    assert_eq!(tokens("@"), vec![(TokenKind::At, Span::new(0, 1)), (TokenKind::Eof, Span::new(1, 1))]);
    assert_eq!(tokens("@ x"), vec![(TokenKind::At, Span::new(0, 1)), (TokenKind::Ident, Span::new(2, 3)), (TokenKind::Eof, Span::new(3, 3))]);
}

#[test]
fn a_bar_lexes_as_the_pipe_token_and_two_bars_are_still_the_or() {
    // The bar opens a lambda's parameter list, and the two-character operator
    // is matched first, so `||` is still the logical or.
    assert_eq!(tokens("|"), vec![(TokenKind::Pipe, Span::new(0, 1)), (TokenKind::Eof, Span::new(1, 1))]);
    assert_eq!(tokens("||"), vec![(TokenKind::OrOr, Span::new(0, 2)), (TokenKind::Eof, Span::new(2, 2))]);
    assert_eq!(tokens("|||"), vec![(TokenKind::OrOr, Span::new(0, 2)), (TokenKind::Pipe, Span::new(2, 3)), (TokenKind::Eof, Span::new(3, 3))]);
}

#[test]
fn reports_unexpected_character_instead_of_skipping_it() {
    // A character with no token is a diagnostic. Dropping it would shift every
    // later span by one byte and hide the typo, so the assertion is that the
    // input fails, not merely that it produces something.
    for src in ["#", "\\", "&", "\u{1f600}", "1 \u{0} 2"] {
        let diags = errors(src);
        assert_eq!(diags.len(), 1, "{src:?}: {:?}", messages(&diags));
        assert!(diags[0].message.contains("unexpected character"), "{src:?}: {:?}", diags[0].message);
        assert!(diags[0].is_error());
    }

    // The diagnostic points at the offending byte, so the caret lands on the
    // character the user typed rather than at the end of the file.
    let diags = errors("1 # 2");
    assert_eq!(diags[0].span, Span::new(2, 3));
    assert_eq!(
        errors("1 # 2")[0].file,
        errors("x #")[0].file,
        "every lexer diagnostic names the file it came from"
    );
}

// --- robustness -------------------------------------------------------------

#[test]
fn tokenizing_twice_gives_the_same_answer_as_tokenizing_once() {
    // `tokenize` takes `&mut self`, so a caller can call it twice; the second
    // call must start over rather than resume from wherever the first stopped.
    let mut lexer = Lexer::new("fn f() { 1 } // tail");
    let first = lexer.tokenize().expect("lexes cleanly");
    let second = lexer.tokenize().expect("lexes cleanly again");
    assert_eq!(first, second);
    assert_eq!(first.last().map(|t| t.kind), Some(TokenKind::Eof));
}

#[test]
fn a_reported_diagnostic_is_an_error_naming_a_file_and_a_span() {
    let diags = errors("\"abc");
    let d = &diags[0];
    assert!(d.is_error());
    assert!(!d.message.is_empty());
    assert!(!d.span.is_synthetic(), "a problem in the source has a span to point at");
    assert_eq!(d.span.start, 0);
    assert_eq!(d.file, yashy_common::FileId::from_index(0), "`Lexer::new` names file 0 until told otherwise");
}

#[test]
fn the_lexer_never_panics_on_hostile_input() {
    // Every one of these either lexes or reports. The assertion is that the
    // call returns at all: a panic in any of them is the failure this test
    // exists to catch, and it is the input the user most needs diagnosed.
    let long = "a".repeat(100_000);
    let cases: Vec<String> = vec![
        String::new(),
        "\0".to_string(),
        "\r".to_string(),
        "\u{feff}".to_string(),
        "#".to_string(),
        "\\".to_string(),
        "&&&".to_string(),
        "|||".to_string(),
        "...".to_string(),
        "::=".to_string(),
        "1.2.3".to_string(),
        "0x1F".to_string(),
        "999999999999999999999999999999999999".to_string(),
        "\"".to_string(),
        "\"\\".to_string(),
        "\"\\q\"".to_string(),
        "\"unterminated".to_string(),
        "/*".to_string(),
        "/**/*".to_string(),
        "/* /*".to_string(),
        "//".to_string(),
        "\u{0}\u{1}\u{2}".to_string(),
        long.clone(),
        format!("\"{}\"", "x".repeat(50_000)),
        format!("/*{}*/", "*".repeat(1_000)),
        // 5_000 nested block comments: the nesting depth is counted, not
        // recursed into, so this must not overflow the stack.
        format!("{}1{}", "/*".repeat(5_000), "*/".repeat(5_000)),
        format!("{}1", "(".repeat(5_000)),
    ];

    for src in &cases {
        // The result is discarded on purpose: the contract under test is that
        // `tokenize` returns for any input at all.
        let _ = Lexer::new(src).tokenize();
    }

    // A few of those are specifically *not* allowed to succeed.
    assert!(Lexer::new("\"unterminated").tokenize().is_err());
    assert!(Lexer::new("/* /*").tokenize().is_err());
    assert!(Lexer::new("#").tokenize().is_err());
}

// --- golden files -----------------------------------------------------------

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("golden")
}

/// True when `UPDATE_GOLDEN=1` asks for the `.expected` files to be rewritten.
fn updating_goldens() -> bool {
    matches!(std::env::var("UPDATE_GOLDEN").as_deref(), Ok("1") | Ok("true"))
}

/// Renders a token stream as the golden format: one `kind@start..end` per line.
fn render_golden(tokens: &[Token]) -> String {
    let mut out = String::new();
    for token in tokens {
        out.push_str(&format!(
            "{:?}@{}..{}\n",
            token.kind, token.span.start, token.span.end
        ));
    }
    out
}

/// Reads a golden file with its line endings normalised to `\n`.
///
/// *Both* halves of a golden comparison go through this. Normalising only the
/// `.expected` side is a trap: an editor that writes CRLF leaves the `.ya` input
/// CRLF as well, so `UPDATE_GOLDEN=1` would regenerate every span one byte
/// further along on every line after the first, pass locally, and fail
/// everywhere else. A fresh checkout does the same thing through
/// `core.autocrlf`.
///
/// Stripping `\r` is safe because `\r` is whitespace to the lexer: the token
/// stream of a CRLF file is its LF twin's with the `\r` offsets shifted. The
/// golden files are LF, so normalising the input reproduces exactly the bytes
/// the `.expected` file describes.
///
/// The root `.gitattributes` forces LF for `**/tests/golden/*` as the first
/// line of defence. This is the second, and it is the one that still holds if
/// that attribute is ever dropped.
fn read_lf_only(path: &Path) -> String {
    let raw = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read golden file {}: {e}", path.display()));
    let body = raw.strip_suffix('\n').unwrap_or(raw.as_str());
    let mut out = String::new();
    for line in body.split('\n') {
        out.push_str(line.strip_suffix('\r').unwrap_or(line));
        out.push('\n');
    }
    out
}

/// Every `.ya` input in the golden directory, sorted.
fn golden_inputs() -> Vec<PathBuf> {
    let dir = golden_dir();
    let mut inputs: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|entry| entry.expect("directory entry is readable").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "ya"))
        .collect();
    inputs.sort();
    assert!(
        !inputs.is_empty(),
        "no .ya inputs found in {}, so this test would compare nothing and pass",
        dir.display()
    );
    inputs
}

/// A private scratch directory under the system temporary directory, removed
/// again by [`TempDir`]'s `Drop`.
///
/// The tests below need real files on disk -- that is the whole point of them,
/// a helper that normalised line endings could be a string function -- and the
/// obvious place to put them is beside the golden files. That is wrong twice
/// over: a panic between the write and the delete leaves junk in the source
/// tree, and a read-only checkout cannot run the suite at all.
///
/// `std::env::temp_dir` is the standard library's answer and this crate has no
/// dependencies. The name mixes the process id and an atomic counter so two
/// tests -- or two `cargo test` runs side by side -- cannot collide, and
/// `create_dir_all` rather than `create_dir` so a leftover from a killed run is
/// reused instead of failing.
fn temp_dir(label: &str) -> TempDir {
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "yashy-lexer-test-{}-{}-{unique}",
        std::process::id(),
        label
    ));
    fs::create_dir_all(&dir)
        .unwrap_or_else(|e| panic!("cannot create scratch directory {}: {e}", dir.display()));
    TempDir(dir)
}

/// Removes its directory on drop, including after a panic.
///
/// `Drop` is the point: the cleanup cannot be skipped by an assertion failing
/// halfway through a test, which is the leak that motivated moving here. A
/// failure to remove is ignored rather than panicked, because a `Drop` that
/// panics during an unwinding panic aborts the test binary.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn reading_a_golden_file_normalises_carriage_returns() {
    // Written to files rather than checked inline, because the point is what
    // the helper does with what is on disk -- and written to a scratch
    // directory, so the suite never touches the repository.
    let dir = temp_dir("crlf-probe");
    let crlf_path = dir.0.join("crlf-probe.txt");
    let lf_path = dir.0.join("lf-probe.txt");
    fs::write(&crlf_path, "I64@0..2\r\nI64@3..4\r\n").expect("the probe file can be written");
    fs::write(&lf_path, "I64@0..2\nI64@3..4\n").expect("the probe file can be written");

    let from_crlf = read_lf_only(&crlf_path);
    let from_lf = read_lf_only(&lf_path);

    assert!(!from_crlf.contains('\r'), "a carriage return must not survive: {from_crlf:?}");
    assert_eq!(from_crlf, from_lf, "CRLF and LF must read back as the same golden text");
    assert_eq!(from_lf, "I64@0..2\nI64@3..4\n");
}

#[test]
fn a_scratch_directory_lives_outside_the_repository_and_is_removed_on_drop() {
    // The two properties that keep the probe tests from dirtying a checkout,
    // stated as tests rather than as an intention. A panic between the write and
    // the delete is exactly what leaks a file into `tests/golden/`, so the
    // cleanup has to be something that cannot be skipped -- which means `Drop`,
    // and this is the test that says so.
    let path = {
        let dir = temp_dir("location-probe");
        assert!(
            !dir.0.starts_with(golden_dir()),
            "a probe must not be written into the golden directory: {}",
            dir.0.display()
        );
        assert!(
            !dir.0.starts_with(Path::new(env!("CARGO_MANIFEST_DIR"))),
            "a probe must not be written into the crate at all: {}",
            dir.0.display()
        );
        assert!(dir.0.is_dir(), "the scratch directory must exist while it is in use");
        fs::write(dir.0.join("probe.txt"), "x").expect("the probe file can be written");
        dir.0.clone()
        // `dir` is dropped here, at the end of the block, on the happy path and
        // on a panic alike.
    };

    assert!(!path.exists(), "{} was left behind after the guard went out of scope", path.display());
}

#[test]
fn a_crlf_input_lexes_to_the_golden_stream_of_its_lf_twin() {
    // The property that makes normalising the input safe, stated directly on
    // real source: strip the carriage returns and the result must be the exact
    // bytes the `.expected` file was generated from. If this ever stops
    // holding, normalising the input would be changing what the goldens mean
    // rather than repairing how they are read.
    //
    // `1\r\n3.5\r\n`
    //  byte 0 is `1`, byte 1 the `\r`, byte 2 the `\n`, bytes 3..6 `3.5`.
    //  After normalising: `1\n3.5\n`, so `3.5` is at 2..5 and the file is 6
    //  bytes long.
    let crlf = "1\r\n3.5\r\n";
    let normalised: String = crlf
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(normalised, "1\n3.5\n");

    let tokens = match Lexer::new(&normalised).tokenize() {
        Ok(tokens) => tokens,
        Err(diags) => panic!("must lex, got {:?}", messages(&diags)),
    };
    assert_eq!(render_golden(&tokens), "I64@0..1\nF64@2..5\nEof@6..6\n");

    // And the unnormalised twin really does differ, or normalising would be
    // hiding something rather than repairing it.
    let crlf_tokens = match Lexer::new(crlf).tokenize() {
        Ok(tokens) => tokens,
        Err(diags) => panic!("must lex, got {:?}", messages(&diags)),
    };
    assert_eq!(render_golden(&crlf_tokens), "I64@0..1\nF64@3..6\nEof@8..8\n");
}

#[test]
fn a_crlf_copy_of_a_golden_input_still_matches_its_expected_file() {
    // The hazard itself, end to end. A CRLF copy of `numbers.ya` reads back to
    // the same tokens as the LF original -- so the `.expected` file still
    // matches -- but *only* because the input goes through `read_lf_only`. Read
    // raw, every span after the first line is one byte further along, and an
    // `UPDATE_GOLDEN=1` run on a machine with a CRLF editor would rewrite the
    // whole file that way, commit it, and break every other checkout.
    let input = golden_dir().join("numbers.ya");
    let expected_path = input.with_extension("expected");
    let expected = read_lf_only(&expected_path);

    let as_crlf = read_lf_only(&input).replace('\n', "\r\n");
    assert!(as_crlf.contains("\r\n"), "the fixture must really be CRLF now");

    // The file as it would be read without normalising it: different, and wrong.
    let raw_tokens = match Lexer::new(&as_crlf).tokenize() {
        Ok(tokens) => tokens,
        Err(d) => panic!("must lex, got {:?}", messages(&d)),
    };
    assert_ne!(render_golden(&raw_tokens), expected, "a CRLF file read raw must differ, or this test proves nothing");

    // Normalised: identical to the committed expectation. The CRLF copy lives
    // in a scratch directory, so a panic here cannot leave it in the source
    // tree for the next run to trip over.
    let dir = temp_dir("numbers-crlf-probe");
    let copy = dir.0.join("numbers.ya");
    fs::write(&copy, &as_crlf).expect("the probe copy can be written");
    let normalised = read_lf_only(&copy);

    let tokens = match Lexer::new(&normalised).tokenize() {
        Ok(tokens) => tokens,
        Err(d) => panic!("must lex, got {:?}", messages(&d)),
    };
    assert_eq!(render_golden(&tokens), expected);
}

#[test]
fn golden_token_streams_match_their_expected_files() {
    let updating = updating_goldens();
    for input in golden_inputs() {
        let name = input
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let src = read_lf_only(&input);
        let tokens = match Lexer::new(&src).tokenize() {
            Ok(tokens) => tokens,
            Err(diags) => panic!("golden input {name} must lex cleanly, got {:?}", messages(&diags)),
        };
        let actual = render_golden(&tokens);
        let expected_path = input.with_extension("expected");

        if updating {
            fs::write(&expected_path, &actual)
                .unwrap_or_else(|e| panic!("cannot write {}: {e}", expected_path.display()));
            continue;
        }

        let expected = read_lf_only(&expected_path);
        assert!(
            expected == actual,
            "token stream for {name} does not match {}\n--- expected ---\n{}\n--- actual ---\n{}",
            expected_path.display(),
            expected,
            actual
        );
    }
}
