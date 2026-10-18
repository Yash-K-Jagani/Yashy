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
//! void otter_print_str(i8*);      // NUL-terminated string, NO trailing newline
//! void otter_print_i64(i64);      // decimal
//! void otter_print_f64(double);   // %.17g
//! ```
//!
//! None of them writes a newline. A program prints one explicitly, as the
//! `print_str("\n")` calls in the plan's examples do.

use std::ffi::{c_char, c_double, c_longlong};

/// The runtime's C source, embedded so the driver can hand it to the linker
/// without knowing where this crate was built.
///
/// The driver links freshly generated IR, not a Rust binary, so it cannot rely
/// on `cargo`'s link line. Embedding the text means the runtime a compiled
/// Otter program links against is exactly the one the print tests exercise,
/// with no library search path to get wrong and no stale archive to be tricked
/// by.
pub const RUNTIME_C_SOURCE: &str = include_str!("../runtime.c");

/// The header [`RUNTIME_C_SOURCE`] includes, embedded for the same reason.
pub const RUNTIME_HEADER: &str = include_str!("../include/otter_runtime.h");

/// The header's file name, which `runtime.c`'s `#include` has to match.
pub const RUNTIME_HEADER_NAME: &str = "otter_runtime.h";

/// The source file's file name, used when the driver writes it to disk.
pub const RUNTIME_SOURCE_NAME: &str = "runtime.c";

// The IR says `i64`. If a target ever gave `long long` a different width, the
// generated IR and the compiled runtime would disagree silently, so refuse to
// build instead.
const _: () = assert!(std::mem::size_of::<c_longlong>() == std::mem::size_of::<i64>());

// Three separate blocks: an extern item cannot carry a doc comment, so the
// descriptions live on the items themselves.

unsafe extern "C" {
    /// Writes the NUL-terminated string to stdout. Nothing else, no newline.
    pub fn otter_print_str(s: *const c_char);
}

unsafe extern "C" {
    /// Writes a decimal integer to stdout, no newline.
    pub fn otter_print_i64(value: c_longlong);
}

unsafe extern "C" {
    /// Writes a double with `%.17g`, no newline — enough digits to round-trip.
    pub fn otter_print_f64(value: c_double);
}
