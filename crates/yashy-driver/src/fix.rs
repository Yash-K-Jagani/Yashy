//! Apply machine-applicable suggestions from diagnostics.
//!
//! One edit model serves the CLI, the LSP and agents: a byte span, a
//! replacement and an applicability class. Edits are applied back-to-front,
//! must not overlap, and only to text whose hash matches the hash the
//! diagnostic was computed from.

use yashy_common::{Applicability, Diagnostic, SourceMap, Span, Suggestion};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FixError {
    Overlapping,
    StaleSource,
    OutOfBounds,
}

impl std::fmt::Display for FixError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FixError::Overlapping => write!(f, "two suggestions overlap; refusing to guess"),
            FixError::StaleSource => {
                write!(f, "the source changed since the diagnostic was computed")
            }
            FixError::OutOfBounds => write!(f, "a suggestion points outside the source"),
        }
    }
}

/// Turn a "did you mean `name`?" note into a machine-applicable edit.
///
/// The checker already knows the nearest name; this only turns the fact it
/// printed into the edit model the CLI, SARIF and the LSP share. It never
/// guesses a name the compiler did not suggest, and it refuses to rewrite a
/// span whose source text is no longer an identifier.
pub fn attach_typo_suggestions(sources: &SourceMap, diagnostics: &mut [Diagnostic]) {
    for diagnostic in diagnostics.iter_mut() {
        if !diagnostic.suggestions.is_empty() {
            continue;
        }
        let Some(near) = suggested_name(&diagnostic.message) else {
            continue;
        };
        let text = sources.file(diagnostic.file);
        let start = diagnostic.span.start as usize;
        if text.is_empty()
            || start >= text.len()
            || !text.as_bytes()[start].is_ascii_alphabetic() && text.as_bytes()[start] != b'_'
        {
            continue;
        }
        let mut end = start;
        while end < text.len() {
            let byte = text.as_bytes()[end];
            if byte.is_ascii_alphanumeric() || byte == b'_' {
                end += 1;
            } else {
                break;
            }
        }
        let written = &text[start..end];
        if written == near || near.is_empty() {
            continue;
        }
        diagnostic.suggestions.push(Suggestion {
            span: Span::new(start as u32, end as u32),
            replacement: near.to_string(),
            applicability: Applicability::MachineApplicable,
            message: format!("use `{near}`"),
        });
    }
}

fn suggested_name(message: &str) -> Option<&str> {
    let at = message.find("did you mean `")? + "did you mean `".len();
    let rest = &message[at..];
    let close = rest.find('`')?;
    Some(&rest[..close])
}

/// A stable hash of the bytes a diagnostic was computed from.
pub fn source_hash(source: &str) -> u64 {
    let mut state = 0xcbf2_9ce4_8422_2325u64;
    for byte in source.as_bytes() {
        state ^= u64::from(*byte);
        state = state.wrapping_mul(0x100_0000_01b3);
    }
    state
}

/// Apply `suggestions` to `source`, checking the expected hash first.
pub fn apply_suggestions_checked(
    source: &str,
    suggestions: &[Suggestion],
    expected_hash: Option<u64>,
) -> Result<String, FixError> {
    if expected_hash.is_some_and(|hash| hash != source_hash(source)) {
        return Err(FixError::StaleSource);
    }
    apply_suggestions(source, suggestions)
}

/// Apply `suggestions` to `source` without a hash check.
pub fn apply_suggestions(source: &str, suggestions: &[Suggestion]) -> Result<String, FixError> {
    let mut edits: Vec<&Suggestion> = suggestions
        .iter()
        .filter(|suggestion| suggestion.applicability == Applicability::MachineApplicable)
        .collect();
    edits.sort_by_key(|suggestion| std::cmp::Reverse(suggestion.span.start));
    let mut result = source.to_string();
    let mut previous_start = usize::MAX;
    for edit in edits {
        let start = edit.span.start as usize;
        let end = edit.span.end as usize;
        if end > source.len() || start > end {
            return Err(FixError::OutOfBounds);
        }
        if end > previous_start {
            return Err(FixError::Overlapping);
        }
        // String byte indices must land on char boundaries; all generated
        // replacements take whole ASCII keywords, but a stale span must be
        // refused rather than producing invalid UTF-8.
        if !source.is_char_boundary(start) || !source.is_char_boundary(end) {
            return Err(FixError::OutOfBounds);
        }
        result.replace_range(start..end, &edit.replacement);
        previous_start = start;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use yashy_common::{Diagnostic, FileId, Severity, SourceMap};

    #[test]
    fn a_did_you_mean_message_becomes_a_machine_applicable_edit() {
        let mut sources = SourceMap::new();
        let file = sources.add_file(
            "main.ya".to_string(),
            "fn main() { let x = 1; retun x; }\n".to_string(),
        );
        let mut diagnostic = Diagnostic::new(
            Severity::Error,
            file,
            yashy_common::Span::new(23, 28),
            "unknown name `retun`; did you mean `return`?".to_string(),
        );
        attach_typo_suggestions(&sources, std::slice::from_mut(&mut diagnostic));
        assert_eq!(diagnostic.suggestions.len(), 1);
        assert_eq!(diagnostic.suggestions[0].replacement, "return");
        assert_eq!(
            apply_suggestions(sources.file(file), &diagnostic.suggestions).unwrap(),
            "fn main() { let x = 1; return x; }\n"
        );
    }

    #[test]
    fn a_message_without_a_suggestion_adds_no_edit() {
        let mut sources = SourceMap::new();
        let file = sources.add_file("main.ya".to_string(), "fn main() {}\n".to_string());
        let mut diagnostic = Diagnostic::new(
            Severity::Error,
            file,
            yashy_common::Span::new(12, 13),
            "type mismatch".to_string(),
        );
        attach_typo_suggestions(&sources, std::slice::from_mut(&mut diagnostic));
        assert!(diagnostic.suggestions.is_empty());
    }
}
