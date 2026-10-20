//! The Otter runtime.
//!
//! The runtime itself is `runtime.c`, compiled by this crate's `build.rs` and
//! linked into every dependent binary. This module re-declares its entry points
//! so Rust code — tests today, the driver tomorrow — can call them, and so the
//! signatures are checked in one place instead of drifting from the header.
//!
//! These declarations are the same contract the code generator emits IR
//! against, and they are frozen:
//!
//! ```text
//! void otter_print_str(otter_str*);  // a string value, NO trailing newline
//! void otter_print_i64(i64);         // decimal
//! void otter_print_f64(double);      // %.17g
//! ```
//!
//! None of them writes a newline. A program prints one explicitly, as the
//! `print_str("\n")` calls in the plan's examples do.

use std::ffi::{c_char, c_double, c_int, c_longlong, c_void};

/// The runtime's refcounted string value, as C lays it out:
/// `struct otter_str { long long refcount; long long len; long long cap; char *data; }`.
///
/// Rust tests build one of these to hand a string to the runtime the same way
/// the generated IR does — a pointer to the header, with the bytes reachable
/// through `data`. A `refcount` of `-1` is a *static* string: the runtime never
/// frees it, which is what a literal is.
#[repr(C)]
pub struct OtterStr {
    /// 1 when owned, -1 when static (never freed).
    pub refcount: c_longlong,
    /// Byte length, excluding the NUL.
    pub len: c_longlong,
    /// Allocated capacity, excluding the NUL.
    pub cap: c_longlong,
    /// NUL-terminated bytes.
    pub data: *mut c_char,
}

/// The runtime's C source, embedded so the driver can hand it to the linker
/// without knowing where this crate was built.
///
/// The driver links freshly generated IR, not a Rust binary, so it cannot rely
/// on `cargo`'s link line. Embedding the text means the runtime a compiled
/// Otter program links against is exactly the one the print tests exercise,
/// with no library search path to get wrong and no stale archive to be tricked
/// by.
pub const RUNTIME_C_SOURCE: &str = include_str!("../runtime.c");

/// The map runtime, a separate translation unit that includes the same header.
pub const HASHMAP_C_SOURCE: &str = include_str!("../hashmap.c");

/// The header [`RUNTIME_C_SOURCE`] includes, embedded for the same reason.
pub const RUNTIME_HEADER: &str = include_str!("../include/otter_runtime.h");

/// The header's file name, which `runtime.c`'s `#include` has to match.
pub const RUNTIME_HEADER_NAME: &str = "otter_runtime.h";

/// The source file's file name, used when the driver writes it to disk.
pub const RUNTIME_SOURCE_NAME: &str = "runtime.c";

/// The map source's file name, used when the driver writes it to disk.
pub const HASHMAP_SOURCE_NAME: &str = "hashmap.c";

// The IR says `i64`. If a target ever gave `long long` a different width, the
// generated IR and the compiled runtime would disagree silently, so refuse to
// build instead.
const _: () = assert!(std::mem::size_of::<c_longlong>() == std::mem::size_of::<i64>());

// Three separate blocks: an extern item cannot carry a doc comment, so the
// descriptions live on the items themselves.

unsafe extern "C" {
    /// Writes the NUL-terminated string to stdout. Nothing else, no newline.
    pub fn otter_print_str(s: *const OtterStr);
}

unsafe extern "C" {
    /// Writes a decimal integer to stdout, no newline.
    pub fn otter_print_i64(value: c_longlong);
}

unsafe extern "C" {
    /// Writes a double with `%.17g`, no newline — enough digits to round-trip.
    pub fn otter_print_f64(value: c_double);
}

// The memory half of the contract. The generated IR calls these by name; these
// declarations are what lets Rust tests exercise the same linked code.

unsafe extern "C" {
    /// Allocates `size` bytes aligned to `align`, or returns null. The bytes are
    /// uninitialised. Free the result with [`otter_free`] exactly once.
    pub fn otter_alloc(size: c_longlong, align: c_longlong) -> *mut c_void;
}

unsafe extern "C" {
    /// Releases a block from [`otter_alloc`]. Tolerates null.
    pub fn otter_free(ptr: *mut c_void);
}

unsafe extern "C" {
    /// Creates an arena. Free it with [`otter_arena_free_all`]; individual
    /// allocations from it are never freed one at a time.
    pub fn otter_arena_new() -> *mut c_void;
}

unsafe extern "C" {
    /// Hands out `size` bytes aligned to `align` from `arena`, or returns null.
    pub fn otter_arena_alloc(arena: *mut c_void, size: c_longlong, align: c_longlong) -> *mut c_void;
}

unsafe extern "C" {
    /// Releases every block in `arena` and the arena header. Tolerates null.
    pub fn otter_arena_free_all(arena: *mut c_void);
}

/// The runtime's slice header, as C lays it out:
/// `struct otter_slice { void *data; long long len; long long cap; }`.
///
/// This is also the IR shape `{ i8*, i64, i64 }`, so a Rust test can build a
/// header and drive the same `push`/`free` the generated code calls.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct OtterSlice {
    /// The backing array, or null when empty.
    pub data: *mut c_void,
    /// Elements in use.
    pub len: c_longlong,
    /// Elements the allocation can hold before the next grow.
    pub cap: c_longlong,
}

unsafe extern "C" {
    /// Appends `element_size` bytes from `element` to `slice`, growing the
    /// backing array (from 0 to 4, then doubling) when it is full. A failed
    /// grow leaves the slice unchanged.
    pub fn otter_slice_push(
        slice: *mut OtterSlice,
        element: *const c_void,
        element_size: c_longlong,
    );
}

unsafe extern "C" {
    /// Releases `slice`'s backing array and zeroes the header. Tolerates null.
    pub fn otter_slice_free(slice: *mut OtterSlice);
}

unsafe extern "C" {
    /// Copies the element at `index` into `out` and returns 1, or returns 0
    /// when `index` is outside `[0, len)`.
    pub fn otter_slice_get(
        slice: *mut OtterSlice,
        index: c_longlong,
        out: *mut c_void,
        element_size: c_longlong,
    ) -> c_longlong;
}

unsafe extern "C" {
    /// Removes and copies the last element, returning 1, or returns 0 when the
    /// slice is empty.
    pub fn otter_slice_pop(
        slice: *mut OtterSlice,
        out: *mut c_void,
        element_size: c_longlong,
    ) -> c_longlong;
}

/// The map's table, opaque to Rust as it is to generated code.
///
/// The fields live in `hashmap.c`; a test only ever holds the pointer.
#[repr(C)]
pub struct OtterMap {
    _private: [u8; 0],
}

unsafe extern "C" {
    /// Creates an empty table. `key_kind` is 0 for `i64`, 1 for `bool`, 2 for
    /// `string`. Nothing is allocated until the first insert.
    pub fn otter_map_new(
        key_kind: c_longlong,
        key_size: c_longlong,
        value_size: c_longlong,
    ) -> *mut OtterMap;
}

unsafe extern "C" {
    /// Frees the table, releasing every retained string key. Tolerates null.
    pub fn otter_map_free(map: *mut OtterMap);
}

unsafe extern "C" {
    /// Copies `key` and `value` into the table, replacing the value when the key
    /// is already present. A string key is retained.
    pub fn otter_map_insert(map: *mut OtterMap, key: *const c_void, value: *const c_void);
}

unsafe extern "C" {
    /// Copies the value for `key` into `out` and returns 1, or returns 0 when
    /// the key is missing.
    pub fn otter_map_get(map: *const OtterMap, key: *const c_void, out: *mut c_void) -> c_longlong;
}

unsafe extern "C" {
    /// Copies the value for `key` into `out`, removes the entry, and returns 1;
    /// returns 0 when the key is missing.
    pub fn otter_map_remove(map: *mut OtterMap, key: *const c_void, out: *mut c_void) -> c_longlong;
}

unsafe extern "C" {
    /// The number of live entries. Null is zero.
    pub fn otter_map_len(map: *const OtterMap) -> c_longlong;
}

// The string half of the contract. A `string` value is a pointer to an
// `OtterStr` in both the IR and here, so Rust tests can drive the same linked
// operations the generated code calls.

unsafe extern "C" {
    /// The byte length of `s`, not counting the NUL. Null is length zero.
    pub fn otter_str_len(s: *const OtterStr) -> c_longlong;
}

unsafe extern "C" {
    /// The byte at `index`, or `0` when `index` is out of range. `0` is also
    /// the byte a NUL contributes, so callers that care about the difference
    /// must compare against [`otter_str_len`] first.
    pub fn otter_str_byte_at(s: *const OtterStr, index: c_longlong) -> c_longlong;
}

unsafe extern "C" {
    /// A fresh owned string holding `a` followed by `b`. Neither operand is
    /// modified, and the result's refcount is 1: the caller owns it.
    pub fn otter_str_concat(a: *const OtterStr, b: *const OtterStr) -> *mut OtterStr;
}

unsafe extern "C" {
    /// `value` in decimal, as a fresh owned string.
    pub fn otter_str_from_i64(value: c_longlong) -> *mut OtterStr;
}

unsafe extern "C" {
    /// `value` with `%.17g`, matching [`otter_print_f64`], as a fresh owned
    /// string.
    pub fn otter_str_from_f64(value: c_double) -> *mut OtterStr;
}

unsafe extern "C" {
    /// `"true"` or `"false"`, as a fresh owned string. Takes an `i64` where the
    /// IR passes a zero-extended `i1`, because C has no one-bit type.
    pub fn otter_str_from_bool(value: c_longlong) -> *mut OtterStr;
}

unsafe extern "C" {
    /// Byte equality. Null equals only null.
    pub fn otter_str_eq(a: *const OtterStr, b: *const OtterStr) -> c_int;

    /// One line of standard input without its terminator, or `""` at end of
    /// input. The caller owns the result.
    pub fn otter_read_line() -> *mut OtterStr;

    /// The integer `s` denotes, or zero when it denotes none (including a value
    /// too large for `i64`).
    pub fn otter_parse_i64(s: *const OtterStr) -> c_longlong;

    /// The floating-point number `s` denotes, or zero when it denotes none.
    pub fn otter_parse_f64(s: *const OtterStr) -> c_double;

    /// Adds one to `s`'s reference count and returns `s`. A no-op on a static
    /// string (refcount -1).
    pub fn otter_str_retain(s: *mut OtterStr) -> *mut OtterStr;

    /// Drops one reference to `s`, freeing it when the count reaches zero. A
    /// no-op on null and on a static string.
    pub fn otter_str_release(s: *mut OtterStr);
}
