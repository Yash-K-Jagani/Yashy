//! Errors produced by the type checker, before they become diagnostics.

use yashy_common::{Diagnostic, FileId, Severity, Span};

/// A type error: what was wrong, and where in the source it was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeError {
    pub span: Span,
    pub message: String,
}

impl TypeError {
    pub fn new(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
        }
    }

    /// Turn this type error into a diagnostic attached to `file`.
    pub fn into_diagnostic(&self, file: FileId) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            message: self.message.clone(),
            file,
            span: self.span,
            label: None,
        }
    }
}
