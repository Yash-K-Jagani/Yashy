//! `yashyc bench --measure` produces a real baseline row file.

mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

fn work_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("yashy-bench-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("the bench work directory must be creatable");
    dir
}

#[test]
fn measure_writes_a_baseline_that_can_be_checked() {
    let dir = work_dir();
    fs::write(dir.join("bench.ya"), "fn main() {}\n").expect("the benchmark source must be writable");
    let output = Command::new(common::yashyc())
        .args([
            "bench",
            "--measure",
            "bench.ya",
            "--iterations",
            "5",
            "--warmup",
            "1",
            "--key",
            "noop",
            "--save-baseline",
            "baseline.bench",
            "--json",
        ])
        .current_dir(&dir)
        .output()
        .expect("the yashyc CLI must run");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let json = String::from_utf8_lossy(&output.stdout);
    assert!(json.contains("\"median_ms\""), "unexpected output: {json}");
    assert!(json.contains("\"p95_ms\""), "unexpected output: {json}");

    let baseline = fs::read_to_string(dir.join("baseline.bench")).expect("the baseline must exist");
    assert!(baseline.contains("noop.median_ms="), "unexpected baseline: {baseline}");
    assert!(baseline.contains("noop.p95_ms="), "unexpected baseline: {baseline}");

    let checked = Command::new(common::yashyc())
        .args(["bench", "--check-baseline", "baseline.bench", "baseline.bench"])
        .current_dir(&dir)
        .output()
        .expect("the yashyc CLI must run");
    assert!(checked.status.success(), "{}", String::from_utf8_lossy(&checked.stderr));
    let _ = fs::remove_dir_all(Path::new(&dir));
}
