//! The Yashy compiler driver, usable both as a library and as a binary so that
//! integration tests can call into it directly.
//!
//! `modules` finds and loads imported files, `pipeline` owns the whole build —
//! lex, parse, check, emit IR, invoke the toolchain — and `toolchain` is the
//! only place that knows how to find `clang`. No argument parsing lives here:
//! that is `main.rs`, so the library surface stays a set of functions a test
//! can call with plain values.

pub mod context;
pub mod doc;
pub mod index;
pub mod mcp;
pub mod modules;
pub mod opt_report;
pub mod outline;
pub mod pipeline;
pub mod profile;
pub mod sandbox;
pub mod toolchain;

pub use modules::{
    default_stdlib_root, load, load_sources, module_roots, resolve_import, Module, ModuleSource,
};
pub use pipeline::{
    check_and_verify, check_and_verify_timed, compile_hir, compile_hir_timed, compile_to_binary,
    load_and_check, load_and_check_exports_timed, load_and_check_timed, resolve_opt, timed,
    BuildOptions, CommandKind, CpuTarget, OptLevel, TimingRow,
};
pub use toolchain::{detect, detect_with, Toolchain};
