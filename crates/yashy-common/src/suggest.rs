//! "Did you mean" suggestions for names the compiler could not find.
//!
//! A misspelling is the most common error a beginner makes, and the fix is
//! obvious once the compiler names the candidate. The distance is measured with
//! Damerau-Levenshtein (insert, delete, substitute, adjacent transpose),
//! case-insensitively, and a candidate is only offered when it is close enough
//! for the name's length -- a suggestion that is merely the nearest of a bad
//! set is worse than none.

/// The best candidate for `name`, or `None` when nothing is close enough.
///
/// `None` is also the answer for an exact match: the caller only asks after it
/// has failed to resolve the name, and echoing the name back would be noise.
pub fn closest<'a>(
    name: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    let lower = name.to_lowercase();
    let bound = bound_for(lower.chars().count());
    // The best candidate so far: its distance, whether it starts with the same
    // character as `name`, and the candidate itself.
    let mut best: Option<(usize, bool, &str)> = None;
    for candidate in candidates {
        if candidate.is_empty() {
            continue;
        }
        let candidate_lower = candidate.to_lowercase();
        // The caller only asks after a lookup failed, so a candidate that
        // matches exactly is the name it already tried. Echoing it back would
        // tell the reader nothing.
        if candidate_lower == lower {
            return None;
        }
        let distance = distance(&lower, &candidate_lower);
        if distance > bound {
            continue;
        }
        let same_first = first_char(&lower) == first_char(&candidate_lower);
        let better = match best {
            None => true,
            Some((best_distance, best_same_first, best_name)) => {
                distance < best_distance
                    || (distance == best_distance && same_first && !best_same_first)
                    || (distance == best_distance
                        && same_first == best_same_first
                        && candidate < best_name)
            }
        };
        if better {
            best = Some((distance, same_first, candidate));
        }
    }
    best.map(|(_, _, candidate)| candidate.to_string())
}

/// The largest edit distance worth suggesting for a name of `len` characters.
///
/// The bound grows with the name because one edit in a three-letter name is a
/// different word, while one edit in a twelve-letter name is usually a typo.
fn bound_for(len: usize) -> usize {
    if len <= 4 {
        1
    } else if len <= 8 {
        2
    } else {
        3
    }
}

fn first_char(text: &str) -> Option<char> {
    text.chars().next()
}

/// Damerau-Levenshtein distance, optimal-string-alignment variant.
///
/// Insert, delete, substitute and adjacent transpose each cost one. The inputs
/// are already lowercased by the caller.
fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    // `rows[i][j]` is the distance between the first `i` characters of `a` and
    // the first `j` characters of `b`. Keeping all rows is what makes the
    // transpose case possible: it looks two rows back.
    let mut rows: Vec<Vec<usize>> = vec![vec![0; b.len() + 1]; a.len() + 1];
    for (i, row) in rows.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=b.len() {
        rows[0][j] = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            let mut value = (rows[i - 1][j] + 1)
                .min(rows[i][j - 1] + 1)
                .min(rows[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                value = value.min(rows[i - 2][j - 2] + 1);
            }
            rows[i][j] = value;
        }
    }
    rows[a.len()][b.len()]
}

#[cfg(test)]
mod tests {
    use super::closest;

    #[test]
    fn an_exact_match_is_not_a_suggestion() {
        assert_eq!(closest("count", ["count", "counter"]), None);
    }

    #[test]
    fn one_substitution_is_suggested() {
        assert_eq!(closest("counx", ["count"]), Some("count".to_string()));
    }

    #[test]
    fn one_insertion_is_suggested() {
        assert_eq!(
            closest("prntln", ["println"]),
            Some("println".to_string())
        );
    }

    #[test]
    fn one_deletion_is_suggested() {
        assert_eq!(closest("printl", ["println"]), Some("println".to_string()));
    }

    #[test]
    fn an_adjacent_transpose_is_suggested() {
        assert_eq!(closest("cuont", ["count"]), Some("count".to_string()));
    }

    #[test]
    fn matching_ignores_case() {
        assert_eq!(closest("CouNt", ["count"]), None);
        assert_eq!(closest("Countt", ["count"]), Some("count".to_string()));
    }

    #[test]
    fn the_bound_depends_on_the_length_of_the_name() {
        // Four characters: one edit is the most that is offered.
        assert_eq!(closest("abcd", ["abce"]), Some("abce".to_string()));
        assert_eq!(closest("abcd", ["abxy"]), None);
        // Five to eight: two edits.
        assert_eq!(closest("abcde", ["abcxy"]), Some("abcxy".to_string()));
        assert_eq!(closest("abcde", ["abxyz"]), None);
        // More than eight: three edits.
        assert_eq!(
            closest("abcdefghij", ["abcdefgxyz"]),
            Some("abcdefgxyz".to_string())
        );
        assert_eq!(closest("abcdefghij", ["abcdefwxyz"]), None);
    }

    #[test]
    fn a_tie_prefers_the_candidate_with_the_same_first_character() {
        // Both are one edit away; `abd` keeps the `a`.
        assert_eq!(closest("abc", ["xbc", "abd"]), Some("abd".to_string()));
    }

    #[test]
    fn a_remaining_tie_is_broken_alphabetically() {
        assert_eq!(closest("abc", ["abe", "abd"]), Some("abd".to_string()));
    }

    #[test]
    fn an_unrelated_name_gets_no_suggestion() {
        assert_eq!(closest("count", ["println", "arguments"]), None);
    }

    #[test]
    fn an_empty_name_gets_no_suggestion() {
        assert_eq!(closest("", ["count"]), None);
    }
}
