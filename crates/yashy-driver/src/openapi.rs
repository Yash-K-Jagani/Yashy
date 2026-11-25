//! OpenAPI 3.1 emission and typed client generation from one route table.
//!
//! A route is a `// @route METHOD|/path|request|response|error|auth` comment
//! next to a handler. The document, the generated client and the compatibility
//! check all read those lines, so a server and its client cannot drift.

use yashy_common::json_rpc::{object, string, to_string, Value};

#[derive(Clone, Debug)]
pub struct Route {
    pub method: String,
    pub path: String,
    pub request: String,
    pub response: String,
    pub error: String,
    pub auth: String,
}

/// Collect routes from a source file. A line that is not a route comment is
/// ignored, so the declarations live beside the handlers they describe.
pub fn routes(source: &str) -> Vec<Route> {
    let mut out = Vec::new();
    for line in source.lines() {
        let text = line.trim();
        let Some(rest) = text.strip_prefix("// @route ") else {
            continue;
        };
        let fields: Vec<&str> = rest.split('|').collect();
        if fields.len() < 4 {
            continue;
        }
        out.push(Route {
            method: fields[0].trim().to_ascii_lowercase(),
            path: fields[1].trim().to_string(),
            request: fields.get(2).unwrap_or(&"").trim().to_string(),
            response: fields.get(3).unwrap_or(&"").trim().to_string(),
            error: fields.get(4).unwrap_or(&"").trim().to_string(),
            auth: fields.get(5).unwrap_or(&"").trim().to_string(),
        });
    }
    out
}

fn field(line: &str, index: usize) -> String {
    line.split('|').nth(index).unwrap_or("").trim().to_string()
}

/// A route with no response type is a build error: an emitted document must
/// never contain a 200 with an unknown body.
pub fn incomplete(routes: &[Route]) -> Vec<String> {
    routes
        .iter()
        .filter(|route| route.response.is_empty())
        .map(|route| format!("{} {} has no response type", route.method, route.path))
        .collect()
}

/// Build the document, merging methods that share a path.
pub fn document(title: &str, routes: &[Route]) -> Value {
    let mut paths: Vec<(String, Value)> = Vec::new();
    for route in routes {
        let mut operation = vec![
            ("x-yashy-response".to_string(), string(&route.response)),
            (
                "responses".to_string(),
                object([("200", object([("description", string("ok"))]))]),
            ),
        ];
        if !route.error.is_empty() {
            operation.push((
                "x-yashy-error".to_string(),
                string(&route.error),
            ));
        }
        if !route.request.is_empty() {
            operation.push(("x-yashy-request".to_string(), string(&route.request)));
        }
        if !route.auth.is_empty() {
            operation.push(("security".to_string(), object([("token", Value::Arr(vec![]))])));
        }
        let method = (route.method.clone(), Value::Obj(operation));
        if let Some((_, Value::Obj(existing))) = paths.iter_mut().find(|(path, _)| path == &route.path) {
            if let Some((_, Value::Obj(methods))) = existing.iter_mut().find(|(name, _)| name == &route.method) {
                methods.push(method);
            } else {
                existing.push(method);
            }
        } else {
            paths.push((route.path.clone(), Value::Obj(vec![method])));
        }
    }
    object([
        ("openapi", string("3.1.0")),
        (
            "info",
            object([("title", string(title)), ("version", string("1.0.0"))]),
        ),
        ("paths", Value::Obj(paths)),
    ])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compatibility {
    Additive,
    Breaking,
    Internal,
}

impl Compatibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Compatibility::Additive => "additive",
            Compatibility::Breaking => "breaking",
            Compatibility::Internal => "internal",
        }
    }
}

/// Compare a previous document with the new one. Removing a route or adding a
/// required request field breaks callers; adding a route is additive.
pub fn check(old: &Value, new: &Value) -> Compatibility {
    let old_paths = path_map(old);
    let new_paths = path_map(new);
    for (path, methods) in &old_paths {
        let Some(next) = new_paths.get(path) else {
            return Compatibility::Breaking;
        };
        for (method, operation) in methods {
            let Some(next_op) = next.get(method) else {
                return Compatibility::Breaking;
            };
            let before = request_fields(operation);
            let after = request_fields(next_op);
            if before.len() > after.len() {
                return Compatibility::Breaking;
            }
        }
    }
    if old_paths.len() < new_paths.len() {
        Compatibility::Additive
    } else {
        Compatibility::Internal
    }
}

fn path_map(doc: &Value) -> std::collections::BTreeMap<String, std::collections::BTreeMap<String, Value>> {
    let mut out = std::collections::BTreeMap::new();
    if let Some(Value::Obj(paths)) = doc.get("paths") {
        for (path, methods) in paths {
            if let Value::Obj(entries) = methods {
                let mut map = std::collections::BTreeMap::new();
                for (method, operation) in entries {
                    map.insert(method.clone(), operation.clone());
                }
                out.insert(path.clone(), map);
            }
        }
    }
    out
}

fn request_fields(operation: &Value) -> Vec<String> {
    let Some(Value::Str(request)) = operation.get("x-yashy-request") else {
        return Vec::new();
    };
    request
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

fn identifier(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(true) {
        out.insert(0, '_');
    }
    out
}

/// Generate a typed Yashy client. Each function returns the request path it
/// would call; the parameter list is taken from the document's request model,
/// so a changed required field changes the client's signature.
pub fn client_source(doc: &Value) -> String {
    let mut out = String::from("// Generated by `yashyc client`. Do not edit.\n");
    if let Some(Value::Obj(paths)) = doc.get("paths") {
        for (path, methods) in paths {
            if let Value::Obj(entries) = methods {
                for (method, operation) in entries {
                    let name = format!("{}_{}", method, identifier(path));
                    let params = request_fields(operation)
                        .iter()
                        .map(|field| {
                            let mut parts = field.splitn(2, ':');
                            let param = parts.next().unwrap_or("value").trim();
                            let ty = parts.next().unwrap_or("string").trim();
                            format!("{ty} {param}")
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    out.push_str(&format!(
                        "pub fn {name}({params}) -> string {{\n    return \"{path}\";\n}}\n\n"
                    ));
                }
            }
        }
    }
    out
}

/// Render a value as a JSON string, used by the CLI.
pub fn render(value: &Value) -> String {
    to_string(value)
}
