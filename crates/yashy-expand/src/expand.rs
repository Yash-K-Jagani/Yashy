//! The expander: the one place expansion happens, before the HIR exists.
//!
//! A macro is a set of arms; each arm is a pattern over [`TokenTree`]s with
//! typed metavariables and a replacement. The expander walks a token stream,
//! finds invocations (`name(args)` and `name { ... }`), matches them against
//! the arms, substitutes the bindings, and expands the result again. Every
//! nested expansion carries its depth in an explicit counter and is refused
//! past [`MAX_EXPANSION_DEPTH`], so a self-recursive macro is `E1601`, never a
//! stack overflow.

use std::collections::BTreeMap;

use yashy_common::{codes, Diagnostic, FileId, Severity, SourceMap, Span};
use yashy_lexer::{Token, TokenKind};
use yashy_parser::ast::Item;

use crate::hygiene::MarkCounter;
use crate::r#match::{matches, no_match_diagnostic, Bindings};
use crate::tt::{token_trees, TokenTree, TreeError};

/// Depth of nested expansion a single invocation may reach.
pub const MAX_EXPANSION_DEPTH: usize = 64;

/// Approximate token budget for one invocation's output: 1 MiB at four
/// bytes a token.
pub const MAX_EXPANSION_TOKENS: usize = 262_144;

impl ExpansionMap {
    /// Rewrite diagnostics whose spans lie in an expansion arm so they read
    /// as occurring at the invocation, with the arm and the call site kept
    /// in expansion notes.
    pub fn annotate(&self, sources: &SourceMap, file: FileId, diagnostics: &mut [Diagnostic]) {
        for diagnostic in diagnostics {
            let Some(record) = self
                .records
                .iter()
                .find(|record| overlaps(record.arm_span, diagnostic.span))
            else {
                continue;
            };
            diagnostic.span = record.invocation_span;
            let arm_line = sources.locate(file, record.arm_span).line;
            let invocation_line = sources.locate(file, record.invocation_span).line;
            diagnostic.notes.push(format!(
                "expansion: in macro `{}`, arm at line {arm_line}",
                record.macro_name
            ));
            diagnostic
                .notes
                .push(format!("expansion: expanded from line {invocation_line}"));
        }
    }
}

fn overlaps(left: Span, right: Span) -> bool {
    left.start < right.end && right.start < left.end
}

/// One expansion, in source order, for `yashy expand --json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpansionRecord {
    pub macro_name: String,
    pub arm_span: Span,
    pub invocation_span: Span,
    pub output_tokens: usize,
}

/// Every expansion a run performed, in the order they happened.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExpansionMap {
    pub records: Vec<ExpansionRecord>,
}

struct StoredArm {
    pattern: Vec<TokenTree>,
    replacement: Vec<TokenTree>,
    span: Span,
}

/// The macro table and the expansion state.
pub struct Expander<'a> {
    table: BTreeMap<String, Vec<StoredArm>>,
    source: &'a SourceMap,
    file: FileId,
    marks: MarkCounter,
    records: Vec<ExpansionRecord>,
    declaration_spans: Vec<Span>,
}

impl<'a> Expander<'a> {
    /// Build the table from a program's macro declarations.
    ///
    /// A name declared twice, or colliding with a function in the same
    /// module, is `E1603`: an invocation resolves by name, so one name can
    /// only expand one way.
    pub fn new(
        items: &[Item],
        source: &'a SourceMap,
        file: FileId,
    ) -> Result<Expander<'a>, Diagnostic> {
        let mut functions = Vec::new();
        let mut table: BTreeMap<String, Vec<StoredArm>> = BTreeMap::new();
        let mut declaration_spans = Vec::new();
        for item in items {
            match item {
                Item::Fn { name, span, .. } | Item::ConstFn { name, span, .. } => {
                    functions.push((name.clone(), *span));
                }
                Item::Macro { name, arms, span } => {
                    declaration_spans.push(*span);
                    if table.contains_key(name) {
                        return Err(Diagnostic::new(
                            Severity::Error,
                            file,
                            *span,
                            format!("macro `{name}` is already declared"),
                        )
                        .with_code(codes::MACRO_DUPLICATE));
                    }
                    let mut stored = Vec::new();
                    for arm in arms {
                        let pattern = token_trees(&arm.pattern)
                            .map_err(|error| err_tree(error, file, *span))?;
                        let replacement = token_trees(&arm.replacement)
                            .map_err(|error| err_tree(error, file, *span))?;
                        stored.push(StoredArm {
                            pattern,
                            replacement,
                            span: arm.span,
                        });
                    }
                    table.insert(name.clone(), stored);
                }
                _ => {}
            }
        }
        for (name, span) in functions {
            if table.contains_key(&name) {
                return Err(Diagnostic::new(
                    Severity::Error,
                    file,
                    span,
                    format!("macro `{name}` collides with a function of the same name"),
                )
                .with_code(codes::MACRO_DUPLICATE));
            }
        }
        Ok(Expander {
            table,
            source,
            file,
            marks: MarkCounter::new(),
            records: Vec::new(),
            declaration_spans,
        })
    }

    /// Expand a token stream, returning the expanded tokens and the map of
    /// expansions that produced them.
    pub fn expand(mut self, tokens: &[Token]) -> (Result<Vec<Token>, Diagnostic>, ExpansionMap) {
        let result = self.expand_tokens(tokens);
        (
            result,
            ExpansionMap {
                records: self.records,
            },
        )
    }

    fn expand_tokens(&mut self, tokens: &[Token]) -> Result<Vec<Token>, Diagnostic> {
        let tokens: Vec<Token> = tokens
            .iter()
            .copied()
            .filter(|token| {
                !self
                    .declaration_spans
                    .iter()
                    .any(|span| contains(*span, &token.span))
            })
            .collect();
        let trees =
            token_trees(&tokens).map_err(|error| err_tree(error, self.file, Span::SYNTHETIC))?;
        let expanded = self.expand_trees(&trees, 0)?;
        let mut out: Vec<Token> = expanded.iter().flat_map(TokenTree::leaves).collect();
        if !matches!(out.last(), Some(token) if token.kind == TokenKind::Eof) {
            out.push(Token {
                kind: TokenKind::Eof,
                span: Span::SYNTHETIC,
            });
        }
        Ok(out)
    }

    fn expand_trees(
        &mut self,
        trees: &[TokenTree],
        depth: usize,
    ) -> Result<Vec<TokenTree>, Diagnostic> {
        if depth > MAX_EXPANSION_DEPTH {
            return Err(Diagnostic::new(
                Severity::Error,
                self.file,
                trees
                    .first()
                    .map(TokenTree::span)
                    .unwrap_or(Span::SYNTHETIC),
                format!("macro expansion exceeded the depth limit of {MAX_EXPANSION_DEPTH}"),
            )
            .with_code(codes::MACRO_DEPTH));
        }

        let mut out: Vec<TokenTree> = Vec::new();
        let mut index = 0usize;
        while index < trees.len() {
            if let Some(expanded) = self.try_expand(trees, index, depth)? {
                out.extend(expanded.0);
                index = expanded.1;
                continue;
            }
            out.push(self.expand_one(&trees[index], depth)?);
            index = index.saturating_add(1);
        }
        Ok(out)
    }

    fn expand_one(&mut self, tree: &TokenTree, depth: usize) -> Result<TokenTree, Diagnostic> {
        match tree {
            TokenTree::Delimited { trees, .. } => {
                let expanded = self.expand_trees(trees, depth)?;
                Ok(TokenTree::Delimited {
                    open: open_of(tree),
                    trees: expanded,
                    close: close_of(tree),
                    span: tree.span(),
                })
            }
            TokenTree::Leaf(_) => Ok(tree.clone()),
        }
    }

    /// If `trees[index]` opens a macro invocation, expand it and return
    /// `(output, next_index)`.
    fn try_expand(
        &mut self,
        trees: &[TokenTree],
        index: usize,
        depth: usize,
    ) -> Result<Option<(Vec<TokenTree>, usize)>, Diagnostic> {
        let TokenTree::Leaf(name_token) = &trees[index] else {
            return Ok(None);
        };
        if name_token.kind != TokenKind::Ident {
            return Ok(None);
        }
        let name = self.source.snippet(self.file, name_token.span).to_string();
        let Some(arms) = self.table.get(&name) else {
            return Ok(None);
        };
        let Some(TokenTree::Delimited {
            trees: arguments, ..
        }) = trees.get(index.saturating_add(1))
        else {
            return Ok(None);
        };

        for arm in arms {
            match matches(
                &arm.pattern,
                self.source,
                self.file,
                arguments,
                self.source,
                self.file,
            ) {
                Ok(Some(bindings)) => {
                    self.marks.next();
                    let substituted = self.substitute(&arm.replacement, &bindings);
                    let output_tokens = count_tokens(&substituted);
                    if output_tokens > MAX_EXPANSION_TOKENS {
                        return Err(Diagnostic::new(
                            Severity::Error,
                            self.file,
                            name_token.span,
                            "macro expansion exceeded the 1 MiB output limit for one invocation",
                        )
                        .with_code(codes::MACRO_DEPTH));
                    }
                    self.records.push(ExpansionRecord {
                        macro_name: name.clone(),
                        arm_span: arm.span,
                        invocation_span: name_token.span,
                        output_tokens,
                    });
                    let expanded = self.expand_trees(&substituted, depth.saturating_add(1))?;
                    return Ok(Some((expanded, index.saturating_add(2))));
                }
                Ok(None) => continue,
                Err(failure) => {
                    return Err(failure
                        .diagnostic(self.file, self.source)
                        .unwrap_or_else(|| {
                            Diagnostic::new(
                                Severity::Error,
                                self.file,
                                name_token.span,
                                format!("{failure:?}"),
                            )
                            .with_code(codes::MACRO_FRAGMENT)
                        }))
                }
            }
        }

        let (invocation, invocation_span) = self.render_invocation(trees, index, name_token.span);
        let patterns: Vec<String> = arms
            .iter()
            .map(|arm| self.render_trees_span(&arm.pattern))
            .collect();
        Err(no_match_diagnostic(
            &name,
            self.file,
            invocation_span,
            &invocation,
            &patterns,
        ))
    }

    /// The invocation as written, `name(args)`, and the span it covers. The
    /// message is the tokens the caller actually wrote: "no arm matched" with
    /// an empty quotation is a support ticket, not a diagnostic.
    fn render_invocation(
        &self,
        trees: &[TokenTree],
        index: usize,
        fallback: Span,
    ) -> (String, Span) {
        let Some(name) = trees.get(index) else {
            return (String::new(), fallback);
        };
        let start = name.span().start;
        let end = match trees.get(index.saturating_add(1)) {
            Some(arguments @ TokenTree::Delimited { .. }) => arguments.span().end,
            _ => name.span().end,
        };
        let span = Span::new(start, end);
        (self.source.snippet(self.file, span).to_string(), span)
    }

    /// The source text of a token sequence, from its first token's start to
    /// its last token's end, so `$cond:expr` keeps the spelling the user
    /// wrote instead of being spaced token by token.
    fn render_trees_span(&self, trees: &[TokenTree]) -> String {
        match (trees.first(), trees.last()) {
            (Some(first), Some(last)) => {
                let span = Span::new(first.span().start, last.span().end);
                self.source.snippet(self.file, span).to_string()
            }
            _ => String::new(),
        }
    }

    fn substitute(&self, trees: &[TokenTree], bindings: &Bindings) -> Vec<TokenTree> {
        let mut out = Vec::new();
        let mut index = 0usize;
        while index < trees.len() {
            if is_dollar(&trees[index]) {
                if let Some(variable) = self.metavariable_name(&trees[index..]) {
                    if let Some(bound) = bindings.get(&variable) {
                        out.extend(bound.iter().cloned());
                        let has_kind = matches!(
                            trees.get(index.saturating_add(2)),
                            Some(TokenTree::Leaf(Token {
                                kind: TokenKind::Colon,
                                ..
                            }))
                        );
                        index = index.saturating_add(if has_kind { 4 } else { 2 });
                        continue;
                    }
                }
            }
            out.push(match &trees[index] {
                TokenTree::Delimited {
                    trees: children, ..
                } => {
                    let tree = &trees[index];
                    TokenTree::Delimited {
                        open: open_of(tree),
                        trees: self.substitute(children, bindings),
                        close: close_of(tree),
                        span: tree.span(),
                    }
                }
                leaf => leaf.clone(),
            });
            index = index.saturating_add(1);
        }
        out
    }

    fn metavariable_name(&self, trees: &[TokenTree]) -> Option<String> {
        let Some(TokenTree::Leaf(name)) = trees.get(1) else {
            return None;
        };
        if name.kind != TokenKind::Ident {
            return None;
        }
        Some(self.source.snippet(self.file, name.span).to_string())
    }
}

fn err_tree(error: TreeError, file: FileId, span: Span) -> Diagnostic {
    let (at, message) = match error {
        TreeError::Unbalanced { open } => (open.span, "an unclosed delimiter is not a token tree"),
        TreeError::Stray { close } => (close.span, "a stray closer is not part of a token tree"),
        TreeError::TooDeep { at } => (at.span, "the token trees are nested too deeply"),
    };
    Diagnostic::new(
        Severity::Error,
        file,
        if at == Span::SYNTHETIC { span } else { at },
        message,
    )
}

fn contains(outer: Span, inner: &Span) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

fn is_dollar(tree: &TokenTree) -> bool {
    matches!(
        tree,
        TokenTree::Leaf(Token {
            kind: TokenKind::Dollar,
            ..
        })
    )
}

fn open_of(tree: &TokenTree) -> crate::tt::Delim {
    match tree {
        TokenTree::Delimited { open, .. } => *open,
        TokenTree::Leaf(_) => crate::tt::Delim::Paren,
    }
}

fn close_of(tree: &TokenTree) -> Option<Token> {
    match tree {
        TokenTree::Delimited { close, .. } => *close,
        TokenTree::Leaf(_) => None,
    }
}

fn count_tokens(trees: &[TokenTree]) -> usize {
    trees.iter().map(|tree| tree.leaves().len()).sum()
}

pub fn expand_tokens<'a>(
    items: &[Item],
    source: &'a SourceMap,
    file: FileId,
    tokens: &[Token],
) -> (Result<Vec<Token>, Diagnostic>, ExpansionMap) {
    match Expander::new(items, source, file) {
        Ok(expander) => expander.expand(tokens),
        Err(diagnostic) => (Err(diagnostic), ExpansionMap::default()),
    }
}
