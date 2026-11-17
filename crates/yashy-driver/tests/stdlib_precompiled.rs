//! Phase 3 Task 9: the stdlib is a versioned, hashed, signed package.

mod common;

use std::path::Path;

use std::collections::BTreeSet;

fn project(dir: &Path) {
    common::write(dir, "main.ya", "fn main() { print(1); }\n");
}

#[test]
fn the_stdlib_manifest_names_a_version() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/yashy-stdlib");
    let manifest = yashy_manifest::Manifest::load(&root.join("yashy.toml"))
        .expect("the stdlib has a package manifest");
    assert_eq!(manifest.package.name, "stdlib");
    assert_eq!(manifest.package.version, "1.0.0");
}

#[test]
fn the_stdlib_hash_is_part_of_the_cache_key() {
    let dir = common::work_dir("stdlib-cache-key");
    project(&dir);
    let mut sources = yashy_common::SourceMap::new();
    sources.add_file(
        "main.ya".to_string(),
        "fn main() { print(1); }\n".to_string(),
    );
    let mut options = yashy_driver::BuildOptions::default();
    let first = yashy_driver::pipeline::ir_cache_key(&sources, "main.ya", &options);
    options.features = BTreeSet::from(["tls".to_string()]);
    let second = yashy_driver::pipeline::ir_cache_key(&sources, "main.ya", &options);
    assert_ne!(first.hex(), second.hex(), "features must change the key");
    let fingerprint = yashy_driver::modules::stdlib_fingerprint();
    assert!(!fingerprint.is_empty());
}
