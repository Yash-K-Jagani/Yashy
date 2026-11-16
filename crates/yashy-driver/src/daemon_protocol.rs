//! Wire protocol for the build daemon.
//!
//! The daemon speaks JSON-RPC 2.0 over a loopback TCP socket, framed the same
//! way LSP frames its stdio messages (`Content-Length: N\r\n\r\n<body>`). It is
//! deliberately small: seven verbs, no shell execution, no paths outside the
//! workspace root. The LSP can reuse the same framing and request shape.

use std::io::{self, BufRead, Read, Write};

use yashy_common::json_rpc::{self, number, object, parse, string, Value};

/// The verbs the daemon answers. They are the compiler's own verbs, not a
/// separate language server vocabulary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Check { file: String },
    Verify { file: String },
    Build { file: String, out: String },
    Test { root: String },
    Diagnostics,
    Shutdown,
}

impl Request {
    pub fn name(&self) -> &'static str {
        match self {
            Request::Check { .. } => "check",
            Request::Verify { .. } => "verify",
            Request::Build { .. } => "build",
            Request::Test { .. } => "test",
            Request::Diagnostics => "diagnostics",
            Request::Shutdown => "shutdown",
        }
    }

    pub fn to_json(&self, id: i64) -> Value {
        let params = match self {
            Request::Check { file } | Request::Verify { file } => [("file", string(file))].to_vec(),
            Request::Build { file, out } => [("file", string(file)), ("out", string(out))].to_vec(),
            Request::Test { root } => [("root", string(root))].to_vec(),
            Request::Diagnostics | Request::Shutdown => Vec::new(),
        };
        object([
            ("jsonrpc", string("2.0")),
            ("id", number(id)),
            ("method", string(self.name())),
            (
                "params",
                Value::Obj(
                    params
                        .into_iter()
                        .map(|(key, value)| (key.to_string(), value))
                        .collect(),
                ),
            ),
        ])
    }

    pub fn from_json(message: &Value) -> Result<Self, String> {
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .ok_or_else(|| "request has no method".to_string())?;
        let params = message.get("params");
        let string = |key: &str| -> Result<String, String> {
            params
                .and_then(|p| p.get(key))
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| format!("request is missing `{key}`"))
        };
        match method {
            "check" => Ok(Request::Check {
                file: string("file")?,
            }),
            "verify" => Ok(Request::Verify {
                file: string("file")?,
            }),
            "build" => Ok(Request::Build {
                file: string("file")?,
                out: string("out")?,
            }),
            "test" => Ok(Request::Test {
                root: string("root")?,
            }),
            "diagnostics" => Ok(Request::Diagnostics),
            "shutdown" => Ok(Request::Shutdown),
            other => Err(format!("unknown method `{other}`")),
        }
    }
}

/// The result of one verb, alongside its request id.
#[derive(Clone, Debug, PartialEq)]
pub enum ResponseKind {
    /// Diagnostics as schema-v1 JSON objects, plus the daemon's cache hits.
    Diagnostics {
        diagnostics: Vec<Value>,
        cache_hits: u64,
    },
    Built {
        path: String,
    },
    Test {
        passed: usize,
        failed: usize,
        compile_errors: usize,
    },
    Ok,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Response {
    pub id: i64,
    pub result: Result<ResponseKind, String>,
}

impl Response {
    pub fn to_json(&self) -> Value {
        match &self.result {
            Ok(kind) => {
                let result = match kind {
                    ResponseKind::Diagnostics {
                        diagnostics,
                        cache_hits,
                    } => object([
                        ("diagnostics", Value::Arr(diagnostics.clone())),
                        ("cache_hits", number(*cache_hits as i64)),
                    ]),
                    ResponseKind::Built { path } => object([("path", string(path))]),
                    ResponseKind::Test {
                        passed,
                        failed,
                        compile_errors,
                    } => object([
                        ("passed", number(*passed as i64)),
                        ("failed", number(*failed as i64)),
                        ("compile_errors", number(*compile_errors as i64)),
                    ]),
                    ResponseKind::Ok => object([]),
                };
                object([
                    ("jsonrpc", string("2.0")),
                    ("id", number(self.id)),
                    ("result", result),
                ])
            }
            Err(error) => object([
                ("jsonrpc", string("2.0")),
                ("id", number(self.id)),
                ("error", object([("message", string(error))])),
            ]),
        }
    }

    pub fn from_json(message: &Value) -> Result<Self, String> {
        let id = message
            .get("id")
            .and_then(Value::as_i64)
            .ok_or_else(|| "response has no id".to_string())?;
        if let Some(error) = message.get("error") {
            let text = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error")
                .to_string();
            return Ok(Response {
                id,
                result: Err(text),
            });
        }
        let result = message
            .get("result")
            .ok_or_else(|| "response has neither result nor error".to_string())?;
        let kind = if let Some(list) = result.get("diagnostics") {
            let diagnostics = list.as_array().unwrap_or(&[]).to_vec();
            let cache_hits = result
                .get("cache_hits")
                .and_then(Value::as_i64)
                .unwrap_or(0) as u64;
            ResponseKind::Diagnostics {
                diagnostics,
                cache_hits,
            }
        } else if let Some(path) = result.get("path").and_then(Value::as_str) {
            ResponseKind::Built {
                path: path.to_string(),
            }
        } else if result.get("passed").is_some() {
            let number = |key: &str| result.get(key).and_then(Value::as_i64).unwrap_or(0) as usize;
            ResponseKind::Test {
                passed: number("passed"),
                failed: number("failed"),
                compile_errors: number("compile_errors"),
            }
        } else {
            ResponseKind::Ok
        };
        Ok(Response {
            id,
            result: Ok(kind),
        })
    }
}

/// Write one framed JSON-RPC message.
pub fn write_message<W: Write>(out: &mut W, value: &Value) -> io::Result<()> {
    let body = yashy_common::json_rpc::to_string(value);
    write!(out, "Content-Length: {}\r\n\r\n{body}", body.len())?;
    out.flush()
}

/// Read one framed JSON-RPC message. `Ok(None)` is a clean end of stream.
pub fn read_message<R: BufRead>(reader: &mut R) -> io::Result<Option<Value>> {
    let mut length: Option<usize> = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length:") {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let length = length.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "a frame had no Content-Length header",
        )
    })?;
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    let text = String::from_utf8(body)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "a frame was not UTF-8"))?;
    parse(&text)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}
