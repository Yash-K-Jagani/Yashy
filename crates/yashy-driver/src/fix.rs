//! Apply machine-applicable suggestions from diagnostics.
//!
//! One edit model serves the CLI, the LSP and agents: a byte span, a
//! replacement and an applicability class. Edits are applied back-to-front,
//! must not overlap, and only to text whose hash matches the hash the
//! diagnostic was computed from.

use yashy_common::{Applicability, Suggestion};

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
