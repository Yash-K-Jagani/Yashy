//! Byte spans, the source map that turns them into line/column positions, and
//! the diagnostics every stage of the pipeline reports through.

pub mod codes;
pub mod diagnostic;
pub mod json;
pub mod source;
pub mod span;
pub mod suggest;

pub use codes::Code;
pub use diagnostic::{
    Diagnostic, DiagnosticBag, Severity, EXIT_COMPILE_ERROR, EXIT_INTERNAL, EXIT_OK, EXIT_USAGE,
};
pub use json::Json;
pub use source::{FileId, Location, SourceMap};
pub use span::Span;
pub use suggest::closest;
