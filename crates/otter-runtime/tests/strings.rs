//! Exercises the runtime's string operations through the same declarations the
//! generated IR calls, so a signature that drifts from the header fails here
//! rather than in a linked Otter program.
//!
//! The values these functions return live in the runtime's internal string
//! arena and are never freed individually, so the tests read them and move on
//! rather than pretending to clean up.

use std::ffi::{c_char, CStr, CString};

/// A NUL-terminated C string literal for the duration of the call.
fn c(s: &str) -> CString {
    CString::new(s).expect("test strings must not contain a NUL byte")
}

/// Read a runtime-owned C string back into a Rust `String`.
///
/// Every conversion goes through this, so a returned null is a test failure
/// rather than undefined behaviour.
unsafe fn read(pointer: *const c_char) -> String {
    assert!(!pointer.is_null(), "the runtime returned a null string");
    CStr::from_ptr(pointer)
        .to_str()
        .expect("the runtime must return valid UTF-8 for these inputs")
        .to_string()
}

#[test]
fn concat_joins_the_operands_in_order() {
    unsafe {
        let a = c("Hello, ");
        let b = c("Otter!");
        let joined = otter_runtime::otter_str_concat(a.as_ptr(), b.as_ptr());
        assert_eq!(read(joined), "Hello, Otter!");
        assert_eq!(otter_runtime::otter_str_len(joined), 13);
    }
}

#[test]
fn concat_leaves_its_operands_untouched() {
    // The result is a fresh string, so a program that concatenates a literal
    // with itself cannot corrupt the literal's bytes.
    unsafe {
        let a = c("xy");
        let b = c("z");
        let joined = otter_runtime::otter_str_concat(a.as_ptr(), b.as_ptr());
        assert_eq!(read(joined), "xyz");
        assert_eq!(read(a.as_ptr()), "xy", "the left operand changed");
        assert_eq!(read(b.as_ptr()), "z", "the right operand changed");
    }
}

#[test]
fn concat_with_an_empty_string_is_a_copy() {
    unsafe {
        let empty = c("");
        let text = c("abc");
        let right = otter_runtime::otter_str_concat(text.as_ptr(), empty.as_ptr());
        let left = otter_runtime::otter_str_concat(empty.as_ptr(), text.as_ptr());
        assert_eq!(read(right), "abc");
        assert_eq!(read(left), "abc");
    }
}

#[test]
fn concat_treats_null_as_the_empty_string() {
    unsafe {
        let text = c("abc");
        let joined = otter_runtime::otter_str_concat(std::ptr::null(), text.as_ptr());
        assert_eq!(read(joined), "abc");
    }
}

#[test]
fn from_i64_formats_decimal_and_the_extremes() {
    unsafe {
        assert_eq!(read(otter_runtime::otter_str_from_i64(0)), "0");
        assert_eq!(read(otter_runtime::otter_str_from_i64(-42)), "-42");
        let min = read(otter_runtime::otter_str_from_i64(i64::MIN));
        assert_eq!(min, "-9223372036854775808");
        assert_eq!(read(otter_runtime::otter_str_from_i64(i64::MAX)), "9223372036854775807");
    }
}

#[test]
fn from_bool_spells_true_and_false() {
    unsafe {
        assert_eq!(read(otter_runtime::otter_str_from_bool(1)), "true");
        assert_eq!(read(otter_runtime::otter_str_from_bool(0)), "false");
        // Any non-zero is true, the same reading the IR's `i1` has.
        assert_eq!(read(otter_runtime::otter_str_from_bool(7)), "true");
    }
}

#[test]
fn from_f64_matches_the_print_format() {
    unsafe {
        // `%.17g` is what `print_f64` writes, so the string form and the printed
        // form of the same number are the same bytes.
        assert_eq!(read(otter_runtime::otter_str_from_f64(0.5)), "0.5");
        assert_eq!(read(otter_runtime::otter_str_from_f64(1.0)), "1");
        assert_eq!(read(otter_runtime::otter_str_from_f64(-2.25)), "-2.25");
    }
}

#[test]
fn len_counts_bytes_not_characters() {
    unsafe {
        let text = c("a\u{00e9}b"); // "aéb": four bytes in UTF-8
        assert_eq!(otter_runtime::otter_str_len(text.as_ptr()), 4);
        assert_eq!(otter_runtime::otter_str_len(std::ptr::null()), 0);
    }
}

#[test]
fn byte_at_returns_the_byte_or_zero_out_of_range() {
    unsafe {
        let text = c("abc");
        assert_eq!(otter_runtime::otter_str_byte_at(text.as_ptr(), 0), b'a' as i64);
        assert_eq!(otter_runtime::otter_str_byte_at(text.as_ptr(), 2), b'c' as i64);
        // Out of range, and a negative index, both read as 0 rather than
        // trapping: the language promises a value, not a fault.
        assert_eq!(otter_runtime::otter_str_byte_at(text.as_ptr(), 3), 0);
        assert_eq!(otter_runtime::otter_str_byte_at(text.as_ptr(), 99), 0);
        assert_eq!(otter_runtime::otter_str_byte_at(text.as_ptr(), -1), 0);
    }
}

#[test]
fn eq_compares_bytes_and_null_equals_only_null() {
    unsafe {
        let abc = c("abc");
        let also_abc = c("abc");
        let abd = c("abd");
        let ab = c("ab");
        assert_eq!(otter_runtime::otter_str_eq(abc.as_ptr(), also_abc.as_ptr()), 1);
        assert_eq!(otter_runtime::otter_str_eq(abc.as_ptr(), abd.as_ptr()), 0);
        assert_eq!(otter_runtime::otter_str_eq(abc.as_ptr(), ab.as_ptr()), 0);
        assert_eq!(otter_runtime::otter_str_eq(std::ptr::null(), std::ptr::null()), 1);
        assert_eq!(otter_runtime::otter_str_eq(abc.as_ptr(), std::ptr::null()), 0);
    }
}

#[test]
fn building_many_strings_does_not_exhaust_memory() {
    unsafe {
        // Each concat grows the string arena rather than calling `malloc` per
        // result. The loop is short enough to be a smoke test, not a leak proof:
        // what it catches is a concat that loses track of its arena and
        // reallocates a page for every call.
        let piece = c("0123456789");
        let mut total: i64 = 0;
        for _ in 0..2_000 {
            let joined = otter_runtime::otter_str_concat(piece.as_ptr(), piece.as_ptr());
            total += otter_runtime::otter_str_len(joined);
        }
        assert_eq!(total, 2_000 * 20);
    }
}

#[test]
fn concurrent_builders_never_share_a_string_block() {
    // The string arena is thread-local, and this is the test that says why: as a
    // single shared arena it handed two threads the same block -- allocation is
    // an unsynchronized pointer increment -- and one thread's bytes appeared in
    // another thread's string. Sixteen threads building strings in a loop hit it
    // almost every run; here, every string must come back exactly as built.
    //
    // The Rust test harness runs tests in parallel by default, so a shared arena
    // also made *other* tests in this file flaky.
    let builders: Vec<std::thread::JoinHandle<()>> = (0..16)
        .map(|worker| {
            std::thread::spawn(move || {
                let left = c(&format!("left-{worker}-"));
                let right = c(&format!("right-{worker}"));
                let expected = format!("left-{worker}-right-{worker}");
                for _ in 0..200 {
                    let joined =
                        unsafe { otter_runtime::otter_str_concat(left.as_ptr(), right.as_ptr()) };
                    let text = unsafe { read(joined) };
                    assert_eq!(text, expected, "thread {worker} read another's bytes");
                }
            })
        })
        .collect();
    for builder in builders {
        builder.join().expect("no builder thread may panic");
    }
}
