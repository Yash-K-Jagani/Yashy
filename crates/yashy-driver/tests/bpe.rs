//! The language-side half of the Phase 4 BPE audit.
//!
//! The external tokenizers (`o200k`, `cl100k`, a Llama family) are not vendored
//! in this repository, so this test cannot claim their single-token property.
//! What it can prove is the Yashy lexer's side: every spelling Phase 4 added
//! is one lexical token, except the two-word forms that are deliberately two.
//! `docs/bpe.md` records the same measurement and the missing half.

use yashy_lexer::Lexer;

fn tokens(source: &str) -> Vec<String> {
    Lexer::new(source)
        .tokenize()
        .unwrap_or_else(|error| panic!("{source:?} must lex: {error:?}"))
        .iter()
        .map(|token| format!("{:?}", token.kind))
        .collect()
}

#[test]
fn every_single_word_form_added_by_phase_4_is_one_token() {
    for spelling in [
        "trait", "type", "derive", "unsafe", "noalias", "packed", "align",
        "transparent", "matches", "in", "is", "and", "or", "not", "@",
    ] {
        let found = tokens(spelling);
        assert_eq!(
            found.len(),
            1,
            "`{spelling}` must be one lexical token, got {found:?}"
        );
    }
}

#[test]
fn the_two_word_forms_are_two_tokens_by_design() {
    for (spelling, expected) in [("const fn", 2), ("let else", 2)] {
        let found = tokens(spelling);
        assert_eq!(
            found.len(),
            expected,
            "`{spelling}` has {expected} lexer tokens, got {found:?}"
        );
    }
}
