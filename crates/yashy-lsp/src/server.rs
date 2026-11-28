//! The language server: diagnostics, hover, definition, and semantic tokens.
//!
//! The server reuses the compiler rather than re-implementing it: a document
//! is loaded by `yashy-driver`'s module loader (so an `import` resolves and
//! checks as it does in a build), then type-checked by `yashy-sema`. Everything
//! here is presentation: byte offsets become LSP positions, diagnostics become
//! ranges, and tokens become the semantic-token encoding.
//!
//! # Framing
//!
//! LSP over stdio is `Content-Length`-framed JSON. [`serve`] reads frames until
//! end of input, `shutdown`, or `exit`. A frame that is not JSON is reported on
//! stderr and the server keeps going: an editor can send anything, and a
//! language server that dies is worse than one that ignores a bad message.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use yashy_common::{FileId, SourceMap};
use yashy_driver::load;
use yashy_lexer::{Lexer, Token, TokenKind};
use yashy_sema::{check_modules_with, CheckOptions};

use crate::json::{number, object, string, Value};
use crate::text::{path_to_uri, uri_to_path, word_at, LineIndex};

/// Semantic token types, in the order the legend advertises them. The numeric
/// value in a token is this index.
const TOKEN_KEYWORD: u32 = 0;
const TOKEN_TYPE: u32 = 1;
const TOKEN_FUNCTION: u32 = 2;
const TOKEN_STRING: u32 = 3;
const TOKEN_NUMBER: u32 = 4;
const TOKEN_COMMENT: u32 = 5;

/// The names spelled as types in Yashy source.
const TYPE_NAMES: [&str; 10] = [
    "int", "float", "bool", "string", "void", "usize", "Option", "Result", "Channel", "map",
];

/// Run the server on `reader` until end of input, `shutdown`, or `exit`.
pub fn serve<R: BufRead, W: Write>(mut reader: R, writer: &mut W) -> io::Result<()> {
    let mut server = Server::default();
    loop {
        let message = match read_message(&mut reader) {
            Ok(Some(message)) => message,
            Ok(None) => return Ok(()),
            Err(error) => {
                eprintln!("yashy-lsp: {error}");
                return Ok(());
            }
        };
        if !server.handle(&message, writer)? {
            return Ok(());
        }
    }
}

#[derive(Default)]
struct Server {
    /// Open documents by URI. A document the editor has never sent is read
    /// from disk when a request needs it.
    documents: HashMap<String, String>,
    /// The workspace root from `initialize`, when the editor supplied one.
    /// A rename that would span other files is refused rather than silently
    /// editing only the open document.
    root: Option<PathBuf>,
}

impl Server {
    /// Handle one message. Returns false when the server must stop.
    fn handle<W: Write>(&mut self, message: &Value, out: &mut W) -> io::Result<bool> {
        let id = message.get("id").cloned();
        let method = message.get("method").and_then(Value::as_str);
        let params = message.get("params");

        match method {
            Some("initialize") => {
                let response = self.initialize(params);
                self.reply(out, id, response)?
            }
            Some("initialized") => {}
            Some("shutdown") => self.reply(out, id, Value::Null)?,
            Some("exit") => return Ok(false),
            Some("textDocument/didOpen") => self.did_open(params, out)?,
            Some("textDocument/didChange") => self.did_change(params, out)?,
            Some("textDocument/didSave") => self.did_save(params, out)?,
            Some("textDocument/hover") => {
                let result = self.hover(params);
                self.reply(out, id, result)?;
            }
            Some("textDocument/definition") => {
                let result = self.definition(params);
                self.reply(out, id, result)?;
            }
            Some("textDocument/completion") => {
                let result = self.completion(params);
                self.reply(out, id, result)?;
            }
            Some("textDocument/semanticTokens/full") => {
                let result = self.semantic_tokens(params);
                self.reply(out, id, result)?;
            }
            Some("textDocument/documentSymbol") => {
                self.reply(out, id, self.document_symbols(params))?
            }
            Some("textDocument/references") => self.reply(out, id, self.references(params))?,
            Some("textDocument/prepareRename") => {
                self.reply(out, id, self.prepare_rename(params))?
            }
            Some("textDocument/rename") => self.reply(out, id, self.rename(params))?,
            Some("textDocument/foldingRange") => {
                self.reply(out, id, self.folding_ranges(params))?
            }
            Some("textDocument/selectionRange") => {
                self.reply(out, id, self.selection_ranges(params))?
            }
            Some("textDocument/documentHighlight") => {
                self.reply(out, id, self.highlights(params))?
            }
            Some("textDocument/diagnostic") => {
                self.reply(out, id, self.pull_diagnostics(params))?
            }
            Some("textDocument/codeAction") => self.reply(out, id, self.code_actions(params))?,
            Some("textDocument/formatting") => self.reply(out, id, self.formatting(params))?,
            Some("textDocument/signatureHelp") => {
                self.reply(out, id, self.signature_help(params))?
            }
            Some("textDocument/inlayHint") => self.reply(out, id, self.inlay_hints(params))?,
            Some("workspace/symbol") => self.reply(out, id, self.workspace_symbols(params))?,
            Some("textDocument/prepareCallHierarchy") => {
                self.reply(out, id, self.prepare_call_hierarchy(params))?
            }
            Some("callHierarchy/incomingCalls") => self.reply(out, id, Value::Arr(Vec::new()))?,
            Some("callHierarchy/outgoingCalls") => self.reply(out, id, Value::Arr(Vec::new()))?,
            Some("textDocument/prepareTypeHierarchy") => {
                self.reply(out, id, self.prepare_type_hierarchy(params))?
            }
            Some("typeHierarchy/supertypes") => self.reply(out, id, Value::Arr(Vec::new()))?,
            Some("typeHierarchy/subtypes") => self.reply(out, id, Value::Arr(Vec::new()))?,
            Some("textDocument/codeLens") => self.reply(out, id, self.code_lens(params))?,
            Some(other) => {
                if let Some(id) = id {
                    self.reply_error(out, id, -32601, &format!("unknown method `{other}`"))?;
                }
            }
            // A response from the client. This server sends no requests, so
            // there is nothing to match it against.
            None => {}
        }
        Ok(true)
    }

    fn did_open<W: Write>(&mut self, params: Option<&Value>, out: &mut W) -> io::Result<()> {
        let Some(document) = params.and_then(|p| p.get("textDocument")) else {
            return Ok(());
        };
        let Some(uri) = document.get("uri").and_then(Value::as_str) else {
            return Ok(());
        };
        let Some(text) = document.get("text").and_then(Value::as_str) else {
            return Ok(());
        };
        self.documents.insert(uri.to_string(), text.to_string());
        self.publish(uri, text, out)
    }

    fn did_change<W: Write>(&mut self, params: Option<&Value>, out: &mut W) -> io::Result<()> {
        let Some(document) = params.and_then(|p| p.get("textDocument")) else {
            return Ok(());
        };
        let Some(uri) = document.get("uri").and_then(Value::as_str) else {
            return Ok(());
        };
        // The server advertises full sync, so the last change carries the
        // whole document.
        let Some(text) = params
            .and_then(|p| p.get("contentChanges"))
            .and_then(Value::as_array)
            .and_then(|changes| changes.last())
            .and_then(|change| change.get("text"))
            .and_then(Value::as_str)
        else {
            return Ok(());
        };
        self.documents.insert(uri.to_string(), text.to_string());
        self.publish(uri, text, out)
    }

    fn did_save<W: Write>(&mut self, params: Option<&Value>, out: &mut W) -> io::Result<()> {
        let Some(uri) = params
            .and_then(|p| p.get("textDocument"))
            .and_then(|document| document.get("uri"))
            .and_then(Value::as_str)
        else {
            return Ok(());
        };
        let text = self
            .documents
            .get(uri)
            .cloned()
            .or_else(|| std::fs::read_to_string(uri_to_path(uri)).ok());
        match text {
            Some(text) => self.publish(uri, &text, out),
            None => Ok(()),
        }
    }

    /// Check the document and publish its diagnostics.
    ///
    /// Diagnostics that belong to an imported file are dropped: the client
    /// would need a `publishDiagnostics` per file, and an editor showing the
    /// entry file should not be told about a file it has not opened.
    fn publish<W: Write>(&self, uri: &str, text: &str, out: &mut W) -> io::Result<()> {
        let diagnostics = self.analyze(uri, text);
        let index = LineIndex::new(text);
        let mut items = Vec::new();
        for diagnostic in diagnostics {
            let start = diagnostic.span.start as usize;
            let end = (diagnostic.span.end as usize)
                .max(start + 1)
                .min(text.len());
            let (start_line, start_character) = index.position(text, start);
            let (end_line, end_character) = index.position(text, end);
            items.push(object([
                (
                    "range",
                    object([
                        (
                            "start",
                            object([
                                ("line", number(start_line as i64)),
                                ("character", number(start_character as i64)),
                            ]),
                        ),
                        (
                            "end",
                            object([
                                ("line", number(end_line as i64)),
                                ("character", number(end_character as i64)),
                            ]),
                        ),
                    ]),
                ),
                ("severity", number(severity(diagnostic.severity))),
                ("source", string("yashy")),
                ("message", string(diagnostic.message)),
            ]));
        }
        let params = object([("uri", string(uri)), ("diagnostics", Value::Arr(items))]);
        self.notify(out, "textDocument/publishDiagnostics", params)
    }

    /// Load and check a document, returning the diagnostics for that document.
    ///
    /// The loader adds the entry itself, so the entry's text is the caller's
    /// and not a copy in the map: pass it in, and filter by the file the loader
    /// assigned it. A diagnostic in an imported module is dropped, because the
    /// client is only being told about the document it opened.
    fn analyze(&self, uri: &str, text: &str) -> Vec<yashy_common::Diagnostic> {
        let path = uri_to_path(uri);
        let entry_name = path.to_string_lossy().into_owned();
        let mut sources = SourceMap::new();
        let modules = match load(&mut sources, &path, text.to_string()) {
            Ok(modules) => modules,
            Err(diagnostics) => return keep_entry(&sources, diagnostics, &entry_name),
        };
        let Some(entry) = modules.last().map(|module| module.file) else {
            return Vec::new();
        };
        let borrowed: Vec<(FileId, &yashy_parser::Program)> = modules
            .iter()
            .map(|module| (module.file, &module.program))
            .collect();
        match check_modules_with(&borrowed, &sources, CheckOptions::default()) {
            Ok(_) => Vec::new(),
            Err(diagnostics) => diagnostics
                .into_iter()
                .filter(|diagnostic| diagnostic.file == entry)
                .collect(),
        }
    }

    fn hover(&self, params: Option<&Value>) -> Value {
        let Some((_uri, text, offset)) = self.cursor(params) else {
            return Value::Null;
        };
        let Some(word) = word_at(&text, offset) else {
            return Value::Null;
        };
        let signature = builtin_signature(word)
            .map(str::to_string)
            .or_else(|| macro_signature(&text, word))
            .or_else(|| user_signature(&text, word));
        match signature {
            Some(signature) => object([(
                "contents",
                object([
                    ("kind", string("markdown")),
                    ("value", string(format!("```yashy\n{signature}\n```"))),
                ]),
            )]),
            None => Value::Null,
        }
    }

    fn definition(&self, params: Option<&Value>) -> Value {
        let Some((uri, text, offset)) = self.cursor(params) else {
            return Value::Null;
        };
        let Some(word) = word_at(&text, offset) else {
            return Value::Null;
        };
        let path = uri_to_path(&uri);
        let entry_name = path.to_string_lossy().into_owned();
        // Trait-aware choice first: a call through a type parameter's
        // bound lands on the trait requirement; a plain method call
        // lands on the (single, static) impl.
        let defs = fn_defs(&text);
        let trait_def = defs.iter().find(|d| d.is_trait_method && d.name == word);
        if let Some(trait_def) = trait_def {
            let trait_name = trait_def.trait_name.clone().unwrap_or_default();
            let bound_call = defs
                .iter()
                .filter(|d| !d.is_method && !d.is_trait_method)
                .filter(|d| {
                    d.body
                        .map(|(s, e)| offset >= s && offset < e)
                        .unwrap_or(false)
                })
                .any(|d| {
                    let header = &text[d.span.0..];
                    let bound = format!(": {trait_name}");
                    header
                        .get(..header.find('{').unwrap_or(header.len()))
                        .map(|h| h.contains(&bound))
                        .unwrap_or(false)
                });
            let target = if bound_call {
                Some(trait_def)
            } else {
                defs.iter().find(|d| d.is_method && d.name == word)
            };
            if let Some(target) = target {
                let index = LineIndex::new(&text);
                let (start_line, start_character) = index.position(&text, target.span.0);
                let (end_line, end_character) = index.position(&text, target.span.1);
                return object([
                    ("uri", string(uri.clone())),
                    (
                        "range",
                        object([
                            (
                                "start",
                                object([
                                    ("line", number(start_line as i64)),
                                    ("character", number(start_character as i64)),
                                ]),
                            ),
                            (
                                "end",
                                object([
                                    ("line", number(end_line as i64)),
                                    ("character", number(end_character as i64)),
                                ]),
                            ),
                        ]),
                    ),
                ]);
            }
        }
        let mut sources = SourceMap::new();
        let modules = match load(&mut sources, &path, text.clone()) {
            Ok(modules) => modules,
            Err(_) => return Value::Null,
        };
        for module in &modules {
            // The entry's unsaved text is the one the editor is looking at.
            let module_text = if sources.path(module.file) == entry_name {
                text.clone()
            } else {
                std::fs::read_to_string(&module.path).unwrap_or_default()
            };
            if let Some(span) = function_name_span(&module_text, word) {
                let index = LineIndex::new(&module_text);
                let (start_line, start_character) = index.position(&module_text, span.0);
                let (end_line, end_character) = index.position(&module_text, span.1);
                return object([
                    ("uri", string(path_to_uri(&module.path))),
                    (
                        "range",
                        object([
                            (
                                "start",
                                object([
                                    ("line", number(start_line as i64)),
                                    ("character", number(start_character as i64)),
                                ]),
                            ),
                            (
                                "end",
                                object([
                                    ("line", number(end_line as i64)),
                                    ("character", number(end_character as i64)),
                                ]),
                            ),
                        ]),
                    ),
                ]);
            }
        }
        Value::Null
    }

    fn completion(&self, params: Option<&Value>) -> Value {
        let Some((_uri, text, offset)) = self.cursor(params) else {
            return Value::Null;
        };
        let defs = fn_defs(&text);
        // After `Type::`: associated functions only, never a method that
        // needs a receiver.
        let prefix = &text[..offset];
        if prefix.ends_with("::") {
            let before = &prefix[..prefix.len() - 2];
            let type_name: String = before
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            if !type_name.is_empty() {
                let items: Vec<Value> = defs
                    .iter()
                    .filter(|d| d.type_name.as_deref() == Some(type_name.as_str()) && !d.has_self)
                    .map(|d| {
                        object([
                            ("label", string(d.name.clone())),
                            ("kind", number(3)),
                            ("detail", string(d.signature.clone())),
                        ])
                    })
                    .collect();
                return object([("items", Value::Arr(items))]);
            }
        }
        // Inside an `impl Trait for Type` block but outside any method
        // body: the trait's methods this impl has not written yet.
        if let Some(block) = impl_block_at(&defs, &text, offset) {
            if let Some(trait_name) = block.trait_name.clone() {
                let done: std::collections::HashSet<&str> = defs
                    .iter()
                    .filter(|d| d.span.0 >= block.open && d.span.0 < block.close && d.is_method)
                    .map(|d| d.name.as_str())
                    .collect();
                let items: Vec<Value> = defs
                    .iter()
                    .filter(|d| {
                        d.trait_name.as_deref() == Some(trait_name.as_str()) && d.is_trait_method
                    })
                    .filter(|d| !done.contains(d.name.as_str()))
                    .map(|d| {
                        object([
                            ("label", string(d.name.clone())),
                            ("kind", number(3)),
                            ("detail", string(d.signature.clone())),
                        ])
                    })
                    .collect();
                if !items.is_empty() {
                    return object([("items", Value::Arr(items))]);
                }
            }
        }
        Value::Null
    }

    fn semantic_tokens(&self, params: Option<&Value>) -> Value {
        let Some(uri) = params
            .and_then(|p| p.get("textDocument"))
            .and_then(|document| document.get("uri"))
            .and_then(Value::as_str)
        else {
            return empty_tokens();
        };
        let Some(text) = self.documents.get(uri) else {
            return empty_tokens();
        };
        let tokens = match Lexer::new(text).tokenize() {
            Ok(tokens) => tokens,
            Err(_) => return empty_tokens(),
        };
        let index = LineIndex::new(text);
        let mut items: Vec<(u32, u32, u32, u32)> = Vec::new();

        for (position, token) in tokens.iter().enumerate() {
            if token.kind == TokenKind::Eof {
                break;
            }
            let token_text = text
                .get(token.span.start as usize..token.span.end as usize)
                .unwrap_or("");
            let previous = position
                .checked_sub(1)
                .and_then(|before| tokens.get(before))
                .map(|before| before.kind);
            let next = tokens.get(position + 1).map(|after| after.kind);
            if let Some(kind) = classify(token.kind, token_text, previous, next) {
                push_span(
                    &index,
                    text,
                    token.span.start as usize,
                    token.span.end as usize,
                    kind,
                    &mut items,
                );
            }
        }

        for (start, end) in comment_spans(text, &tokens) {
            push_span(&index, text, start, end, TOKEN_COMMENT, &mut items);
        }

        items.sort_by_key(|(line, start, _, _)| (*line, *start));
        let mut data = Vec::new();
        let mut previous_line = 0u32;
        let mut previous_start = 0u32;
        for (line, start, length, kind) in items {
            let delta_line = line - previous_line;
            let delta_start = if delta_line == 0 {
                start.saturating_sub(previous_start)
            } else {
                start
            };
            data.push(number(delta_line as i64));
            data.push(number(delta_start as i64));
            data.push(number(length as i64));
            data.push(number(kind as i64));
            data.push(number(0));
            previous_line = line;
            previous_start = start;
        }
        object([("data", Value::Arr(data))])
    }

    /// The URI, text, and byte offset a position points at.
    fn cursor(&self, params: Option<&Value>) -> Option<(String, String, usize)> {
        let document = params?.get("textDocument")?;
        let uri = document.get("uri")?.as_str()?.to_string();
        let text = self.documents.get(&uri)?.clone();
        let position = params?.get("position")?;
        let line = position.get("line")?.as_i64()? as u32;
        let character = position.get("character")?.as_i64()? as u32;
        let offset = LineIndex::new(&text).offset(&text, line, character)?;
        Some((uri, text, offset))
    }

    /// The document text for a URI, from the open buffer or from disk.
    fn text_for(&self, uri: &str) -> Option<String> {
        if let Some(text) = self.documents.get(uri) {
            return Some(text.clone());
        }
        std::fs::read_to_string(uri_to_path(uri)).ok()
    }

    fn model_for(&self, params: Option<&Value>) -> Option<(String, crate::model::LspModel, usize)> {
        let (uri, text, offset) = self.cursor(params)?;
        let model = crate::model::LspModel::new(&text);
        Some((uri, model, offset))
    }

    fn document_symbols(&self, params: Option<&Value>) -> Value {
        let Some(uri) = params
            .and_then(|p| p.get("textDocument"))
            .and_then(|d| d.get("uri"))
            .and_then(Value::as_str)
        else {
            return Value::Null;
        };
        let Some(text) = self.text_for(uri) else {
            return Value::Null;
        };
        let model = crate::model::LspModel::new(&text);
        Value::Arr(
            model
                .symbols
                .iter()
                .map(|symbol| {
                    let (sl, sc, el, ec) = model.range(symbol.start, symbol.end);
                    object([
                        ("name", string(&symbol.name)),
                        ("kind", number(symbol.kind.lsp())),
                        ("range", range(sl, sc, el, ec)),
                        ("selectionRange", range(sl, sc, el, ec)),
                    ])
                })
                .collect(),
        )
    }

    fn references(&self, params: Option<&Value>) -> Value {
        let Some((_uri, model, offset)) = self.model_for(params) else {
            return Value::Null;
        };
        let Some(word) = word_at(&model.text, offset) else {
            return Value::Null;
        };
        let Some(occurrences) = model.occurrences.get(word) else {
            return Value::Null;
        };
        Value::Arr(
            occurrences
                .iter()
                .map(|(start, end)| {
                    let (sl, sc, el, ec) = model.range(*start, *end);
                    object([("range", range(sl, sc, el, ec))])
                })
                .collect(),
        )
    }

    fn prepare_rename(&self, params: Option<&Value>) -> Value {
        let Some((_uri, model, offset)) = self.model_for(params) else {
            return Value::Null;
        };
        let Some(word) = word_at(&model.text, offset) else {
            return Value::Null;
        };
        let Some((start, end)) = model
            .occurrences
            .get(word)
            .and_then(|occurrences| occurrences.first())
        else {
            return Value::Null;
        };
        let (sl, sc, el, ec) = model.range(*start, *end);
        object([
            ("range", range(sl, sc, el, ec)),
            ("placeholder", string(word)),
        ])
    }

    fn rename(&self, params: Option<&Value>) -> Value {
        let Some(new_name) = params
            .and_then(|p| p.get("newName"))
            .and_then(Value::as_str)
        else {
            return Value::Null;
        };
        // A rename must be an identifier, and must not collide with another
        // declared name. Refuse rather than corrupt the document.
        if new_name.is_empty()
            || !new_name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
            || new_name.chars().next().is_some_and(|c| c.is_ascii_digit())
        {
            return object([("error", string("not a valid Yashy identifier"))]);
        }
        let Some((uri, model, offset)) = self.model_for(params) else {
            return Value::Null;
        };
        let Some(word) = word_at(&model.text, offset) else {
            return Value::Null;
        };
        if model.symbols.iter().any(|symbol| symbol.name == new_name) {
            return object([("error", string("the new name is already declared"))]);
        }
        if self.name_is_used_elsewhere(new_name, uri.as_str()) {
            return object([(
                "error",
                string("the new name is used in another file of the workspace"),
            )]);
        }
        let Some(occurrences) = model.occurrences.get(word) else {
            return Value::Null;
        };
        let edits: Vec<Value> = occurrences
            .iter()
            .map(|(start, end)| {
                let (sl, sc, el, ec) = model.range(*start, *end);
                object([
                    ("range", range(sl, sc, el, ec)),
                    ("newText", string(new_name)),
                ])
            })
            .collect();
        object([("changes", object([(uri.as_str(), Value::Arr(edits))]))])
    }

    fn folding_ranges(&self, params: Option<&Value>) -> Value {
        let Some((_uri, model, _)) = self.model_for(params) else {
            return Value::Null;
        };
        Value::Arr(
            model
                .symbols
                .iter()
                .map(|symbol| {
                    let (sl, _, el, _) = model.range(symbol.start, symbol.end);
                    object([
                        ("startLine", number(i64::from(sl))),
                        ("endLine", number(i64::from(el))),
                    ])
                })
                .collect(),
        )
    }

    fn selection_ranges(&self, params: Option<&Value>) -> Value {
        let Some((_uri, model, offset)) = self.model_for(params) else {
            return Value::Null;
        };
        let Some(word) = word_at(&model.text, offset) else {
            return Value::Null;
        };
        let occurrences = model
            .occurrences
            .get(word)
            .cloned()
            .unwrap_or_else(|| vec![(offset, offset)]);
        Value::Arr(
            occurrences
                .iter()
                .map(|(start, end)| {
                    let (sl, sc, el, ec) = model.range(*start, *end);
                    object([("range", range(sl, sc, el, ec))])
                })
                .collect(),
        )
    }

    fn highlights(&self, params: Option<&Value>) -> Value {
        self.references(params)
    }

    fn pull_diagnostics(&self, params: Option<&Value>) -> Value {
        let Some(uri) = params
            .and_then(|p| p.get("textDocument"))
            .and_then(|d| d.get("uri"))
            .and_then(Value::as_str)
        else {
            return Value::Null;
        };
        let Some(text) = self.text_for(uri) else {
            return Value::Null;
        };
        let diagnostics = diagnostics_text(&text);
        // `resultId` lets a client skip re-rendering an unchanged result. It is
        // a hash of the text and the rendered items, so the same document and
        // the same diagnostics always produce the same id in this session.
        let mut rendered = String::new();
        for item in &diagnostics {
            rendered.push_str(&format!("{item:?}"));
        }
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for byte in text.as_bytes().iter().chain(rendered.as_bytes()) {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        let result_id = format!("{hash:016x}");
        let previous = params
            .and_then(|p| p.get("previousResultId"))
            .and_then(Value::as_str);
        let items = if previous == Some(result_id.as_str()) {
            Vec::new()
        } else {
            diagnostics
        };
        object([
            ("items", Value::Arr(items)),
            ("resultId", string(&result_id)),
        ])
    }

    fn code_actions(&self, params: Option<&Value>) -> Value {
        let Some((uri, model, offset)) = self.model_for(params) else {
            return Value::Null;
        };
        let mut actions = Vec::new();

        // Removed keywords have one obvious replacement. The same edit is
        // offered by `yashyc fix`; neither side formats text on its own.
        for (word, replacement) in [("var", "let"), ("println", "print")] {
            for (start, end) in model.occurrences.get(word).cloned().unwrap_or_default() {
                if start <= offset && offset <= end {
                    actions.push(quick_fix(
                        &uri,
                        &model,
                        start,
                        end,
                        replacement,
                        format!("use `{replacement}`"),
                        "quickfix",
                    ));
                }
            }
        }

        // Every edit the checker already proved, offered at the cursor. A
        // diagnostic's suggestions are one action, not one per edit: an
        // argument-order fix is two spans that only make sense together, and
        // applying half of it would produce a different wrong call.
        for diagnostic in &check_text(&model.text) {
            let edits: Vec<(usize, usize, String)> = diagnostic
                .suggestions
                .iter()
                .filter(|suggestion| {
                    suggestion.applicability == yashy_common::Applicability::MachineApplicable
                })
                .map(|suggestion| {
                    let start = (suggestion.span.start as usize).min(model.text.len());
                    let end = (suggestion.span.end as usize)
                        .max(start)
                        .min(model.text.len());
                    (start, end, suggestion.replacement.clone())
                })
                .collect();
            if edits.is_empty() {
                continue;
            }
            let start = (diagnostic.span.start as usize).min(model.text.len());
            let end = (diagnostic.span.end as usize)
                .max(start)
                .min(model.text.len());
            if offset < start || offset > end {
                continue;
            }
            actions.push(quick_fix_edits(
                &uri,
                &model,
                &edits,
                format!("fix: {}", diagnostic.message),
                "quickfix",
            ));
        }

        // Rename stays as a refactor: the user supplies the new name, so this
        // action is intentionally not a machine-applicable fix.
        if let Some(word) = word_at(&model.text, offset) {
            if let Some((start, end)) = model
                .occurrences
                .get(word)
                .and_then(|occurrences| occurrences.first())
            {
                let (sl, sc, el, ec) = model.range(*start, *end);
                actions.push(object([
                    ("title", string(format!("rename `{word}`"))),
                    ("kind", string("refactor.rename")),
                    ("edit", object([("changes", Value::Arr(vec![]))])),
                    ("range", range(sl, sc, el, ec)),
                ]));
            }
        }
        Value::Arr(actions)
    }

    fn formatting(&self, params: Option<&Value>) -> Value {
        let Some(uri) = params
            .and_then(|p| p.get("textDocument"))
            .and_then(|d| d.get("uri"))
            .and_then(Value::as_str)
        else {
            return Value::Null;
        };
        let Some(text) = self.text_for(uri) else {
            return Value::Null;
        };
        // The formatter is the single source of layout. A rename of this verb
        // must not become a second formatting implementation in the LSP.
        let formatted = yashy_fmt::format(&text).unwrap_or_else(|_| text.clone());
        Value::Arr(vec![object([
            ("newText", string(formatted)),
            ("range", range(0, 0, u32::MAX, u32::MAX)),
        ])])
    }

    fn signature_help(&self, params: Option<&Value>) -> Value {
        let Some((_uri, model, offset)) = self.model_for(params) else {
            return Value::Null;
        };
        // Walk back to the unmatched `(` and read the callee before it, so a
        // request anywhere inside the argument list names the same signature.
        let bytes = model.text.as_bytes();
        let mut at = offset.min(bytes.len());
        let mut depth = 0usize;
        while at > 0 {
            at -= 1;
            match bytes[at] {
                b')' => depth += 1,
                b'(' => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
        let mut end = at;
        while end > 0 && bytes[end - 1].is_ascii_whitespace() {
            end -= 1;
        }
        let mut start = end;
        while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
            start -= 1;
        }
        let name = model.text.get(start..end).unwrap_or("");
        if name.is_empty() {
            return Value::Null;
        }
        let signature = model
            .symbols
            .iter()
            .find(|symbol| symbol.name == name)
            .map(|symbol| symbol.signature.clone())
            .or_else(|| builtin_signature(name).map(str::to_string));
        match signature {
            Some(signature) => object([
                (
                    "signatures",
                    Value::Arr(vec![object([
                        ("label", string(signature)),
                        ("parameters", Value::Arr(Vec::new())),
                    ])]),
                ),
                ("activeSignature", number(0)),
                ("activeParameter", number(0)),
            ]),
            None => Value::Null,
        }
    }

    fn inlay_hints(&self, params: Option<&Value>) -> Value {
        let Some((_uri, model, _)) = self.model_for(params) else {
            return Value::Null;
        };
        // An untyped `let` infers its type from a literal initialiser. Showing
        // that type at the binding is the one hint the checker can prove
        // without a second inference pass.
        Value::Arr(
            inferred_bindings(&model.text)
                .into_iter()
                .map(|(end, label)| {
                    let (line, character) = model.lines.position(&model.text, end);
                    object([
                        (
                            "position",
                            object([
                                ("line", number(i64::from(line))),
                                ("character", number(i64::from(character))),
                            ]),
                        ),
                        ("label", string(format!(": {label}"))),
                        ("kind", number(1)),
                    ])
                })
                .collect(),
        )
    }

    fn workspace_symbols(&self, params: Option<&Value>) -> Value {
        let query = params
            .and_then(|params| params.get("query"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase();
        // With a workspace root, the symbols come from the compiler's index
        // snapshot rather than from the open documents: an editor that has not
        // opened a file still sees its declarations.
        if let Some(root) = &self.root {
            if let Ok(model) = yashy_driver::index::build_index(root) {
                let mut from_index = Vec::new();
                for symbol in model.symbols {
                    if !query.is_empty() && !symbol.name.to_lowercase().contains(&query) {
                        continue;
                    }
                    let Ok(text) = std::fs::read_to_string(&symbol.file) else {
                        continue;
                    };
                    let lines = crate::text::LineIndex::new(&text);
                    let (sl, sc) = lines.position(&text, symbol.span.start as usize);
                    let (el, ec) = lines.position(&text, symbol.span.end as usize);
                    from_index.push(object([
                        ("name", string(&symbol.name)),
                        ("kind", number(symbol_kind(&symbol.kind))),
                        (
                            "location",
                            object([
                                ("uri", string(path_to_uri(Path::new(&symbol.file)))),
                                ("range", range(sl, sc, el, ec)),
                            ]),
                        ),
                    ]));
                }
                return Value::Arr(from_index);
            }
        }
        let mut symbols = Vec::new();
        for (uri, text) in &self.documents {
            let model = crate::model::LspModel::new(text);
            for symbol in &model.symbols {
                if !query.is_empty() && !symbol.name.to_lowercase().contains(&query) {
                    continue;
                }
                let (sl, sc, el, ec) = model.range(symbol.start, symbol.end);
                symbols.push(object([
                    ("name", string(&symbol.name)),
                    ("kind", number(symbol.kind.lsp())),
                    (
                        "location",
                        object([("uri", string(uri)), ("range", range(sl, sc, el, ec))]),
                    ),
                ]));
            }
        }
        Value::Arr(symbols)
    }

    fn hierarchy_item(&self, params: Option<&Value>) -> Value {
        let Some((uri, model, offset)) = self.model_for(params) else {
            return Value::Null;
        };
        let Some(word) = word_at(&model.text, offset) else {
            return Value::Null;
        };
        let Some(symbol) = model.symbols.iter().find(|symbol| symbol.name == word) else {
            return Value::Null;
        };
        let (sl, sc, el, ec) = model.range(symbol.start, symbol.end);
        object([
            ("name", string(&symbol.name)),
            ("kind", number(symbol.kind.lsp())),
            ("uri", string(&uri)),
            ("range", range(sl, sc, el, ec)),
            ("selectionRange", range(sl, sc, el, ec)),
        ])
    }

    fn prepare_call_hierarchy(&self, params: Option<&Value>) -> Value {
        self.hierarchy_item(params)
    }

    fn prepare_type_hierarchy(&self, params: Option<&Value>) -> Value {
        self.hierarchy_item(params)
    }

    fn code_lens(&self, params: Option<&Value>) -> Value {
        let Some((uri, model, _)) = self.model_for(params) else {
            return Value::Null;
        };
        Value::Arr(
            model
                .symbols
                .iter()
                .map(|symbol| {
                    let (sl, sc, el, ec) = model.range(symbol.start, symbol.end);
                    let count = model
                        .occurrences
                        .get(&symbol.name)
                        .map(Vec::len)
                        .unwrap_or(0);
                    object([
                        ("range", range(sl, sc, el, ec)),
                        (
                            "command",
                            object([
                                ("title", string(format!("{count} references"))),
                                ("command", string("editor.action.findReferences")),
                                (
                                    "arguments",
                                    Value::Arr(vec![string(uri.clone()), number(i64::from(sl))]),
                                ),
                            ]),
                        ),
                    ])
                })
                .collect(),
        )
    }

    fn reply<W: Write>(&self, out: &mut W, id: Option<Value>, result: Value) -> io::Result<()> {
        let Some(id) = id else {
            return Ok(());
        };
        write_message(
            out,
            &object([("jsonrpc", string("2.0")), ("id", id), ("result", result)]),
        )
    }

    fn reply_error<W: Write>(
        &self,
        out: &mut W,
        id: Value,
        code: i64,
        message: &str,
    ) -> io::Result<()> {
        write_message(
            out,
            &object([
                ("jsonrpc", string("2.0")),
                ("id", id),
                (
                    "error",
                    object([("code", number(code)), ("message", string(message))]),
                ),
            ]),
        )
    }

    fn notify<W: Write>(&self, out: &mut W, method: &str, params: Value) -> io::Result<()> {
        write_message(
            out,
            &object([
                ("jsonrpc", string("2.0")),
                ("method", string(method)),
                ("params", params),
            ]),
        )
    }

    fn initialize(&mut self, params: Option<&Value>) -> Value {
        self.root = params
            .and_then(|params| params.get("rootUri"))
            .and_then(Value::as_str)
            .map(uri_to_path)
            .or_else(|| {
                params
                    .and_then(|params| params.get("rootPath"))
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
            });
        initialize()
    }

    /// Whether the new name already appears in a different file of the
    /// workspace. A project-wide use cannot be renamed by editing one
    /// document, so the caller must refuse rather than produce a partial edit.
    fn name_is_used_elsewhere(&self, name: &str, current: &str) -> bool {
        let Some(root) = &self.root else {
            return false;
        };
        let mut files = Vec::new();
        collect_ya_files(root, &mut files);
        files
            .into_iter()
            .filter(|path| path_to_uri(path) != current)
            .any(|path| {
                let Ok(text) = std::fs::read_to_string(&path) else {
                    return false;
                };
                let Ok(tokens) = Lexer::new(&text).tokenize() else {
                    return false;
                };
                tokens.iter().any(|token| {
                    token.kind == TokenKind::Ident
                        && &text[token.span.start as usize..token.span.end as usize] == name
                })
            })
    }
}

/// Map a compiler index kind to the LSP symbol kind number.
fn symbol_kind(kind: &str) -> i64 {
    match kind {
        "fn" => 12,       // Function
        "struct" => 23,   // Struct
        "enum" => 10,     // Enum
        "trait" => 26,    // Interface
        "const" => 21,    // Constant
        "variable" => 13, // Variable
        _ => 13,          // Variable
    }
}

fn collect_ya_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let skip = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == "target" || name.starts_with('.'));
            if !skip {
                collect_ya_files(&path, out);
            }
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("ya") {
            out.push(path);
        }
    }
}

fn initialize() -> Value {
    object([
        (
            "capabilities",
            object([
                // 1 is full sync: a change carries the whole document.
                ("textDocumentSync", number(1)),
                ("hoverProvider", Value::Bool(true)),
                ("definitionProvider", Value::Bool(true)),
                ("referencesProvider", Value::Bool(true)),
                (
                    "renameProvider",
                    object([("prepareProvider", Value::Bool(true))]),
                ),
                ("documentSymbolProvider", Value::Bool(true)),
                ("foldingRangeProvider", Value::Bool(true)),
                ("selectionRangeProvider", Value::Bool(true)),
                ("documentHighlightProvider", Value::Bool(true)),
                ("codeActionProvider", Value::Bool(true)),
                ("documentFormattingProvider", Value::Bool(true)),
                (
                    "signatureHelpProvider",
                    object([(
                        "triggerCharacters",
                        Value::Arr(vec![string("("), string(",")]),
                    )]),
                ),
                ("inlayHintProvider", Value::Bool(true)),
                ("workspaceSymbolProvider", Value::Bool(true)),
                ("callHierarchyProvider", Value::Bool(true)),
                ("typeHierarchyProvider", Value::Bool(true)),
                (
                    "codeLensProvider",
                    object([("resolveProvider", Value::Bool(false))]),
                ),
                (
                    "diagnosticProvider",
                    object([
                        ("interFileDependencies", Value::Bool(false)),
                        ("workspaceDiagnostics", Value::Bool(false)),
                    ]),
                ),
                (
                    "completionProvider",
                    object([("triggerCharacters", Value::Arr(vec![string(":")]))]),
                ),
                (
                    "semanticTokensProvider",
                    object([
                        (
                            "legend",
                            object([
                                (
                                    "tokenTypes",
                                    Value::Arr(
                                        [
                                            "keyword", "type", "function", "string", "number",
                                            "comment",
                                        ]
                                        .map(string)
                                        .to_vec(),
                                    ),
                                ),
                                ("tokenModifiers", Value::Arr(Vec::new())),
                            ]),
                        ),
                        ("full", Value::Bool(true)),
                    ]),
                ),
            ]),
        ),
        (
            "serverInfo",
            object([
                ("name", string("yashy-lsp")),
                ("version", string(env!("CARGO_PKG_VERSION"))),
            ]),
        ),
    ])
}

fn empty_tokens() -> Value {
    object([("data", Value::Arr(Vec::new()))])
}

/// The diagnostics that belong to the entry document, for a load that failed
/// before the loader could hand back module ids.
fn keep_entry(
    sources: &SourceMap,
    diagnostics: Vec<yashy_common::Diagnostic>,
    entry: &str,
) -> Vec<yashy_common::Diagnostic> {
    diagnostics
        .into_iter()
        .filter(|diagnostic| sources.path(diagnostic.file) == entry)
        .collect()
}

/// An LSP Range from four zero-based coordinates.
fn range(sl: u32, sc: u32, el: u32, ec: u32) -> Value {
    object([
        (
            "start",
            object([
                ("line", number(i64::from(sl))),
                ("character", number(i64::from(sc))),
            ]),
        ),
        (
            "end",
            object([
                ("line", number(i64::from(el))),
                ("character", number(i64::from(ec))),
            ]),
        ),
    ])
}

/// One `let` binding whose initialiser is a literal, so its type is provable
/// from the token alone. Returns the byte offset at the end of the name and the
/// type label.
fn inferred_bindings(text: &str) -> Vec<(usize, &'static str)> {
    let mut sources = SourceMap::new();
    let file = sources.add_file("model.ya".to_string(), text.to_string());
    let Ok(tokens) = Lexer::with_file(text, file).tokenize() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        if tokens[index].kind != TokenKind::Let {
            index += 1;
            continue;
        }
        let mut at = index + 1;
        let mut names = Vec::new();
        while at < tokens.len()
            && !matches!(
                tokens[at].kind,
                TokenKind::Eq | TokenKind::Semi | TokenKind::Eof
            )
        {
            if tokens[at].kind == TokenKind::Ident {
                names.push(at);
            }
            at += 1;
        }
        // One identifier before `=` is an untyped binding; two or more mean the
        // first one is the written type, which needs no hint.
        if names.len() == 1 && tokens.get(at).map(|token| token.kind) == Some(TokenKind::Eq) {
            let label = tokens.get(at + 1).and_then(|token| match token.kind {
                TokenKind::I64 => Some("int"),
                TokenKind::F64 => Some("float"),
                TokenKind::Str => Some("string"),
                TokenKind::Char => Some("char"),
                TokenKind::True | TokenKind::False => Some("bool"),
                _ => None,
            });
            if let Some(label) = label {
                out.push((tokens[names[0]].span.end as usize, label));
            }
        }
        index = at.max(index + 1);
    }
    out
}

/// Diagnostics for a source string, in LSP form, for pull diagnostics.
fn diagnostics_text(text: &str) -> Vec<Value> {
    diagnostics_to_lsp(text, &check_text(text))
}

/// The checker's diagnostics for a source string, with their edits attached.
///
/// The LSP and `yashyc check` must agree about what is fixable, so this runs
/// the driver's loader and the same checker, then the same suggestion
/// attacher. Nothing here invents an edit the CLI would not also offer.
fn check_text(text: &str) -> Vec<yashy_common::Diagnostic> {
    let uri = "file:///model.ya";
    let path = uri_to_path(uri);
    let mut sources = SourceMap::new();
    let mut diagnostics = match load(&mut sources, &path, text.to_string()) {
        Ok(modules) => {
            let borrowed: Vec<(FileId, &yashy_parser::Program)> = modules
                .iter()
                .map(|module| (module.file, &module.program))
                .collect();
            match check_modules_with(&borrowed, &sources, CheckOptions::default()) {
                Ok(_) => Vec::new(),
                Err(diagnostics) => diagnostics,
            }
        }
        Err(diagnostics) => diagnostics,
    };
    yashy_driver::fix::attach_typo_suggestions(&sources, &mut diagnostics);
    diagnostics
}

fn diagnostics_to_lsp(text: &str, diagnostics: &[yashy_common::Diagnostic]) -> Vec<Value> {
    let index = LineIndex::new(text);
    diagnostics
        .iter()
        .map(|diagnostic| {
            let start = diagnostic.span.start as usize;
            let end = (diagnostic.span.end as usize)
                .max(start + 1)
                .min(text.len());
            let (sl, sc) = index.position(text, start);
            let (el, ec) = index.position(text, end);
            object([
                ("range", range(sl, sc, el, ec)),
                ("severity", number(severity(diagnostic.severity))),
                ("source", string("yashy")),
                ("message", string(&diagnostic.message)),
            ])
        })
        .collect()
}

fn severity(severity: yashy_common::Severity) -> i64 {
    match severity {
        yashy_common::Severity::Error => 1,
        yashy_common::Severity::Warning => 2,
    }
}

/// The builtin signatures a reader is most likely to hover. The checker owns
/// the authoritative list; this table only decides what the hover shows, and a
/// builtin missing here simply hovers as its own name via the user-function
/// path.
fn builtin_signature(name: &str) -> Option<&'static str> {
    Some(match name {
        "print" => "fn print(...) -> void",
        "alloc" => "fn alloc(int count) -> *mut int",
        "stack_alloc" => "fn stack_alloc(int count) -> *mut int",
        "len" => "fn len(value) -> int",
        "str_from_i64" => "fn str_from_i64(int value) -> string",
        "read_line" => "fn read_line() -> string",
        "assert" => "fn assert(bool condition)",
        "exit" => "fn exit(int code)",
        _ => return None,
    })
}

/// The source of `macro <word> { ... }`, for a hover on a macro. The arms
/// are the whole answer: a reader hovering an invocation wants to see what
/// shapes it accepts, and the declaration is those shapes.
fn macro_signature(text: &str, word: &str) -> Option<String> {
    let mut from = 0;
    while let Some(found) = text.get(from..)?.find("macro ") {
        let start = from + found;
        let rest = text.get(start + 6..)?;
        let name: String = rest
            .chars()
            .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
            .collect();
        if name == word {
            let open = start + text.get(start..)?.find('{')?;
            let mut depth = 0usize;
            for (offset, character) in text.char_indices().skip(open) {
                match character {
                    '{' => depth = depth.saturating_add(1),
                    '}' => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            let end = offset + character.len_utf8();
                            let declaration = text.get(start..end)?.trim();
                            return Some(
                                declaration.split_whitespace().collect::<Vec<_>>().join(" "),
                            );
                        }
                    }
                    _ => {}
                }
            }
            return None;
        }
        from = start + 6;
    }
    None
}

/// The source text of `fn <word>` up to its body, for a hover on a user
/// function. A doc block above the function is included, so the hover reads
/// as the documentation followed by the signature.
fn user_signature(text: &str, word: &str) -> Option<String> {
    let mut from = 0;
    while let Some(found) = text.get(from..)?.find("fn ") {
        let start = from + found;
        let rest = text.get(start + 3..)?;
        let name: String = rest
            .chars()
            .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
            .collect();
        if name == word {
            let end = text.get(start..)?.find('{').map(|at| start + at)?;
            let signature = text
                .get(start..end)?
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let docs = doc_lines_above(text, start);
            return Some(if docs.is_empty() {
                signature
            } else {
                format!("{}\n\n{signature}", docs.join("\n"))
            });
        }
        from = start + 3;
    }
    None
}

/// The doc lines directly above `at`, nearest last: consecutive lines whose
/// trimmed text starts with `///` (but not `////`).
fn doc_lines_above(text: &str, at: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cursor = at;
    while cursor > 0 {
        // The line that ends just before `cursor`.
        let before = match text.get(..cursor.saturating_sub(1)) {
            Some(before) => before,
            None => break,
        };
        let line_start = before.rfind('\n').map(|at| at + 1).unwrap_or(0);
        let line = match before.get(line_start..) {
            Some(line) => line.trim(),
            None => break,
        };
        if line.starts_with("///") && !line.starts_with("////") {
            lines.push(line.to_string());
            cursor = line_start;
        } else {
            break;
        }
    }
    lines.reverse();
    lines
}

/// The byte span of the identifier `word` in a `fn` declaration.
fn function_name_span(text: &str, word: &str) -> Option<(usize, usize)> {
    let tokens = Lexer::new(text).tokenize().ok()?;
    for pair in tokens.windows(2) {
        if pair[0].kind != TokenKind::Fn || pair[1].kind != TokenKind::Ident {
            continue;
        }
        if text.get(pair[1].span.start as usize..pair[1].span.end as usize) == Some(word) {
            return Some((pair[1].span.start as usize, pair[1].span.end as usize));
        }
    }
    None
}

/// One `fn` definition found by scanning tokens: a free function, a
/// method (inherent, trait-impl or object), or a trait requirement.
struct FnDef {
    name: String,
    span: (usize, usize),
    signature: String,
    type_name: Option<String>,
    trait_name: Option<String>,
    has_self: bool,
    is_method: bool,
    is_trait_method: bool,
    body: Option<(usize, usize)>,
}

/// One `impl [Trait for] Type { .. }` block, by byte range.
struct ImplBlock {
    trait_name: Option<String>,
    type_name: String,
    open: usize,
    close: usize,
}

fn ident_text(text: &str, token: &Token) -> String {
    text.get(token.span.start as usize..token.span.end as usize)
        .unwrap_or("")
        .to_string()
}

/// Scan `text` for function definitions and impl blocks. Token order is
/// the only structure needed: `impl`/`trait`/`object` open a context at
/// their `{` and close at the matching `}`, and a `fn` at the context's
/// top level is a definition of that context.
fn scan_defs(text: &str) -> (Vec<FnDef>, Vec<ImplBlock>) {
    let Ok(tokens) = Lexer::new(text).tokenize() else {
        return (Vec::new(), Vec::new());
    };
    let mut defs = Vec::new();
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let token = tokens[i];
        match token.kind {
            TokenKind::Trait => {
                // `trait Name { ... }`
                let name = tokens
                    .get(i + 1)
                    .map(|t| ident_text(text, t))
                    .unwrap_or_default();
                let open_at = tokens[i..]
                    .iter()
                    .position(|t| t.kind == TokenKind::LBrace)
                    .map(|p| i + p);
                if let Some(open_at) = open_at {
                    let close_at = matching_brace(&tokens, open_at).unwrap_or(tokens.len() - 1);
                    let close = tokens[close_at].span.end as usize;
                    let mut j = open_at + 1;
                    while j < close_at {
                        if tokens[j].kind == TokenKind::Fn {
                            if let Some(def) = parse_fn_def(
                                text,
                                &tokens,
                                j,
                                None,
                                Some(name.clone()),
                                true,
                                false,
                            ) {
                                defs.push(def);
                            }
                        }
                        j += 1;
                    }
                    let _ = close;
                    i = close_at;
                    continue;
                }
            }
            TokenKind::Impl => {
                // `impl [Trait for] Type { ... }`
                let mut j = i + 1;
                let first = tokens
                    .get(j)
                    .map(|t| ident_text(text, t))
                    .unwrap_or_default();
                j += 1;
                let (trait_name, type_name) =
                    if tokens.get(j).map(|t| t.kind) == Some(TokenKind::For) {
                        let ty = tokens
                            .get(j + 1)
                            .map(|t| ident_text(text, t))
                            .unwrap_or_default();
                        j += 2;
                        (Some(first), ty)
                    } else {
                        (None, first)
                    };
                let open_at = tokens[j..]
                    .iter()
                    .position(|t| t.kind == TokenKind::LBrace)
                    .map(|p| j + p);
                if let Some(open_at) = open_at {
                    let close_at = matching_brace(&tokens, open_at).unwrap_or(tokens.len() - 1);
                    let open = tokens[open_at].span.start as usize;
                    let close = tokens[close_at].span.end as usize;
                    blocks.push(ImplBlock {
                        trait_name: trait_name.clone(),
                        type_name: type_name.clone(),
                        open,
                        close,
                    });
                    let mut k = open_at + 1;
                    while k < close_at {
                        if tokens[k].kind == TokenKind::Fn {
                            if let Some(def) = parse_fn_def(
                                text,
                                &tokens,
                                k,
                                Some(type_name.clone()),
                                trait_name.clone(),
                                false,
                                true,
                            ) {
                                defs.push(def);
                            }
                        }
                        k += 1;
                    }
                    i = close_at;
                    continue;
                }
            }
            TokenKind::Object => {
                let name = tokens
                    .get(i + 1)
                    .map(|t| ident_text(text, t))
                    .unwrap_or_default();
                let open_at = tokens[i..]
                    .iter()
                    .position(|t| t.kind == TokenKind::LBrace)
                    .map(|p| i + p);
                if let Some(open_at) = open_at {
                    let close_at = matching_brace(&tokens, open_at).unwrap_or(tokens.len() - 1);
                    let mut k = open_at + 1;
                    while k < close_at {
                        if tokens[k].kind == TokenKind::Fn {
                            if let Some(def) = parse_fn_def(
                                text,
                                &tokens,
                                k,
                                Some(name.clone()),
                                None,
                                false,
                                true,
                            ) {
                                defs.push(def);
                            }
                        }
                        k += 1;
                    }
                    i = close_at;
                    continue;
                }
            }
            TokenKind::Fn => {
                if let Some(def) = parse_fn_def(text, &tokens, i, None, None, false, false) {
                    defs.push(def);
                }
            }
            _ => {}
        }
        i += 1;
    }
    (defs, blocks)
}

fn fn_defs(text: &str) -> Vec<FnDef> {
    scan_defs(text).0
}

fn matching_brace(tokens: &[Token], open_at: usize) -> Option<usize> {
    let mut depth = 0i32;
    for at in open_at..tokens.len() {
        match tokens[at].kind {
            TokenKind::LBrace => depth += 1,
            TokenKind::RBrace => {
                depth -= 1;
                if depth == 0 {
                    return Some(at);
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse one `fn name(params) -> ret { body }` (or `;` for a trait
/// requirement) starting at token `at` (the `fn`).
fn parse_fn_def(
    text: &str,
    tokens: &[Token],
    at: usize,
    type_name: Option<String>,
    trait_name: Option<String>,
    is_trait_method: bool,
    is_method: bool,
) -> Option<FnDef> {
    let name_token = tokens.get(at + 1)?;
    if name_token.kind != TokenKind::Ident {
        return None;
    }
    let name = ident_text(text, name_token);
    let span = (name_token.span.start as usize, name_token.span.end as usize);
    // Parameters: first token inside the parens is the receiver when it
    // is the identifier `self`.
    let mut has_self = false;
    let mut signature_end = tokens[at].span.end as usize;
    let mut body = None;
    let mut j = at + 2;
    while j < tokens.len() {
        match tokens[j].kind {
            TokenKind::LParen => {
                if let Some(first) = tokens.get(j + 1) {
                    if first.kind == TokenKind::Ident && ident_text(text, first) == "self" {
                        has_self = true;
                    }
                }
            }
            TokenKind::LBrace => {
                signature_end = tokens[j].span.start as usize;
                if let Some(close_at) = matching_brace(tokens, j) {
                    body = Some((
                        tokens[j].span.start as usize,
                        tokens[close_at].span.end as usize,
                    ));
                }
                break;
            }
            TokenKind::Semi => {
                signature_end = tokens[j].span.end as usize;
                break;
            }
            TokenKind::RBrace | TokenKind::Eof => break,
            _ => {}
        }
        j += 1;
    }
    let signature = text
        .get(tokens[at].span.start as usize..signature_end)?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    Some(FnDef {
        name,
        span,
        signature,
        type_name,
        trait_name,
        has_self,
        is_method,
        is_trait_method,
        body,
    })
}

/// The `impl Trait for Type` block containing `offset` directly (not
/// inside one of its method bodies), if any.
fn impl_block_at(defs: &[FnDef], text: &str, offset: usize) -> Option<ImplBlock> {
    let (_, blocks) = scan_defs(text);
    blocks.into_iter().find(|block| {
        block.trait_name.is_some()
            && offset >= block.open
            && offset < block.close
            && !defs.iter().any(|d| {
                d.is_method
                    && d.span.0 >= block.open
                    && d.span.0 < block.close
                    && d.body
                        .map(|(s, e)| offset >= s && offset < e)
                        .unwrap_or(false)
            })
    })
}

fn classify(
    kind: TokenKind,
    text: &str,
    previous: Option<TokenKind>,
    next: Option<TokenKind>,
) -> Option<u32> {
    if TokenKind::keyword(text).is_some() {
        return Some(TOKEN_KEYWORD);
    }
    match kind {
        TokenKind::Str => Some(TOKEN_STRING),
        TokenKind::I64 | TokenKind::F64 => Some(TOKEN_NUMBER),
        TokenKind::Ident => {
            if TYPE_NAMES.contains(&text) {
                Some(TOKEN_TYPE)
            } else if previous == Some(TokenKind::Fn) || next == Some(TokenKind::LParen) {
                Some(TOKEN_FUNCTION)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Split a token that spans lines (a block comment) into one item per line;
/// LSP semantic tokens may not cross a line.
fn push_span(
    index: &LineIndex,
    text: &str,
    start: usize,
    end: usize,
    kind: u32,
    out: &mut Vec<(u32, u32, u32, u32)>,
) {
    let mut at = start;
    while at < end {
        let (line, character) = index.position(text, at);
        let rest = text.get(at..end).unwrap_or("");
        let newline = rest.find('\n');
        let piece_end = newline.map(|offset| at + offset).unwrap_or(end);
        let piece = text.get(at..piece_end).unwrap_or("");
        let length = piece.encode_utf16().count();
        if length > 0 {
            out.push((line, character, length as u32, kind));
        }
        at = piece_end;
        if at < end {
            at += 1;
        }
    }
}

/// The byte spans of every comment, read from the gaps between tokens.
fn comment_spans(text: &str, tokens: &[Token]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut end = 0usize;
    for token in tokens {
        let start = token.span.start as usize;
        scan_gap(text.get(end..start).unwrap_or(""), end, &mut spans);
        end = token.span.end as usize;
        if token.kind == TokenKind::Eof {
            break;
        }
    }
    scan_gap(text.get(end..).unwrap_or(""), end, &mut spans);
    spans
}

fn scan_gap(gap: &str, base: usize, out: &mut Vec<(usize, usize)>) {
    let bytes = gap.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/') {
            let start = index;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            out.push((base + start, base + index));
        } else if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            let start = index;
            index += 2;
            let mut depth = 1usize;
            while index < bytes.len() && depth > 0 {
                if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
                    depth += 1;
                    index += 2;
                } else if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
                    depth -= 1;
                    index += 2;
                } else {
                    index += 1;
                }
            }
            out.push((base + start, base + index));
        } else {
            index += 1;
        }
    }
}

/// Read one framed message. `None` means the stream ended cleanly.
fn read_message<R: BufRead>(reader: &mut R) -> Result<Option<Value>, String> {
    let mut length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| format!("cannot read a header: {error}"))?;
        if read == 0 {
            return Ok(None);
        }
        let header = line.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }
        if let Some(rest) = header.strip_prefix("Content-Length:") {
            length = rest.trim().parse::<usize>().ok().or(length);
        }
    }
    let Some(length) = length else {
        return Err("a frame had no Content-Length header".to_string());
    };
    let mut body = vec![0u8; length];
    reader
        .read_exact(&mut body)
        .map_err(|error| format!("cannot read a body: {error}"))?;
    let text =
        String::from_utf8(body).map_err(|error| format!("the body is not UTF-8: {error}"))?;
    crate::json::parse(&text).map(Some)
}

fn write_message<W: Write>(out: &mut W, value: &Value) -> io::Result<()> {
    let body = crate::json::to_string(value);
    write!(out, "Content-Length: {}\r\n\r\n{body}", body.len())?;
    out.flush()
}
/// One workspace edit for a machine-applicable quick fix.
fn quick_fix(
    uri: &str,
    model: &crate::model::LspModel,
    start: usize,
    end: usize,
    replacement: &str,
    title: String,
    kind: &str,
) -> Value {
    quick_fix_edits(
        uri,
        model,
        &[(start, end, replacement.to_string())],
        title,
        kind,
    )
}

/// One workspace edit carrying every change a diagnostic proved at once.
///
/// `edits` are byte spans in the same document, in any order. The action's own
/// range spans all of them, so an editor that shows the action at the
/// diagnostic (`range`) still accepts it from anywhere in the list.
fn quick_fix_edits(
    uri: &str,
    model: &crate::model::LspModel,
    edits: &[(usize, usize, String)],
    title: String,
    kind: &str,
) -> Value {
    let mut first = model.text.len();
    let mut last = 0usize;
    let changes: Vec<Value> = edits
        .iter()
        .map(|(start, end, replacement)| {
            first = first.min(*start);
            last = last.max(*end);
            let (sl, sc, el, ec) = model.range(*start, *end);
            object([
                ("range", range(sl, sc, el, ec)),
                ("newText", string(replacement)),
            ])
        })
        .collect();
    let (sl, sc, el, ec) = model.range(first, last);
    object([
        ("title", string(title)),
        ("kind", string(kind)),
        (
            "edit",
            object([("changes", object([(uri, Value::Arr(changes))]))]),
        ),
        ("range", range(sl, sc, el, ec)),
    ])
}
