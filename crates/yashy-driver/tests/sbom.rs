//! Phase 3 Task 5: SBOM output and package verification.

mod common;

use std::fs;
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
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
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

#[test]
fn verify_package_checks_the_index_entry_when_a_registry_is_named() {
    let dir = common::work_dir("verify-package-index");
    let pkg = dir.join("pkg");
    let registry = dir.join("registry");
    package(&pkg);
    let entry = yashy_driver::registry::publish(&pkg, &registry).expect("the package must publish");
    let tarball = yashy_driver::registry::tarball_path(&registry, &entry);
    let tarball_name = tarball.to_str().expect("a UTF-8 tarball path");

    let good = run(
        &dir,
        &["verify-package", "--registry", "registry", tarball_name],
    );
    let good_json = String::from_utf8_lossy(&good.stdout);
    assert!(
        !good_json.contains("E1809"),
        "the published hash must match an index entry: {good_json}"
    );

    // A registry that names a different hash for the package is the mismatch
    // this check exists to catch. The signature and SBOM findings may differ;
    // only E1809 is asserted here.
    fs::write(
        registry.join("index.json"),
        format!(
            "{{\"packages\":[{{\"name\":\"demo\",\"version\":\"1.0.0\",\"hash\":\"sha256:{}\",\"description\":\"\"}}]}}",
            "0".repeat(64)
        ),
    )
    .expect("the index must be writable");
    let bad = run(
        &dir,
        &["verify-package", "--registry", "registry", tarball_name],
    );
    assert_eq!(bad.status.code(), Some(1), "a mismatch must fail");
    let bad_json = String::from_utf8_lossy(&bad.stdout);
    assert!(
        bad_json.contains("E1809"),
        "the mismatch must be named: {bad_json}"
    );
}
