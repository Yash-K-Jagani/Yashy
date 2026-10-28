//! The Yashy compiler driver, usable both as a library and as a binary so that
//! integration tests can call into it directly.
//!
//! `modules` finds and loads imported files, `pipeline` owns the whole build —
//! lex, parse, check, emit IR, invoke the toolchain — and `toolchain` is the
//! only place that knows how to find `clang`. No argument parsing lives here:
//! that is `main.rs`, so the library surface stays a set of functions a test
//! can call with plain values.

pub mod modules;
pub mod pipeline;
pub mod toolchain;

pub use modules::{default_stdlib_root, load, module_roots, resolve_import, Module};
pub use pipeline::{
    check_and_verify, compile_hir, compile_to_binary, load_and_check, resolve_opt, BuildOptions,
    CpuTarget, OptLevel,
};
pub use toolchain::{detect, detect_with, Toolchain};
