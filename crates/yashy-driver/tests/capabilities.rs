//! Phase 3 Task 10: capability manifests and the dependency policy.

mod common;

use std::path::Path;

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
    let plain = yashy_manifest::Manifest::parse(
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let resolved = yashy_driver::policy::resolve(&plain, &[]);
    assert!(resolved.granted.entries().is_empty());
}
