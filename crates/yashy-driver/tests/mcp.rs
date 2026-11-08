//! `yashy mcp`: seven compiler verbs over JSON-RPC, sandboxed to the root.

mod common;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

fn yashy() -> &'static str {
    common::yashy()
}

fn workspace_dir() -> PathBuf {
    let dir = common::work_dir("mcp-workspace");
    common::write(
        &dir,
        "main.ya",
        "fn main() {\n    print(\"hi\");\n}\n",
    );
    common::write(
        &dir,
        "bad.ya",
        "fn main() {\n    let int x = \"three\";\n}\n",
    );
    dir
}

struct Mcp {
    child: Child,
    stdin: std::process::ChildStdin,
    stdout: std::io::BufReader<std::process::ChildStdout>,
}

impl Mcp {
    fn start(root: &Path) -> Mcp {
        let mut child = Command::new(yashy())
            .arg("mcp")
            .arg("--root")
            .arg(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("the yashy CLI must be runnable");
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = std::io::BufReader::new(child.stdout.take().expect("stdout is piped"));
        Mcp {
            child,
            stdin,
            stdout,
        }
    }

    fn call(&mut self, request: &str) -> String {
        use std::io::BufRead;
        writeln!(self.stdin, "{request}").expect("write the request");
        self.stdin.flush().expect("flush the request");
        let mut line = String::new();
        self.stdout.read_line(&mut line).expect("read the response");
        line
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn request(id: i64, method: &str, params: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":{params}}}"#)
}

#[test]
fn initialize_advertises_exactly_the_seven_tools() {
    let dir = workspace_dir();
    let mut mcp = Mcp::start(&dir);
    let response = mcp.call(&request(1, "initialize", "{}"));
    for tool in ["check", "build", "test", "verify", "explain", "context", "index"] {
        assert!(
            response.contains(&format!(r#""name":"{tool}""#)),
            "tool {tool} is advertised:\n{response}"
        );
    }
    assert!(
        !response.contains("shell"),
        "no shell verb:\n{response}"
    );
}

#[test]
fn check_on_a_bad_file_returns_diagnostics_with_codes() {
    let dir = workspace_dir();
    let mut mcp = Mcp::start(&dir);
    let response = mcp.call(&request(
        2,
        "tools/call",
        r#"{"name":"check","arguments":{"path":"bad.ya"}}"#,
    ));
    assert!(
        response.contains("-32000"),
        "a tool error, not a protocol error:\n{response}"
    );
    assert!(
        response.contains("E0100"),
        "the diagnostic code is in the data:\n{response}"
    );
}

#[test]
fn a_path_traversal_is_refused_and_no_file_is_read() {
    let dir = workspace_dir();
    let mut mcp = Mcp::start(&dir);
    let response = mcp.call(&request(
        3,
        "tools/call",
        r#"{"name":"check","arguments":{"path":"../../etc/passwd"}}"#,
    ));
    assert!(
        response.contains("-32001"),
        "traversal is refused with -32001:\n{response}"
    );
    assert!(
        !response.contains("root:"),
        "no file contents leaked:\n{response}"
    );
}

#[test]
fn a_malformed_request_is_an_error_and_the_server_stays_alive() {
    let dir = workspace_dir();
    let mut mcp = Mcp::start(&dir);
    let bad = mcp.call("{not json");
    assert!(bad.contains("-32700"), "a parse error:\n{bad}");
    let good = mcp.call(&request(4, "initialize", "{}"));
    assert!(
        good.contains("\"protocolVersion\""),
        "the server answers after the malformed request:\n{good}"
    );
}

#[test]
fn an_unknown_method_is_minus_32601() {
    let dir = workspace_dir();
    let mut mcp = Mcp::start(&dir);
    let response = mcp.call(&request(5, "nope", "{}"));
    assert!(
        response.contains("-32601"),
        "method not found:\n{response}"
    );
}

#[test]
fn an_unknown_tool_is_minus_32602() {
    let dir = workspace_dir();
    let mut mcp = Mcp::start(&dir);
    let response = mcp.call(&request(
        6,
        "tools/call",
        r#"{"name":"shell","arguments":{}}"#,
    ));
    assert!(
        response.contains("-32602"),
        "invalid params for an unknown tool:\n{response}"
    );
}
