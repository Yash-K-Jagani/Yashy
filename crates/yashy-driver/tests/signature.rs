//! Phase 3 Task 4: signed registry artifacts.

mod common;

use std::path::Path;

const SEED: [u8; 32] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
    27, 28, 29, 30, 31, 32,
];

fn package(dir: &Path) {
    common::write(
        dir,
        "yashy.toml",
        "[package]\nname = \"demo\"\nversion = \"1.0.0\"\nlicense = \"MIT\"\n",
    );
    common::write(dir, "main.ya", "fn main() { print(1); }\n");
}

#[test]
fn a_signature_verifies_its_exact_bytes() {
    let signature = yashy_manifest::sign(b"tarball", "release", &SEED).unwrap();
    let mut keys = yashy_manifest::Keyring::default();
    keys.insert("release", &yashy_manifest::public_key(&SEED).unwrap())
        .unwrap();
    assert!(yashy_manifest::verify(b"tarball", &signature, &keys).is_ok());
    assert!(yashy_manifest::verify(b"tarbalX", &signature, &keys).is_err());
}

#[test]
fn publishing_writes_a_tarball_signature_and_sbom() {
    let dir = common::work_dir("signed-publish");
    let package_dir = dir.join("package");
    let registry_dir = dir.join("registry");
    std::fs::create_dir_all(&package_dir).unwrap();
    std::fs::create_dir_all(&registry_dir).unwrap();
    package(&package_dir);
    let (entry, files) =
        yashy_driver::registry::publish_signed(&package_dir, &registry_dir, "release", &SEED)
            .expect("signed publish");
    assert_eq!(entry.name, "demo");
    assert_eq!(files.len(), 4);
    for file in &files {
        assert!(file.is_file(), "{} must exist", file.display());
    }
    let bytes = std::fs::read(&files[0]).unwrap();
    let sig = yashy_driver::registry::tarball_signature(
        &yashy_driver::registry::RegistrySource::Local(registry_dir.clone()),
        &entry,
    )
    .expect("read detached signature");
    let mut keys = yashy_manifest::Keyring::default();
    keys.insert("release", &yashy_manifest::public_key(&SEED).unwrap())
        .unwrap();
    assert!(yashy_driver::registry::verify_tarball(&bytes, &sig, &keys).is_ok());
    let mut flipped = bytes.clone();
    flipped[0] ^= 0xff;
    assert!(yashy_driver::registry::verify_tarball(&flipped, &sig, &keys).is_err());
    assert!(yashy_driver::registry::verify_index(
        &yashy_driver::registry::RegistrySource::Local(registry_dir),
        &keys
    )
    .is_ok());
}

#[test]
fn an_spdx_sbom_names_the_package() {
    let dir = common::work_dir("signed-sbom");
    package(&dir);
    let entry = yashy_manifest::sbom::from_directory(&dir).expect("SBOM");
    assert_eq!(entry.name, "demo");
    assert_eq!(entry.version, "1.0.0");
    let json = yashy_manifest::spdx_json(&[entry]).render();
    assert!(json.contains("SPDX-2.3"));
    assert!(json.contains("demo"));
}
#[test]
fn yanking_a_version_requires_a_reason_and_is_reversible_from_the_backup() {
    let dir = common::work_dir("registry-yank");
    let package = dir.join("package");
    let registry = dir.join("registry");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::create_dir_all(&registry).unwrap();
    common::write(&package, "yashy.toml", "[package]\nname = \"demo\"\nversion = \"1.0.0\"\n");
    common::write(&package, "main.ya", "fn main() {}\n");
    yashy_driver::registry::publish(&package, &registry).unwrap();
    assert!(yashy_driver::registry::yank(&registry, "demo", "1.0.0", "").is_err());
    let files = yashy_driver::registry::yank(&registry, "demo", "1.0.0", "broken build").unwrap();
    assert!(files.iter().any(|path| path.to_string_lossy().ends_with("index.json.bak")));
    let entries = yashy_driver::registry::read_index(&registry).unwrap();
    let entry = entries.iter().find(|entry| entry.name == "demo").unwrap();
    assert!(entry.yanked);
    assert_eq!(entry.yanked_reason.as_deref(), Some("broken build"));
    let source = yashy_driver::registry::RegistrySource::Local(registry);
    let req = yashy_manifest::VersionReq::parse("^1.0").unwrap();
    assert!(yashy_driver::registry::select_source(&source, "demo", &req).unwrap().is_none());
}
