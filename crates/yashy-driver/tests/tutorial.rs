//! The tutorial is executable: every ```yashy block in `docs/tutorial` is
//! compiled and run, and its stdout must equal the `// =>` lines inside the
//! block.
//!
//! The convention is deliberately small:
//!
//! - Only blocks tagged `yashy` are considered, and every one of them must be a
//!   whole program with a `fn main` -- a fragment that cannot be compiled is a
//!   fragment that cannot rot safely.
//! - A line `// => text` inside the block is one line of expected stdout, in
//!   order. Expected output always ends with a newline, because every example
//!   prints whole lines.
//! - The directive is itself a comment, so the block the reader copies into a
//!   file is the block the test compiles.
//!
//! A chapter with no block, or a block with no expected output, still passes:
//! the point is that what *is* claimed is true, not that every chapter prints.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(60);

fn yashy() -> &'static str {
    common::yashy()
}

fn tutorial_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/tutorial")
}

fn work_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yashy-tutorial-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("the tutorial work directory must be creatable");
    dir
}

struct Example {
    /// The line the fence opens on, for a readable failure.
    line: usize,
    code: String,
    expected: String,
}

/// Extract the runnable examples from one chapter.
fn examples(markdown: &str) -> Vec<Example> {
    let mut examples = Vec::new();
    let mut code = String::new();
    let mut expected = String::new();
    let mut start = 0usize;
    let mut inside = false;

    for (index, line) in markdown.lines().enumerate() {
        if !inside {
            if line.trim_start().starts_with("```yashy") {
                inside = true;
                start = index + 1;
                code.clear();
                expected.clear();
            }
            continue;
        }
        if line.trim() == "```" {
            inside = false;
            examples.push(Example {
                line: start,
                code: std::mem::take(&mut code),
                expected: std::mem::take(&mut expected),
            });
            continue;
        }
        code.push_str(line);
        code.push('\n');
        if let Some(directive) = line.trim_start().strip_prefix("// =>") {
            // One space after the arrow is the separator, not output.
            let text = directive.strip_prefix(' ').unwrap_or(directive);
            expected.push_str(text);
            expected.push('\n');
        }
    }
    examples
}

fn chapters() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(tutorial_dir())
        .expect("the tutorial directory must be readable")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("md"))
        .collect();
    files.sort();
    files
}

fn build(source: &Path, exe: &Path) -> Output {
    Command::new(yashy())
        .arg("build")
        .arg(source)
        .arg("-o")
        .arg(exe)
        .output()
        .expect("the yashy CLI must be runnable")
}

fn wait(mut child: std::process::Child, exe: &Path) -> Output {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if started.elapsed() >= TIMEOUT {
                    let _ = child.kill();
                    panic!("{} did not terminate within {TIMEOUT:?}", exe.display());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("the tutorial executable must be waitable: {error}"),
        }
    }
    child
        .wait_with_output()
        .expect("the finished executable must be waitable")
}

/// Start a freshly built executable, or answer why it could not start.
///
/// Application Control on some Windows machines refuses a brand-new binary
/// with "os error 4551". A blocked *path* tends to stay blocked, so a retry
/// here is short: the caller rebuilds under a new name instead. Any failure
/// that is not that block is a real one and is reported.
fn spawn(exe: &Path) -> Result<std::process::Child, String> {
    common::launch_retrying(exe, |path| {
        Command::new(path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    })
    .map_err(|error| error.to_string())
}

/// Build `source` and run it.
///
/// A launch blocked by Application Control is retried by rebuilding to a new
/// name: the block is a property of the written binary, and a new file is a
/// new question. A binary that stays blocked fails the test -- an
/// environmental block is never counted as a pass.
fn build_and_run(source: &Path, dir: &Path, tag: &str, label: &str) -> Output {
    let mut last = String::new();
    for attempt in 0..6 {
        let exe = dir.join(format!(
            "{tag}_run{attempt}{}",
            std::env::consts::EXE_SUFFIX
        ));
        let built = build(source, &exe);
        assert!(
            built.status.success(),
            "{label} does not compile:\n{}",
            String::from_utf8_lossy(&built.stderr)
        );
        match spawn(&exe) {
            Ok(child) => return wait(child, &exe),
            Err(reason) => last = reason,
        }
    }
    panic!("{label} was blocked on every launch attempt (os error 4551): {last}");
}

#[test]
fn every_tutorial_example_builds_runs_and_matches_its_output() {
    let chapters = chapters();
    assert!(
        chapters.len() >= 11,
        "the tutorial has {} chapters; the plan asks for eleven",
        chapters.len()
    );

    let dir = work_dir();
    let mut total = 0usize;
    for chapter in chapters {
        let markdown = fs::read_to_string(&chapter)
            .unwrap_or_else(|error| panic!("{} must be readable: {error}", chapter.display()));
        for example in examples(&markdown) {
            let name = format!("example_{total}");
            let source = dir.join(format!("{name}.ya"));
            fs::write(&source, &example.code).expect("the example must be writable");

            let label = format!("{}:{}", chapter.display(), example.line);
            let ran = build_and_run(&source, &dir, &name, &label);
            assert!(
                ran.status.success(),
                "{}:{} exits with {}",
                chapter.display(),
                example.line,
                ran.status
            );
            let stdout = String::from_utf8_lossy(&ran.stdout);
            assert_eq!(
                stdout.replace("\r\n", "\n"),
                example.expected,
                "{}:{} prints something the tutorial does not claim",
                chapter.display(),
                example.line
            );
            total += 1;
        }
    }
    assert!(total >= 11, "only {total} examples ran");
}
