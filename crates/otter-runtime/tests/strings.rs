//! Exercises the runtime's string operations through the same declarations the
//! generated IR calls, so a signature that drifts from the header fails here
//! rather than in a linked Otter program.
//!
//! A `string` value is a pointer to an `OtterStr` (refcount, len, cap, data) in
//! the runtime and in the IR, so a test builds one the same way the compiler
//! emits one: a header pointing at NUL-terminated bytes. Strings the runtime
//! *produces* come back owned (refcount 1) and are released here, which is what
//! makes these tests exercise the refcount and not just the bytes.

use std::ffi::{c_char, CStr, CString};

use otter_runtime::OtterStr;

/// A `CString` and the *static* `OtterStr` that points at its bytes.
///
/// The value is `refcount -1`, exactly what a literal is, so releasing it is a
/// no-op and the tests never depend on when it goes away. The bytes are owned
/// by the `CString` and outlive any borrow of the value.
struct Static {
    /// Held only to own the bytes `value.data` points at.
    _bytes: CString,
    value: OtterStr,
}

impl Static {
    fn new(text: &str) -> Self {
        let bytes = CString::new(text).expect("test strings must not contain a NUL byte");
        let len = bytes.as_bytes().len() as i64;
        let value = OtterStr {
            refcount: -1,
            len,
            cap: len,
            data: bytes.as_ptr() as *mut c_char,
        };
        // Moving `bytes` into the struct moves the `CString` handle, not the
        // heap buffer it points at, so `value.data` stays valid.
        Static {
            _bytes: bytes,
            value,
        }
    }

    fn ptr(&self) -> *const OtterStr {
        &self.value
    }
}

/// Read a runtime string back into a Rust `String`.
///
/// Every read goes through this, so a returned null is a test failure rather
/// than undefined behaviour.
unsafe fn read(string: *const OtterStr) -> String {
    assert!(!string.is_null(), "the runtime returned a null string");
    let data = (*string).data;
    assert!(!data.is_null(), "a string's data pointer must not be null");
    CStr::from_ptr(data)
        .to_str()
        .expect("the runtime must return valid UTF-8 for these inputs")
        .to_string()
}

#[test]
fn concat_joins_the_operands_in_order() {
    unsafe {
        let a = Static::new("Hello, ");
        let b = Static::new("Otter!");
        let joined = otter_runtime::otter_str_concat(a.ptr(), b.ptr());
        assert_eq!(read(joined), "Hello, Otter!");
        assert_eq!(otter_runtime::otter_str_len(joined), 13);
        assert_eq!(
            (*joined).refcount,
            1,
            "a built string comes back owned by the caller"
        );
        otter_runtime::otter_str_release(joined);
    }
}

#[test]
fn concat_leaves_its_operands_untouched() {
    // The result is a fresh string, so a program that concatenates a literal
    // with itself cannot corrupt the literal's bytes.
    unsafe {
        let a = Static::new("xy");
        let b = Static::new("z");
        let joined = otter_runtime::otter_str_concat(a.ptr(), b.ptr());
        assert_eq!(read(joined), "xyz");
        assert_eq!(read(a.ptr()), "xy", "the left operand changed");
        assert_eq!(read(b.ptr()), "z", "the right operand changed");
        otter_runtime::otter_str_release(joined);
    }
}

#[test]
fn concat_with_an_empty_string_is_a_copy() {
    unsafe {
        let empty = Static::new("");
        let text = Static::new("abc");
        let right = otter_runtime::otter_str_concat(text.ptr(), empty.ptr());
        let left = otter_runtime::otter_str_concat(empty.ptr(), text.ptr());
        assert_eq!(read(right), "abc");
        assert_eq!(read(left), "abc");
        otter_runtime::otter_str_release(right);
        otter_runtime::otter_str_release(left);
    }
}

#[test]
fn concat_treats_null_as_the_empty_string() {
    unsafe {
        let text = Static::new("abc");
        let joined = otter_runtime::otter_str_concat(std::ptr::null(), text.ptr());
        assert_eq!(read(joined), "abc");
        otter_runtime::otter_str_release(joined);
    }
}

#[test]
fn from_i64_formats_decimal_and_the_extremes() {
    unsafe {
        assert_eq!(read(otter_runtime::otter_str_from_i64(0)), "0");
        assert_eq!(read(otter_runtime::otter_str_from_i64(-42)), "-42");
        let min = read(otter_runtime::otter_str_from_i64(i64::MIN));
        assert_eq!(min, "-9223372036854775808");
        assert_eq!(
            read(otter_runtime::otter_str_from_i64(i64::MAX)),
            "9223372036854775807"
        );
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
        let text = Static::new("a\u{00e9}b"); // "aéb": four bytes in UTF-8
        assert_eq!(otter_runtime::otter_str_len(text.ptr()), 4);
        assert_eq!(otter_runtime::otter_str_len(std::ptr::null()), 0);
    }
}

#[test]
fn len_is_the_stored_count_not_a_scan_for_the_nul() {
    // The representation carries the length, so a string with bytes after the
    // count is measured by the count. This is what makes `string` O(1) to
    // measure and what would let a future slicing operation avoid a scan.
    let mut text = Static::new("abcdef");
    text.value.len = 3;
    let pointer = text.ptr();
    assert_eq!(
        unsafe { otter_runtime::otter_str_len(pointer) },
        3,
        "the stored length decides, not the NUL"
    );
}

#[test]
fn byte_at_returns_the_byte_or_zero_out_of_range() {
    unsafe {
        let text = Static::new("abc");
        assert_eq!(otter_runtime::otter_str_byte_at(text.ptr(), 0), b'a' as i64);
        assert_eq!(otter_runtime::otter_str_byte_at(text.ptr(), 2), b'c' as i64);
        // Out of range, and a negative index, both read as 0 rather than
        // trapping: the language promises a value, not a fault.
        assert_eq!(otter_runtime::otter_str_byte_at(text.ptr(), 3), 0);
        assert_eq!(otter_runtime::otter_str_byte_at(text.ptr(), 99), 0);
        assert_eq!(otter_runtime::otter_str_byte_at(text.ptr(), -1), 0);
    }
}

#[test]
fn eq_compares_bytes_and_null_equals_only_null() {
    unsafe {
        let abc = Static::new("abc");
        let also_abc = Static::new("abc");
        let abd = Static::new("abd");
        let ab = Static::new("ab");
        assert_eq!(otter_runtime::otter_str_eq(abc.ptr(), also_abc.ptr()), 1);
        assert_eq!(otter_runtime::otter_str_eq(abc.ptr(), abd.ptr()), 0);
        assert_eq!(otter_runtime::otter_str_eq(abc.ptr(), ab.ptr()), 0);
        assert_eq!(
            otter_runtime::otter_str_eq(std::ptr::null(), std::ptr::null()),
            1
        );
        assert_eq!(otter_runtime::otter_str_eq(abc.ptr(), std::ptr::null()), 0);
    }
}

#[test]
fn retain_and_release_are_no_ops_on_a_static_string() {
    // A literal has refcount -1 and no owner, so keeping or dropping a
    // reference must not change it and must never free it. This is the property
    // that lets a binding treat a literal and a built string the same way.
    unsafe {
        let text = Static::new("literal");
        let pointer = text.ptr().cast_mut();
        otter_runtime::otter_str_retain(pointer);
        otter_runtime::otter_str_release(pointer);
        otter_runtime::otter_str_release(pointer);
        assert_eq!(text.value.refcount, -1, "a static string is not counted");
        assert_eq!(read(text.ptr()), "literal", "and it is still alive");
    }
}

#[test]
fn retain_counts_references_and_release_frees_at_zero() {
    // The count tracks owners: a built string starts at 1, each `retain` adds
    // one, and a `release` per owner brings it back to zero, at which point the
    // storage is freed. The last release is deliberately the last touch.
    unsafe {
        let text = Static::new("abc");
        let owned = otter_runtime::otter_str_concat(text.ptr(), text.ptr());
        assert_eq!((*owned).refcount, 1);
        assert_eq!(
            otter_runtime::otter_str_retain(owned),
            owned,
            "retain returns its argument"
        );
        assert_eq!((*owned).refcount, 2);
        otter_runtime::otter_str_release(owned);
        assert_eq!((*owned).refcount, 1, "one owner is left");
        assert_eq!(read(owned), "abcabc", "and the bytes are still there");
        otter_runtime::otter_str_release(owned);
    }
}

#[test]
fn building_many_strings_allocates_and_releases_cleanly() {
    // Each result is owned, so releasing it returns the memory. A leak in the
    // allocator or a release that frees twice would show up as a crash or a
    // wrong length long before the loop finished.
    unsafe {
        let piece = Static::new("0123456789");
        let mut total: i64 = 0;
        for _ in 0..2_000 {
            let joined = otter_runtime::otter_str_concat(piece.ptr(), piece.ptr());
            total += otter_runtime::otter_str_len(joined);
            otter_runtime::otter_str_release(joined);
        }
        assert_eq!(total, 2_000 * 20);
    }
}

#[test]
fn concurrent_builders_never_share_bytes() {
    // Every built string is its own allocation, so builders on different
    // threads cannot overwrite one another. Sixteen threads building and
    // releasing strings in a loop is the shape that caught a shared bump
    // allocator when strings were arena-allocated; here it is a regression
    // guard on the per-string allocation staying per-string.
    let builders: Vec<std::thread::JoinHandle<()>> = (0..16)
        .map(|worker| {
            std::thread::spawn(move || {
                let left = Static::new(&format!("left-{worker}-"));
                let right = Static::new(&format!("right-{worker}"));
                let expected = format!("left-{worker}-right-{worker}");
                for _ in 0..200 {
                    let joined = unsafe {
                        otter_runtime::otter_str_concat(left.ptr(), right.ptr())
                    };
                    let text = unsafe { read(joined) };
                    assert_eq!(text, expected, "thread {worker} read another's bytes");
                    unsafe { otter_runtime::otter_str_release(joined) };
                }
            })
        })
        .collect();
    for builder in builders {
        builder.join().expect("no builder thread may panic");
    }
}
