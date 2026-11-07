//! Token trees: a flat token stream grouped by its delimiters.
//!
//! The lexer produces a flat stream; after a macro invocation is recognised,
//! everything the expander moves around is twenty-seven the delimiting
//! structure of that stream. A `(`, `[` or `{` opens one [`TokenTree::Delimited`]
//! that owns every tree inside it, and its matching closer becomes the group's
//! [`TokenTree::close`]. Once a group is built the expander can match a
//! metavariable against the leaves inside it without ever looking at the
//! delimiters themselves, and it can splicing one group's contents into a
//! replacement without re-tokenising anything.
//!
//! A tree borrows nothing: every leaf keeps the [`Token`] it came from,
//! including its [`Span`]. That is what lets diagnostics name the invocation
//! site after expansion, and it is why this builder stays small — it walks the
//! token slice once with a bracket stack and two counters. The depth cap stops
//! a hostile file from making the stack grow without bound; thirty such trees
//! are ordinary, and the cap is generous.

use yashy_common::Span;
use yashy_lexer::{Token, TokenKind};

/// The three delimiter pairs a group can open with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delim {
    Paren,
    Bracket,
    Brace,
}

/// One node of a token tree.
#[derive(Clone, Debug, PartialEq)]
pub enum TokenTree {
    /// A token that opens nothing: an identifier, a literal, an operator or a
    /// keyword.
    Leaf(Token),
    /// A delimiter pair and everything inside it.
    Delimited {
        open: Delim,
        trees: Vec<TokenTree>,
        /// The closer token, so an arm's `=> { .. }` keeps its braces. `Eof`
        /// and synthetic tokens have no closer to report; `None` then.
        close: Option<Token>,
        span: Span,
    },
}

/// The deepest this builder will nest before refusing the input.
pub const MAX_TREE_DEPTH: usize = 256;

/// What a token tree cannot be built from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeError {
    /// An opener whose closer never arrived.
    Unbalanced { open: Token },
    /// A closer that matches nothing.
    Stray { close: Token },
    /// Nesting past [`MAX_TREE_DEPTH`].
    TooDeep { at: Token },
}

/// Group `tokens` into token trees. `Eof` is not part of the result.
pub fn token_trees(tokens: &[Token]) -> Result<Vec<TokenTree>, TreeError> {
    build(tokens)
}

fn build(tokens: &[Token]) -> Result<Vec<TokenTree>, TreeError> {
    let mut roots: Vec<TokenTree> = Vec::new();
    let mut frames: Vec<(Delim, Span, Vec<TokenTree>)> = Vec::new();

    for &token in tokens {
        match token.kind {
            TokenKind::Eof => continue,
            TokenKind::LParen => open(&mut frames, Delim::Paren, token)?,
            TokenKind::LBracket => open(&mut frames, Delim::Bracket, token)?,
            TokenKind::LBrace => open(&mut frames, Delim::Brace, token)?,
            TokenKind::RParen => close(&mut frames, Delim::Paren, token, &mut roots)?,
            TokenKind::RBracket => close(&mut frames, Delim::Bracket, token, &mut roots)?,
            TokenKind::RBrace => close(&mut frames, Delim::Brace, token, &mut roots)?,
            _ => push(&mut frames, TokenTree::Leaf(token), &mut roots),
        }
    }

    if let Some((kind, span, _)) = frames.last() {
        let kind = match kind {
            Delim::Paren => TokenKind::LParen,
            Delim::Bracket => TokenKind::LBracket,
            Delim::Brace => TokenKind::LBrace,
        };
        return Err(TreeError::Unbalanced {
            open: Token { kind, span: *span },
        });
    }
    Ok(roots)
}

fn open(
    frames: &mut Vec<(Delim, Span, Vec<TokenTree>)>,
    kind: Delim,
    token: Token,
) -> Result<(), TreeError> {
    if frames.len() >= MAX_TREE_DEPTH {
        return Err(TreeError::TooDeep { at: token });
    }
    frames.push((kind, token.span, Vec::new()));
    Ok(())
}

fn close(
    frames: &mut Vec<(Delim, Span, Vec<TokenTree>)>,
    kind: Delim,
    token: Token,
    roots: &mut Vec<TokenTree>,
) -> Result<(), TreeError> {
    match frames.last() {
        Some((open, _, _)) if *open == kind => {
            let (open, start, trees) = frames.pop().expect("last was checked");
            let span = Span::new(start.start, token.span.end);
            push(
                frames,
                TokenTree::Delimited {
                    open,
                    trees,
                    close: Some(token),
                    span,
                },
                roots,
            );
            Ok(())
        }
        _ => Err(TreeError::Stray { close: token }),
    }
}

fn push(
    frames: &mut Vec<(Delim, Span, Vec<TokenTree>)>,
    tree: TokenTree,
    roots: &mut Vec<TokenTree>,
) {
    match frames.last_mut() {
        Some((_, _, trees)) => trees.push(tree),
        None => roots.push(tree),
    }
}

impl TokenTree {
    /// The span this tree covers in the source it was tokenised from.
    pub fn span(&self) -> Span {
        match self {
            TokenTree::Leaf(token) => token.span,
            TokenTree::Delimited { span, .. } => *span,
        }
    }

    /// The token leaves of this tree, left to right.
    pub fn leaves(&self) -> Vec<Token> {
        let mut out = Vec::new();
        collect(self, &mut out);
        out
    }

    /// The opener this tree needs in order to be spelled out again.
    pub fn open(&self) -> Option<TokenKind> {
        match self {
            TokenTree::Delimited { open, .. } => Some(match open {
                Delim::Paren => TokenKind::LParen,
                Delim::Bracket => TokenKind::LBracket,
                Delim::Brace => TokenKind::LBrace,
            }),
            TokenTree::Leaf(_) => None,
        }
    }

    /// The closer this tree needs in order to be spelled out again.
    pub fn close(&self) -> Option<TokenKind> {
        match self {
            TokenTree::Delimited { open, .. } => Some(match open {
                Delim::Paren => TokenKind::RParen,
                Delim::Bracket => TokenKind::RBracket,
                Delim::Brace => TokenKind::RBrace,
            }),
            TokenTree::Leaf(_) => None,
        }
    }
}

fn collect(tree: &TokenTree, out: &mut Vec<Token>) {
    match tree {
        TokenTree::Leaf(token) => out.push(*token),
        TokenTree::Delimited {
            open, trees, close, ..
        } => {
            out.push(Token {
                kind: match open {
                    Delim::Paren => TokenKind::LParen,
                    Delim::Bracket => TokenKind::LBracket,
                    Delim::Brace => TokenKind::LBrace,
                },
                span: tree.span(),
            });
            for child in trees {
                collect(child, out);
            }
            if let Some(closer) = close {
                out.push(*closer);
            }
        }
    }
}
