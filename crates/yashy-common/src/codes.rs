//! Stable error codes and the catalog pages they name.
//!
//! A code is assigned where the diagnostic is built, never inferred from the
//! message text: two diagnostics that happen to share a sentence are not
//! necessarily the same mistake, and a code that changes when a message is
//! reworded is not stable.
//!
//! The long-form page is embedded with [`include_str!`], so the catalog is
//! available offline and a code without a page does not compile.

/// One entry in the error catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Code {
    /// The stable identifier: `E` followed by four digits.
    pub id: &'static str,
    /// A short name for the mistake.
    pub title: &'static str,
    /// One sentence explaining the mistake.
    pub summary: &'static str,
    /// The long-form page, embedded in the binary.
    pub page: &'static str,
}

/// A value or function name is not in scope.
pub const UNKNOWN_NAME: &str = "E0001";
/// A type name is not declared or imported.
pub const UNKNOWN_TYPE: &str = "E0002";
/// An expression's type does not match its context.
pub const TYPE_MISMATCH: &str = "E0100";
/// A bitwise operator is applied to a non-integer (or non-bool for the logical forms).
pub const BITWISE_NOT_INTEGER: &str = "E0103";
/// A bitwise operator is applied to two different types.
pub const BITWISE_MIXED_TYPES: &str = "E0104";
/// A struct or enum variant has no such field.
pub const UNKNOWN_FIELD: &str = "E0200";
/// A struct literal or pattern leaves a field out.
pub const MISSING_FIELD: &str = "E0201";
/// A `Result` is used where its success value is expected.
pub const MISSING_QUESTION: &str = "E0300";
/// A `const` binding is assigned after its declaration.
pub const CONST_REASSIGNMENT: &str = "E0400";
/// A linear value is used after it was moved or freed.
pub const MOVED_VALUE: &str = "E0500";
/// A `switch` does not cover every case.
pub const NON_EXHAUSTIVE_MATCH: &str = "E0600";
/// A call passes the wrong number of arguments.
pub const WRONG_ARGUMENT_COUNT: &str = "E0700";
/// A linear value is never released.
pub const LEAK: &str = "E1000";
/// A contract clause is not proved.
pub const CONTRACT: &str = "E1100";
/// Checked integer arithmetic is provably out of range.
pub const OVERFLOW: &str = "E1201";
/// A declared effect is exceeded by the body.
pub const EFFECT: &str = "E1300";
/// An unsafe call outside an unsafe function.
pub const UNSAFE_CALL: &str = "E1301";
/// A resource is released twice, or released in the wrong state.
pub const DOUBLE_CLOSE: &str = "E1400";
/// A resource is used after it has been released.
pub const USE_AFTER_CLOSE: &str = "E1401";
/// A bare linear value is shared across a thread boundary.
pub const RACE: &str = "E1402";
/// A `const` or `let` binding is never read.
pub const UNUSED_BINDING: &str = "E2000";
/// An `import` is never used.
pub const UNUSED_IMPORT: &str = "E2001";
/// A statement cannot be reached.
pub const UNREACHABLE_CODE: &str = "E2002";
/// A function parameter is never read.
pub const UNUSED_PARAMETER: &str = "E2003";

/// Every R1 code, in catalog order.
pub const CODES: &[Code] = &[
    Code {
        id: UNKNOWN_NAME,
        title: "unknown name",
        summary: "A value or function name is not in scope.",
        page: include_str!("../../../docs/errors/E0001.md"),
    },
    Code {
        id: UNKNOWN_TYPE,
        title: "unknown type",
        summary: "A type name is not declared or imported.",
        page: include_str!("../../../docs/errors/E0002.md"),
    },
    Code {
        id: TYPE_MISMATCH,
        title: "type mismatch",
        summary: "An expression's type does not match its context.",
        page: include_str!("../../../docs/errors/E0100.md"),
    },
    Code {
        id: BITWISE_NOT_INTEGER,
        title: "bitwise operand is not an integer",
        summary: "A bitwise operator was applied to a value that is not an integer.",
        page: include_str!("../../../docs/errors/E0103.md"),
    },
    Code {
        id: BITWISE_MIXED_TYPES,
        title: "bitwise operands have different types",
        summary: "A bitwise operator was applied to two integers of different types.",
        page: include_str!("../../../docs/errors/E0104.md"),
    },
    Code {
        id: UNKNOWN_FIELD,
        title: "unknown field",
        summary: "A struct or enum variant has no such field.",
        page: include_str!("../../../docs/errors/E0200.md"),
    },
    Code {
        id: MISSING_FIELD,
        title: "missing field",
        summary: "A struct literal or pattern leaves a field out.",
        page: include_str!("../../../docs/errors/E0201.md"),
    },
    Code {
        id: MISSING_QUESTION,
        title: "missing `?`",
        summary: "A `Result` is used where its success value is expected.",
        page: include_str!("../../../docs/errors/E0300.md"),
    },
    Code {
        id: CONST_REASSIGNMENT,
        title: "const reassignment",
        summary: "A `const` binding is assigned after its declaration.",
        page: include_str!("../../../docs/errors/E0400.md"),
    },
    Code {
        id: MOVED_VALUE,
        title: "moved value",
        summary: "A linear value is used after it was moved or freed.",
        page: include_str!("../../../docs/errors/E0500.md"),
    },
    Code {
        id: NON_EXHAUSTIVE_MATCH,
        title: "non-exhaustive match",
        summary: "A `switch` does not cover every case.",
        page: include_str!("../../../docs/errors/E0600.md"),
    },
    Code {
        id: WRONG_ARGUMENT_COUNT,
        title: "wrong argument count",
        summary: "A call passes the wrong number of arguments.",
        page: include_str!("../../../docs/errors/E0700.md"),
    },
    Code {
        id: LEAK,
        title: "leaked linear value",
        summary: "A linear value is never released.",
        page: include_str!("../../../docs/errors/E1000.md"),
    },
    Code {
        id: CONTRACT,
        title: "undischarged contract",
        summary: "A contract clause is not proved.",
        page: include_str!("../../../docs/errors/E1100.md"),
    },
    Code {
        id: OVERFLOW,
        title: "provable integer overflow",
        summary: "Checked integer arithmetic always overflows.",
        page: include_str!("../../../docs/errors/E1201.md"),
    },
    Code {
        id: EFFECT,
        title: "effect exceeded",
        summary: "A declared effect is exceeded by the body.",
        page: include_str!("../../../docs/errors/E1300.md"),
    },
    Code {
        id: UNSAFE_CALL,
        title: "unsafe call in a declared function",
        summary: "An `unsafe` call appears in a function that declared it would not make one.",
        page: include_str!("../../../docs/errors/E1301.md"),
    },
    Code {
        id: DOUBLE_CLOSE,
        title: "double close",
        summary: "A resource is released twice, or released in the wrong state.",
        page: include_str!("../../../docs/errors/E1400.md"),
    },
    Code {
        id: USE_AFTER_CLOSE,
        title: "use after close",
        summary: "A resource is used after it has been released.",
        page: include_str!("../../../docs/errors/E1401.md"),
    },
    Code {
        id: RACE,
        title: "race",
        summary: "A bare linear value is shared across a thread boundary.",
        page: include_str!("../../../docs/errors/E1402.md"),
    },
    Code {
        id: UNUSED_BINDING,
        title: "unused binding",
        summary: "A `const` or `let` binding is never read.",
        page: include_str!("../../../docs/errors/E2000.md"),
    },
    Code {
        id: UNUSED_IMPORT,
        title: "unused import",
        summary: "An `import` is never used.",
        page: include_str!("../../../docs/errors/E2001.md"),
    },
    Code {
        id: UNREACHABLE_CODE,
        title: "unreachable code",
        summary: "A statement cannot be reached.",
        page: include_str!("../../../docs/errors/E2002.md"),
    },
    Code {
        id: UNUSED_PARAMETER,
        title: "unused parameter",
        summary: "A function parameter is never read.",
        page: include_str!("../../../docs/errors/E2003.md"),
    },
];

/// The catalog entry for `id`, if the code exists.
pub fn find(id: &str) -> Option<&'static Code> {
    CODES.iter().find(|code| code.id == id)
}

/// The catalog entry nearest to `id`, for a mistyped code.
///
/// This is the same distance rule the name suggestions use, so `E010` offers
/// `E0100` and an unrelated string offers nothing.
pub fn closest(id: &str) -> Option<&'static Code> {
    let candidate = crate::closest(id, CODES.iter().map(|code| code.id))?;
    find(&candidate)
}

/// The public catalog page for a code.
pub fn docs_url(id: &str) -> String {
    format!("https://yashy.dev/errors/{id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_has_a_unique_id_and_an_embedded_page() {
        let mut seen = std::collections::HashSet::new();
        for code in CODES {
            assert!(seen.insert(code.id), "duplicate code {}", code.id);
            assert!(
                code.id.len() == 5 && code.id.starts_with('E'),
                "code {} is not E plus four digits",
                code.id
            );
            assert!(!code.title.is_empty(), "{} has no title", code.id);
            assert!(!code.summary.is_empty(), "{} has no summary", code.id);
            assert!(
                code.page.starts_with("# "),
                "{} has no Markdown heading",
                code.id
            );
            assert!(
                code.page.contains(code.id),
                "{}'s page does not name the code",
                code.id
            );
            assert!(
                code.page.contains("## Cause")
                    && code.page.contains("## Example")
                    && code.page.contains("## Fix"),
                "{}'s page is missing cause, example, or fix",
                code.id
            );
        }
    }

    #[test]
    fn find_returns_the_entry_and_unknown_codes_are_none() {
        assert_eq!(find("E0001").map(|code| code.title), Some("unknown name"));
        assert_eq!(find("E0100").map(|code| code.title), Some("type mismatch"));
        assert!(find("E9999").is_none());
        assert!(find("0100").is_none());
    }

    #[test]
    fn a_near_miss_offers_the_nearest_code() {
        assert_eq!(closest("E010").map(|code| code.id), Some("E0100"));
        assert!(closest("E9999").is_none());
        assert!(closest("not-a-code").is_none());
    }

    #[test]
    fn the_docs_url_is_the_catalog_page() {
        assert_eq!(docs_url("E0100"), "https://yashy.dev/errors/E0100");
    }
}
