//! `yashyc openapi` emits a document from typed routes, `yashyc client` emits
//! a client from that document, and `--check` classifies a breaking change.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn work_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yashy-client-gen-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("the client-gen work directory must be creatable");
    dir
}

fn cli(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(common::yashyc())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("the yashyc CLI must run")
}

const ROUTES: &str = "// @route GET|/items||Item||token\n// @route POST|/items|name:string,age:int|Item|Problem|token\n";

#[test]
fn openapi_and_client_generation_and_check() {
    let dir = work_dir();
    let source = dir.join("routes.ya");
    let old = dir.join("old.json");
    let generated = dir.join("client.ya");
    fs::write(&source, format!("{ROUTES}\nfn main() {{}}\n")).expect("the source must be writable");

    let emitted = cli(
        &dir,
        &["openapi", "routes.ya", "--out", "old.json", "--title", "Items"],
    );
    assert!(emitted.status.success(), "{}", String::from_utf8_lossy(&emitted.stderr));
    let document = fs::read_to_string(&old).expect("the document must exist");
    assert!(document.contains("\"openapi\":\"3.1.0\""));
    assert!(document.contains("\"/items\""));

    let client = cli(&dir, &["client", "--from", "old.json", "--out", "client.ya"]);
    assert!(client.status.success(), "{}", String::from_utf8_lossy(&client.stderr));
    let source_text = fs::read_to_string(&generated).expect("the client must exist");
    assert!(source_text.contains("pub fn get__items()"), "unexpected client: {source_text}");
    assert!(source_text.contains("string name"), "unexpected client: {source_text}");
    assert!(source_text.contains("int age"), "unexpected client: {source_text}");

    let checked = cli(&dir, &["check", "client.ya"]);
    assert!(checked.status.success(), "the generated client must check: {}", String::from_utf8_lossy(&checked.stderr));

    // Removing a route breaks callers generated from the old document.
    fs::write(&source, "// @route GET|/items||Item||token\nfn main() {}\n").expect("the edit must be writable");
    let breaking = cli(&dir, &["openapi", "routes.ya", "--check", "old.json"]);
    assert_eq!(breaking.status.code(), Some(1), "a removed route must be breaking");
    assert_eq!(String::from_utf8_lossy(&breaking.stdout).trim(), "breaking");

    // Adding one is additive.
    fs::write(
        &source,
        format!("{ROUTES}// @route GET|/health||Status||\nfn main() {{}}\n"),
    )
    .expect("the edit must be writable");
    let additive = cli(&dir, &["openapi", "routes.ya", "--check", "old.json"]);
    assert!(additive.status.success(), "{}", String::from_utf8_lossy(&additive.stderr));
    assert_eq!(String::from_utf8_lossy(&additive.stdout).trim(), "additive");
}
