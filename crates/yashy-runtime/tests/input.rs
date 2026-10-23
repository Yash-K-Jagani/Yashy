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

/// Parse `text` as an `i64` through the runtime.
fn parse_i64(text: &str) -> i64 {
    let (_bytes, value) = c(text);
    unsafe { yashy_runtime::yashy_parse_i64(&value) }
}

/// Parse `text` as an `f64` through the runtime.
fn parse_f64(text: &str) -> f64 {
    let (_bytes, value) = c(text);
    unsafe { yashy_runtime::yashy_parse_f64(&value) }
}

#[test]
fn parse_i64_reads_a_plain_integer() {
    assert_eq!(parse_i64("0"), 0);
    assert_eq!(parse_i64("42"), 42);
    assert_eq!(parse_i64("-7"), -7);
    assert_eq!(parse_i64("9223372036854775807"), i64::MAX);
    assert_eq!(parse_i64("-9223372036854775808"), i64::MIN);
}

#[test]
fn parse_i64_rejects_trailing_text() {
    // The whole string has to be the number. Reading the leading digits and
    // ignoring the rest is what `strtoll` does unbidden, and it turns a
    // malformed line into a plausible wrong number.
    assert_eq!(parse_i64("12x"), 0);
    assert_eq!(parse_i64("12 34"), 0);
    assert_eq!(parse_i64("1e3"), 0);
    assert_eq!(parse_i64("0x10"), 0);
    assert_eq!(parse_i64("+"), 0);
    assert_eq!(parse_i64("-"), 0);
    assert_eq!(parse_i64("."), 0);
}

#[test]
fn parse_i64_allows_surrounding_whitespace() {
    // A line read from a file or a terminal may arrive with spaces; those are
    // not part of the number and are not trailing garbage either.
    assert_eq!(parse_i64(" 7 "), 7);
    assert_eq!(parse_i64("\t-3\n"), -3);
    assert_eq!(parse_i64("\r\n"), 0, "whitespace alone is not a number");
}

#[test]
fn parse_i64_folds_overflow_into_the_same_answer() {
    // There is one failure answer, so a value too large to represent is not
    // distinguished from text that is not a number at all.
    assert_eq!(parse_i64("9223372036854775808"), 0, "one past i64::MAX");
    assert_eq!(parse_i64("99999999999999999999999"), 0);
}

#[test]
fn parse_f64_reads_a_plain_number_and_rejects_the_rest() {
    assert_eq!(parse_f64("0"), 0.0);
    assert_eq!(parse_f64("2.5"), 2.5);
    assert_eq!(parse_f64("-0.125"), -0.125);
    assert_eq!(parse_f64("1e3"), 1000.0);
    assert_eq!(parse_f64("  3.5  "), 3.5);
    assert_eq!(parse_f64("3.5x"), 0.0);
    assert_eq!(parse_f64("x"), 0.0);
    assert_eq!(parse_f64(""), 0.0);
}

#[test]
fn a_parsed_integer_round_trips_through_the_string_conversion() {
    // The two halves of "read a number, print a number" agree on the digits,
    // which is the pair a program using input actually calls.
    let printed = unsafe { yashy_runtime::yashy_str_from_i64(-1234) };
    assert_eq!(parse_i64("-1234"), -1234, "the source text parses");
    let text = unsafe {
        assert!(!printed.is_null(), "the conversion must produce a string");
        std::ffi::CStr::from_ptr((*printed).data)
            .to_str()
            .expect("the conversion must yield UTF-8")
    };
    assert_eq!(parse_i64(text), -1234, "the printed form parses back");
    unsafe { yashy_runtime::yashy_str_release(printed) };
}
