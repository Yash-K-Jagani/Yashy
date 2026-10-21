//! The layout of a closure's environment.
//!
//! A closure's captured values live in an *environment*: a reference-counted
//! block the closure carries and reads through. The runtime owns the count and
//! the release of what the block captured; the generated code builds the
//! block and the backend needs to know where in it each captured value sits.
//!
//! The block is:
//!
//! ```text
//!   [0]   the reference count          (the runtime's)
//!   [8]   how many values need release  (the runtime's)
//!   [16]  the release table: one entry per captured value
//!   ...   the captured values themselves
//! ```
//!
//! The header and the table entry are the two sizes the runtime and this
//! file have to agree on; they are `OTTER_ENV_HEADER_SIZE` and
//! `OTTER_ENV_SLOT_SIZE` in the runtime's header, and the tests on each side
//! pin them.

use otter_sema::{HirCapture, TypeKind};

/// The environment's header: the reference count and the table's length.
pub const HEADER_SIZE: u64 = 16;

/// One table entry: what to release, and the reference to release.
pub const SLOT_SIZE: u64 = 16;

/// The kind of a table entry that releases a captured `string`.
pub const SLOT_STRING: i64 = 1;

/// The kind of a table entry that releases nothing.
pub const SLOT_NONE: i64 = 0;

/// The size of one captured value.
///
/// Only the types a closure may capture are counted here: a number, a
/// boolean, or a string. A captured value is copied into the environment, so
/// it has to be a value the environment can own, and those are the ones whose
/// size the backend knows exactly.
pub fn capture_size(kind: &TypeKind) -> u64 {
    match kind {
        TypeKind::I64 | TypeKind::F64 | TypeKind::Bool | TypeKind::Str => 8,
        // A type the checker never lets a closure capture. The size is the
        // harmless one; the checker has already refused the program.
        _ => 8,
    }
}

/// Round `offset` up to the alignment every captured value has.
fn align_up(offset: u64) -> u64 {
    (offset + 7) & !7
}

/// The offset of each captured value in the environment.
///
/// The offsets are in the order the captures are listed, which is the order
/// the checker found them in.
pub fn capture_offsets(captures: &[HirCapture]) -> Vec<u64> {
    let mut offset = HEADER_SIZE + SLOT_SIZE * captures.len() as u64;
    let mut offsets = Vec::with_capacity(captures.len());
    for capture in captures {
        offset = align_up(offset);
        offsets.push(offset);
        offset += capture_size(&capture.ty.kind);
    }
    offsets
}

/// The size of the environment's payload: the captured values.
pub fn payload_size(captures: &[HirCapture]) -> u64 {
    let total = capture_offsets(captures)
        .last()
        .map(|last| last + capture_size(&captures[captures.len() - 1].ty.kind))
        .unwrap_or(HEADER_SIZE);
    total.saturating_sub(HEADER_SIZE + SLOT_SIZE * captures.len() as u64)
}

/// The offset of a capture's table entry.
pub fn slot_offset(index: usize) -> u64 {
    HEADER_SIZE + SLOT_SIZE * index as u64
}

/// Whether a capture is a `string`, which the environment owns a reference to.
pub fn is_string(capture: &HirCapture) -> bool {
    matches!(capture.ty.kind, TypeKind::Str)
}
