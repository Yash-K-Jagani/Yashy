//! The crypto package's native dependencies are pinned by exact bytes.
//!
//! `packages/crypto/NATIVE.lock` is the lockfile for the C sources the driver
//! compiles into a program. There is no Cargo or Yashy manifest row for them
//! yet, so this test is the gate: a source edit without a lockfile update is
//! a failure, the same way a stale `yashy.lock` is for a Yashy package.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
}

fn bytes(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()))
}

fn files_for(name: &str) -> Vec<&'static str> {
    match name {
        "monocypher" => vec![
            "native/monocypher.c",
            "native/monocypher.h",
            "native/optional/monocypher-ed25519.c",
            "native/optional/monocypher-ed25519.h",
        ],
        "miniz" => vec!["native/miniz.c"],
        "tiny_sha3" => vec!["native/sha3/sha3.c", "native/sha3/sha3.h", "native/sha3/LICENSE"],
        "zstd" => vec![
            "native/zstd/zstd.c",
            "native/zstd/zstd.h",
            "native/zstd/zstd_errors.h",
        ],
        other => panic!("{other} has no file list in the pin test"),
    }
}

#[test]
fn native_lock_matches_the_compiled_sources() {
    let lock = root().join("packages/crypto/NATIVE.lock");
    let text = std::fs::read_to_string(&lock)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", lock.display()));
    let mut rows = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (id, pinned) = line
            .split_once('=')
            .unwrap_or_else(|| panic!("malformed pin row: {line}"));
        let (_version, expected) = pinned
            .split_once(':')
            .unwrap_or_else(|| panic!("malformed pin row: {line}"));
        let mut all = Vec::new();
        for relative in files_for(id) {
            all.extend(bytes(&root().join("packages/crypto").join(relative)));
        }
        let actual = yashy_manifest::lock::sha256_hex(&all);
        assert_eq!(actual, expected, "{id}: VENDOR.md and NATIVE.lock disagree");
        rows += 1;
    }
    assert_eq!(rows, 4, "all four native dependencies must be pinned");
}

#[test]
fn every_pinned_source_is_referenced_by_a_link_clause() {
    let module = root().join("crates/yashy-stdlib/yashy/crypto.ya");
    let text = std::fs::read_to_string(&module).expect("the crypto module must be readable");
    for needle in [
        "native/yashy_crypto_shim.c",
        "native/monocypher.c",
        "native/optional/monocypher-ed25519.c",
        "native/miniz.c",
        "native/sha3/sha3.c",
        "native/zstd/zstd.c",
    ] {
        assert!(text.contains(needle), "no link clause for {needle}");
    }
}
