//! `yashyc mcp`: the seven compiler verbs over JSON-RPC 2.0 on stdio.
//!
//! The MCP server is a thin adapter over the same in-process functions the
//! CLI calls. It never runs a shell, never widens its own permissions, and
//! never accepts a path outside the workspace root: containment is checked
//! by canonicalising and comparing against the root, with the same code the
//! CLI uses, not by a check in the adapter.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use yashy_common::json_rpc::{self, Value};
use yashy_common::{FileId, Json, SourceMap, Span};

/// The tools, in a stable order.
pub const TOOLS: [&str; 13] = [
    "check", "build", "test", "verify", "explain", "context", "index", "edit", "refactor",
    "format", "lint", "fix", "doc",
];

/// A tool error: JSON-RPC `code: -32000` with the diagnostic array in `data`.
pub fn tool_error(id: Value, diagnostics: Vec<yashy_common::Diagnostic>) -> Value {
    let data: Vec<Value> = diagnostics
        .iter()
        .map(|diagnostic| {
            json_rpc::object([
                ("message", json_rpc::string(&diagnostic.message)),
                (
                    "code",
                    diagnostic.code.map(json_rpc::string).unwrap_or(Value::Null),
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

/// One MCP connection: the workspace, the policy it runs under, and the audit
/// log every call appends to.
pub struct Session {
    pub root: PathBuf,
    pub policy: crate::agent::AgentPolicy,
    pub audit: crate::agent::AuditLog,
    /// Milliseconds spent in tool calls so far.
    pub spent_ms: u64,
    /// Request+response bytes divided by four, rounded up.
    pub spent_tokens: u64,
    /// Response bytes produced so far.
    pub spent_output: u64,
}

impl Session {
    /// Open a session for `root`. A policy or audit directory that cannot be
    /// read is an error: the session must not start ungoverned.
    pub fn open(root: &Path) -> Result<Session, String> {
        let policy = crate::agent::AgentPolicy::load(root)?;
        let audit = crate::agent::AuditLog::open(root, policy.audit_max_bytes)?;
        Ok(Session {
            root: root.to_path_buf(),
            policy,
            audit,
            spent_ms: 0,
            spent_tokens: 0,
            spent_output: 0,
        })
    }
}

/// Every path a request body names: the top-level `path`, and each edit's
/// `file` inside a patch. The policy compares these without running the tool.
fn request_paths(params: Option<&Value>) -> Vec<String> {
    let mut out = Vec::new();
    let Some(arguments) = params.and_then(|params| params.get("arguments")) else {
        return out;
    };
    if let Some(path) = arguments.get("path").and_then(Value::as_str) {
        out.push(path.to_string());
    }
    if let Some(edits) = arguments
        .get_path(&["patch", "edits"])
        .and_then(Value::as_array)
    {
        for edit in edits {
            if let Some(file) = edit.get("file").and_then(Value::as_str) {
                out.push(file.to_string());
            }
        }
    }
    out
}

fn patch_edit_count(params: Option<&Value>) -> usize {
    params
        .and_then(|params| params.get("arguments"))
        .and_then(|arguments| arguments.get_path(&["patch", "edits"]))
        .and_then(Value::as_array)
        .map_or(0, |edits| edits.len())
}

/// A refusal that names a diagnostic code in `error.data` as well as its
/// JSON-RPC code, so an agent can branch on the rule rather than the prose.
fn refusal(id: Value, code: i64, diagnostic: &str, message: &str) -> Value {
    json_rpc::object([
        ("jsonrpc", json_rpc::string("2.0")),
        ("id", id),
        (
            "error",
            json_rpc::object([
                ("code", Value::Num(code as f64)),
                (
                    "message",
                    json_rpc::string(format!("{message} ({diagnostic})")),
                ),
                (
                    "data",
                    json_rpc::object([("code", json_rpc::string(diagnostic))]),
                ),
            ]),
        ),
    ])
}

/// One request, answered. Never panics on a malformed request.
pub fn handle(session: &mut Session, request: &Value) -> Value {
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
        "tools/list" => ok(
            id,
            json_rpc::object([(
                "tools",
                Value::Arr(
                    TOOLS
                        .iter()
                        .map(|tool| {
                            json_rpc::object([
                                ("name", json_rpc::string(*tool)),
                                ("annotations", crate::mcp_extras::annotations(tool)),
                            ])
                        })
                        .collect(),
                ),
            )]),
        ),
        "resources/list" => ok(
            id,
            json_rpc::object([("resources", crate::mcp_extras::list_resources())]),
        ),
        "resources/read" => {
            let uri = request
                .get("params")
                .and_then(|params| params.get("uri"))
                .and_then(Value::as_str)
                .unwrap_or("");
            match crate::mcp_extras::read_resource(&session.root, uri) {
                Some(value) => ok(id, value),
                None => error(id, -32002, &format!("unknown resource `{uri}`")),
            }
        }
        "prompts/list" => ok(
            id,
            json_rpc::object([("prompts", crate::mcp_extras::list_prompts())]),
        ),
        "prompts/get" => {
            let params = request.get("params");
            let name = params
                .and_then(|params| params.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            match crate::mcp_extras::get_prompt(name, params.and_then(|p| p.get("arguments"))) {
                Some(value) => ok(id, value),
                None => error(id, -32002, &format!("unknown prompt `{name}`")),
            }
        }
        "tools/call" => {
            let params = request.get("params");
            let name = params
                .and_then(|params| params.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            // An unknown verb is a bad request, not a policy question, but it
            // is still a call and must still be audited.
            if !TOOLS.contains(&name.as_str()) {
                let reason = format!("unknown tool `{name}`");
                if let Err(message) = session.audit.record(
                    &name,
                    "refused",
                    Some(crate::agent::POLICY_DENIED),
                    &reason,
                ) {
                    return refusal(id, -32005, crate::agent::AUDIT_FAILURE, &message);
                }
                return refusal(id, -32602, crate::agent::POLICY_DENIED, &reason);
            }
            let paths = request_paths(params);
            // Budgets are session-wide and only ever narrowed by policy. A
            // breach stops the next call before it runs and names the limit.
            let request_bytes = json_rpc::to_string(request).len() as u64;
            let budget_reason = session
                .policy
                .max_wall_ms
                .filter(|limit| session.spent_ms >= *limit)
                .map(|limit| format!("wall budget of {limit} ms is exhausted"))
                .or_else(|| {
                    session
                        .policy
                        .max_tokens
                        .filter(|limit| session.spent_tokens >= *limit)
                        .map(|limit| format!("token budget of {limit} is exhausted"))
                })
                .or_else(|| {
                    session
                        .policy
                        .max_output_bytes
                        .filter(|limit| session.spent_output >= *limit)
                        .map(|limit| format!("output budget of {limit} bytes is exhausted"))
                });
            if let Some(reason) = budget_reason {
                if let Err(message) = session.audit.record(
                    &name,
                    "refused",
                    Some(crate::agent::POLICY_DENIED),
                    &reason,
                ) {
                    return refusal(id, -32005, crate::agent::AUDIT_FAILURE, &message);
                }
                return refusal(id, -32002, crate::agent::POLICY_DENIED, &reason);
            }
            let refusal_reason = if !session.policy.allows(&name) {
                Some(format!("policy denies tool `{name}`"))
            } else if let Some(hit) = session.policy.protected_hit(&paths) {
                Some(format!("policy protects path `{hit}`"))
            } else {
                session.policy.max_edits.and_then(|limit| {
                    let edits = patch_edit_count(params);
                    (edits > limit)
                        .then(|| format!("patch has {edits} edits; policy allows {limit}"))
                })
            };
            if let Some(reason) = refusal_reason {
                let diagnostic = crate::agent::POLICY_DENIED;
                if let Err(message) =
                    session
                        .audit
                        .record(&name, "refused", Some(diagnostic), &reason)
                {
                    return refusal(id, -32005, crate::agent::AUDIT_FAILURE, &message);
                }
                return refusal(id, -32002, diagnostic, &reason);
            }
            // An approval is consumed before the verb runs, so a failed or
            // replayed call cannot spend the same token twice.
            let approval_error = if session
                .policy
                .require_approval
                .iter()
                .any(|verb| verb == &name)
            {
                let patch = params
                    .and_then(|params| params.get("arguments"))
                    .and_then(|arguments| arguments.get("patch"))
                    .unwrap_or(&Value::Null);
                let tree = crate::edit::tree_hash(&session.root).unwrap_or_default();
                let token = params
                    .and_then(|params| params.get("arguments"))
                    .and_then(|arguments| arguments.get("approval"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if token.is_empty() {
                    Some(format!("approval token is required for `{name}`"))
                } else {
                    crate::approval::consume(&session.root, token, &tree, patch, &name).err()
                }
            } else {
                None
            };
            if let Some(message) = approval_error {
                let reason = format!("approval refused for `{name}`: {message}");
                let diagnostic = crate::agent::POLICY_DENIED;
                if let Err(audit_error) =
                    session
                        .audit
                        .record(&name, "refused", Some(diagnostic), &reason)
                {
                    return refusal(id, -32005, crate::agent::AUDIT_FAILURE, &audit_error);
                }
                return refusal(id, -32002, diagnostic, &reason);
            }
            let started = std::time::Instant::now();
            let response = call_tool(&session.root, id.clone(), &name, params);
            let response_bytes = json_rpc::to_string(&response).len() as u64;
            session.spent_ms = session
                .spent_ms
                .saturating_add(started.elapsed().as_millis() as u64);
            session.spent_tokens = session
                .spent_tokens
                .saturating_add(request_bytes.saturating_add(response_bytes).div_ceil(4));
            session.spent_output = session.spent_output.saturating_add(response_bytes);
            let outcome = if response.get("error").is_some() {
                "error"
            } else {
                "ok"
            };
            let detail = format!("{} {}", name, paths.join(" "));
            if let Err(message) = session
                .audit
                .record(&name, outcome, None, detail.trim_end())
            {
                return refusal(id, -32005, crate::agent::AUDIT_FAILURE, &message);
            }
            response
        }
        _ => error(id, -32601, &format!("unknown method `{method}`")),
    }
}

fn check_text(path: &Path, text: &str) -> Result<(), Vec<String>> {
    let mut sources = SourceMap::new();
    match crate::pipeline::check_and_verify(
        &mut sources,
        path,
        text.to_string(),
        false,
        &yashy_verify::VerifyOptions {
            contracts: yashy_verify::ContractMode::Enforce,
        },
        1,
    ) {
        Ok(_) => Ok(()),
        Err(diagnostics) => Err(diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect()),
    }
}

fn parse_patch(value: &Value) -> Result<crate::edit::Patch, String> {
    let expected_tree_hash = value
        .get("expected_tree_hash")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let mut edits = Vec::new();
    for item in value.get("edits").and_then(Value::as_array).unwrap_or(&[]) {
        let file = item
            .get("file")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let span = item.get("span").ok_or("an edit needs a span")?;
        let start = span.get("start").and_then(Value::as_i64).unwrap_or(-1);
        let end = span.get("end").and_then(Value::as_i64).unwrap_or(-1);
        if start < 0 || end < start {
            return Err("an edit span must have non-negative start and end".into());
        }
        edits.push(crate::edit::Edit {
            file,
            span: Span::new(start as u32, end as u32),
            expected_hash: item
                .get("expected_hash")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            replacement: item
                .get("replacement")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            rationale: item
                .get("rationale")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        });
    }
    Ok(crate::edit::Patch {
        edits,
        expected_tree_hash,
    })
}

fn call_fix_doc_tool(root: &Path, id: Value, name: &str, arguments: Option<&Value>) -> Value {
    if name == "doc" {
        return match crate::doc::build_doc(root, false) {
            Ok(model) => ok(id, json_rpc::string(crate::doc::to_json(&model).render())),
            Err(message) => error(id, -32001, &message),
        };
    }
    let arguments = match arguments {
        Some(arguments) => arguments,
        None => return error(id, -32602, "`fix` needs arguments"),
    };
    let shown = arguments.get("path").and_then(Value::as_str).unwrap_or("");
    let path = match contain(root, shown) {
        Ok(path) => path,
        Err(message) => return error(id, -32001, &message),
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(io_error) => return error(id, -32001, &format!("{shown}: {io_error}")),
    };
    let mut sources = SourceMap::new();
    let outcome = crate::pipeline::check_and_verify(
        &mut sources,
        &path,
        text.clone(),
        false,
        &yashy_verify::VerifyOptions {
            contracts: yashy_verify::ContractMode::Enforce,
        },
        1,
    );
    let diagnostics = match outcome {
        Ok((_, report)) => report.diagnostics,
        Err(diagnostics) => diagnostics,
    };
    let mut edits = Vec::new();
    for diagnostic in &diagnostics {
        for suggestion in &diagnostic.suggestions {
            if suggestion.applicability == yashy_common::Applicability::MachineApplicable {
                edits.push(crate::edit::Edit {
                    file: shown.to_string(),
                    span: suggestion.span,
                    expected_hash: String::new(),
                    replacement: suggestion.replacement.clone(),
                    rationale: suggestion.message.clone(),
                });
            }
        }
    }
    if edits.is_empty() {
        return ok(id, json_rpc::object([("applied", Value::Arr(Vec::new()))]));
    }
    let hash = match crate::edit::file_hash(&path) {
        Ok(hash) => hash,
        Err(edit_error) => return error(id, -32001, &edit_error.message()),
    };
    let tree = match crate::edit::tree_hash(root) {
        Ok(tree) => tree,
        Err(edit_error) => return error(id, -32001, &edit_error.message()),
    };
    for edit in &mut edits {
        edit.expected_hash = hash.clone();
    }
    let patch = crate::edit::Patch {
        expected_tree_hash: tree,
        edits,
    };
    match crate::edit::apply_patch(root, &patch, check_text) {
        Ok(applied) => {
            let applied: Vec<Value> = applied.into_iter().map(json_rpc::string).collect();
            ok(id, json_rpc::object([("applied", Value::Arr(applied))]))
        }
        Err(edit_error) => error(id, -32001, &edit_error.message()),
    }
}

fn call_source_tool(root: &Path, id: Value, name: &str, arguments: Option<&Value>) -> Value {
    let arguments = match arguments {
        Some(arguments) => arguments,
        None => return error(id, -32602, "format and lint need arguments"),
    };
    let shown = arguments.get("path").and_then(Value::as_str).unwrap_or("");
    let path = match contain(root, shown) {
        Ok(path) => path,
        Err(message) => return error(id, -32001, &message),
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(io_error) => return error(id, -32001, &format!("{shown}: {io_error}")),
    };
    if name == "lint" {
        let group = arguments
            .get("group")
            .and_then(Value::as_str)
            .and_then(crate::lint::LintGroup::parse);
        let diagnostics: Vec<Value> =
            crate::lint::check_filtered(&text, FileId::from_index(0), group)
                .iter()
                .map(|diagnostic| {
                    json_rpc::object([
                        ("message", json_rpc::string(&diagnostic.message)),
                        (
                            "code",
                            diagnostic.code.map(json_rpc::string).unwrap_or(Value::Null),
                        ),
                    ])
                })
                .collect();
        return ok(
            id,
            json_rpc::object([
                ("diagnostics", Value::Arr(diagnostics)),
                ("success", Value::Bool(true)),
            ]),
        );
    }
    let formatted = match yashy_fmt::format(&text) {
        Ok(formatted) => formatted,
        Err(diagnostics) => return tool_error(id, diagnostics),
    };
    if formatted == text {
        return ok(id, json_rpc::object([("applied", Value::Arr(Vec::new()))]));
    }
    let hash = match crate::edit::file_hash(&path) {
        Ok(hash) => hash,
        Err(edit_error) => return error(id, -32001, &edit_error.message()),
    };
    let tree = match crate::edit::tree_hash(root) {
        Ok(tree) => tree,
        Err(edit_error) => return error(id, -32001, &edit_error.message()),
    };
    let patch = crate::edit::Patch {
        expected_tree_hash: tree,
        edits: vec![crate::edit::Edit {
            file: shown.to_string(),
            span: Span::new(0, text.len() as u32),
            expected_hash: hash,
            replacement: formatted,
            rationale: "format".to_string(),
        }],
    };
    match crate::edit::apply_patch(root, &patch, check_text) {
        Ok(applied) => {
            let applied: Vec<Value> = applied.into_iter().map(json_rpc::string).collect();
            ok(id, json_rpc::object([("applied", Value::Arr(applied))]))
        }
        Err(edit_error) => error(id, -32001, &edit_error.message()),
    }
}

fn call_edit_tool(root: &Path, id: Value, name: &str, arguments: Option<&Value>) -> Value {
    let arguments = match arguments {
        Some(arguments) => arguments,
        None => return error(id, -32602, "edit and refactor need arguments"),
    };
    let patch = if name == "edit" {
        match arguments.get("patch") {
            Some(value) => match parse_patch(value) {
                Ok(patch) => patch,
                Err(message) => return error(id, -32602, &message),
            },
            None => return error(id, -32602, "`edit` needs a patch"),
        }
    } else {
        let shown = arguments.get("path").and_then(Value::as_str).unwrap_or("");
        let path = match contain(root, shown) {
            Ok(path) => path,
            Err(message) => return error(id, -32001, &message),
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(io_error) => return error(id, -32001, &format!("{shown}: {io_error}")),
        };
        let from = arguments.get("from").and_then(Value::as_str).unwrap_or("");
        let to = arguments.get("to").and_then(Value::as_str).unwrap_or("");
        if from.is_empty() || from == to {
            return error(id, -32602, "`refactor` needs a `from` and a different `to`");
        }
        let mut edits = match crate::edit::rename_tokens(&text, from, to, "rename") {
            Ok(edits) => edits,
            Err(edit_error) => return error(id, -32001, &edit_error.message()),
        };
        if edits.is_empty() {
            return error(id, -32001, &format!("no token named `{from}` in {shown}"));
        }
        let hash = match crate::edit::file_hash(&path) {
            Ok(hash) => hash,
            Err(edit_error) => return error(id, -32001, &edit_error.message()),
        };
        for edit in &mut edits {
            edit.file = shown.to_string();
            edit.expected_hash = hash.clone();
        }
        let tree = match crate::edit::tree_hash(root) {
            Ok(tree) => tree,
            Err(edit_error) => return error(id, -32001, &edit_error.message()),
        };
        crate::edit::Patch {
            edits,
            expected_tree_hash: tree,
        }
    };
    match crate::edit::apply_patch(root, &patch, check_text) {
        Ok(applied) => {
            let applied: Vec<Value> = applied.into_iter().map(json_rpc::string).collect();
            let tree = crate::edit::tree_hash(root)
                .map(json_rpc::string)
                .unwrap_or(Value::Null);
            ok(
                id,
                json_rpc::object([("applied", Value::Arr(applied)), ("tree_hash", tree)]),
            )
        }
        Err(edit_error) => error(id, -32001, &edit_error.message()),
    }
}

fn call_tool(root: &Path, id: Value, name: &str, params: Option<&Value>) -> Value {
    if !TOOLS.contains(&name) {
        return error(id, -32602, &format!("unknown tool `{name}`"));
    }
    let arguments = params.and_then(|params| params.get("arguments"));
    if name == "edit" || name == "refactor" {
        return call_edit_tool(root, id, name, arguments);
    }
    if name == "format" || name == "lint" {
        return call_source_tool(root, id, name, arguments);
    }
    if name == "fix" || name == "doc" {
        return call_fix_doc_tool(root, id, name, arguments);
    }
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
                                    diagnostic.code.map(json_rpc::string).unwrap_or(Value::Null),
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
            Ok(model) => ok(id, json_rpc::string(crate::index::to_json(&model).render())),
            Err(message) => error(id, -32001, &message),
        },
        _ => error(id, -32602, "unreachable tool"),
    }
}

/// Serve one connection: a loop over stdin lines, each at most 1 MiB, until
/// the client closes the connection or sends `shutdown`.
pub fn serve(root: &Path) -> std::io::Result<()> {
    let mut session = Session::open(root)
        .map_err(|message| std::io::Error::new(std::io::ErrorKind::InvalidInput, message))?;
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
        let response = handle(&mut session, &request);
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
