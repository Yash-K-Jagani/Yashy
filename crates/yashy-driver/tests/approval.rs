//! Approval tokens bound to the tree, the patch, the verb and an expiry.
//!
//! The human runner mints a token; the MCP session may spend it once. A token
//! for another tree, another patch or another verb is refused before the tool
//! runs, and a replay has no file left to spend.

mod common;

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

fn yashyc() -> &'static str {
    common::yashyc()
}

fn workspace(name: &str) -> PathBuf {
    let dir = common::work_dir(name);
    common::write(
        &dir,
        "main.ya",
        "fn double(int x) -> int { return x * 2; }\nfn main() { print(double(3)); }\n",
    );
    common::write(
        &dir,
        ".yashy/policy.toml",
        "allow_verbs = [\"edit\"]\nrequire_approval = [\"edit\"]\n",
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

fn field(json: &str, name: &str) -> String {
    json.lines()
        .find_map(|line| {
            let line = line.trim();
            let key = format!("\"{name}\": ");
            line.strip_prefix(&key)
                .map(|value| value.trim_end_matches(',').trim_matches('"').to_string())
        })
        .unwrap_or_default()
}

/// A real patch with current hashes, plus its `double` -> `twice` edit.
fn patch_for(dir: &Path) -> (String, String) {
    let source = std::fs::read_to_string(dir.join("main.ya")).unwrap();
    let tree = yashy_driver::edit::tree_hash(dir).unwrap();
    let hash = yashy_driver::edit::file_hash(&dir.join("main.ya")).unwrap();
    let start = source.find("x * 2").unwrap();
    let patch = format!(
        "{{\"expected_tree_hash\":\"{tree}\",\"edits\":[{{\"file\":\"main.ya\",\
         \"span\":{{\"start\":{start},\"end\":{end}}},\"expected_hash\":\"{hash}\",\
         \"replacement\":\"double(x)\",\"rationale\":\"call the named function\"}}]}}",
        end = start + 5
    );
    (tree, patch)
}

fn mint(dir: &Path, tree: &str, ttl: &str, nonce: &str) -> String {
    let patch_file = dir.join("patch.json");
    let output = Command::new(yashyc())
        .args([
            "agent",
            "approve",
            "--json",
            "--root",
            dir.to_str().unwrap(),
            "--patch",
            "patch.json",
            "--tree",
            tree,
            "--verbs",
            "edit",
            "--ttl",
            ttl,
            "--nonce",
            nonce,
        ])
        .output()
        .expect("the CLI must run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let token = field(&stdout, "token");
    assert_eq!(token.len(), 64, "{stdout}");
    token
}

fn edit_call(patch: &str, token: &str) -> String {
    // The patch argument is raw JSON, so build the envelope without format
    // braces that could be confused with the patch's own.
    let mut request = String::from(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"edit\",\"arguments\":{\"patch\":",
    );
    request.push_str(patch.trim_end_matches('}'));
    request.push_str("},\"approval\":\"");
    request.push_str(token);
    request.push_str("\"}}}");
    request
}

#[test]
fn an_approval_is_spent_once() {
    let dir = workspace("approval-once");
    let (tree, patch) = patch_for(&dir);
    std::fs::write(dir.join("patch.json"), &patch).unwrap();
    let token = mint(&dir, &tree, "300", "nonce-once");
    let mut mcp = Mcp::start(&dir);
    let first = mcp.call(&edit_call(&patch, &token));
    assert!(first.contains("\"applied\""), "{first}");
    let source = std::fs::read_to_string(dir.join("main.ya")).unwrap();
    assert!(source.contains("return double(x)"), "{source}");

    let second = mcp.call(&edit_call(&patch, &token));
    assert!(second.contains("-32002"), "{second}");
    assert!(
        second.contains("approval token is unknown or already used"),
        "{second}"
    );
    assert!(second.contains("E1807"), "{second}");
}

#[test]
fn a_token_for_another_tree_is_refused() {
    let dir = workspace("approval-tree");
    let (tree, patch) = patch_for(&dir);
    std::fs::write(dir.join("patch.json"), &patch).unwrap();
    let token = mint(&dir, "some-other-tree", "300", "nonce-tree");
    let mut mcp = Mcp::start(&dir);
    let response = mcp.call(&edit_call(&patch, &token));
    assert!(
        response.contains("approval token is bound to another tree"),
        "{response}"
    );
    assert!(response.contains("E1807"), "{response}");
    // A refused call does not burn the token.
    assert!(dir.join(".yashy/approvals").read_dir().unwrap().count() > 0);
}

#[test]
fn an_expired_token_is_refused() {
    let dir = workspace("approval-expired");
    let (tree, patch) = patch_for(&dir);
    std::fs::write(dir.join("patch.json"), &patch).unwrap();
    let token = mint(&dir, &tree, "0", "nonce-expired");
    let mut mcp = Mcp::start(&dir);
    let response = mcp.call(&edit_call(&patch, &token));
    assert!(
        response.contains("approval token has expired"),
        "{response}"
    );
}

#[test]
fn a_missing_token_is_refused() {
    let dir = workspace("approval-missing");
    let (_, patch) = patch_for(&dir);
    let mut mcp = Mcp::start(&dir);
    let response = mcp.call(&edit_call(&patch, ""));
    assert!(
        response.contains("approval token is required"),
        "{response}"
    );
    assert!(response.contains("-32002"), "{response}");
}

#[test]
fn plan_names_files_symbols_and_capabilities() {
    let dir = workspace("approval-plan");
    common::write(
        &dir,
        "yashy.toml",
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[capabilities]\nnet = [\"*\"]\n",
    );
    let (_, patch) = patch_for(&dir);
    std::fs::write(dir.join("patch.json"), &patch).unwrap();
    let output = Command::new(yashyc())
        .args([
            "agent",
            "plan",
            "--json",
            "--root",
            dir.to_str().unwrap(),
            "--patch",
            "patch.json",
        ])
        .output()
        .expect("the CLI must run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"files\""), "{stdout}");
    assert!(stdout.contains("main.ya"), "{stdout}");
    assert!(stdout.contains("\"double\""), "{stdout}");
    assert!(stdout.contains("\"net\""), "{stdout}");
    assert!(stdout.contains("\"verbs\""), "{stdout}");
}
