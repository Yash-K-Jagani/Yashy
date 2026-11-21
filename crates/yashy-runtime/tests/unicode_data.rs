//! The Unicode data header is generated from the vendored UCD files and must
//! stay current. This test verifies the files are present and pinned to
//! Unicode 18.0.0, and that the generated header declares non-trivial tables
//! with a recorded hash for every input. The generator
//! (`scripts/gen_unicode_data.py`) is a development-time tool; builds and tests
//! need no network and no Python.
//!
//! Run with: cargo test -p yashy-runtime --test unicode_data

use std::ffi::c_uint;
use std::path::Path;

use yashy_runtime::{yashy_ucd_case_fold, yashy_ucd_width};

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

const SOURCES: &[&str] = &[
    "UnicodeData.txt",
    "CaseFolding.txt",
    "CompositionExclusions.txt",
    "EastAsianWidth.txt",
    "NormalizationTest.txt",
    "ReadMe.txt",
];

#[test]
fn vendored_unicode_data_is_present() {
    let dir = root().join("data").join("unicode");
    for name in SOURCES {
        let path = dir.join(name);
        let bytes =
            std::fs::read(&path).unwrap_or_else(|e| panic!("{} is missing: {e}", path.display()));
        assert!(bytes.len() > 100, "{} is unexpectedly small", name);
        let text = std::fs::read_to_string(&path).expect("UCD files are ASCII");
        // Every file except the two big tables carries a `18.0.0` marker in
        // its own header; the big tables are identified by the README pin and
        // the recorded hashes in the generated header.
        if *name != "UnicodeData.txt" && *name != "EastAsianWidth.txt" {
            assert!(
                text.contains("18.0.0"),
                "{} does not identify Unicode 18.0.0",
                name
            );
        }
    }
    let readme = std::fs::read_to_string(dir.join("ReadMe.txt")).expect("README");
    assert!(
        readme.contains("18.0.0"),
        "the README must state the pinned release"
    );
}

#[test]
fn generated_header_records_the_vendored_versions() {
    let header = std::fs::read_to_string(root().join("src").join("unicode_data.h"))
        .expect("unicode_data.h is generated and checked in");
    assert!(
        header.contains("Unicode 18.0.0"),
        "the header names its release"
    );
    assert!(
        header.contains("Do not edit by hand"),
        "the header marks itself generated"
    );
    // Every recorded sha256 must match the vendored file's actual bytes. This
    // is what makes the pin auditable: a changed data file without regeneration
    // fails here rather than silently changing the generated tables.
    let dir = root().join("data").join("unicode");
    for name in SOURCES {
        let bytes = std::fs::read(dir.join(name)).expect("vendored file");
        let expected = format!("{name} sha256:{}", yashy_manifest::lock::sha256_hex(&bytes));
        assert!(
            header.contains(&expected),
            "the header's recorded hash for {name} is stale or missing"
        );
    }
    // Both tables must be non-trivial: an empty generated table would pass a
    // structural check but make the vendoring pointless.
    let mut folds = 0;
    let mut wide = 0;
    for line in header.lines() {
        if let Some(rest) = line.strip_prefix("static const int UCD_CASE_FOLD_count = ") {
            folds = rest
                .trim_end_matches(';')
                .parse()
                .expect("an integer count");
        }
        if let Some(rest) = line.strip_prefix("static const int UCD_EAW_WIDE_count = ") {
            wide = rest
                .trim_end_matches(';')
                .parse()
                .expect("an integer count");
        }
    }
    assert!(folds > 1000, "case folding table is too small: {folds}");
    assert!(wide > 100, "wide-range table is too small: {wide}");
}

#[test]
fn case_fold_lookups_match_the_source_data() {
    // Parse the F and C mappings from CaseFolding.txt and check the generated
    // C table answers each one. F wins over C, matching the generator.
    use std::collections::BTreeMap;
    let text = std::fs::read_to_string(root().join("data/unicode/CaseFolding.txt"))
        .expect("CaseFolding.txt");
    let mut expected: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for line in text.lines() {
        let line = line.splitn(2, '#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split(';').map(str::trim).collect();
        if parts.len() < 3 {
            continue;
        }
        let from = u32::from_str_radix(parts[0], 16).expect("a code point");
        let targets: Vec<u32> = parts[2]
            .split_whitespace()
            .map(|x| u32::from_str_radix(x, 16).expect("a code point"))
            .collect();
        match parts[1] {
            "F" => {
                expected.insert(from, targets);
            }
            "C" => {
                expected.entry(from).or_insert(targets);
            }
            _ => {}
        }
    }
    assert!(expected.len() > 1000, "expected mapping set is too small");

    let mut checked = 0;
    for (from, targets) in &expected {
        let mut out = [0u32; 3];
        let count = unsafe { yashy_ucd_case_fold(*from as c_uint, out.as_mut_ptr()) };
        assert_eq!(
            count as usize,
            targets.len(),
            "fold count for U+{from:X} differs from the source"
        );
        for (index, target) in targets.iter().enumerate() {
            assert_eq!(
                out[index], *target,
                "fold mapping for U+{from:X} differs from the source"
            );
        }
        checked += 1;
    }
    assert!(checked > 1000, "too few folds checked: {checked}");

    // A code point with no fold answers zero.
    let mut out = [0u32; 3];
    assert_eq!(unsafe { yashy_ucd_case_fold(0x4E00, out.as_mut_ptr()) }, 0);
}

#[test]
fn width_lookups_match_the_source_data() {
    // The two named cases the plan calls out: CJK is wide, ASCII is narrow.
    assert_eq!(
        unsafe { yashy_ucd_width(0x4E00) },
        2,
        "CJK ideograph is wide"
    );
    assert_eq!(unsafe { yashy_ucd_width('A' as u32) }, 1, "ASCII is narrow");
    assert_eq!(
        unsafe { yashy_ucd_width(0x00) },
        0,
        "a control has width zero"
    );
    assert_eq!(unsafe { yashy_ucd_width(0xFF21) }, 2, "fullwidth A is wide");
}
