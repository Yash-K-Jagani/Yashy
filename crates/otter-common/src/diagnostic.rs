//! Diagnostics: the messages the compiler reports, and the bag that collects
//! them during a run.
//!
//! Every stage of the pipeline reports problems this way instead of writing to
//! the console itself, so the driver decides when and where anything is
//! printed, and a stage can be tested by inspecting its [`DiagnosticBag`].

use crate::source::FileId;
use crate::span::Span;

/// The process exit code for a successful compile.
pub const EXIT_OK: i32 = 0;

/// The process exit code for a compile that produced errors.
pub const EXIT_COMPILE_ERROR: i32 = 1;

/// How serious a diagnostic is.
///
/// The variants are declared in increasing order of severity, so the derived
/// `Ord` puts errors above warnings: the bag can then report "is the worst
/// thing here an error?" with a plain comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Compilation is fine, but something is worth saying.
    Warning,
    /// Compilation cannot produce a correct program.
    Error,
}

impl Severity {
    /// The word used when rendering, lowercase like every other compiler.
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One thing the compiler wants to tell the user about a piece of source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    /// The message, without any location prefix -- [`crate::SourceMap::render`]
    /// adds that.
    pub message: String,
    /// The file the message is about.
    pub file: FileId,
    /// The source the caret points at. [`Span::SYNTHETIC`] means the message
    /// is about the compiler's own output rather than anything the user wrote,
    /// and renders at the start of the file.
    pub span: Span,
    /// An optional note placed after the caret, such as a suggested fix.
    pub label: Option<String>,
}

impl Diagnostic {
    /// A diagnostic of the given severity at `span`, with no label.
    pub fn new(severity: Severity, file: FileId, span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic { severity, message: message.into(), file, span, label: None }
    }

    /// Adds a label, and returns the diagnostic so it can be built in one
    /// expression.
    pub fn with_label(mut self, label: impl Into<String>) -> Diagnostic {
        self.label = Some(label.into());
        self
    }

    /// True for [`Severity::Error`].
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// The diagnostics collected so far, in the order they were reported.
///
/// Ordering is not cosmetic: "cannot find value `x`" followed by "cannot find
/// value `y`" is far easier to act on than the same two messages in whatever
/// order two passes happened to run in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiagnosticBag {
    diags: Vec<Diagnostic>,
}

impl DiagnosticBag {
    /// An empty bag.
    pub fn new() -> DiagnosticBag {
        DiagnosticBag { diags: Vec::new() }
    }

    /// Appends a diagnostic.
    pub fn push(&mut self, d: Diagnostic) {
        self.diags.push(d);
    }

    /// True when nothing has been reported.
    pub fn is_empty(&self) -> bool {
        self.diags.is_empty()
    }

    /// How many diagnostics have been reported.
    pub fn len(&self) -> usize {
        self.diags.len()
    }

    /// The diagnostics in the order they were reported.
    pub fn iter(&self) -> std::slice::Iter<'_, Diagnostic> {
        self.diags.iter()
    }

    /// Consumes the bag and returns the diagnostics.
    pub fn into_vec(self) -> Vec<Diagnostic> {
        self.diags
    }

    /// How many of the diagnostics are errors.
    ///
    /// Warnings are not failures, so this is what decides between
    /// [`EXIT_OK`] and [`EXIT_COMPILE_ERROR`].
    pub fn error_count(&self) -> usize {
        self.diags.iter().filter(|d| d.is_error()).count()
    }
}

impl<'a> IntoIterator for &'a DiagnosticBag {
    type Item = &'a Diagnostic;
    type IntoIter = std::slice::Iter<'a, Diagnostic>;

    fn into_iter(self) -> Self::IntoIter {
        self.diags.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::{Diagnostic, DiagnosticBag, Severity};
    use crate::source::{FileId, SourceMap};
    use crate::span::Span;

    fn a_diagnostic(severity: Severity) -> Diagnostic {
        Diagnostic::new(severity, FileId::from_index(0), Span::new(0, 1), "something")
    }

    #[test]
    fn severity_renders_as_a_lowercase_word() {
        assert_eq!(Severity::Error.as_str(), "error");
        assert_eq!(Severity::Warning.as_str(), "warning");
        assert_eq!(Severity::Error.to_string(), "error");
    }

    #[test]
    fn a_label_can_be_attached_without_a_second_statement() {
        let d = a_diagnostic(Severity::Error).with_label("try `let x = 1;`");
        assert_eq!(d.label.as_deref(), Some("try `let x = 1;`"));
        assert!(a_diagnostic(Severity::Error).label.is_none());
    }

    #[test]
    fn errors_sort_above_warnings() {
        assert!(Severity::Error > Severity::Warning);
        assert!(Severity::Warning < Severity::Error);
    }

    #[test]
    fn a_default_bag_is_empty() {
        let bag = DiagnosticBag::default();
        assert!(bag.is_empty());
        assert_eq!(bag.len(), 0);
        assert_eq!(bag.error_count(), 0);
        assert_eq!(bag.iter().count(), 0);
        assert!(bag.into_vec().is_empty());
    }

    #[test]
    fn a_bag_keeps_push_order_and_counts_errors() {
        let mut bag = DiagnosticBag::new();
        let error = a_diagnostic(Severity::Error);
        let warning = a_diagnostic(Severity::Warning);
        bag.push(warning.clone());
        bag.push(error.clone());

        assert_eq!(bag.len(), 2);
        assert_eq!(bag.error_count(), 1);
        assert_eq!(bag.iter().collect::<Vec<_>>(), vec![&warning, &error]);
        assert_eq!(bag.into_vec(), vec![warning, error]);
    }

    #[test]
    fn iterating_a_bag_by_reference_yields_the_diagnostics() {
        let mut sm = SourceMap::new();
        let file = sm.add_file("main.ot", "fn main() {}\n".to_string());
        let mut bag = DiagnosticBag::new();
        bag.push(Diagnostic::new(Severity::Warning, file, Span::new(3, 7), "unused"));

        let seen: Vec<&str> = (&bag).into_iter().map(|d| d.message.as_str()).collect();
        assert_eq!(seen, vec!["unused"]);
    }
}