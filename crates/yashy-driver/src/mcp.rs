//! `yashy mcp`: the seven compiler verbs over JSON-RPC 2.0 on stdio.
//!
//! The MCP server is a thin adapter over the same in-process functions the
//! CLI calls. It never runs a shell, never widens its own permissions, and
//! never accepts a path outside the workspace root: containment is checked
//! by canonicalising and comparing against the root, with the same code the
//! CLI uses, not by a check in the adapter.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use yashy_common::{FileId, Json, SourceMap};
use yashy_common::json_rpc::{self, Value};

/// The seven tools, in a stable order.
pub const TOOLS: [&str; 7] = ["check", "build", "test", "verify", "explain", "context", "index"];

/// A tool error: JSON-RPC `code: -32000` with the diagnostic array in `data`.
pub fn tool_error(id: Value, diagnostics: Vec<yashy_common::Diagnostic>) -> Value {
    let data: Vec<Value> = diagnostics
        .iter()
        .map(|diagnostic| {
            json_rpc::object([
                ("message", json_rpc::string(&diagnostic.message)),
                (
                    "code",
                    diagnostic
                        .code
                        .map(json_rpc::string)
                        .unwrap_or(Value::Null),
                ),
            ])
        })
        .collect();
    json_rpc::object([
        ("jsonrpc", json_rpc::string("2.0")),
        ("id", id),
        (
            "error",
            json_rpc::object([
                ("code", Value::Num(-32000.0)),
                ("message", json_rpc::string("tool failed")),
                ("data", Value::Arr(data)),
            ]),
        ),
    ])
}

fn ok(id: Value, result: Value) -> Value {
    json_rpc::object([
        ("jsonrpc", json_rpc::string("2.0")),
        ("id", id),
        ("result", result),
    ])
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json_rpc::object([
        ("jsonrpc", json_rpc::string("2.0")),
        ("id", id),
        (
            "error",
            json_rpc::object([
                ("code", Value::Num(code as f64)),
                ("message", json_rpc::string(message)),
            ]),
        ),
    ])
}

/// Canonicalise `path` and refuse it when it is outside `root`.
pub fn contain(root: &Path, path: &str) -> Result<PathBuf, String> {
    let root = root
        .canonicalize()
        .map_err(|error| format!("root is not readable: {error}"))?;
    let candidate = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        root.join(path)
    };
    // Canonicalise the nearest existing ancestor and re-append the rest, so
    // a path that does not exist yet is still containment-checked (a
    // nonexistent traversal is refused, not reported unreadable).
    let mut resolved = candidate.clone();
    let mut suffix: Vec<std::ffi::OsString> = Vec::new();
    while let Err(error) = resolved.canonicalize() {
        if error.kind() != std::io::ErrorKind::NotFound {
            return Err(format!("{path} is not readable: {error}"));
        }
        match resolved.file_name().map(|name| name.to_os_string()) {
            Some(name) => {
                suffix.push(name);
                resolved.pop();
            }
            None => return Err(format!("{path} is not readable: {error}")),
        }
    }
    let candidate = resolved.canonicalize().unwrap_or(resolved);
    let candidate = suffix
        .into_iter()
        .rev()
        .fold(candidate, |acc, name| acc.join(name));
    if !candidate.starts_with(&root) {
        return Err(format!("{path} is outside the workspace root"));
    }
    Ok(candidate)
}

/// One request, answered. Never panics on a malformed request.
pub fn handle(root: &Path, request: &Value) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    match method {
        "initialize" => ok(
            id,
            json_rpc::object([
                ("protocolVersion", json_rpc::string("2.0")),
                (
                    "tools",
                    Value::Arr(
                        TOOLS
                            .iter()
                            .map(|tool| json_rpc::object([("name", json_rpc::string(*tool))]))
                            .collect(),
                    ),
                ),
            ]),
        ),
        "tools/call" => {
            let params = request.get("params");
            let name = params
                .and_then(|params| params.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            call_tool(root, id, name, params)
        }
        _ => error(id, -32601, &format!("unknown method `{method}`")),
    }
}

fn call_tool(root: &Path, id: Value, name: &str, params: Option<&Value>) -> Value {
    if !TOOLS.contains(&name) {
        return error(id, -32602, &format!("unknown tool `{name}`"));
    }
    let arguments = params.and_then(|params| params.get("arguments"));
    let path_text = arguments
        .and_then(|arguments| arguments.get("path"))
        .and_then(Value::as_str)
        .unwrap_or("");
    // Every tool takes a path; it is canonicalised and contained before
    // anything reads it.
    let path = match contain(root, path_text) {
        Ok(path) => path,
        Err(message) => return error(id, -32001, &message),
    };
    match name {
        "check" | "verify" | "build" | "test" => {
            let source = match std::fs::read_to_string(&path) {
                Ok(source) => source,
                Err(io_error) => return error(id, -32001, &format!("{path_text}: {io_error}")),
            };
            let mut sources = SourceMap::new();
            let outcome = crate::pipeline::check_and_verify(
                &mut sources,
                &path,
                source,
                false,
                &yashy_verify::VerifyOptions {
                    contracts: yashy_verify::ContractMode::Enforce,
                },
                1,
            );
            match outcome {
                Ok((hir, report)) => {
                    let _ = hir;
                    let diagnostics: Vec<Value> = report
                        .diagnostics
                        .iter()
                        .map(|diagnostic| {
                            json_rpc::object([
                                ("message", json_rpc::string(&diagnostic.message)),
                                (
                                    "code",
                                    diagnostic
                                        .code
                                        .map(json_rpc::string)
                                        .unwrap_or(Value::Null),
                                ),
                            ])
                        })
                        .collect();
                    ok(
                        id,
                        json_rpc::object([
                            ("diagnostics", Value::Arr(diagnostics)),
                            ("success", Value::Bool(true)),
                        ]),
                    )
                }
                Err(diagnostics) => tool_error(id, diagnostics),
            }
        }
        "explain" => {
            let code = arguments
                .and_then(|arguments| arguments.get("code"))
                .and_then(Value::as_str)
                .unwrap_or("");
            match yashy_common::codes::find(code) {
                Some(entry) => ok(id, json_rpc::string(entry.page)),
                None => error(id, -32001, &format!("unknown code `{code}`")),
            }
        }
        "context" => {
            let task = arguments
                .and_then(|arguments| arguments.get("task"))
                .and_then(Value::as_str)
                .unwrap_or("");
            match crate::context::build_context(root, task, 8000, None) {
                Ok(pack) => ok(id, json_rpc::string(pack.render_markdown())),
                Err(message) => error(id, -32001, &message),
            }
        }
        "index" => match crate::index::build_index(root) {
            Ok(model) => ok(
                id,
                json_rpc::string(crate::index::to_json(&model).render()),
            ),
            Err(message) => error(id, -32001, &message),
        },
        _ => error(id, -32602, "unreachable tool"),
    }
}

/// Serve one connection: a loop over stdin lines, each at most 1 MiB, until
/// the client closes the connection or sends `shutdown`.
pub fn serve(root: &Path) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut reader = std::io::BufReader::new(stdin.lock());
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            return Ok(());
        }
        if line.len() > 1024 * 1024 {
            let response = error(Value::Null, -32700, "request too large");
            writeln!(out, "{}", json_rpc::to_string(&response))?;
            out.flush()?;
            continue;
        }
        let request = match json_rpc::parse(line.trim()) {
            Ok(value) => value,
            Err(_) => {
                let response = error(Value::Null, -32700, "parse error");
                writeln!(out, "{}", json_rpc::to_string(&response))?;
                out.flush()?;
                continue;
            }
        };
        if request.get("method").and_then(Value::as_str) == Some("shutdown") {
            return Ok(());
        }
        let response = handle(root, &request);
        writeln!(out, "{}", json_rpc::to_string(&response))?;
        out.flush()?;
    }
}

pub fn to_json_value(json: &Json) -> Value {
    match json {
        Json::Null => Value::Null,
        Json::Bool(value) => Value::Bool(*value),
        Json::Number(value) => Value::Num(*value as f64),
        Json::String(value) => Value::Str(value.clone()),
        Json::Array(values) => Value::Arr(values.iter().map(to_json_value).collect()),
        Json::Object(fields) => Value::Obj(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), to_json_value(value)))
                .collect(),
        ),
    }
}

#[allow(dead_code)]
fn unused(_: FileId) {}
