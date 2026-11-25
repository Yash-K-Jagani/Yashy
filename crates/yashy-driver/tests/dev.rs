//! `yashyc dev` emits one JSON generation per successful rebuild and keeps
//! restarting after an edit to a source file.

mod common;

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

fn work_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yashy-dev-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("the dev test directory must be creatable");
    dir
}

#[test]
fn dev_rebuilds_after_an_edit() {
    let dir = work_dir();
    let source = dir.join("main.ya");
    fs::write(&source, "fn main() { print(\"v1\"); }\n").expect("the source must be writable");

    let mut child = Command::new(common::yashyc())
        .arg("dev")
        .arg(&source)
        .arg("--json")
        .arg("--interval")
        .arg("50")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("yashyc dev must start");

    thread::sleep(Duration::from_secs(6));
    fs::write(&source, "fn main() { print(\"v2\"); }\n").expect("the edit must be writable");
    thread::sleep(Duration::from_secs(6));

    let _ = child.kill();
    let _ = child.wait();
    let output = child.stdout.take().expect("stdout was piped");
    let mut text = String::new();
    use std::io::Read;
    let mut reader = output;
    let _ = reader.read_to_string(&mut text);

    assert!(text.contains("\"generation\":1"), "no first generation: {text}");
    assert!(text.contains("\"generation\":2"), "no generation after the edit: {text}");
    assert!(text.contains("\"status\":\"restarted\""), "no restart event: {text}");
}
