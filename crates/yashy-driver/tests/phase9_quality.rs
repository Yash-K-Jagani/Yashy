//! Phase 9 quality-stack harnesses.
//!
//! Each program runs through the CLI and must print its marker and exit 0.
//! Programs that allocate assert a zero live-allocation leak report separately.

mod common;

use std::path::Path;
use std::process::Command;

fn path_separator() -> &'static str {
    if cfg!(windows) { ";" } else { ":" }
}

fn run(root: &Path, source: &str, name: &str, expected: &str, packages: &[&str]) {
    let mut paths = root.join("crates/yashy-stdlib/yashy").display().to_string();
    for package in packages {
        paths.push_str(path_separator());
        paths.push_str(&root.join("packages").join(package).join("src").display().to_string());
    }
    let dir = common::work_dir(name);
    let binary = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    let build = Command::new(common::yashyc())
        .arg("build")
        .arg(root.join(source))
        .arg("-o")
        .arg(&binary)
        .env("YASHY_PATH", &paths)
        .current_dir(root)
        .output()
        .expect("the compiler must run");
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let output = Command::new(&binary).output().expect("the program must run");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), expected);
}

#[test]
fn property_shrinking_program_is_checked() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    run(
        &root,
        "packages/test/tests/properties.ya",
        "p9-properties",
        "p9-properties-ok",
        &["test"],
    );
}

#[test]
fn fuzz_campaign_program_is_checked() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    run(
        &root,
        "packages/fuzz/tests/campaign_test.ya",
        "p9-fuzz",
        "fuzz-campaign-ok",
        &["fuzz"],
    );
}

#[test]
fn value_assertions_program_is_checked() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    run(
        &root,
        "packages/test/tests/values.ya",
        "p9-assertions",
        "p9-assertions-ok",
        &["test"],
    );
}
