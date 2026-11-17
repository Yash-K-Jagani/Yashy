//! Phase 3 Task 11: in-toto/SLSA statements.

mod common;

use std::collections::BTreeMap;

const SEED: [u8; 32] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
    25, 26, 27, 28, 29, 30, 31, 32,
];

#[test]
fn a_statement_names_the_subject_and_materials() {
    let statement = yashy_manifest::for_artifact(
        "demo.tar.gz",
        b"artifact",
        "x86_64-pc-windows-msvc",
        "yashyc 1.0.0",
        BTreeMap::from([("yashy.toml".to_string(), yashy_manifest::sha256_hex(b"manifest"))]),
    );
    let text = String::from_utf8(statement.bytes()).unwrap();
    for needle in [
        "https://in-toto.io/Statement/v1",
        "https://slsa.dev/provenance/v1",
        &yashy_manifest::sha256_hex(b"artifact"),
        "yashy.toml",
    ] {
        assert!(text.contains(needle), "{needle} missing from {text}");
    }
}

#[test]
fn a_signed_statement_verifies_against_its_bytes() {
    let statement = yashy_manifest::for_artifact(
        "demo.tar.gz",
        b"artifact",
        "x86_64-pc-windows-msvc",
        "yashyc 1.0.0",
        BTreeMap::new(),
    );
    let signature = yashy_manifest::sign(&statement.bytes(), "release", &SEED).unwrap();
    let mut keys = yashy_manifest::Keyring::default();
    keys.insert("release", &yashy_manifest::public_key(&SEED).unwrap()).unwrap();
    assert!(yashy_manifest::verify(&statement.bytes(), &signature, &keys).is_ok());
    let mut changed = statement.bytes();
    changed[20] ^= 0xff;
    assert!(yashy_manifest::verify(&changed, &signature, &keys).is_err());
}
#[test]
fn a_statement_round_trips_through_json() {
    let statement = yashy_manifest::for_artifact(
        "demo.tar.gz",
        b"artifact",
        "x86_64-pc-windows-msvc",
        "yashyc 1.0.0",
        BTreeMap::from([("yashy.toml".to_string(), yashy_manifest::sha256_hex(b"manifest"))]),
    );
    let parsed = yashy_manifest::Statement::from_json_text(&statement.bytes()).unwrap();
    assert_eq!(parsed.subject, statement.subject);
    assert_eq!(parsed.predicate.materials, statement.predicate.materials);
    assert_eq!(parsed.predicate.target, "x86_64-pc-windows-msvc");
}
