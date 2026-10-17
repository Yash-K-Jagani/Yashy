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
        TypeKind::Option(_)
        | TypeKind::Result(_, _)
        | TypeKind::Struct(_)
        | TypeKind::Enum(_)
        | TypeKind::Func(_, _) => "void".to_string(),
    }
}
