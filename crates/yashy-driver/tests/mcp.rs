//! `yashyc mcp`: seven compiler verbs over JSON-RPC, sandboxed to the root.

mod common;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

fn yashyc() -> &'static str {
    common::yashyc()
}

fn workspace_dir() -> PathBuf {
    // One directory per caller: tests in this process run in parallel, and
    // sharing a directory races `remove_dir_all` against the other test's
    // writes.
    use std::sync::atomic::{AtomicUsize, Ordering};
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    let dir = common::work_dir(&format!(
        "mcp-workspace-{}",
        CALLS.fetch_add(1, Ordering::SeqCst)
    ));
    common::write(&dir, "main.ya", "fn main() {\n    print(\"hi\");\n}\n");
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
        let mut child = Command::new(yashyc())
            .arg("mcp")
            .arg("--root")
            .arg(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("the yashyc CLI must be runnable");
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
        // Wait before releasing the executable: on Windows the image stays
        // locked until the process is gone, and an unwaited kill leaves the
        // next link step failing with "cannot open file".
        let _ = self.child.wait();
    }
}

fn request(id: i64, method: &str, params: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":{params}}}"#)
}

#[test]
fn initialize_advertises_the_compiler_and_edit_tools() {
    let dir = workspace_dir();
    let mut mcp = Mcp::start(&dir);
    let response = mcp.call(&request(1, "initialize", "{}"));
    for tool in [
        "check", "build", "test", "verify", "explain", "context", "index", "edit", "refactor",
        "format", "lint", "fix", "doc",
    ] {
        assert!(
            response.contains(&format!(r#""name":"{tool}""#)),
            "tool {tool} is advertised:\n{response}"
        );
    }
    assert!(!response.contains("shell"), "no shell verb:\n{response}");
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
    assert!(response.contains("-32601"), "method not found:\n{response}");
}

#[test]
fn refactor_renames_a_function_and_every_call_site() {
    let dir = common::work_dir("mcp-refactor-rename");
    common::write(
        &dir,
        "main.ya",
        "fn double(int x) -> int { return x * 2; }\nfn main() { print(double(3)); }\n",
    );
    let mut mcp = Mcp::start(&dir);
    let response = mcp.call(&request(
        7,
        "tools/call",
        r#"{"name":"refactor","arguments":{"path":"main.ya","from":"double","to":"twice"}}"#,
    ));
    assert!(response.contains("\"applied\""), "{response}");
    let source = std::fs::read_to_string(dir.join("main.ya")).unwrap();
    assert!(source.contains("fn twice"), "{source}");
    assert!(source.contains("twice(3)"), "{source}");
    assert!(!source.contains("double"), "{source}");
}

#[test]
fn lint_reports_diagnostics_without_mutating() {
    let dir = workspace_dir();
    let before = std::fs::read(dir.join("bad.ya")).unwrap();
    let mut mcp = Mcp::start(&dir);
    let response = mcp.call(&request(
        8,
        "tools/call",
        r#"{"name":"lint","arguments":{"path":"bad.ya"}}"#,
    ));
    assert!(response.contains("\"diagnostics\""), "{response}");
    assert_eq!(std::fs::read(dir.join("bad.ya")).unwrap(), before);
}

#[test]
fn format_applies_through_the_edit_transaction() {
    let dir = common::work_dir("mcp-format");
    common::write(&dir, "main.ya", "fn main(){print(1);}");
    let mut mcp = Mcp::start(&dir);
    let response = mcp.call(&request(
        9,
        "tools/call",
        r#"{"name":"format","arguments":{"path":"main.ya"}}"#,
    ));
    assert!(response.contains("\"applied\""), "{response}");
    let source = std::fs::read_to_string(dir.join("main.ya")).unwrap();
    assert_ne!(source, "fn main(){print(1);}");
    assert!(source.contains("fn main()"), "{source}");
}

#[test]
fn fix_applies_machine_applicable_suggestions() {
    let dir = common::work_dir("mcp-fix");
    common::write(&dir, "main.ya", "fn main() { var int x = 1; }\n");
    let mut mcp = Mcp::start(&dir);
    let response = mcp.call(&request(
        10,
        "tools/call",
        r#"{"name":"fix","arguments":{"path":"main.ya"}}"#,
    ));
    assert!(response.contains("\"applied\""), "{response}");
    let source = std::fs::read_to_string(dir.join("main.ya")).unwrap();
    assert!(source.contains("let int x"), "{source}");
}

#[test]
fn doc_returns_a_json_model() {
    let dir = common::work_dir("mcp-doc");
    common::write(&dir, "main.ya", "/// A documented entry.\nfn main() {}\n");
    let mut mcp = Mcp::start(&dir);
    let response = mcp.call(&request(
        11,
        "tools/call",
        r#"{"name":"doc","arguments":{}}"#,
    ));
    assert!(response.contains(r#"\"modules\""#), "{response}");
    assert!(response.contains(r#"\"schema_version\""#), "{response}");
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
