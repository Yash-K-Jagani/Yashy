//! Phase 3 Task 3: features are selected, sorted and cached.

mod common;

use std::path::Path;
use std::process::Command;

use std::collections::BTreeSet;

fn project(dir: &Path) {
    common::write(
        dir,
        "yashy.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n\
         [features]\ndefault = [\"std\"]\nstd = []\ntls = []\n\n\
         [dependencies.net]\nversion = \"1.0.0\"\noptional = true\n",
    );
    common::write(dir, "main.ya", "fn main() { print(1); }\n");
}

#[test]
fn manifests_carry_features_and_optional_dependencies() {
    let manifest = yashy_manifest::Manifest::parse(
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n\
         [features]\ndefault = [\"std\"]\nstd = []\n\n\
         [dependencies.net]\nversion = \"1.0.0\"\noptional = true\n",
    )
    .expect("feature manifest parses");
    assert!(manifest.features.contains_key("default"));
    assert!(manifest.optional_dependencies.contains("net"));
}

#[test]
fn selected_features_are_sorted_and_cacheable() {
    let dir = common::work_dir("features-selection");
    project(&dir);
    let manifest = yashy_manifest::Manifest::load(&dir.join("yashy.toml")).expect("load");
    let features =
        yashy_driver::features::select(&manifest, &["tls".to_string(), "std".to_string()], false)
            .expect("select");
    assert_eq!(
        features.enabled.iter().cloned().collect::<Vec<_>>(),
        vec!["std", "tls"]
    );
    let mut options = yashy_driver::BuildOptions::default();
    options.features = features.enabled.clone();
    options.ir_cache = false;
    assert_eq!(options.features, features.enabled);
}

#[test]
fn feature_keys_are_visible_in_cfg() {
    let features = BTreeSet::from(["tls".to_string()]);
    let set = yashy_driver::cfg::CfgSet::for_target("x86_64-pc-windows-msvc", false)
        .with_features(&features);
    assert!(set.is_active("feature:tls"));
    assert!(!set.is_active("feature:missing"));
}
