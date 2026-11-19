//! Phase 3 Task 10: capability manifests and the dependency policy.

mod common;

use std::process::Command;

use yashy_manifest::CapabilitySet;

fn manifest() -> yashy_manifest::Manifest {
    yashy_manifest::Manifest::parse(
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n\
         [capabilities]\nfs.read = [\"data/*\"]\nnet = []\n",
    )
    .expect("capability manifest")
}

#[test]
fn a_declared_capability_is_a_grant() {
    let set: CapabilitySet = manifest().capabilities.unwrap();
    assert!(set.has("fs.read"));
    assert!(set.allows_path("fs.read", "data/x"));
    assert!(!set.allows_path("fs.read", "data/../secret"));
}

#[test]
fn a_policy_is_a_ceiling_not_a_source_of_rights() {
    let resolved = yashy_driver::policy::resolve(&manifest(), &["net".to_string()]);
    assert!(resolved.granted.has("net"));
    assert!(!resolved.granted.has("fs.read"));
    assert_eq!(resolved.denied, vec!["fs.read".to_string()]);
}

#[test]
fn add_allow_accepts_known_names_only() {
    assert!(yashy_driver::policy::check_request("net", &[]).is_ok());
    assert!(yashy_driver::policy::check_request("net", &["fs.read".to_string()]).is_err());
    assert!(yashy_driver::policy::check_request("telepathy", &[]).is_err());
}

#[test]
fn an_undeclared_package_has_no_capabilities() {
    let plain = yashy_manifest::Manifest::parse("[package]\nname = \"app\"\nversion = \"0.1.0\"\n")
        .unwrap();
    let resolved = yashy_driver::policy::resolve(&plain, &[]);
    assert!(resolved.granted.entries().is_empty());
}
#[test]
fn build_refuses_a_call_without_its_capability() {
    let dir = common::work_dir("capability-build");
    common::write(&dir, "yashy.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[capabilities]\nfs.read = [\"data/*\"]\n");
    common::write(&dir, "main.ya", "fn main() { let x = getenv(\"HOME\"); }\n");
    let output = Command::new(common::yashyc())
        .args(["check", "main.ya"])
        .current_dir(&dir)
        .output()
        .expect("yashyc must run");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(all.contains("E1808"), "the capability must be named: {all}");
}
#[test]
fn audit_policy_parses_all_fields() {
    let manifest = yashy_manifest::Manifest::parse(
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n\
         [audit]\nallow-capabilities = [\"fs.read\"]\ndeny-capabilities = [\"net\"]\n\
         max-transitive = 8\ndeny-licenses = [\"GPL-3.0\"]\n\
         deny-registries = [\"untrusted.example\"]\nrequire-provenance = true\n",
    )
    .expect("full audit policy parses");
    let policy = manifest.audit.unwrap();
    assert_eq!(policy.allow_capabilities, vec!["fs.read"]);
    assert_eq!(policy.deny_capabilities, vec!["net"]);
    assert_eq!(policy.max_transitive, 8);
    assert!(policy.require_provenance);
}

#[test]
fn an_unknown_capability_in_the_policy_is_a_diagnostic() {
    let manifest = "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n\
                    [audit]\ndeny-capabilities = [\"telepathy\"]\n";
    assert!(yashy_manifest::Manifest::parse(manifest).is_err());
}

#[test]
fn audit_capabilities_prints_the_transitive_union() {
    // The root grants `fs.read`; its path dependency grants `net`. The audit
    // must name both, because both packages run in the same process. Printing
    // only the root's set would hide the dependency's reach until the linker
    // or a sandbox refused something.
    let dir = common::work_dir("capability-union");
    common::write(
        &dir,
        "yashy.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nlicense = \"MIT\"\n\n\
         [capabilities]\nfs.read = [\"data/*\"]\n\n\
         [dependencies.lib]\npath = \"lib\"\n",
    );
    common::write(&dir, "main.ya", "fn main() { print(\"hi\"); }\n");
    common::write(
        &dir,
        "lib/yashy.toml",
        "[package]\nname = \"lib\"\nversion = \"1.0.0\"\nlicense = \"MIT\"\n\n\
         [capabilities]\nnet = []\n",
    );
    common::write(&dir, "lib/lib.ya", "pub fn ping() -> int { 1 }\n");
    let output = Command::new(common::yashyc())
        .args(["audit", "--capabilities"])
        .current_dir(&dir)
        .output()
        .expect("yashyc must run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains("capabilities: fs.read, net"),
        "both packages must appear: {stdout}{stderr}"
    );
    assert!(
        output.status.success(),
        "the fixture is a clean audit: {stdout}{stderr}"
    );
}
