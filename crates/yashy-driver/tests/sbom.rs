//! Phase 3 Task 5: SBOM output and package verification.

mod common;

use std::path::Path;
use std::process::Command;

fn package(dir: &Path) {
    common::write(
        dir,
        "yashy.toml",
        "[package]\nname = \"demo\"\nversion = \"1.0.0\"\nlicense = \"MIT\"\n",
    );
    common::write(dir, "main.ya", "fn main() { print(1); }\n");
}

fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(common::yashyc())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("yashyc must run")
}

#[test]
fn sbom_lists_the_package_and_the_compiler() {
    let dir = common::work_dir("sbom-cli");
    package(&dir);
    let output = run(&dir, &["sbom", "--format", "spdx-json"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("SPDX-2.3"), "{stdout}");
    assert!(stdout.contains("demo"), "{stdout}");
    assert!(stdout.contains("yashyc"), "{stdout}");
}

#[test]
fn verify_package_names_distinct_mismatch_codes() {
    let dir = common::work_dir("verify-package");
    let tarball = dir.join("demo.tar.gz");
    std::fs::write(&tarball, b"not a real tarball").expect("write tarball");
    let output = run(&dir, &["verify-package", "demo.tar.gz"]);
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("E1805"), "signature finding: {stdout}");
    assert!(stdout.contains("E1806"), "SBOM finding: {stdout}");
}
