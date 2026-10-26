//! Byte spans, the source map that turns them into line/column positions, and
//! the diagnostics every stage of the pipeline reports through.

pub mod diagnostic;
pub mod source;
pub mod span;
pub mod suggest;

pub use diagnostic::{Diagnostic, DiagnosticBag, Severity, EXIT_COMPILE_ERROR, EXIT_OK};
pub use source::{FileId, Location, SourceMap};
pub use span::Span;
pub use suggest::closest;
