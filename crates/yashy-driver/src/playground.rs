//! The playground: the phase 11 sandbox behind a small HTTP front end.
//!
//! The playground compiles on the server. There is no WASM compiler, and
//! there is not going to be one in this phase: a browser build of Yashy is a
//! 2.0 project with its own performance story, and the sandbox already
//! exists.
//!
//! Every run is a `yashy run --sandbox` with a directory of its own, the
//! default-deny capability set, a wall clock, a memory cap and an output
//! cap. The run may *read* its own directory and write nothing anywhere: a
//! playground program has no reason to write, and a program that can write is
//! a program that can fill the disk.
//!
//! A program that tries to escape is refused by the same code the escape
//! corpus tests -- the playground is the sandbox's customer, not a second
//! sandbox.
//!
//! # What the server refuses, and how
//!
//! | Refusal | Status | Why |
//! |---|---|---|
//! | the source is over the cap | `413` | the body is too large to be a program a person meant to write |
//! | the caller is over its rate | `429` | one caller cannot take the queue for everyone |
//! | the queue is full | `429` | a refusal is better than a hang |
//! | the run broke a limit | `200` with the diagnostic | the program ran and the sandbox stopped it |
//!
//! A refusal is never a hang, and never a crash: the front end gets a JSON
//! document either way.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use yashy_common::Json;

/// The limits every run is under.
///
/// The defaults are the ones the plan names: a 64 KB source, a two second
/// wall clock, 128 MB of memory, 64 KB of output, a queue of eight and a
/// hundred requests a minute per caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The largest source the server will accept.
    pub max_source_bytes: usize,
    /// The wall clock for one run.
    pub wall_ms: u64,
    /// The memory one run may use.
    pub memory_mb: u64,
    /// The output one run may produce.
    pub output_bytes: u64,
    /// How many runs may be in flight at once.
    pub queue_depth: usize,
    /// How many requests one caller may make a minute.
    pub requests_per_minute: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_source_bytes: 64 * 1024,
            wall_ms: 2000,
            memory_mb: 128,
            output_bytes: 64 * 1024,
            queue_depth: 8,
            requests_per_minute: 100,
        }
    }
}

/// A request: the program and the arguments it is run with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Request {
    pub source: String,
    pub args: Vec<String>,
}

/// What the server answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// The program ran. `exit` is its exit status, and the output is what it
    /// printed.
    Ran {
        stdout: String,
        stderr: String,
        exit: i32,
    },
    /// The request was refused before anything ran.
    Refused { status: u16, message: String },
}

impl Reply {
    /// The HTTP status the reply carries.
    pub fn status(&self) -> u16 {
        match self {
            Reply::Ran { .. } => 200,
            Reply::Refused { status, .. } => *status,
        }
    }

    /// The reply as the JSON document the front end reads.
    ///
    /// The shape is the same either way, so the front end has one code path:
    /// `stdout`, `stderr`, `exit` and `diagnostics` for a run, and `status`
    /// and `message` for a refusal.
    pub fn json(&self) -> String {
        let mut object = Json::object();
        match self {
            Reply::Ran {
                stdout,
                stderr,
                exit,
            } => {
                object.set("ok", Json::bool(true));
                object.set("stdout", Json::string(stdout));
                object.set("stderr", Json::string(stderr));
                object.set("exit", Json::number(*exit as i64));
                object.set("diagnostics", Json::array(Vec::new()));
            }
            Reply::Refused { status, message } => {
                object.set("ok", Json::bool(false));
                object.set("status", Json::number(*status as i64));
                object.set("message", Json::string(message));
                object.set("stdout", Json::string(""));
                object.set("stderr", Json::string(""));
                object.set("exit", Json::number(0));
                object.set("diagnostics", Json::array(Vec::new()));
            }
        }
        object.render()
    }
}

/// The server's state: who called, how often, and how much is running.
pub struct Server {
    limits: Limits,
    /// The times each caller asked, oldest first.
    callers: HashMap<String, Vec<Instant>>,
    /// How many runs are in flight.
    running: usize,
    /// The compiler the server shells out to.
    compiler: PathBuf,
    /// Where the runs' directories are made.
    root: PathBuf,
}

impl Server {
    /// A server with the default limits.
    pub fn new(limits: Limits) -> Server {
        Server::with_paths(
            limits,
            default_compiler(),
            std::env::temp_dir().join("yashy-playground"),
        )
    }

    /// A server that runs the compiler at `compiler` and makes its
    /// directories under `root`.
    pub fn with_paths(limits: Limits, compiler: PathBuf, root: PathBuf) -> Server {
        Server {
            limits,
            callers: HashMap::new(),
            running: 0,
            compiler,
            root,
        }
    }

    /// The limits this server enforces.
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Handle one request from `ip`.
    ///
    /// The order of the checks is the order a reader expects: the caller
    /// first (a caller over its rate is not worth reading), then the queue
    /// (a full queue is not worth compiling), then the source (a body that is
    /// too large is not worth writing down).
    pub fn handle(&mut self, ip: &str, request: Request) -> Reply {
        if let Some(message) = self.over_rate(ip) {
            return Reply::Refused {
                status: 429,
                message,
            };
        }
        if self.running >= self.limits.queue_depth {
            return Reply::Refused {
                status: 429,
                message: format!(
                    "the playground is busy: {} runs are already in flight",
                    self.limits.queue_depth
                ),
            };
        }
        if request.source.len() > self.limits.max_source_bytes {
            return Reply::Refused {
                status: 413,
                message: format!(
                    "the source is {} bytes; the playground accepts {}",
                    request.source.len(),
                    self.limits.max_source_bytes
                ),
            };
        }
        self.run(request)
    }

    /// Whether `ip` has asked for more than its share this minute.
    fn over_rate(&mut self, ip: &str) -> Option<String> {
        let now = Instant::now();
        let window = Duration::from_secs(60);
        let times = self.callers.entry(ip.to_string()).or_default();
        times.retain(|at| now.duration_since(*at) < window);
        if times.len() as u32 >= self.limits.requests_per_minute {
            return Some(format!(
                "too many requests: the limit is {} a minute",
                self.limits.requests_per_minute
            ));
        }
        times.push(now);
        None
    }

    /// Compile and run the program, in a directory of its own.
    fn run(&mut self, request: Request) -> Reply {
        let id = self.next_id();
        let directory = self.root.join(&id);
        if let Err(error) = std::fs::create_dir_all(&directory) {
            return Reply::Refused {
                status: 500,
                message: format!("cannot make a run directory: {error}"),
            };
        }
        let source = directory.join("main.ya");
        if let Err(error) = std::fs::write(&source, &request.source) {
            let _ = std::fs::remove_dir_all(&directory);
            return Reply::Refused {
                status: 500,
                message: format!("cannot write the program: {error}"),
            };
        }

        let mut command = std::process::Command::new(&self.compiler);
        command
            .arg("run")
            .arg("--sandbox")
            .arg("--allow-read")
            .arg(&directory)
            .arg("--time")
            .arg(self.limits.wall_ms.to_string())
            .arg("--memory")
            .arg(self.limits.memory_mb.to_string())
            .arg("--output-cap")
            .arg(self.limits.output_bytes.to_string())
            .arg(&source)
            .arg("--");
        command.args(&request.args);
        command.current_dir(&directory);

        self.running += 1;
        let output = command.output();
        self.running -= 1;
        let _ = std::fs::remove_dir_all(&directory);

        match output {
            Ok(output) => {
                let mut stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();
                // The output cap is the sandbox's, but the server keeps its own:
                // a program that printed past the cap is truncated rather than
                // allowed to fill the front end's memory.
                truncate(&mut stdout, self.limits.output_bytes);
                truncate(&mut stderr, self.limits.output_bytes);
                Reply::Ran {
                    stdout,
                    stderr,
                    exit: output.status.code().unwrap_or(-1),
                }
            }
            Err(error) => Reply::Refused {
                status: 500,
                message: format!("cannot run the compiler: {error}"),
            },
        }
    }

    /// A directory name for one run.
    fn next_id(&self) -> String {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_nanos())
            .unwrap_or(0);
        format!("run-{stamp}-{}", std::process::id())
    }
}

/// Cut `text` to `limit` bytes, on a character boundary.
fn truncate(text: &mut String, limit: u64) {
    if text.len() as u64 <= limit {
        return;
    }
    let mut end = (limit as usize).min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push_str("\n[output truncated]\n");
}

/// The compiler the playground drives.
///
/// The server is the compiler's sibling: `yashy playground` runs beside
/// `yashy`, and the server shells out to the very binary that is running it.
/// `YASHY_BIN` names another one, which is what a test does.
fn default_compiler() -> PathBuf {
    if let Some(path) = std::env::var_os("YASHY_BIN") {
        return PathBuf::from(path);
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("yashy.exe")))
        .unwrap_or_else(|| PathBuf::from("yashy"))
}

/// Parse the request body: `{"source": "...", "args": ["..."]}`.
///
/// The playground accepts one shape and rejects everything else, so the parser
/// is small on purpose: a string, and an array of strings. Anything else is a
/// refusal, not a guess.
pub fn parse_request(body: &str) -> Result<Request, String> {
    let value = parse_json(body)?;
    let object = match value {
        JsonValue::Object(fields) => fields,
        _ => return Err("the body must be a JSON object".to_string()),
    };
    let mut request = Request::default();
    for (key, value) in object {
        match (key.as_str(), value) {
            ("source", JsonValue::Text(text)) => request.source = text,
            ("args", JsonValue::List(items)) => {
                let mut args = Vec::new();
                for item in items {
                    match item {
                        JsonValue::Text(text) => args.push(text),
                        _ => return Err("`args` must be an array of strings".to_string()),
                    }
                }
                request.args = args;
            }
            _ => {}
        }
    }
    Ok(request)
}

/// The JSON the parser understands.
#[derive(Clone, Debug, PartialEq, Eq)]
enum JsonValue {
    Null,
    Bool(bool),
    Number(i64),
    Text(String),
    List(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

/// Parse one JSON document.
fn parse_json(text: &str) -> Result<JsonValue, String> {
    let bytes: Vec<char> = text.chars().collect();
    let mut index = 0usize;
    let value = parse_value(&bytes, &mut index)?;
    skip_space(&bytes, &mut index);
    if index != bytes.len() {
        return Err("the body has text after the JSON document".to_string());
    }
    Ok(value)
}

fn skip_space(bytes: &[char], index: &mut usize) {
    while *index < bytes.len() && bytes[*index].is_whitespace() {
        *index += 1;
    }
}

fn parse_value(bytes: &[char], index: &mut usize) -> Result<JsonValue, String> {
    skip_space(bytes, index);
    let Some(&character) = bytes.get(*index) else {
        return Err("the body is empty".to_string());
    };
    match character {
        '{' => parse_object(bytes, index),
        '[' => parse_array(bytes, index),
        '"' => parse_text(bytes, index).map(JsonValue::Text),
        't' => parse_literal(bytes, index, "true", JsonValue::Bool(true)),
        'f' => parse_literal(bytes, index, "false", JsonValue::Bool(false)),
        'n' => parse_literal(bytes, index, "null", JsonValue::Null),
        _ => parse_number(bytes, index),
    }
}

fn parse_literal(
    bytes: &[char],
    index: &mut usize,
    word: &str,
    value: JsonValue,
) -> Result<JsonValue, String> {
    for expected in word.chars() {
        if bytes.get(*index) != Some(&expected) {
            return Err(format!("the body is not JSON: expected `{word}`"));
        }
        *index += 1;
    }
    Ok(value)
}

fn parse_number(bytes: &[char], index: &mut usize) -> Result<JsonValue, String> {
    let start = *index;
    if bytes.get(*index) == Some(&'-') {
        *index += 1;
    }
    while let Some(character) = bytes.get(*index) {
        if character.is_ascii_digit() {
            *index += 1;
        } else {
            break;
        }
    }
    if start == *index {
        return Err("the body is not JSON".to_string());
    }
    let text: String = bytes[start..*index].iter().collect();
    text.parse::<i64>()
        .map(JsonValue::Number)
        .map_err(|_| "the body is not JSON: a number is out of range".to_string())
}

fn parse_text(bytes: &[char], index: &mut usize) -> Result<String, String> {
    if bytes.get(*index) != Some(&'"') {
        return Err("the body is not JSON: expected a string".to_string());
    }
    *index += 1;
    let mut out = String::new();
    while let Some(&character) = bytes.get(*index) {
        *index += 1;
        match character {
            '"' => return Ok(out),
            '\\' => {
                let Some(&escape) = bytes.get(*index) else {
                    return Err("the body is not JSON: a string ends in a backslash".to_string());
                };
                *index += 1;
                match escape {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    '/' => out.push('/'),
                    'b' => out.push('\u{8}'),
                    'f' => out.push('\u{c}'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    'u' => {
                        let mut code = 0u32;
                        for _ in 0..4 {
                            let Some(&digit) = bytes.get(*index) else {
                                return Err("the body is not JSON: a short escape".to_string());
                            };
                            *index += 1;
                            let digit = digit
                                .to_digit(16)
                                .ok_or_else(|| "the body is not JSON: a bad escape".to_string())?;
                            code = code * 16 + digit;
                        }
                        out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                    }
                    _ => return Err("the body is not JSON: an unknown escape".to_string()),
                }
            }
            other => out.push(other),
        }
    }
    Err("the body is not JSON: a string is not closed".to_string())
}

fn parse_array(bytes: &[char], index: &mut usize) -> Result<JsonValue, String> {
    *index += 1;
    let mut items = Vec::new();
    skip_space(bytes, index);
    if bytes.get(*index) == Some(&']') {
        *index += 1;
        return Ok(JsonValue::List(items));
    }
    loop {
        items.push(parse_value(bytes, index)?);
        skip_space(bytes, index);
        match bytes.get(*index) {
            Some(&',') => {
                *index += 1;
            }
            Some(&']') => {
                *index += 1;
                return Ok(JsonValue::List(items));
            }
            _ => return Err("the body is not JSON: an array is not closed".to_string()),
        }
    }
}

fn parse_object(bytes: &[char], index: &mut usize) -> Result<JsonValue, String> {
    *index += 1;
    let mut fields = Vec::new();
    skip_space(bytes, index);
    if bytes.get(*index) == Some(&'}') {
        *index += 1;
        return Ok(JsonValue::Object(fields));
    }
    loop {
        skip_space(bytes, index);
        let key = parse_text(bytes, index)?;
        skip_space(bytes, index);
        if bytes.get(*index) != Some(&':') {
            return Err("the body is not JSON: a field has no value".to_string());
        }
        *index += 1;
        let value = parse_value(bytes, index)?;
        fields.push((key, value));
        skip_space(bytes, index);
        match bytes.get(*index) {
            Some(&',') => {
                *index += 1;
            }
            Some(&'}') => {
                *index += 1;
                return Ok(JsonValue::Object(fields));
            }
            _ => return Err("the body is not JSON: an object is not closed".to_string()),
        }
    }
}

/// The front end's directory: the static files the server hands out.
pub fn frontend_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../playground")
        .to_path_buf()
}

/// The file name for a path in the front end, refusing to leave it.
pub fn frontend_file(path: &str) -> Option<PathBuf> {
    let name = path.trim_start_matches('/');
    if name.is_empty() {
        return Some(frontend_dir().join("index.html"));
    }
    if name.contains("..") || name.contains('\\') {
        return None;
    }
    Some(frontend_dir().join(name))
}

/// Serve the playground until the process is stopped.
///
/// The server is deliberately small: one request at a time per connection, a
/// body it reads to its end, and a reply it writes whole. A playground that
/// needs a framework is a playground that needs a toolchain to build, and
/// this one has to run beside a compiler binary.
pub fn serve(server: &mut Server, listener: std::net::TcpListener) -> std::io::Result<()> {
    for stream in listener.incoming() {
        let mut stream = stream?;
        let caller = stream
            .peer_addr()
            .map(|address| address.ip().to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        let _ = handle_connection(server, &mut stream, &caller);
    }
    Ok(())
}

/// Read one request and write one reply.
fn handle_connection(
    server: &mut Server,
    stream: &mut std::net::TcpStream,
    caller: &str,
) -> std::io::Result<()> {
    use std::io::{Read, Write};

    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    // Read the head first: the request line and the headers end with a blank
    // line, and the body is as long as `Content-Length` says.
    let head_end = loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break buffer.len();
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = find_head_end(&buffer) {
            break position;
        }
        if buffer.len() > 64 * 1024 {
            break buffer.len();
        }
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default().to_string();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();

    let mut length = 0usize;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    // The body is what is left, plus whatever has not arrived yet.
    let mut body = buffer[head_end.min(buffer.len())..].to_vec();
    while body.len() < length {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }

    let (status, content_type, payload) = match (method.as_str(), path.as_str()) {
        ("POST", "/run") => {
            let reply = match parse_request(&String::from_utf8_lossy(&body)) {
                Ok(request) => server.handle(caller, request),
                Err(message) => Reply::Refused {
                    status: 400,
                    message,
                },
            };
            (reply.status(), "application/json", reply.json())
        }
        ("GET", _) => match frontend_file(path.split('?').next().unwrap_or(path.as_str())) {
            Some(file) => match std::fs::read(&file) {
                Ok(bytes) => (
                    200,
                    content_type_for(&file),
                    String::from_utf8_lossy(&bytes).into_owned(),
                ),
                Err(_) => (404, "text/plain", "not found\n".to_string()),
            },
            None => (403, "text/plain", "forbidden\n".to_string()),
        },
        _ => (405, "text/plain", "method not allowed\n".to_string()),
    };

    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

/// Where the blank line that ends the request head is.
fn find_head_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

/// The content type a static file is served with.
fn content_type_for(path: &Path) -> &'static str {
    match path.extension().and_then(|value| value.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        _ => "text/plain; charset=utf-8",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_read_from_the_json_body() {
        let request = parse_request(r#"{"source": "fn main() {}", "args": ["a", "b"]}"#)
            .expect("the body must parse");
        assert_eq!(request.source, "fn main() {}");
        assert_eq!(request.args, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn an_escape_in_the_source_is_read_as_written() {
        let request =
            parse_request(r#"{"source": "print(\"a\\\"b\");"}"#).expect("the body must parse");
        assert_eq!(request.source, "print(\"a\\\"b\");");
    }

    #[test]
    fn a_body_that_is_not_json_is_refused() {
        assert!(parse_request("not json").is_err());
        assert!(parse_request("").is_err());
        assert!(parse_request("[1, 2]").is_err());
    }

    #[test]
    fn the_frontend_never_leaves_its_directory() {
        assert!(frontend_file("../../etc/passwd").is_none());
        assert!(frontend_file("/index.html").is_some());
    }
}
