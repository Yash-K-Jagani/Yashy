//! Deterministic mode: a concurrent program produces identical stdout and a
//! normalized event trace across two runs with the same fixed seed.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn work_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yashy-deterministic-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("the deterministic work directory must be creatable");
    dir
}

fn normalise(text: &str) -> String {
    text.lines()
        .map(|line| {
            line.split_whitespace()
                .map(|token| {
                    let head = token.split(',').next().unwrap_or(token);
                    if head.len() > 4 && head.chars().all(|c| c.is_ascii_hexdigit()) {
                        format!("ADDR{}", &token[head.len()..])
                    } else {
                        token.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn deterministic_runs_have_equal_output_and_trace() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = work_dir();
    let binary = dir.join(format!("program{}", std::env::consts::EXE_SUFFIX));
    let build = Command::new(common::yashyc())
        .arg("build")
        .arg(root.join("crates/yashy-driver/tests/programs/atomic_test.ya"))
        .arg("-o")
        .arg(&binary)
        .env("YASHY_PATH", root.join("crates/yashy-stdlib/yashy"))
        .current_dir(&root)
        .output()
        .expect("the compiler must run");
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));

    let mut outputs = Vec::new();
    let mut traces = Vec::new();
    for index in 1..=2 {
        let trace = dir.join(format!("events{index}.txt"));
        let run = Command::new(&binary)
            .env("YASHY_DETERMINISTIC", "1")
            .env("YASHY_DETERMINISTIC_SEED", "1700000000000")
            .env("YASHY_TRACE_FILE", &trace)
            .output()
            .expect("the program must run");
        assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
        outputs.push(String::from_utf8_lossy(&run.stdout).to_string());
        traces.push(normalise(&fs::read_to_string(&trace).expect("the trace must exist")));
    }
    assert_eq!(outputs[0], outputs[1], "stdout diverged");
    assert_eq!(traces[0], traces[1], "the normalized event trace diverged");
}
