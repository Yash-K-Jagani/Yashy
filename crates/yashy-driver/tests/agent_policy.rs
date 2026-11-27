//! The agent policy and its hash-chained audit log.
//!
//! The policy is enforced by the MCP adapter before a tool runs, and every
//! call -- allowed, refused or failed -- lands in `.yashy/audit/session.jsonl`.
//! Removing an entry must be detectable: the chain is the evidence that the
//! log in front of you is the log that was written.

mod common;

use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

fn yashyc() -> &'static str {
    common::yashyc()
}

fn workspace(name: &str, policy: Option<&str>) -> PathBuf {
    let dir = common::work_dir(name);
    common::write(&dir, "main.ya", "fn main() {\n    print(\"hi\");\n}\n");
    if let Some(policy) = policy {
        common::write(&dir, ".yashy/policy.toml", policy);
    }
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
        let _ = self.child.wait();
    }
}

fn call(name: &str, arguments: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
    )
}

fn audit_verify(root: &Path) -> (bool, String, String) {
    let output = Command::new(yashyc())
        .args(["agent", "audit", "--verify", "--json", "--root"])
        .arg(root)
        .output()
        .expect("the CLI must run");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn a_denied_tool_is_refused_with_a_code_and_audited() {
    let dir = workspace("agent-policy-deny", Some("allow_verbs = [\"check\"]\n"));
    {
        let mut mcp = Mcp::start(&dir);
        let response = mcp.call(&call(
            "edit",
            "{\"patch\":{\"expected_tree_hash\":\"\",\"edits\":[]}}",
        ));
        assert!(response.contains("\"code\":-32002"), "{response}");
        assert!(response.contains("E1807"), "{response}");
        assert!(response.contains("policy denies tool `edit`"), "{response}");
    }
    let (ok, stdout, stderr) = audit_verify(&dir);
    assert!(ok, "the refusal is one chained entry: {stdout} {stderr}");
    assert!(stdout.contains("\"entries\": 1"), "{stdout}");
}

#[test]
fn a_protected_path_is_refused_even_when_the_tool_is_allowed() {
    let dir = workspace(
        "agent-policy-protected",
        Some("allow_verbs = [\"edit\"]\nprotected_paths = [\"yashy.lock\", \".github/**\"]\n"),
    );
    let mut mcp = Mcp::start(&dir);
    let response = mcp.call(&call(
        "edit",
        "{\"patch\":{\"expected_tree_hash\":\"\",\"edits\":[{\"file\":\"yashy.lock\",\"span\":{\"start\":0,\"end\":0},\"expected_hash\":\"\",\"replacement\":\"\",\"rationale\":\"x\"}]}}",
    ));
    assert!(
        response.contains("policy protects path `yashy.lock`"),
        "{response}"
    );

    let allowed = mcp.call(&call(
        "edit",
        "{\"patch\":{\"expected_tree_hash\":\"\",\"edits\":[{\"file\":\"main.ya\",\"span\":{\"start\":0,\"end\":0},\"expected_hash\":\"\",\"replacement\":\"\",\"rationale\":\"x\"}]}}",
    ));
    assert!(
        !allowed.contains("policy protects path"),
        "a normal path is not protected: {allowed}"
    );
}

#[test]
fn a_patch_over_the_edit_budget_is_refused_before_it_runs() {
    let dir = workspace(
        "agent-policy-budget",
        Some("allow_verbs = [\"edit\"]\nmax_edits = 1\n"),
    );
    let mut mcp = Mcp::start(&dir);
    let response = mcp.call(&call(
        "edit",
        "{\"patch\":{\"expected_tree_hash\":\"\",\"edits\":[{\"file\":\"main.ya\"},{\"file\":\"main.ya\"}]}}",
    ));
    assert!(
        response.contains("patch has 2 edits; policy allows 1"),
        "{response}"
    );
    assert!(response.contains("-32002"), "{response}");
}

#[test]
fn a_removed_entry_is_named_by_the_verifier() {
    let dir = workspace("agent-audit-tamper", None);
    {
        let mut mcp = Mcp::start(&dir);
        for _ in 0..3 {
            let response = mcp.call(&call("check", "{\"path\":\"main.ya\"}"));
            assert!(response.contains("\"result\""), "{response}");
        }
    }
    let (ok, stdout, stderr) = audit_verify(&dir);
    assert!(ok, "three calls chain: {stdout} {stderr}");
    assert!(stdout.contains("\"entries\": 3"), "{stdout}");

    let path = dir.join(".yashy/audit/session.jsonl");
    let text = fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3);
    fs::write(&path, format!("{}\n{}\n", lines[0], lines[2])).unwrap();

    let (ok, stdout, stderr) = audit_verify(&dir);
    assert!(!ok, "removing an entry must fail verification: {stdout}");
    assert!(stdout.contains("\"ok\": false"), "{stdout}");
    assert!(stdout.contains("\"first_bad_seq\": 3"), "{stdout}");
    assert!(
        stdout.contains("\"code\": \"E1804\"") || stderr.contains("E1804"),
        "{stdout} {stderr}"
    );
}

#[test]
fn no_policy_file_is_the_strict_default_and_still_audits() {
    let dir = workspace("agent-policy-default", None);
    {
        let mut mcp = Mcp::start(&dir);
        let allowed = mcp.call(&call("check", "{\"path\":\"main.ya\"}"));
        assert!(allowed.contains("\"result\""), "{allowed}");
        let refused = mcp.call(&call(
            "edit",
            "{\"patch\":{\"expected_tree_hash\":\"\",\"edits\":[]}}",
        ));
        assert!(refused.contains("policy denies tool `edit`"), "{refused}");
    }
    let (ok, stdout, stderr) = audit_verify(&dir);
    assert!(ok, "{stdout} {stderr}");
    assert!(stdout.contains("\"entries\": 2"), "{stdout}");
}

#[test]
fn the_policy_file_cannot_be_edited_by_the_session_it_governs() {
    let dir = workspace("agent-policy-self", Some("allow_verbs = [\"edit\"]\n"));
    let mut mcp = Mcp::start(&dir);
    let response = mcp.call(&call(
        "edit",
        "{\"patch\":{\"expected_tree_hash\":\"\",\"edits\":[{\"file\":\".yashy/policy.toml\",\"span\":{\"start\":0,\"end\":0},\"expected_hash\":\"\",\"replacement\":\"\",\"rationale\":\"x\"}]}}",
    ));
    assert!(
        response.contains("policy protects path `.yashy/policy.toml`"),
        "{response}"
    );
}

#[test]
fn policy_command_prints_the_effective_policy() {
    let dir = workspace(
        "agent-policy-print",
        Some("allow_verbs = [\"check\"]\nprotected_paths = [\"yashy.lock\"]\nmax_edits = 3\n"),
    );
    let output = Command::new(yashyc())
        .args(["agent", "policy", "--json", "--root"])
        .arg(&dir)
        .output()
        .expect("the CLI must run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"allow\""), "{stdout}");
    assert!(stdout.contains("\"check\""), "{stdout}");
    assert!(stdout.contains("\"max_edits\": 3"), "{stdout}");
}

#[test]
fn budgets_are_parsed_from_the_policy() {
    let policy = yashy_driver::agent::AgentPolicy::parse(
        "allow_verbs = [\"check\"]\nmax_wall_ms = 100\nmax_tokens = 50\nmax_output_bytes = 20\n",
    )
    .expect("budgets parse");
    assert_eq!(policy.max_wall_ms, Some(100));
    assert_eq!(policy.max_tokens, Some(50));
    assert_eq!(policy.max_output_bytes, Some(20));
    let json = policy.to_json().render();
    assert!(json.contains("\"max_wall_ms\": 100"), "{json}");
    assert!(json.contains("\"max_tokens\": 50"), "{json}");
    assert!(json.contains("\"max_output_bytes\": 20"), "{json}");
    let rejected = yashy_driver::agent::AgentPolicy::parse("max_tokens = 0\n");
    assert!(rejected.is_err(), "a non-positive cap is refused");
}

#[test]
fn an_output_budget_stops_the_next_call() {
    let dir = workspace("agent-policy-output-budget", Some("max_output_bytes = 1\n"));
    let mut mcp = Mcp::start(&dir);
    let first = mcp.call(&r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"check","arguments":{"path":"main.ya"}}}"#);
    assert!(first.contains("\"result\""), "{first}");
    let second = mcp.call(&r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"check","arguments":{"path":"main.ya"}}}"#);
    assert!(second.contains("-32002"), "{second}");
    assert!(
        second.contains("output budget of 1 bytes is exhausted"),
        "{second}"
    );
}
