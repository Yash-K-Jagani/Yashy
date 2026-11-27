//! Signed advisory feeds: a crafted advisory refuses a version inside its
//! range, a tampered feed fails signature verification, and `--json` carries
//! the same facts as the human form.

mod common;

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const SEED: [u8; 32] = [9; 32];

fn fixture(name: &str) -> PathBuf {
    let dir = common::work_dir(&format!("advisory-cli-{name}"));
    let advisory = yashy_driver::advisory::Advisory {
        id: "YASHY-2027-0003".to_string(),
        package: "compress".to_string(),
        range: "<2.0.1".to_string(),
        severity: "high".to_string(),
        fixed: "2.0.1".to_string(),
        title: "decoder overflow".to_string(),
    };
    let feed =
        yashy_driver::advisory::sign_feed(vec![advisory], "release", &SEED).expect("a signed feed");
    fs::write(dir.join("feed.json"), feed).unwrap();
    let key = yashy_manifest::signature::public_key(&SEED).expect("a public key");
    let hex: String = key.iter().map(|byte| format!("{byte:02x}")).collect();
    fs::write(dir.join("keys.txt"), format!("release {hex}\n")).unwrap();
    dir
}

fn run(dir: &PathBuf, args: &[&str]) -> (bool, String, String) {
    let output = Command::new(common::yashyc())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("the CLI must run");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn an_affected_version_is_refused_with_the_advisory_and_fix() {
    let dir = fixture("affected");
    let (ok, _, stderr) = run(
        &dir,
        &[
            "advisory",
            "--check",
            "feed.json",
            "--keyring",
            "keys.txt",
            "--package",
            "compress",
            "--version",
            "2.0.0",
        ],
    );
    assert!(!ok, "an affected version fails the check: {stderr}");
    assert!(stderr.contains("YASHY-2027-0003"), "{stderr}");
    assert!(stderr.contains("<2.0.1"), "{stderr}");
    assert!(stderr.contains("fixed in 2.0.1"), "{stderr}");
}

#[test]
fn a_fixed_version_passes_and_json_carries_the_facts() {
    let dir = fixture("fixed");
    let (ok, stdout, stderr) = run(
        &dir,
        &[
            "advisory",
            "--check",
            "feed.json",
            "--keyring",
            "keys.txt",
            "--package",
            "compress",
            "--version",
            "2.0.1",
            "--json",
        ],
    );
    assert!(ok, "{stderr}");
    assert!(stdout.contains("\"affected\": []"), "{stdout}");

    let (ok, stdout, stderr) = run(
        &dir,
        &[
            "advisory",
            "--check",
            "feed.json",
            "--keyring",
            "keys.txt",
            "--package",
            "compress",
            "--version",
            "2.0.0",
            "--json",
        ],
    );
    assert!(!ok, "{stderr}");
    assert!(stdout.contains("YASHY-2027-0003"), "{stdout}");
    assert!(stdout.contains("\"fixed\": \"2.0.1\""), "{stdout}");
}

#[test]
fn a_tampered_feed_is_refused() {
    let dir = fixture("tampered");
    let text = fs::read_to_string(dir.join("feed.json")).unwrap();
    fs::write(
        dir.join("feed.json"),
        text.replace("decoder overflow", "decoder is fine"),
    )
    .unwrap();
    let (ok, _, stderr) = run(
        &dir,
        &["advisory", "--check", "feed.json", "--keyring", "keys.txt"],
    );
    assert!(!ok, "a forged feed fails: {stderr}");
    assert!(stderr.contains("signature"), "{stderr}");
}

#[test]
fn listing_a_feed_shows_every_advisory() {
    let dir = fixture("listing");
    let (ok, stdout, stderr) = run(&dir, &["advisory", "--check", "feed.json", "--json"]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("YASHY-2027-0003"), "{stdout}");
    assert!(stdout.contains("\"advisories\""), "{stdout}");
}
