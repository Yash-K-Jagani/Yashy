//! One snapshot of a document, from which every LSP feature reads.
//!
//! Fourteen features built from fourteen private queries drift. Instead a
//! document becomes one `LspModel`: the symbols it declares, every occurrence
//! of each name, and the line index. A feature that needs a new fact adds it
//! here so the CLI's `index` and the MCP server can see the same fact.

use std::collections::HashMap;

use yashy_common::SourceMap;
use yashy_lexer::{Lexer, TokenKind};
use yashy_parser::ast::Item;
use yashy_parser::Parser;

use crate::text::LineIndex;

/// One declared name and its span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Struct,
    Enum,
    Trait,
    Constant,
    Variable,
}

impl SymbolKind {
    pub fn lsp(self) -> i64 {
        match self {
            SymbolKind::Function => 12, // Function
            SymbolKind::Struct => 23,   // Struct
            SymbolKind::Enum => 10,     // Enum
            SymbolKind::Trait => 26,    // Interface
            SymbolKind::Constant => 21, // Constant
            SymbolKind::Variable => 13, // Variable
        }
    }
}

/// A document snapshot.
pub struct LspModel {
    pub text: String,
    pub lines: LineIndex,
    pub symbols: Vec<Symbol>,
    /// name -> every byte range where that identifier occurs
    pub occurrences: HashMap<String, Vec<(usize, usize)>>,
}

impl LspModel {
    pub fn new(text: &str) -> Self {
        let symbols = declared_symbols(text);
        let mut occurrences: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
        for (name, start, end) in identifier_occurrences(text) {
            occurrences.entry(name).or_default().push((start, end));
        }
        Self {
            text: text.to_string(),
            lines: LineIndex::new(text),
            symbols,
            occurrences,
        }
    }

    /// The symbol whose declaration contains `offset`, if any.
    pub fn symbol_at(&self, offset: usize) -> Option<&Symbol> {
        self.symbols
            .iter()
            .find(|symbol| offset >= symbol.start && offset <= symbol.end)
    }

    pub fn range(&self, start: usize, end: usize) -> (u32, u32, u32, u32) {
        let (sl, sc) = self.lines.position(&self.text, start);
        let (el, ec) = self.lines.position(&self.text, end);
        (sl, sc, el, ec)
    }
}

fn declared_symbols(text: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    let mut sources = SourceMap::new();
    let file = sources.add_file("model.ya".to_string(), text.to_string());
    let Ok(tokens) = Lexer::with_file(text, file).tokenize() else {
        return symbols;
    };
    let Ok(program) = Parser::new(tokens, &sources, file).parse_program() else {
        return symbols;
    };
    for item in &program.items {
        let (name, kind, span) = match item {
            Item::Fn { name, span, .. } => (name.to_string(), SymbolKind::Function, *span),
            Item::Struct { name, span, .. } => (name.to_string(), SymbolKind::Struct, *span),
            Item::Enum { name, span, .. } => (name.to_string(), SymbolKind::Enum, *span),
            Item::Trait { name, span, .. } => (name.to_string(), SymbolKind::Trait, *span),
            Item::Documented { item, .. } => match item.as_ref() {
                Item::Fn { name, span, .. } => (name.to_string(), SymbolKind::Function, *span),
                Item::Struct { name, span, .. } => (name.to_string(), SymbolKind::Struct, *span),
                Item::Enum { name, span, .. } => (name.to_string(), SymbolKind::Enum, *span),
                Item::Trait { name, span, .. } => (name.to_string(), SymbolKind::Trait, *span),
                _ => continue,
            },
            _ => continue,
        };
        symbols.push(Symbol {
            name,
            kind,
            start: span.start as usize,
            end: span.end as usize,
        });
    }
    symbols
}

fn identifier_occurrences(text: &str) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    let mut sources = SourceMap::new();
    let file = sources.add_file("model.ya".to_string(), text.to_string());
    let Ok(tokens) = Lexer::with_file(text, file).tokenize() else {
        return out;
    };
    for token in tokens {
        if matches!(token.kind, TokenKind::Ident) {
            let start = token.span.start as usize;
            let end = token.span.end as usize;
            if let Some(name) = text.get(start..end) {
                out.push((name.to_string(), start, end));
            }
        }
    }
    out
}
