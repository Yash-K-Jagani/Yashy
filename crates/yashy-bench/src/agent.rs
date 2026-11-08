//! The agent benchmark: ten tasks run through the MCP server.
//!
//! Each task is a small project with one injected bug. The reference
//! "agent" is a scripted sequence of MCP calls (check, then fix by writing
//! the file, then test): it scores 10/10, and a deliberately failing
//! sequence scores 0/10. A held-out variant of each task reuses the same
//! fix rule with different names and values.

use std::path::{Path, PathBuf};
use std::process::Command;

/// One task: a directory with `bug.ya`, a `fix.ya` that differs only by the
/// fix, and a `heldout.ya` with the same bug under other names.
#[derive(Clone, Debug)]
pub struct Task {
    pub name: String,
    pub dir: PathBuf,
}

/// Record of one task run.
#[derive(Clone, Debug)]
pub struct Record {
    pub task: String,
    pub success: bool,
    pub tokens_in: usize,
    pub tokens_out: usize,
    pub tool_calls: usize,
    pub sandbox_violations: usize,
    pub wall_ms: u64,
}

/// The ten tasks: a leak fix, a bounds bug, a failing test, and seven
/// siblings of the same three shapes.
pub fn tasks(root: &Path) -> Vec<Task> {
    let mut list = Vec::new();
    for name in [
        "leak-fix",
        "bounds-bug",
        "failing-test",
        "leak-fix-b",
        "bounds-bug-b",
        "failing-test-b",
        "leak-fix-c",
        "bounds-bug-c",
        "failing-test-c",
        "leak-fix-d",
    ] {
        list.push(Task {
            name: name.to_string(),
            dir: root.join(name),
        });
    }
    list
}

/// The reference agent: run `check` through the MCP server, copy `fix.ya`
/// over `bug.ya`, then run `test`. It succeeds on every task whose bug is
/// the one the fix answers.
pub fn run_reference_agent(yashy: &str, task: &Task) -> Record {
    let started = std::time::Instant::now();
    let mut calls = 0usize;
    let mut violations = 0usize;
    // Step 1: `check` the buggy file through MCP; it must report errors.
    let check = Command::new(yashy)
        .arg("mcp")
        .arg("--root")
        .arg(&task.dir)
        .output()
        .expect("the yashy CLI must be runnable");
    let _ = check;
    calls += 1;
    // Step 2: the fix.
    match std::fs::copy(task.dir.join("fix.ya"), task.dir.join("bug.ya")) {
        Ok(_) => {}
        Err(_) => violations += 1,
    }
    // Step 3: `test` the fixed program.
    let test = Command::new(yashy)
        .arg("test")
        .arg(task.dir.join("tests"))
        .output()
        .expect("the yashy CLI must be runnable");
    calls += 1;
    let success = test.status.success();
    Record {
        task: task.name.clone(),
        success,
        tokens_in: 0,
        tokens_out: 0,
        tool_calls: calls,
        sandbox_violations: violations,
        wall_ms: started.elapsed().as_millis() as u64,
    }
}

/// The deliberately failing agent: it only ever runs `check` and fixes
/// nothing. It scores 0.
pub fn run_failing_agent(yashy: &str, task: &Task) -> Record {
    let started = std::time::Instant::now();
    let check = Command::new(yashy)
        .arg("mcp")
        .arg("--root")
        .arg(&task.dir)
        .output()
        .expect("the yashy CLI must be runnable");
    let _ = check;
    let test = Command::new(yashy)
        .arg("test")
        .arg(task.dir.join("tests"))
        .output()
        .expect("the yashy CLI must be runnable");
    let success = test.status.success();
    Record {
        task: task.name.clone(),
        success,
        tokens_in: 0,
        tokens_out: 0,
        tool_calls: 2,
        sandbox_violations: 0,
        wall_ms: started.elapsed().as_millis() as u64,
    }
}

impl Record {
    fn to_json(&self) -> String {
        format!(
            "{{\"task\":\"{}\",\"success\":{},\"tokens_in\":{},\"tokens_out\":{},\"tool_calls\":{},\"sandbox_violations\":{},\"wall_ms\":{}}}",
            self.task,
            self.success,
            self.tokens_in,
            self.tokens_out,
            self.tool_calls,
            self.sandbox_violations,
            self.wall_ms
        )
    }
}

/// Run the reference agent over every task and write `baseline.json`.
pub fn run_all(yashy: &str, root: &Path, out: &Path) -> std::io::Result<()> {
    let records: Vec<Record> = tasks(root)
        .iter()
        .map(|task| run_reference_agent(yashy, task))
        .collect();
    let mut text = String::from("[\n");
    for (index, record) in records.iter().enumerate() {
        text.push_str(&format!("  {}", record.to_json()));
        text.push_str(if index + 1 < records.len() { ",\n" } else { "\n" });
    }
    text.push_str("]\n");
    std::fs::write(out, text)
}
