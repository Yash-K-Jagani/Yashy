//! The Otter compiler driver, usable both as a library and as a binary so that
//! integration tests can call into it directly.

pub mod toolchain;

pub use toolchain::{detect, detect_with, Toolchain};
