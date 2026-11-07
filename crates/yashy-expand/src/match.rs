//! Matching a macro invocation against an arm's pattern.
//!
//! Matching is left to right. A literal token and a delimiter must agree, and
//! a metavariable consumes the longest prefix of the remaining input that
//! parses as one fragment of its kind. Bindings are a [`BTreeMap`] so their
//! iteration order is stable: expansion must be deterministic byte for byte,
//! and a `HashMap` iteration in the expander would be a defect even when the
//! output happens to be stable today.

use std::collections::BTreeMap;

use yashy_common::{codes, Diagnostic, FileId, SourceMap, Span};
use yashy_lexer::{Token, TokenKind};

use crate::fragment::{match_prefix, FragmentKind};
use crate::tt::{TokenTree, TreeError};

/// What a pattern bound a metavariable to: the token trees it consumed.
pub type Bindings = BTreeMap<String, Vec<TokenTree>>;

/// Why an arm did not match.
#[derive(Clone, Debug, PartialEq)]
pub enum MatchFailure {
    /// The pattern and the input simply have different shapes.
    NoMatch,
    /// A metavariable names a kind the input is not a fragment of.
    BadFragment {
        variable: String,
        kind: FragmentKind,
        span: Span,
        input: Vec<TokenTree>,
    },
    /// The metavariable itself is malformed: no name, no colon, or an
    /// unknown kind.
    BadMetavariable { span: Span, message: String },
}

impl MatchFailure {
    /// The diagnostic for a fragment that did not parse as its kind.
    pub fn diagnostic(&self, file: FileId, source: &SourceMap) -> Option<Diagnostic> {
        match self {
            MatchFailure::NoMatch => None,
            MatchFailure::BadFragment {
                variable,
                kind,
                span,
                input,
            } => {
                let input_text = render_trees(input, source, file);
                Some(
                    Diagnostic::new(
                        yashy_common::Severity::Error,
                        file,
                        *span,
                        format!(
                            "`${variable}` is a `{}` fragment, and `{input_text}` is not",
                            kind.as_str()
                        ),
                    )
                    .with_code(codes::MACRO_FRAGMENT),
                )
            }
            MatchFailure::BadMetavariable { span, message } => Some(
                Diagnostic::new(yashy_common::Severity::Error, file, *span, message.clone())
                    .with_code(codes::MACRO_FRAGMENT),
            ),
        }
    }
}

/// Match `input` against `pattern`, binding every metavariable it finds.
///
/// The pattern's spans belong to the macro's definition file and the input's
/// to the invocation file; both source maps are therefore parameters.
pub fn matches(
    pattern: &[TokenTree],
    definition_source: &SourceMap,
    definition_file: FileId,
    input: &[TokenTree],
    invocation_source: &SourceMap,
    invocation_file: FileId,
) -> Result<Option<Bindings>, MatchFailure> {
    let mut bindings = BTreeMap::new();
    match pattern_trees(
        pattern,
        definition_source,
        definition_file,
        input,
        invocation_source,
        invocation_file,
        &mut bindings,
    ) {
        Ok(()) => Ok(Some(bindings)),
        Err(MatchFailure::NoMatch) => Ok(None),
        Err(failure) => Err(failure),
    }
}

fn pattern_trees(
    pattern: &[TokenTree],
    definition_source: &SourceMap,
    definition_file: FileId,
    input: &[TokenTree],
    invocation_source: &SourceMap,
    invocation_file: FileId,
    bindings: &mut Bindings,
) -> Result<(), MatchFailure> {
    let mut pattern_at = 0usize;
    let mut input_at = 0usize;

    while pattern_at < pattern.len() {
        let Some(tree) = pattern.get(pattern_at) else {
            return Err(MatchFailure::NoMatch);
        };
        if is_metavariable_start(tree) {
            let (variable, kind, span) =
                metavariable(&pattern[pattern_at..], definition_source, definition_file)?;
            input_at = match_fragment(
                variable,
                kind,
                span,
                input,
                input_at,
                invocation_source,
                invocation_file,
                bindings,
            )?;
            pattern_at = pattern_at.saturating_add(4);
            continue;
        }
        let Some(candidate) = input.get(input_at) else {
            return Err(MatchFailure::NoMatch);
        };
        if !trees_agree(
            tree,
            candidate,
            definition_source,
            definition_file,
            invocation_source,
            invocation_file,
            bindings,
        )? {
            return Err(MatchFailure::NoMatch);
        }
        pattern_at = pattern_at.saturating_add(1);
        input_at = input_at.saturating_add(1);
    }

    if input_at == input.len() {
        Ok(())
    } else {
        Err(MatchFailure::NoMatch)
    }
}

fn trees_agree(
    pattern: &TokenTree,
    input: &TokenTree,
    definition_source: &SourceMap,
    definition_file: FileId,
    invocation_source: &SourceMap,
    invocation_file: FileId,
    bindings: &mut Bindings,
) -> Result<bool, MatchFailure> {
    match (pattern, input) {
        (TokenTree::Leaf(expected), TokenTree::Leaf(actual)) => Ok(expected.kind == actual.kind),
        (
            TokenTree::Delimited {
                open: expected,
                trees: expected_children,
                ..
            },
            TokenTree::Delimited {
                open: actual,
                trees: actual_children,
                ..
            },
        ) => {
            if expected != actual {
                return Ok(false);
            }
            match pattern_trees(
                expected_children,
                definition_source,
                definition_file,
                actual_children,
                invocation_source,
                invocation_file,
                bindings,
            ) {
                Ok(()) => Ok(true),
                Err(MatchFailure::NoMatch) => Ok(false),
                Err(failure) => Err(failure),
            }
        }
        _ => Ok(false),
    }
}

fn is_metavariable_start(tree: &TokenTree) -> bool {
    matches!(
        tree,
        TokenTree::Leaf(Token {
            kind: TokenKind::Dollar,
            ..
        })
    )
}

fn metavariable(
    pattern: &[TokenTree],
    source: &SourceMap,
    file: FileId,
) -> Result<(String, FragmentKind, Span), MatchFailure> {
    let span = pattern
        .first()
        .map(TokenTree::span)
        .unwrap_or(Span::SYNTHETIC);
    let bad = |message: String| MatchFailure::BadMetavariable { span, message };

    let Some(TokenTree::Leaf(name)) = pattern.get(1) else {
        return Err(bad("a metavariable needs a name: `$name:kind`".into()));
    };
    if name.kind != TokenKind::Ident {
        return Err(bad("a metavariable needs a name: `$name:kind`".into()));
    }
    let Some(TokenTree::Leaf(colon)) = pattern.get(2) else {
        return Err(bad(
            "a metavariable needs a fragment kind: `$name:kind`".into()
        ));
    };
    if colon.kind != TokenKind::Colon {
        return Err(bad(
            "a metavariable needs a colon before its kind: `$name:kind`".into(),
        ));
    }
    let Some(TokenTree::Leaf(kind)) = pattern.get(3) else {
        return Err(bad(
            "a metavariable needs a fragment kind: `$name:kind`".into()
        ));
    };
    if kind.kind != TokenKind::Ident {
        return Err(bad(
            "a metavariable needs a fragment kind: `$name:kind`".into()
        ));
    }

    let variable = source.snippet(file, name.span).to_string();
    let kind_text = source.snippet(file, kind.span).to_string();
    let Some(kind) = FragmentKind::parse(&kind_text) else {
        return Err(bad(format!(
            "there is no `{kind_text}` fragment kind: the whole list is expr, stmt, pat, ty, ident, block, item"
        )));
    };
    Ok((variable, kind, span))
}

fn match_fragment(
    variable: String,
    kind: FragmentKind,
    span: Span,
    input: &[TokenTree],
    at: usize,
    source: &SourceMap,
    file: FileId,
    bindings: &mut Bindings,
) -> Result<usize, MatchFailure> {
    let Some(count) = match_prefix(kind, &input[at..], source, file) else {
        if kind == FragmentKind::Ident {
            return Err(MatchFailure::NoMatch);
        }
        return Err(MatchFailure::BadFragment {
            variable,
            kind,
            span,
            input: input[at..].to_vec(),
        });
    };
    let trees = input[at..at.saturating_add(count)].to_vec();
    if let Some(previous) = bindings.get(&variable) {
        if !same_shape(previous, &trees) {
            return Err(MatchFailure::NoMatch);
        }
    } else {
        bindings.insert(variable, trees);
    }
    Ok(at.saturating_add(count))
}

fn same_shape(left: &[TokenTree], right: &[TokenTree]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(a, b)| match (a, b) {
            (TokenTree::Leaf(a), TokenTree::Leaf(b)) => a.kind == b.kind,
            (
                TokenTree::Delimited {
                    open: a_open,
                    trees: a_children,
                    ..
                },
                TokenTree::Delimited {
                    open: b_open,
                    trees: b_children,
                    ..
                },
            ) => a_open == b_open && same_shape(a_children, b_children),
            _ => false,
        })
}

/// The diagnostic for an invocation that matched no arm: it prints the
/// invocation's tokens and the patterns it was tried against.
pub fn no_match_diagnostic(
    macro_name: &str,
    file: FileId,
    span: Span,
    invocation: &str,
    patterns: &[String],
) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(
        yashy_common::Severity::Error,
        file,
        span,
        format!("macro `{macro_name}` has no arm matching `{invocation}`"),
    )
    .with_label(invocation.to_string())
    .with_code(codes::MACRO_NO_MATCH);
    for pattern in patterns {
        diagnostic = diagnostic.with_note(format!("arm pattern: {pattern}"));
    }
    diagnostic
}

fn render_trees(trees: &[TokenTree], source: &SourceMap, file: FileId) -> String {
    trees
        .iter()
        .map(|tree| match source.snippet(file, tree.span()).trim() {
            "" => match tree {
                TokenTree::Leaf(token) => token.kind.describe(),
                TokenTree::Delimited { open, .. } => format!("{open:?}"),
            },
            text => text.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn token_trees_for(tokens: &[Token]) -> Result<Vec<TokenTree>, TreeError> {
    crate::tt::token_trees(tokens)
}
