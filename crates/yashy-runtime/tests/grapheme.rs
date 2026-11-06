//! Grapheme segmentation through the runtime: count, one cluster, and a
//! clamped range. The same linked code the generated IR calls.

use std::ffi::{c_char, CStr, CString};

use yashy_runtime::{YashyStr, yashy_str_grapheme_at, yashy_str_grapheme_count, yashy_str_grapheme_slice, yashy_str_release};

struct Static {
    _bytes: CString,
    value: YashyStr,
}

impl Static {
    fn new(text: &str) -> Self {
        let bytes = CString::new(text).expect("test strings must not contain a NUL byte");
        let len = bytes.as_bytes().len() as i64;
        let value = YashyStr {
            refcount: -1,
            len,
            cap: len,
            data: bytes.as_ptr() as *mut c_char,
        };
        Static { _bytes: bytes, value }
    }

    fn ptr(&self) -> *const YashyStr {
        &self.value
    }
}

unsafe fn read(string: *const YashyStr) -> String {
    assert!(!string.is_null(), "the runtime returned a null string");
    let data = unsafe { (*string).data };
    unsafe { CStr::from_ptr(data) }
        .to_str()
        .expect("valid UTF-8")
        .to_string()
}

unsafe fn release(string: *mut YashyStr) {
    unsafe { yashy_str_release(string) };
}

fn count(s: &str) -> i64 {
    let st = Static::new(s);
    unsafe { yashy_str_grapheme_count(st.ptr()) }
}

fn at(s: &str, index: i64) -> String {
    let st = Static::new(s);
    let cluster = unsafe { yashy_str_grapheme_at(st.ptr(), index) };
    let text = unsafe { read(cluster) };
    unsafe { release(cluster) };
    text
}

fn slice(s: &str, start: i64, end: i64) -> String {
    let st = Static::new(s);
    let sliced = unsafe { yashy_str_grapheme_slice(st.ptr(), start, end) };
    let text = unsafe { read(sliced) };
    unsafe { release(sliced) };
    text
}

#[test]
fn counts_decomposed_and_precomposed_accents_as_one_cluster() {
    assert_eq!(count("é"), 1); // precomposed: 2 bytes, 1 cluster
    assert_eq!(count("é"), 1); // e + combining acute
    assert_eq!(count("abc"), 3);
    assert_eq!(count(""), 0);
}

#[test]
fn counts_regional_indicator_flags_as_one_cluster() {
    assert_eq!(count("🇺🇸"), 1);
    assert_eq!(count("🇺🇸🇨🇦"), 2);
    assert_eq!(count("🇦"), 1);
}

#[test]
fn counts_zwj_emoji_sequences_as_one_cluster() {
    assert_eq!(count("👨‍👩‍👧‍👦"), 1);
    assert_eq!(count("🏳️‍🌈"), 1);
}

#[test]
fn counts_hangul_syllables_as_one_cluster() {
    assert_eq!(count("각"), 1);
    assert_eq!(count("가"), 1);
}

#[test]
fn counts_crlf_and_controls_as_boundaries() {
    assert_eq!(count("a\r\nb"), 3);
    assert_eq!(count("\r\n"), 1);
    assert_eq!(count("a\x07b"), 3);
}

#[test]
fn at_returns_the_whole_cluster() {
    assert_eq!(at("aé🇺🇸b", 2), "🇺🇸");
    assert_eq!(at("éx", 0), "é");
    // An index out of range is a runtime fault, not a null result: see
    // `yashy_str_grapheme_at` in runtime.c.
}

#[test]
fn slice_clamps_and_never_splits_a_cluster() {
    assert_eq!(slice("aé🇺🇸b", 1, 3), "é🇺🇸");
    assert_eq!(slice("aé🇺🇸b", 0, 99), "aé🇺🇸b");
    assert_eq!(slice("aé🇺🇸b", 2, 2), "");
    assert_eq!(slice("aé🇺🇸b", 3, 1), "");
    assert_eq!(slice("abc", -5, 2), "ab");
}
