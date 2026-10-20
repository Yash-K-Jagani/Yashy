//! The LLVM-level type mapping.
//!
//! One direction only: a checked [`TypeKind`] to the LLVM IR type that
//! represents it. The mapping is pinned by tests because a mismatch here is
//! an ABI break against the C runtime (ledger L1): the runtime's
//! `otter_print_i64` takes an `i64`, and codegen that emitted `i32` would
//! link cleanly and print garbage.

use otter_sema::TypeKind;

/// The LLVM IR type for a checked Otter type.
///
/// The five primitive cases are the whole M0–M2 mapping:
/// `i64`, `double`, `i1`, the `%otter_str` shape for `Str`, and `void` for
/// unit. `Never` is `void` too — a diverging expression produces no value.
///
/// Compound types (`Option`, `Result`, structs, enums, functions) do not
/// have a representation yet: Tasks 11–12 add it. Until then they map to
/// `void` so a program that reaches codegen with one fails loudly at the IR
/// level rather than being silently miscompiled — no accepted M0–M2 program
/// can produce a value of those types through the literal paths this task
/// covers.
pub fn llvm_type_string(t: &TypeKind) -> String {
    match t {
        TypeKind::I64 => "i64".to_string(),
        TypeKind::F64 => "double".to_string(),
        TypeKind::Bool => "i1".to_string(),
        TypeKind::Str => "{ i32, i32, i8* }".to_string(),
        TypeKind::Unit | TypeKind::Never => "void".to_string(),
        // Every pointer is `i8*` under LLVM's opaque pointers: the pointee type
        // exists in Otter's checker (a `*mut i64` is not a `*mut f64`) but not
        // in the IR, which is why `llvm_type_string` ignores the inner type.
        TypeKind::Ptr(_) | TypeKind::CPtr(_) => "i8*".to_string(),
        // A map value is the runtime's opaque table pointer.
        TypeKind::Map(_, _) => "i8*".to_string(),
        TypeKind::CInt(width) | TypeKind::CUInt(width) => match width {
            8 => "i8".to_string(),
            16 => "i16".to_string(),
            32 => "i32".to_string(),
            // 64 and anything the checker never constructs: `i64` is the only
            // width left that Otter can represent, and a silent fallback keeps
            // a bad width from becoming an LLVM parse error.
            _ => "i64".to_string(),
        },
        TypeKind::CFloat(32) => "float".to_string(),
        TypeKind::CFloat(_) => "double".to_string(),
        TypeKind::CSize => "i64".to_string(),
        TypeKind::CVoid => "void".to_string(),
        TypeKind::Slice(_) => "{ i8*, i64, i64 }".to_string(),
        TypeKind::Option(_)
        | TypeKind::Result(_, _)
        | TypeKind::Struct(_)
        | TypeKind::Enum(_)
        | TypeKind::Func(_, _) => "void".to_string(),
    }
}

/// The LLVM representation of a **value** of type `t` at M0–M2.
///
/// This differs from [`llvm_type_string`] in exactly two places, both
/// deliberate:
///
/// - `Str` is `i8*`: its data pointer. L4 pins Str as "an opaque literal
///   with no operations" until M3, and the M0–M2 runtime takes `i8*` (the
///   `%otter_str` shape exists in the header but nothing constructs one
///   yet). M3 switches values to `%otter_str` in a golden-regenerating
///   boundary.
/// - `Option` and `Result` are tagged structs. `Option[T]` is
///   `{ i8, repr(T) }` (tag: `None` = 0, `Some` = 1). `Result[T, E]` is
///   `{ i8, repr(T), repr(E) }` — two payload fields, one per variant, so
///   both halves keep their own representation with no bit-punning; the
///   matching arm reads field 1 (`Ok`, tag 0) or field 2 (`Err`, tag 1).
///
/// Struct and enum values are not represented yet: Task 12 threads the
/// declaration table they need. Until then `Struct` renders as its named
/// `%struct.Name` type (usable by `StructLit`), and `Enum` renders as a
/// placeholder `i8` tag — no M0–M2 test path can construct an enum value,
/// so the placeholder cannot be reached by a passing program.
pub fn repr_type(t: &TypeKind) -> String {
    match t {
        TypeKind::Str => "i8*".to_string(),
        TypeKind::Option(inner) => format!("{{ i8, {} }}", repr_type(&inner.kind)),
        TypeKind::Result(ok, err) => {
            format!(
                "{{ i8, {}, {} }}",
                repr_type(&ok.kind),
                repr_type(&err.kind)
            )
        }
        TypeKind::Struct(name) => format!("%struct.{name}"),
        TypeKind::Enum(_) => "i8".to_string(),
        TypeKind::Func(_, _) => "void".to_string(),
        TypeKind::Slice(_) => "{ i8*, i64, i64 }".to_string(),
        TypeKind::Map(_, _) => "i8*".to_string(),
        other => llvm_type_string(other),
    }
}

/// The number of bytes one value of `kind` occupies in memory.
///
/// This is the stride a slice uses for its elements and the size the literal
/// allocates. Scalars, strings, pointers and other slices are exact. `Option`,
/// `Result`, structs and enums fall back to one machine word: their real layout
/// needs the declaration table (an `Option[i64]` is two words, a struct is the
/// sum of its fields under LLVM's alignment rules), and M6's programs only slice
/// `i64`, `string` and other slices. A slice of a declared type needs that
/// layout work and is recorded as a limitation rather than silently sized
/// wrong in a test.
pub fn size_of(kind: &TypeKind) -> u64 {
    match kind {
        TypeKind::Bool | TypeKind::CInt(8) | TypeKind::CUInt(8) => 1,
        TypeKind::CInt(16) | TypeKind::CUInt(16) => 2,
        TypeKind::CInt(32) | TypeKind::CUInt(32) | TypeKind::CFloat(32) => 4,
        TypeKind::I64
        | TypeKind::F64
        | TypeKind::CSize
        | TypeKind::CFloat(_)
        | TypeKind::Str
        | TypeKind::Ptr(_)
        | TypeKind::CPtr(_)
        | TypeKind::Map(_, _) => 8,
        TypeKind::Slice(_) => 24,
        TypeKind::Option(_)
        | TypeKind::Result(_, _)
        | TypeKind::Struct(_)
        | TypeKind::Enum(_) => 8,
        TypeKind::Unit
        | TypeKind::Never
        | TypeKind::CVoid
        | TypeKind::Func(_, _) => 0,
        // Widths the checker never constructs; one word is the harmless answer.
        TypeKind::CInt(_) | TypeKind::CUInt(_) => 8,
    }
}

/// The tag constant for a variant pattern of a scrutinee type, at M0–M2.
///
/// `Option`: `None` = 0, `Some` = 1. `Result`: `Ok` = 0, `Err` = 1. User
/// enums arrive with Task 12 and are not represented here yet.
pub fn variant_tag(scrutinee: &TypeKind, variant: &str) -> Option<u8> {
    match scrutinee {
        TypeKind::Option(_) => match variant {
            "None" => Some(0),
            "Some" => Some(1),
            _ => None,
        },
        TypeKind::Result(_, _) => match variant {
            "Ok" => Some(0),
            "Err" => Some(1),
            _ => None,
        },
        _ => None,
    }
}

/// The payload field index for a variant of a scrutinee type, at M0–M2.
///
/// `Option` stores its single payload at field 1; `Result` stores `Ok`'s at
/// field 1 and `Err`'s at field 2 (see [`repr_type`]).
pub fn variant_payload_index(scrutinee: &TypeKind, variant: &str) -> Option<u32> {
    match scrutinee {
        TypeKind::Option(_) if variant == "Some" => Some(1),
        TypeKind::Result(_, _) if variant == "Ok" => Some(1),
        TypeKind::Result(_, _) if variant == "Err" => Some(2),
        _ => None,
    }
}
