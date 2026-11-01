//! Exercises the runtime's input functions through the same declarations the
//! generated IR calls, so a signature that drifts from the header fails here
//! rather than in a linked Yashy program.
//!
//! `yashy_read_line` is deliberately absent: it reads the process's standard
//! input, which a test binary shares with the test harness, so the behaviour a
//! test would be pinning (a line, then end of input) belongs in the end-to-end
//! suite where stdin can be piped and captured. What is here is the whole-string
//! parse contract, which is pure.

use std::ffi::{c_char, CString};

use yashy_runtime::YashyStr;

/// A NUL-terminated C string plus the static `YashyStr` header over its bytes.
///
/// Both halves come back so the caller keeps the bytes alive for as long as it
/// uses the value: `let (_bytes, value) = c("7");`.
fn c(s: &str) -> (CString, YashyStr) {
    let bytes = CString::new(s).expect("test strings must not contain a NUL byte");
    let len = bytes.as_bytes().len() as i64;
    let value = YashyStr {
        refcount: -1,
        len,
        cap: len,
        data: bytes.as_ptr() as *mut c_char,
    };
    (bytes, value)
}

/// Parse `text` as an `i64` through the runtime. `None` is the runtime's "this
/// text is not a number" answer; there is no sentinel value.
fn parse_i64(text: &str) -> Option<i64> {
    let (_bytes, value) = c(text);
    let mut out: i64 = 0;
    let ok = unsafe { yashy_runtime::yashy_parse_i64(&value, &mut out) };
    (ok == 1).then_some(out)
}

/// Parse `text` as an `f64` through the runtime.
fn parse_f64(text: &str) -> Option<f64> {
    let (_bytes, value) = c(text);
    let mut out: f64 = 0.0;
    let ok = unsafe { yashy_runtime::yashy_parse_f64(&value, &mut out) };
    (ok == 1).then_some(out)
}

#[test]
fn parse_i64_reads_a_plain_integer() {
    assert_eq!(parse_i64("0"), Some(0));
    assert_eq!(parse_i64("42"), Some(42));
    assert_eq!(parse_i64("-7"), Some(-7));
    assert_eq!(parse_i64("9223372036854775807"), Some(i64::MAX));
    assert_eq!(parse_i64("-9223372036854775808"), Some(i64::MIN));
}

#[test]
fn parse_i64_rejects_trailing_text() {
    // The whole string has to be the number. Reading the leading digits and
    // ignoring the rest is what `strtoll` does unbidden, and it turns a
    // malformed line into a plausible wrong number.
    assert_eq!(parse_i64("12x"), None);
    assert_eq!(parse_i64("12 34"), None);
    assert_eq!(parse_i64("1e3"), None);
    assert_eq!(parse_i64("0x10"), None);
    assert_eq!(parse_i64("+"), None);
    assert_eq!(parse_i64("-"), None);
    assert_eq!(parse_i64("."), None);
}

#[test]
fn parse_i64_allows_surrounding_whitespace() {
    // A line read from a file or a terminal may arrive with spaces; those are
    // not part of the number and are not trailing garbage either.
    assert_eq!(parse_i64(" 7 "), Some(7));
    assert_eq!(parse_i64("\t-3\n"), Some(-3));
    assert_eq!(parse_i64("\r\n"), None, "whitespace alone is not a number");
}

#[test]
fn parse_i64_reports_overflow_as_no_number() {
    // There is one failure answer, so a value too large to represent is not
    // distinguished from text that is not a number at all.
    assert_eq!(parse_i64("9223372036854775808"), None, "one past i64::MAX");
    assert_eq!(parse_i64("99999999999999999999999"), None);
}

#[test]
fn parse_f64_reads_a_plain_number_and_rejects_the_rest() {
    assert_eq!(parse_f64("0"), Some(0.0));
    assert_eq!(parse_f64("2.5"), Some(2.5));
    assert_eq!(parse_f64("-0.125"), Some(-0.125));
    assert_eq!(parse_f64("1e3"), Some(1000.0));
    assert_eq!(parse_f64("  3.5  "), Some(3.5));
    assert_eq!(parse_f64("3.5x"), None);
    assert_eq!(parse_f64("x"), None);
    assert_eq!(parse_f64(""), None);
}

#[test]
fn a_parsed_integer_round_trips_through_the_string_conversion() {
    // The two halves of "read a number, print a number" agree on the digits,
    // which is the pair a program using input actually calls.
    let printed = unsafe { yashy_runtime::yashy_str_from_i64(-1234) };
    assert_eq!(parse_i64("-1234"), Some(-1234), "the source text parses");
    let text = unsafe {
        assert!(!printed.is_null(), "the conversion must produce a string");
        std::ffi::CStr::from_ptr((*printed).data)
            .to_str()
            .expect("the conversion must yield UTF-8")
    };
    assert_eq!(parse_i64(text), Some(-1234), "the printed form parses back");
    unsafe { yashy_runtime::yashy_str_release(printed) };
}
