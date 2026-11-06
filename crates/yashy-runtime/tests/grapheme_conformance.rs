//! The Unicode grapheme conformance suite, vendored from
//! `GraphemeBreakTest.txt` (Unicode 18.0.0, pinned 2026-11-06).
//!
//! Every line names a string as code points separated by `×` (no break) and
//! `÷` (break). The expected cluster count is the number of `÷` plus one.
//! The scanner implements GB3–GB9b, GB11, and GB12/GB13; GB9c (rule `[9.3]`,
//! Indic consonant joining via `LinkingConsonant`) is not implemented,
//! because that property is derived data the scanner's tables do not carry.
//! Cases that need it are excluded by their rule citation in the file's
//! comments, and the exclusion count is pinned: the suite fails if it moves,
//! so a change in the rule set is a visible decision.
//!
//! Run with: cargo test -p yashy-runtime --test grapheme_conformance

use std::ffi::c_char;

use yashy_runtime::{YashyStr, yashy_str_grapheme_count};

struct Static {
    /// Held only to own the bytes `value.data` points at.
    _bytes: Vec<u8>,
    value: YashyStr,
}

impl Static {
    fn new(text: &str) -> Self {
        let bytes = text.as_bytes().to_vec();
        let len = bytes.len() as i64;
        let value = YashyStr {
            refcount: -1,
            len,
            cap: len,
            data: bytes.as_ptr() as *mut c_char,
        };
        Static { _bytes: bytes, value }
    }
}

#[test]
fn every_conformance_case_matches_the_scanner() {
    let data = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("data")
            .join("GraphemeBreakTest.txt"),
    )
    .expect("the vendored conformance file must be readable");

    let mut total = 0u64;
    let mut failed = 0u64;
    let mut excluded_gb9c = 0u64;
    let mut first_failures: Vec<String> = Vec::new();
    for line in data.lines() {
        let (entry, comment) = match line.split_once('#') {
            Some((e, c)) => (e.trim(), c),
            None => (line.trim(), ""),
        };
        if entry.is_empty() {
            continue;
        }
        // Rule `[9.3]` is GB9c (Indic conjunct joining), which the scanner
        // does not implement; see the module header. Excluding by the file's
        // own rule citation, not by whether the case happens to pass.
        if comment.contains("[9.3]") {
            excluded_gb9c += 1;
            continue;
        }
        let mut text = String::new();
        // The file's leading and trailing `÷` mark the string's start and
        // end, not breaks between code points; only `÷` between two code
        // points counts. The cluster count is internal breaks plus one.
        let mut breaks = 0u64;
        let mut seen_cp = false;
        let tokens: Vec<&str> = entry.split_whitespace().collect();
        for (i, token) in tokens.iter().enumerate() {
            match *token {
                "÷" => {
                    let at_edge = i == 0 || i == tokens.len() - 1;
                    if !at_edge {
                        breaks += 1;
                    }
                }
                "×" => {}
                hex => {
                    seen_cp = true;
                    let cp = u32::from_str_radix(hex, 16).expect("hex code point");
                    let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
                    text.push(ch);
                }
            }
        }
        if !seen_cp {
            continue;
        }
        if text.is_empty() {
            continue;
        }
        total += 1;
        let expected = breaks as i64 + 1;
        let st = Static::new(&text);
        let got = unsafe { yashy_str_grapheme_count(&st.value) };
        if got != expected {
            failed += 1;
            if first_failures.len() < 10 {
                first_failures.push(format!("{entry:?} expected {expected}, got {got}"));
            }
        }
    }
    assert!(total > 500, "the suite must hold hundreds of cases, got {total}");
    assert_eq!(
        excluded_gb9c, 26,
        "the GB9c exclusion moved: the Unicode file changed or the scanner's \
         rule set did; update the header and the pinned count together"
    );
    assert_eq!(
        failed, 0,
        "{failed}/{total} cases differ:\n{}",
        first_failures.join("\n")
    );
}
