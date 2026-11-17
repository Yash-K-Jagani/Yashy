//! Errors produced by the type checker, before they become diagnostics.

use yashy_common::{Diagnostic, FileId, Severity, Span, Suggestion};

/// A type error: what was wrong, and where in the source it was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeError {
    pub span: Span,
    pub message: String,
    /// An optional note placed after the caret, such as a suggested spelling.
    pub label: Option<String>,
    /// Location-free notes printed after the caret line.
    pub notes: Vec<String>,
    /// The stable catalog code, assigned where the error is built.
    pub code: Option<&'static str>,
    /// Machine-applicable edits that fix the error.
    ///
    /// An error that knows the exact rewrite — a missing `move`, an argument
    /// order that is a permutation of the parameters — carries it here so
    /// `yashyc fix`, the LSP code action and SARIF all get the same edit
    /// without re-deriving it from the message.
    pub suggestions: Vec<Suggestion>,
}

impl TypeError {
    pub fn new(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
            label: None,
            notes: Vec::new(),
            code: None,
            suggestions: Vec::new(),
        }
    }

    /// Attach a label, and return the error so it can be built in one
    /// expression.
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Attach a note, and return the error.
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    /// Attach the stable catalog code, and return the error.
    pub fn with_code(mut self, code: &'static str) -> Self {
        self.code = Some(code);
        self
    }

    /// Attach machine-applicable edits, and return the error.
    pub fn with_suggestions(mut self, suggestions: Vec<Suggestion>) -> Self {
        self.suggestions = suggestions;
        self
    }

    /// Turn this type error into a diagnostic attached to `file`.
    pub fn into_diagnostic(&self, file: FileId) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            message: self.message.clone(),
            file,
            span: self.span,
            label: self.label.clone(),
            notes: self.notes.clone(),
            code: self.code,
            proof: None,
            suggestions: self.suggestions.clone(),
        }
    }
}
