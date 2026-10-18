//! The Otter compiler driver, usable both as a library and as a binary so that
//! integration tests can call into it directly.
//!
//! `pipeline` owns the whole build — lex, parse, check, emit IR, invoke the
//! toolchain — and `toolchain` is the only place that knows how to find
//! `clang`. No argument parsing lives here: that is `main.rs`, so the library
//! surface stays a set of functions a test can call with plain values.

pub mod pipeline;
pub mod toolchain;

pub use pipeline::{compile_to_binary, BuildOptions, CpuTarget, OptLevel};
pub use toolchain::{detect, detect_with, Toolchain};
