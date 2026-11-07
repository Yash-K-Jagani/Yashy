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
/// `==` or `!=` inside a comparison chain.
pub const CHAINED_EQUALITY: &str = "E0101";
/// A field default is not a literal or a `const` name.
pub const FIELD_DEFAULT_NOT_CONST: &str = "E0102";
/// A bitwise operator is applied to a non-integer (or non-bool for the logical forms).
pub const BITWISE_NOT_INTEGER: &str = "E0103";
/// A bitwise operator is applied to two different types.
pub const BITWISE_MIXED_TYPES: &str = "E0104";
/// A shift count is outside the valid range for the shifted type.
pub const SHIFT_OUT_OF_RANGE: &str = "E0105";
/// `repr(C)` is applied to something that has no C layout, or names a
/// representation that does not exist.
pub const REPR_NOT_ALLOWED: &str = "E0106";
/// `sizeof` or `alignof` names no type this program can see.
pub const SIZE_OF_UNKNOWN: &str = "E0107";
/// `offsetof` asks for a field of a type whose layout is not C's.
pub const OFFSET_OF_NOT_REPR_C: &str = "E0108";
/// A declaration writes the name before the type: `let x: int`.
pub const TYPE_AFTER_NAME: &str = "E0109";
/// A struct or enum variant has no such field.
pub const UNKNOWN_FIELD: &str = "E0200";
/// A struct literal or pattern leaves a field out.
pub const MISSING_FIELD: &str = "E0201";
/// A struct literal writes one field twice.
pub const DUPLICATE_FIELD: &str = "E0202";
/// A `Result` is used where its success value is expected.
pub const MISSING_QUESTION: &str = "E0300";
/// `?` propagates a failure kind the enclosing function cannot return.
pub const MIXED_TRY: &str = "E0301";
/// A `const` binding is assigned after its declaration.
pub const CONST_REASSIGNMENT: &str = "E0400";
/// A linear value is used after it was moved or freed.
pub const MOVED_VALUE: &str = "E0500";
/// A `switch` does not cover every case.
pub const NON_EXHAUSTIVE_MATCH: &str = "E0600";
/// A `switch` arm matches no value because an earlier arm covers it.
pub const UNREACHABLE_ARM: &str = "E0601";
/// A range pattern's bounds are not both `int` or both `char`, or `lo > hi`.
pub const BAD_RANGE_PATTERN: &str = "E0602";
/// An `if let` or `while let` pattern matches every value of its type.
pub const IRREFUTABLE_IF_LET: &str = "E0603";
/// `break` or `continue` names a label no enclosing loop has.
pub const UNKNOWN_LABEL: &str = "E0604";
/// Two nested loops carry the same label.
pub const DUPLICATE_LABEL: &str = "E0605";
/// A call passes the wrong number of arguments.
pub const WRONG_ARGUMENT_COUNT: &str = "E0700";
/// A named argument names no parameter, or a positional argument follows a
/// named one.
pub const UNKNOWN_NAMED_ARGUMENT: &str = "E0701";
/// A parameter is given twice in one call.
pub const DUPLICATE_ARGUMENT: &str = "E0702";
/// Two things claim the same linker symbol: two `extern` declarations that
/// disagree about the signature, or a declaration and a function the program
/// defines.
pub const DUPLICATE_EXTERN_SYMBOL: &str = "E0900";
/// A `link_name` is misplaced or malformed.
pub const MALFORMED_EXTERN: &str = "E0901";
/// `...` is written where a variadic marker cannot appear.
pub const VARIADIC_MISPLACED: &str = "E0902";
/// A linear value is never released.
pub const LEAK: &str = "E1000";
/// An arena-owned value crosses its block's boundary.
pub const ARENA_ESCAPE: &str = "E1003";
/// An arena-owned value is freed individually.
pub const ARENA_FREE: &str = "E1004";
/// A value spread into a literal is used again afterwards.
pub const PARTIAL_MOVE: &str = "E1002";
/// A contract clause is not proved.
pub const CONTRACT: &str = "E1100";
/// Checked integer arithmetic is provably out of range.
pub const OVERFLOW: &str = "E1201";
/// A grapheme index is provably out of range.
pub const GRAPHEME_OOB: &str = "E1200";
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
/// `impl Trait for Type` names a trait that is not declared.
pub const UNKNOWN_TRAIT: &str = "E1500";
/// Two impls of one trait for one type.
pub const CONFLICTING_IMPL: &str = "E1501";
/// An impl does not provide exactly the methods its trait declares.
pub const INCOMPLETE_IMPL: &str = "E1502";
/// An argument type does not satisfy a type parameter's bound, or a method
/// on a type parameter is not provided by any of its bounds.
pub const UNSATISFIED_BOUND: &str = "E1503";
/// A method name is provided by two trait impls for one type.
pub const AMBIGUOUS_METHOD: &str = "E1504";
/// A type path names a method that needs a receiver.
pub const METHOD_THROUGH_TYPE: &str = "E1506";
/// A method calling an associated function through a value.
pub const ASSOC_THROUGH_VALUE: &str = "E1507";
/// A type path names a function the type does not declare.
pub const UNKNOWN_ASSOCIATED_FN: &str = "E1508";
/// A generic type is used with the wrong number of type arguments.
pub const WRONG_TYPE_ARG_COUNT: &str = "E1509";
/// A declared type parameter is used by no field or payload.
pub const UNUSED_TYPE_PARAMETER: &str = "E1510";
/// A user trait carries type parameters.
pub const GENERIC_TRAIT: &str = "E1511";
/// A statement cannot be reached.
pub const UNREACHABLE_CODE: &str = "E2002";
/// A function parameter is never read.
pub const UNUSED_PARAMETER: &str = "E2003";
/// A doc comment is written where no item follows.
pub const DOC_COMMENT_MISPLACED: &str = "E2100";
/// Modules import each other in a cycle.
pub const MODULE_CYCLE: &str = "E0802";
/// A non-`pub` item is used from another module.
pub const PRIVATE_ITEM: &str = "E0801";

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
        id: CHAINED_EQUALITY,
        title: "equality in a comparison chain",
        summary: "`==` and `!=` cannot appear in a chained comparison; write the two comparisons separately with `&&`.",
        page: include_str!("../../../docs/errors/E0101.md"),
    },
    Code {
        id: FIELD_DEFAULT_NOT_CONST,
        title: "field default is not constant",
        summary: "A struct field default must be a literal or a `const` name.",
        page: include_str!("../../../docs/errors/E0102.md"),
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
        id: SHIFT_OUT_OF_RANGE,
        title: "shift count out of range",
        summary: "A shift count is outside `0..bits` for the type being shifted.",
        page: include_str!("../../../docs/errors/E0105.md"),
    },
    Code {
        id: REPR_NOT_ALLOWED,
        title: "repr(C) is not allowed here",
        summary: "`repr(C)` was applied to a type that has no C layout, or named a representation that does not exist.",
        page: include_str!("../../../docs/errors/E0106.md"),
    },
    Code {
        id: SIZE_OF_UNKNOWN,
        title: "sizeof or alignof names no known type",
        summary: "`sizeof` and `alignof` take the name of a declared type.",
        page: include_str!("../../../docs/errors/E0107.md"),
    },
    Code {
        id: OFFSET_OF_NOT_REPR_C,
        title: "offsetof needs a repr(C) struct",
        summary: "`offsetof` takes a `repr(C)` struct and one of its fields.",
        page: include_str!("../../../docs/errors/E0108.md"),
    },
    Code {
        id: TYPE_AFTER_NAME,
        title: "the type is written before the name",
        summary: "A binding and a parameter write the type first: `let int x`, not `let x: int`.",
        page: include_str!("../../../docs/errors/E0109.md"),
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
        id: DUPLICATE_FIELD,
        title: "duplicate field",
        summary: "A struct literal writes one field twice.",
        page: include_str!("../../../docs/errors/E0202.md"),
    },
    Code {
        id: MISSING_QUESTION,
        title: "missing `?`",
        summary: "A `Result` is used where its success value is expected.",
        page: include_str!("../../../docs/errors/E0300.md"),
    },
    Code {
        id: MIXED_TRY,
        title: "mixed `?`",
        summary: "`?` propagates a failure kind the enclosing function cannot return.",
        page: include_str!("../../../docs/errors/E0301.md"),
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
        id: UNREACHABLE_ARM,
        title: "unreachable switch arm",
        summary: "A `switch` arm matches no value because an earlier arm already covers it.",
        page: include_str!("../../../docs/errors/E0601.md"),
    },
    Code {
        id: BAD_RANGE_PATTERN,
        title: "invalid range pattern",
        summary: "A range pattern's bounds are not both `int` or both `char`, or its start is greater than its end.",
        page: include_str!("../../../docs/errors/E0602.md"),
    },
    Code {
        id: IRREFUTABLE_IF_LET,
        title: "`if let` pattern always matches",
        summary: "An `if let` or `while let` pattern matches every value of its type, so the branch or loop exit can never run.",
        page: include_str!("../../../docs/errors/E0603.md"),
    },
    Code {
        id: UNKNOWN_LABEL,
        title: "unknown loop label",
        summary: "`break` or `continue` names a label that no enclosing loop carries.",
        page: include_str!("../../../docs/errors/E0604.md"),
    },
    Code {
        id: DUPLICATE_LABEL,
        title: "duplicate loop label",
        summary: "Two loops nested inside each other carry the same label.",
        page: include_str!("../../../docs/errors/E0605.md"),
    },
    Code {
        id: WRONG_ARGUMENT_COUNT,
        title: "wrong argument count",
        summary: "A call passes the wrong number of arguments.",
        page: include_str!("../../../docs/errors/E0700.md"),
    },
    Code {
        id: UNKNOWN_NAMED_ARGUMENT,
        title: "unknown named argument",
        summary: "A named argument names no parameter of the callee, or a positional argument follows a named one.",
        page: include_str!("../../../docs/errors/E0701.md"),
    },
    Code {
        id: DUPLICATE_ARGUMENT,
        title: "duplicate argument",
        summary: "A parameter is given twice in one call.",
        page: include_str!("../../../docs/errors/E0702.md"),
    },
    Code {
        id: DUPLICATE_EXTERN_SYMBOL,
        title: "duplicate extern symbol",
        summary: "Two `extern` declarations resolve to one symbol with different signatures.",
        page: include_str!("../../../docs/errors/E0900.md"),
    },
    Code {
        id: MALFORMED_EXTERN,
        title: "malformed extern declaration",
        summary: "A `link_name` is written where it is not allowed, or is not a non-empty string.",
        page: include_str!("../../../docs/errors/E0901.md"),
    },
    Code {
        id: VARIADIC_MISPLACED,
        title: "misplaced variadic marker",
        summary: "`...` is written on a Yashy `fn`, in a function type, or somewhere other than the end of an `extern` declaration's parameters.",
        page: include_str!("../../../docs/errors/E0902.md"),
    },
    Code {
        id: LEAK,
        title: "leaked linear value",
        summary: "A linear value is never released.",
        page: include_str!("../../../docs/errors/E1000.md"),
    },
    Code {
        id: PARTIAL_MOVE,
        title: "spread operand used after the literal",
        summary: "A value spread into a struct or array literal has been moved as a whole and cannot be used again.",
        page: include_str!("../../../docs/errors/E1002.md"),
    },
    Code {
        id: ARENA_ESCAPE,
        title: "arena value escapes its block",
        summary: "A value allocated inside an `arena { }` block is returned, stored in an outer binding, or handed to a thread.",
        page: include_str!("../../../docs/errors/E1003.md"),
    },
    Code {
        id: ARENA_FREE,
        title: "arena value freed individually",
        summary: "A value owned by an `arena { }` block is freed with `.free()`, but the block frees it all at exit.",
        page: include_str!("../../../docs/errors/E1004.md"),
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
        id: UNKNOWN_TRAIT,
        title: "unknown trait",
        summary: "`impl Trait for Type` names a trait that is not declared.",
        page: include_str!("../../../docs/errors/E1500.md"),
    },
    Code {
        id: CONFLICTING_IMPL,
        title: "conflicting trait impl",
        summary: "One trait has one implementation per type; a second one is refused.",
        page: include_str!("../../../docs/errors/E1501.md"),
    },
    Code {
        id: INCOMPLETE_IMPL,
        title: "incomplete trait impl",
        summary: "An `impl Trait for Type` does not provide exactly the methods its trait declares, with matching signatures.",
        page: include_str!("../../../docs/errors/E1502.md"),
    },
    Code {
        id: UNSATISFIED_BOUND,
        title: "unsatisfied bound",
        summary: "An argument type does not implement a type parameter's bound, or a method on a type parameter is not provided by any of its bounds.",
        page: include_str!("../../../docs/errors/E1503.md"),
    },
    Code {
        id: AMBIGUOUS_METHOD,
        title: "ambiguous method",
        summary: "Two trait impls for one type provide the same method, so the call has two targets.",
        page: include_str!("../../../docs/errors/E1504.md"),
    },
    Code {
        id: METHOD_THROUGH_TYPE,
        title: "method called through a type",
        summary: "A type path names a method that needs a receiver; call it on a value.",
        page: include_str!("../../../docs/errors/E1506.md"),
    },
    Code {
        id: ASSOC_THROUGH_VALUE,
        title: "associated function called through a value",
        summary: "An associated function has no receiver; call it through the type.",
        page: include_str!("../../../docs/errors/E1507.md"),
    },
    Code {
        id: UNKNOWN_ASSOCIATED_FN,
        title: "unknown associated function",
        summary: "A type path names a function the type does not declare.",
        page: include_str!("../../../docs/errors/E1508.md"),
    },
    Code {
        id: WRONG_TYPE_ARG_COUNT,
        title: "wrong number of type arguments",
        summary: "A generic type is written with a different number of type arguments than it declares, or with none.",
        page: include_str!("../../../docs/errors/E1509.md"),
    },
    Code {
        id: UNUSED_TYPE_PARAMETER,
        title: "unused type parameter",
        summary: "A generic type declares a type parameter that no field or variant payload uses.",
        page: include_str!("../../../docs/errors/E1510.md"),
    },
    Code {
        id: GENERIC_TRAIT,
        title: "generic trait outside the standard library",
        summary: "A user trait may not carry type parameters; that spelling is reserved for the standard library's iterator protocol.",
        page: include_str!("../../../docs/errors/E1511.md"),
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
    Code {
        id: DOC_COMMENT_MISPLACED,
        title: "doc comment with no item",
        summary: "A `///` doc comment must sit above an item, not a statement or the end of the file.",
        page: include_str!("../../../docs/errors/E2100.md"),
    },
    Code {
        id: MODULE_CYCLE,
        title: "import cycle",
        summary: "Modules import each other in a cycle, which has no loading order.",
        page: include_str!("../../../docs/errors/E0802.md"),
    },
    Code {
        id: PRIVATE_ITEM,
        title: "private item used from another module",
        summary: "A non-`pub` item or field is used outside its module.",
        page: include_str!("../../../docs/errors/E0801.md"),
    },
    Code {
        id: GRAPHEME_OOB,
        title: "grapheme index out of range",
        summary: "A `char_at` index is provably outside the string's cluster count.",
        page: include_str!("../../../docs/errors/E1200.md"),
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
