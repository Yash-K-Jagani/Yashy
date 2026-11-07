//! Fragment kinds: the typed holes a macro pattern can declare.
//!
//! A metavariable in a macro arm names a kind — `$cond:expr` — and the input
//! tokens in that position are re-read with the parser's own entry point for
//! that kind. There is deliberately no `tt` kind: a metavariable that can be
//! anything is one the checker cannot re-parse deterministically. The seven
//! kinds below are the whole list in v1.

use yashy_common::{FileId, SourceMap, Span};
use yashy_lexer::{Token, TokenKind};
use yashy_parser::Parser;

use crate::tt::TokenTree;

/// A fragment kind a metavariable can declare.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FragmentKind {
    Expr,
    Stmt,
    Pat,
    Ty,
    Ident,
    Block,
    Item,
}

impl FragmentKind {
    pub fn parse(text: &str) -> Option<FragmentKind> {
        Some(match text {
            "expr" => FragmentKind::Expr,
            "stmt" => FragmentKind::Stmt,
            "pat" => FragmentKind::Pat,
            "ty" => FragmentKind::Ty,
            "ident" => FragmentKind::Ident,
            "block" => FragmentKind::Block,
            "item" => FragmentKind::Item,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            FragmentKind::Expr => "expr",
            FragmentKind::Stmt => "stmt",
            FragmentKind::Pat => "pat",
            FragmentKind::Ty => "ty",
            FragmentKind::Ident => "ident",
            FragmentKind::Block => "block",
            FragmentKind::Item => "item",
        }
    }
}

/// Match input of `kind` against the start of `input`, returning how many
/// token trees it consumed.
///
/// The fragment parse is greedy within the kind: the longest prefix of the
/// input that parses as one complete fragment of that kind is the match. That
/// is the only way `$cond:expr` in front of `x > 0, ...` stops at the comma
/// while still taking `x > 0` whole.
pub fn match_prefix(
    kind: FragmentKind,
    input: &[TokenTree],
    source: &SourceMap,
    file: FileId,
) -> Option<usize> {
    if input.is_empty() {
        return None;
    }
    if kind == FragmentKind::Ident {
        return match input.first() {
            Some(TokenTree::Leaf(token)) if token.kind == TokenKind::Ident => Some(1),
            _ => None,
        };
    }
    if kind == FragmentKind::Block {
        return match input.first() {
            Some(TokenTree::Delimited {
                open: crate::tt::Delim::Brace,
                ..
            }) if parses(kind, input, source, file, 1) => Some(1),
            _ => None,
        };
    }
    for count in (1..=input.len()).rev() {
        if parses(kind, input, source, file, count) {
            return Some(count);
        }
    }
    None
}

fn parses(
    kind: FragmentKind,
    input: &[TokenTree],
    source: &SourceMap,
    file: FileId,
    count: usize,
) -> bool {
    let mut tokens: Vec<Token> = input[..count].iter().flat_map(TokenTree::leaves).collect();
    tokens.push(Token {
        kind: TokenKind::Eof,
        span: eof_span(&tokens),
    });
    let mut parser = Parser::new(tokens, source, file);
    let result = match kind {
        FragmentKind::Expr => parser.parse_expr().is_ok(),
        FragmentKind::Stmt => parser.parse_stmt().is_ok(),
        FragmentKind::Pat => parser.parse_pattern().is_ok(),
        FragmentKind::Ty => parser.parse_type_expr().is_ok(),
        FragmentKind::Block => parser.parse_block().is_ok(),
        FragmentKind::Item => parser.parse_item().is_ok(),
        FragmentKind::Ident => false,
    };
    result && parser.peek() == TokenKind::Eof
}

fn eof_span(tokens: &[Token]) -> Span {
    tokens
        .last()
        .map(|token| Span::new(token.span.end, token.span.end))
        .unwrap_or(Span::SYNTHETIC)
}
