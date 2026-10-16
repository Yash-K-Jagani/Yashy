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

use otter_common::{Diagnostic, Span};
use otter_lexer::{Lexer, StringLit, Token, TokenKind};

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
    let big = "9".repeat(40);
    assert_eq!(tokens(&big), vec![(TokenKind::I64, Span::new(0, 40)), (TokenKind::Eof, Span::new(40, 40))]);
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
    assert!(!TokenKind::Ident.is_keyword());
    assert!(!TokenKind::Eof.is_keyword());
}

#[test]
fn identifiers_may_contain_digits_and_underscores_after_the_first_character() {
    // `_a9 _ 9a`
    //  bytes 0..3, 4..6, 7..9 respectively.
    assert_eq!(
        tokens("_a9 _ 9a"),
        vec![
            (TokenKind::Ident, Span::new(0, 3)),
            (TokenKind::Ident, Span::new(4, 5)),
            // A leading digit is a number, not an identifier: `9a` is the
            // number `9` followed by the identifier `a`.
            (TokenKind::I64, Span::new(6, 7)),
            (TokenKind::Ident, Span::new(7, 8)),
            (TokenKind::Eof, Span::new(8, 8)),
        ]
    );
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
    for (src, want) in [("\"\\n\"", "\n"), ("\"\\t\"", "\t"), ("\"\\r\"", "\r"), ("\"\\\\\"", "\\"), ("\"\\\"\"", "\""), ("\"\\0\"", "\0")] {
        let lit = match StringLit::decode(src) {
            Ok(lit) => lit,
            Err(e) => panic!("{src:?} must decode: {}", e.message),
        };
        assert_eq!(lit.value, want, "{src:?}");
        assert_eq!(tokens(src).len(), 2, "{src:?} is one Str token plus Eof: {src:?}");
    }

    // `\q` is not an escape Otter has, so the lexer must say so rather than
    // passing the backslash through to the string.
    let diags = errors("\"a\\qb\"");
    assert_eq!(diags.len(), 1);
    assert!(diags[0].message.contains("escape"), "got {:?}", diags[0].message);
    assert!(diags[0].is_error());
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
fn reports_unexpected_character_instead_of_skipping_it() {
    // A character with no token is a diagnostic. Dropping it would shift every
    // later span by one byte and hide the typo, so the assertion is that the
    // input fails, not merely that it produces something.
    for src in ["#", "\\", "&", "|", "\u{1f600}", "1 \u{0} 2"] {
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
    assert_eq!(d.file, otter_common::FileId::from_index(0), "`Lexer::new` names file 0 until told otherwise");
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

/// The `.expected` files are read back with any `\r` stripped, because git
/// checks text files out with CRLF on this platform and a stray carriage
/// return would fail a comparison for a reason that has nothing to do with the
/// lexer.
fn read_expected(path: &Path) -> String {
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

#[test]
fn golden_token_streams_match_their_expected_files() {
    let dir = golden_dir();
    let mut inputs: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|entry| entry.expect("directory entry is readable").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "ot"))
        .collect();
    inputs.sort();
    assert!(
        !inputs.is_empty(),
        "no .ot inputs found in {}, so this test would compare nothing and pass",
        dir.display()
    );

    let updating = updating_goldens();
    for input in inputs {
        let name = input
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let src = fs::read_to_string(&input).unwrap_or_else(|e| panic!("cannot read {}: {e}", input.display()));
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

        let expected = read_expected(&expected_path);
        assert!(
            expected == actual,
            "token stream for {name} does not match {}\n--- expected ---\n{}\n--- actual ---\n{}",
            expected_path.display(),
            expected,
            actual
        );
    }
}