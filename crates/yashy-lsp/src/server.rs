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
}

impl Server {
    /// Handle one message. Returns false when the server must stop.
    fn handle<W: Write>(&mut self, message: &Value, out: &mut W) -> io::Result<bool> {
        let id = message.get("id").cloned();
        let method = message.get("method").and_then(Value::as_str);
        let params = message.get("params");

        match method {
            Some("initialize") => self.reply(out, id, initialize())?,
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
            Some("textDocument/semanticTokens/full") => {
                let result = self.semantic_tokens(params);
                self.reply(out, id, result)?;
            }
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
            let end = (diagnostic.span.end as usize).max(start + 1).min(text.len());
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
        let params = object([
            ("uri", string(uri)),
            ("diagnostics", Value::Arr(items)),
        ]);
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
        let signature = builtin_signature(word).map(str::to_string).or_else(|| {
            user_signature(&text, word)
        });
        match signature {
            Some(signature) => object([(
                "contents",
                object([
                    ("kind", string("markdown")),
                    (
                        "value",
                        string(format!("```yashy\n{signature}\n```")),
                    ),
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

    fn reply<W: Write>(&self, out: &mut W, id: Option<Value>, result: Value) -> io::Result<()> {
        let Some(id) = id else {
            return Ok(());
        };
        write_message(
            out,
            &object([
                ("jsonrpc", string("2.0")),
                ("id", id),
                ("result", result),
            ]),
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
                                            "keyword",
                                            "type",
                                            "function",
                                            "string",
                                            "number",
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
        "print_str" => "fn print_str(value: string)",
        "print_i64" => "fn print_i64(value: int)",
        "print_f64" => "fn print_f64(value: float)",
        "alloc" => "fn alloc(count: int) -> *mut int",
        "dealloc" => "fn dealloc(pointer: *mut int)",
        "len" => "fn len(value) -> int",
        "str_from_i64" => "fn str_from_i64(value: int) -> string",
        "read_line" => "fn read_line() -> string",
        "assert" => "fn assert(condition: bool)",
        "exit" => "fn exit(code: int)",
        _ => return None,
    })
}

/// The source text of `fn <word>` up to its body, for a hover on a user
/// function.
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
            let signature = text.get(start..end)?.split_whitespace().collect::<Vec<_>>().join(" ");
            return Some(signature);
        }
        from = start + 3;
    }
    None
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
    let text = String::from_utf8(body).map_err(|error| format!("the body is not UTF-8: {error}"))?;
    crate::json::parse(&text).map(Some)
}

fn write_message<W: Write>(out: &mut W, value: &Value) -> io::Result<()> {
    let body = crate::json::to_string(value);
    write!(out, "Content-Length: {}\r\n\r\n{body}", body.len())?;
    out.flush()
}
