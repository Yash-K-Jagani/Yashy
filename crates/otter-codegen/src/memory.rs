//! Heap builtins and ownership moves.
//!
//! Three constructs reach the backend as ordinary expressions and are rewritten
//! here rather than in [`crate::expr_gen`]: `alloc`, `dealloc` and `move`. Each
//! one is a thin wrapper over the runtime C ABI in ledger L1 — every symbol
//! emitted here has a `declare` in [`crate::module`] and a definition in
//! `otter-runtime`, and those three places change together.

use otter_sema::{linearity_of, HirExpr, Linearity, TypeKind};

use crate::expr_gen::{gen_expr, FnCtx, GenValue};
use crate::ir::repr_type;

/// The size, in bytes, of one `i64` element of an allocation.
pub const I64_SIZE: i64 = 8;

/// The alignment, in bytes, `otter_alloc` is asked for.
///
/// M3's only pointee is `i64` (see the `alloc` revision in the master plan's
/// ledger L3), so both constants are the `i64` layout. When M6 gives pointers
/// more than one element type, the element's own layout replaces them.
pub const I64_ALIGN: i64 = 8;

/// Lower `alloc(n) -> *mut i64`.
///
/// `n` counts *elements*, not bytes: `alloc(16)` is room for sixteen `i64`s,
/// which is what the `*mut i64` result suggests. The runtime asks for a byte
/// count (ledger L1), so the multiply by the element size happens here. The
/// memory is uninitialised, and the checker has already guaranteed that the one
/// allocation reaches exactly one `dealloc` or one `move`.
pub fn gen_alloc(ctx: &mut FnCtx<'_>, args: &[HirExpr], ret: &TypeKind) -> GenValue {
    // The checker proved `alloc` takes exactly one argument, so a missing one
    // is a compiler bug rather than a bad program. A zero count is the harmless
    // reading: it produces a valid call and a well-typed value instead of a
    // panic, which this project forbids.
    let count = args
        .first()
        .map(|a| gen_expr(ctx, a))
        .filter(|v| !v.value.is_empty())
        .map(|v| v.value)
        .unwrap_or_else(|| "0".to_string());
    let bytes = ctx.tmp();
    ctx.push(format!("{bytes} = mul i64 {count}, {I64_SIZE}"));
    let ptr = ctx.tmp();
    ctx.push(format!(
        "{ptr} = call i8* @otter_alloc(i64 {bytes}, i64 {I64_ALIGN})"
    ));
    GenValue {
        value: ptr,
        ty: ret.clone(),
    }
}

/// Lower `dealloc(p) -> ()`.
///
/// One call and no value. `otter_free` tolerates a null pointer, which is what
/// lets [`gen_move`] poison a moved slot without a guard here.
pub fn gen_dealloc(ctx: &mut FnCtx<'_>, args: &[HirExpr]) -> GenValue {
    let ptr = args
        .first()
        .map(|a| gen_expr(ctx, a))
        .filter(|v| !v.value.is_empty())
        .map(|v| v.value)
        .unwrap_or_else(|| "null".to_string());
    ctx.push(format!("call void @otter_free(i8* {ptr})"));
    GenValue {
        value: String::new(),
        ty: TypeKind::Unit,
    }
}

/// Lower `move x`.
///
/// The value is produced exactly as reading `x` would produce it, and then the
/// source slot is overwritten with null when `x` names a linear binding. That is
/// belt and braces: the checker has already made a second use of a moved value
/// impossible, but if the backend ever drifts, the failure is a `free` of null
/// (a no-op) rather than a fresh copy of a pointer the program has given away.
pub fn gen_move(ctx: &mut FnCtx<'_>, inner: &HirExpr, ty: &TypeKind) -> GenValue {
    let value = gen_expr(ctx, inner);
    if matches!(linearity_of(ty), Linearity::Linear) {
        if let HirExpr::Ident(path, _, _) = inner {
            let name = path.last().cloned().unwrap_or_default();
            if let Some((slot, slot_ty)) = ctx.locals.get(&name).cloned() {
                let repr = repr_type(&slot_ty);
                if !slot.is_empty() && repr != "void" {
                    ctx.push(format!("store {repr} null, ptr {slot}"));
                }
            }
        }
    }
    value
}
