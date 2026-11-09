#![no_main]
//! Fuzz target: manifest parsing.
//!
//! `yashy_manifest::Manifest::parse` promises totality: every input is a
//! `Result`, never a panic, and every rejection carries a line and column.
//! The lockfile reader has the same promise, so both are driven here.
//!
//! As in `lex_parse`, the input is bytes: a manifest is a file on disk, and an
//! invalid encoding must become a diagnostics-bearing error rather than a
//! crash inside the reader.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = yashy_manifest::Manifest::parse(&text);
    let _ = yashy_manifest::Lockfile::parse(&text);
});
