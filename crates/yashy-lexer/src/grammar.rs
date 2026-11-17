//! The one model every editor grammar is rendered from.
//!
//! The lexer owns the facts: a keyword is a keyword because `TokenKind::keyword`
//! says so, and an operator is an operator because it appears in
//! [`TokenKind::PUNCTUATION`]. The formats live in `yashy-driver`; this module
//! only turns the token classes into data a formatter can read.

use crate::TokenKind;

/// One source position: a `GrammarModel` is the whole vocabulary, not a
/// particular file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrammarModel {
    /// The reserved words, in lexer order.
    pub keywords: Vec<&'static str>,
    /// Type names the checker knows, including the C spellings that only mean
    /// something on a `repr(C)` type.
    pub types: Vec<&'static str>,
    /// Operators and assignment forms (`+`, `<<=`, `&&`, ...).
    pub operators: Vec<&'static str>,
    /// Structural punctuation (`(`, `;`, `->`, `...`, ...).
    pub delimiters: Vec<&'static str>,
    /// `(start, end)` for each comment form; an empty `end` is a line comment.
    pub comments: Vec<(&'static str, &'static str)>,
    /// Documentation comment prefixes, which are tokens rather than trivia.
    pub docs: Vec<&'static str>,
    /// Literal forms, used for the generated probe file and the documentation.
    pub literals: Vec<&'static str>,
}

/// The canonical type spellings.
///
/// These are the spellings `yashy-sema` resolves: the primitives, the generic
/// types the compiler knows, and the C spellings a `repr(C)` declaration
/// licenses. A type added to the checker has to be added here as well; the
/// editor grammar test pins the list so the two cannot drift unnoticed.
pub const TYPES: &[&str] = &[
    "int", "float", "bool", "char", "string", "void", "Mutex", "Channel", "Task", "Self", "Option",
    "Result", "Iterator", "i64", "f64", "u8", "i32", "usize",
];

/// The punctuation that groups tokens rather than combining values.
const DELIMITER_SPELLINGS: &[&str] = &[
    "(", ")", "{", "}", "[", "]", ",", ";", ":", "::", ":=", ".", "..", "...", "->", "=>", "@",
    "$", "?",
];

/// Build the model from the lexer's own tables.
pub fn model() -> GrammarModel {
    let mut operators = Vec::new();
    let mut delimiters = Vec::new();
    for kind in TokenKind::PUNCTUATION {
        let Some(spelling) = kind.spelling() else {
            continue;
        };
        if DELIMITER_SPELLINGS.contains(&spelling) {
            delimiters.push(spelling);
        } else {
            operators.push(spelling);
        }
    }
    GrammarModel {
        keywords: TokenKind::KEYWORDS.to_vec(),
        types: TYPES.to_vec(),
        operators,
        delimiters,
        comments: vec![("//", ""), ("/*", "*/")],
        docs: vec!["///"],
        literals: vec!["\"text\"", "'A'", "42", "3.5", "true", "false"],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_model_partitions_every_punctuation_kind_once() {
        let grammar = model();
        let mut all: Vec<&str> = grammar
            .operators
            .iter()
            .chain(grammar.delimiters.iter())
            .copied()
            .collect();
        all.sort_unstable();
        let mut expected: Vec<&str> = TokenKind::PUNCTUATION
            .iter()
            .filter_map(|kind| kind.spelling())
            .collect();
        expected.sort_unstable();
        assert_eq!(
            all, expected,
            "an operator or delimiter is missing or duplicated"
        );
    }

    #[test]
    fn every_class_has_a_unique_spelling() {
        let grammar = model();
        for class in [
            &grammar.keywords,
            &grammar.types,
            &grammar.operators,
            &grammar.delimiters,
        ] {
            let mut sorted = class.clone();
            sorted.sort_unstable();
            let count = sorted.len();
            sorted.dedup();
            assert_eq!(sorted.len(), count, "a spelling appears twice in one class");
        }
    }
}
