//! Files, the environment, and substrings: the M6 Task 10 runtime additions.
//!
//! These call the same linked C the generated IR calls. The contract worth
//! pinning here is the one the language depends on: a fallible read answers a
//! tag and writes its payload through `out`, so "missing" is never spelled as an
//! empty value. A file body is a byte range, so a NUL in the middle survives a
//! round trip.

use std::ffi::{c_char, CString};
use std::path::PathBuf;

use yashy_runtime::{
    yashy_getenv, yashy_read_file, yashy_str_from_i64, yashy_str_release, yashy_str_slice,
    yashy_write_file, YashyStr,
};

unsafe fn bytes(text: *mut yashy_runtime::YashyStr) -> Vec<u8> {
    if text.is_null() {
        return Vec::new();
    }
    std::slice::from_raw_parts((*text).data as *const u8, (*text).len as usize).to_vec()
}

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("yashy-runtime-{}-{name}", std::process::id()))
}

fn c_path(path: &PathBuf) -> CString {
    CString::new(path.to_string_lossy().as_bytes()).expect("a temp path has no NUL")
}

#[test]
fn a_slice_clamps_its_bounds_instead_of_faulting() {
    unsafe {
        let text = yashy_str_from_i64(12345); // "12345"
        let middle = yashy_str_slice(text, 1, 3);
        assert_eq!(bytes(middle), b"23");
        let past_end = yashy_str_slice(text, 3, 99);
        assert_eq!(bytes(past_end), b"45");
        let inverted = yashy_str_slice(text, 4, 2);
        assert_eq!(bytes(inverted), b"", "an inverted range is empty");
        let negative = yashy_str_slice(text, -5, 2);
        assert_eq!(bytes(negative), b"12", "a negative start is zero");
        let null = yashy_str_slice(std::ptr::null(), 0, 4);
        assert_eq!(bytes(null), b"");
        for value in [middle, past_end, inverted, negative, null, text] {
            yashy_str_release(value);
        }
    }
}

#[test]
fn a_file_round_trips_every_byte_including_a_nul() {
    unsafe {
        let path = temp_path("roundtrip");
        let path_c = c_path(&path);
        let body: &[u8] = b"line one\nline\0two\n";

        let wrote = yashy_write_file(
            path_c.as_ptr(),
            body.as_ptr() as *const c_char,
            body.len() as i64,
        );
        assert_eq!(wrote, 0, "a writable temp directory must accept the file");

        let mut read: *mut YashyStr = std::ptr::null_mut();
        assert_eq!(yashy_read_file(path_c.as_ptr(), &mut read), 0);
        assert_eq!(
            bytes(read),
            body,
            "the length is authoritative: a NUL does not end the file"
        );
        yashy_str_release(read);
        let _ = std::fs::remove_file(&path);
    }
}

#[test]
fn a_missing_file_and_a_null_path_read_as_a_failure_tag() {
    unsafe {
        let path = temp_path("no-such-directory").join("file.txt");
        let _ = std::fs::remove_file(&path);
        let path_c = c_path(&path);
        let mut read: *mut YashyStr = std::ptr::null_mut();
        assert_ne!(
            yashy_read_file(path_c.as_ptr(), &mut read),
            0,
            "a missing file is an error, not an empty file"
        );
        assert!(read.is_null(), "a failed read writes no payload");

        let mut null_read: *mut YashyStr = std::ptr::null_mut();
        assert_ne!(yashy_read_file(std::ptr::null(), &mut null_read), 0);
        assert!(null_read.is_null());

        assert_ne!(
            yashy_write_file(std::ptr::null(), std::ptr::null(), 0),
            0,
            "a null path cannot be opened"
        );
    }
}

#[test]
fn an_unset_variable_answers_none() {
    unsafe {
        let missing = CString::new("YASHY_RUNTIME_TEST_UNSET_3F6C1A").expect("no NUL");
        let mut value: *mut YashyStr = std::ptr::null_mut();
        assert_eq!(yashy_getenv(missing.as_ptr(), &mut value), 0);
        assert!(value.is_null(), "an absent variable writes no payload");

        let mut null_value: *mut YashyStr = std::ptr::null_mut();
        assert_eq!(yashy_getenv(std::ptr::null(), &mut null_value), 0);
        assert!(null_value.is_null());
    }
}

#[test]
fn a_set_variable_is_owned_text() {
    unsafe {
        // `PATH` is set in every environment a test runs in. An empty value is
        // still a set variable, so the assertion is that the answer is a usable
        // owned string rather than that it has a particular length.
        let name = CString::new("PATH").expect("no NUL");
        let mut value: *mut YashyStr = std::ptr::null_mut();
        assert_eq!(yashy_getenv(name.as_ptr(), &mut value), 1);
        assert!(!value.is_null(), "an owned string, not a null pointer");
        assert_eq!(
            (*value).refcount,
            1,
            "the caller owns the one reference the runtime made"
        );
        yashy_str_release(value);
    }
}
