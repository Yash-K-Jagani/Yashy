//! Phase 3 Task 1: ranges and pinned resolution.

mod common;

use std::path::Path;

use yashy_manifest::{Manifest, Version, VersionReq};

#[test]
fn a_manifest_accepts_ranges_but_not_nonsense() {
    let good = "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n\
                [dependencies.demo]\nversion = \"^1.2\"\n";
    let manifest = Manifest::parse(good).expect("a caret range is a dependency");
    assert!(matches!(
        manifest.dependencies.get("demo"),
        Some(yashy_manifest::Dependency::Registry { version }) if version == "^1.2"
    ));
    let bad = good.replace("^1.2", "maybe");
    assert!(
        Manifest::parse(&bad).is_err(),
        "a bad range is a diagnostic"
    );
}

#[test]
fn pinned_versions_are_semver() {
    assert!(Version::parse("1.0.0-alpha.1+build.2").is_some());
    assert!(Version::parse("1.2").is_none());
}

#[test]
fn selection_prefers_the_highest_satisfying_version() {
    let requirement = VersionReq::parse("^1.2").expect("a valid range");
    let candidates = ["1.2.0", "1.9.0", "2.0.0", "1.10.0"]
        .iter()
        .filter_map(|text| Version::parse(text))
        .filter(|version| requirement.matches(version))
        .max();
    assert_eq!(
        candidates.map(|version| version.to_string()).as_deref(),
        Some("1.10.0")
    );
}

#[test]
fn the_phase_plan_is_present() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/superpowers/plans");
    assert!(root
        .join("2026-11-15-phase-03-packages-distribution.md")
        .is_file());
}
