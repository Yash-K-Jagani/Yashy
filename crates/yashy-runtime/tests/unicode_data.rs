//! The Unicode data header is generated from the vendored UCD files and must
//! stay current. This test verifies the files are present and pinned to
//! Unicode 18.0.0, and that the generated header declares non-trivial tables
//! with a recorded hash for every input. The generator
//! (`scripts/gen_unicode_data.py`) is a development-time tool; builds and tests
//! need no network and no Python.
//!
//! Run with: cargo test -p yashy-runtime --test unicode_data

use std::ffi::{c_char, c_uint, CStr, CString};
use std::path::Path;

use yashy_runtime::{
    yashy_str_case_fold, yashy_str_display_width, yashy_str_nfc, yashy_str_nfd, yashy_str_nfkc,
    yashy_str_nfkd, yashy_str_release, yashy_ucd_case_fold, yashy_ucd_width, YashyStr,
};

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

/// Holds the UTF-8 bytes a `YashyStr` points at.
struct Static {
    _bytes: CString,
    value: YashyStr,
}

impl Static {
    fn new(text: &str) -> Self {
        let bytes = CString::new(text).expect("test strings contain no NUL");
        let len = bytes.as_bytes().len() as i64;
        let value = YashyStr {
            refcount: -1,
            len,
            cap: len,
            data: bytes.as_ptr() as *mut c_char,
        };
        Static {
            _bytes: bytes,
            value,
        }
    }

    fn ptr(&self) -> *const YashyStr {
        &self.value
    }
}

fn run(form: unsafe extern "C" fn(*const YashyStr) -> *mut YashyStr, text: &str) -> String {
    let input = Static::new(text);
    let output = unsafe { form(input.ptr()) };
    assert!(!output.is_null(), "normalization returned null");
    let result = unsafe { CStr::from_ptr((*output).data) }
        .to_str()
        .expect("normalization returns valid UTF-8")
        .to_string();
    unsafe { yashy_str_release(output) };
    result
}

fn nfc(text: &str) -> String {
    run(yashy_str_nfc, text)
}
fn nfd(text: &str) -> String {
    run(yashy_str_nfd, text)
}
fn nfkc(text: &str) -> String {
    run(yashy_str_nfkc, text)
}
fn nfkd(text: &str) -> String {
    run(yashy_str_nfkd, text)
}
fn fold(text: &str) -> String {
    run(yashy_str_case_fold, text)
}

/// Parses one column of `NormalizationTest.txt` (`0044 0307`) as a string.
fn column(text: &str) -> String {
    let code_points: Vec<u32> = text
        .split_whitespace()
        .map(|piece| u32::from_str_radix(piece, 16).expect("a hex code point"))
        .collect();
    code_points
        .into_iter()
        .map(|cp| char::from_u32(cp).expect("a scalar value"))
        .collect()
}

#[test]
fn normalization_conforms_to_the_unicode_test_file() {
    // UAX #15 conformance: for every row, each of the four forms applied to
    // any column must equal the column the spec says it maps to.
    let text = std::fs::read_to_string(root().join("data/unicode/NormalizationTest.txt"))
        .expect("NormalizationTest.txt");
    let mut rows = 0;
    let mut failures = Vec::new();
    for line in text.lines() {
        let line = line.splitn(2, '#').next().unwrap_or("").trim();
        if line.is_empty() || line.starts_with('@') {
            continue;
        }
        let parts: Vec<&str> = line.split(';').map(str::trim).collect();
        if parts.len() < 5 {
            continue;
        }
        let c1 = column(parts[0]);
        let c2 = column(parts[1]);
        let c3 = column(parts[2]);
        let c4 = column(parts[3]);
        let c5 = column(parts[4]);
        // NormalizationTest.txt columns: source; NFC; NFD; NFKC; NFKD. The
        // conformance invariants are the ones the file's header states.
        let checks = [
            ("NFC", nfc(&c1), &c2),
            ("NFC", nfc(&c2), &c2),
            ("NFC", nfc(&c3), &c2),
            ("NFC", nfc(&c4), &c4),
            ("NFC", nfc(&c5), &c4),
            ("NFD", nfd(&c1), &c3),
            ("NFD", nfd(&c2), &c3),
            ("NFD", nfd(&c3), &c3),
            ("NFD", nfd(&c4), &c5),
            ("NFD", nfd(&c5), &c5),
            ("NFKC", nfkc(&c1), &c4),
            ("NFKC", nfkc(&c2), &c4),
            ("NFKC", nfkc(&c3), &c4),
            ("NFKC", nfkc(&c4), &c4),
            ("NFKC", nfkc(&c5), &c4),
            ("NFKD", nfkd(&c1), &c5),
            ("NFKD", nfkd(&c2), &c5),
            ("NFKD", nfkd(&c3), &c5),
            ("NFKD", nfkd(&c4), &c5),
            ("NFKD", nfkd(&c5), &c5),
        ];
        for (form, got, want) in checks {
            if got != *want {
                failures.push(format!(
                    "{form}({}) = {} but the file says {}",
                    parts[0],
                    escape(&got),
                    escape(want)
                ));
            }
        }
        rows += 1;
    }
    assert!(rows > 1000, "the conformance file should hold many rows");
    assert!(
        failures.is_empty(),
        "{} of {} checks differ:\n{}",
        failures.len(),
        rows * 20,
        failures
            .iter()
            .take(12)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

fn escape(text: &str) -> String {
    text.chars()
        .map(|ch| format!("U+{:04X}", ch as u32))
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn canonical_forms_fold_the_named_cases() {
    // é has one canonical decomposition and composes back.
    assert_eq!(nfd("é"), "e\u{301}");
    assert_eq!(nfc("e\u{301}"), "é");
    // Full compatibility folds a ligature and a full-width letter.
    assert_eq!(nfkd("\u{FB01}"), "fi");
    assert_eq!(nfkc("\u{FF21}"), "A");
    // Full case folding is multi-character and not Turkish-specific.
    assert_eq!(fold("Straße"), "strasse");
    assert_eq!(fold("\u{130}"), "i\u{307}");
    // Turkish dotless i folds to itself.
    assert_eq!(fold("\u{131}"), "\u{131}");
}

#[test]
fn display_width_counts_wide_characters_as_two() {
    let wide = Static::new("漢字");
    assert_eq!(unsafe { yashy_str_display_width(wide.ptr()) }, 4);
    let ascii = Static::new("abc");
    assert_eq!(unsafe { yashy_str_display_width(ascii.ptr()) }, 3);
    let mixed = Static::new("a漢");
    assert_eq!(unsafe { yashy_str_display_width(mixed.ptr()) }, 3);
    let combining = Static::new("e\u{301}");
    assert_eq!(unsafe { yashy_str_display_width(combining.ptr()) }, 1);
}
