//! Arena builtins.
//!
//! An arena is a bump allocator: `arena_new()` hands out an opaque handle,
//! `arena_alloc(arena, n)` hands out room for `n` `i64`s, and
//! `arena_free_all(arena)` releases every block the arena ever allocated in one
//! call. There is no per-allocation free, because an arena exists to make many
//! small allocations cheap and to release them all at once.
//!
//! Every symbol emitted here has a `declare` in [`crate::module`] and a
//! definition in `yashy-runtime`, and those three places change together.

use yashy_sema::{HirExpr, TypeKind};

use crate::expr_gen::{gen_expr, FnCtx, GenValue};

/// The size, in bytes, of one `i64` element of an arena allocation.
const I64_SIZE: i64 = 8;

/// The alignment, in bytes, `yashy_arena_alloc` is asked for.
const I64_ALIGN: i64 = 8;

/// Lower `arena_new() -> *mut i64`.
pub fn gen_arena_new(ctx: &mut FnCtx<'_>, _args: &[HirExpr], ret: &TypeKind) -> GenValue {
    let ptr = ctx.tmp();
    ctx.push(format!("{ptr} = call i8* @yashy_arena_new()"));
    GenValue {
        value: ptr,
        ty: ret.clone(),
    }
}

/// Lower `arena_alloc(arena, n) -> *mut i64`.
///
/// `n` counts *elements*, not bytes, matching `alloc`. The runtime asks for a
/// byte count (ledger L1), so the multiply by the element size happens here.
pub fn gen_arena_alloc(ctx: &mut FnCtx<'_>, args: &[HirExpr], ret: &TypeKind) -> GenValue {
    let arena = args
        .first()
        .map(|a| gen_expr(ctx, a))
        .filter(|v| !v.value.is_empty())
        .map(|v| v.value)
        .unwrap_or_else(|| "null".to_string());
    let count = args
        .get(1)
        .map(|a| gen_expr(ctx, a))
        .filter(|v| !v.value.is_empty())
        .map(|v| v.value)
        .unwrap_or_else(|| "0".to_string());
    let bytes = ctx.tmp();
    ctx.push(format!("{bytes} = mul i64 {count}, {I64_SIZE}"));
    let ptr = ctx.tmp();
    ctx.push(format!(
        "{ptr} = call i8* @yashy_arena_alloc(i8* {arena}, i64 {bytes}, i64 {I64_ALIGN})"
    ));
    GenValue {
        value: ptr,
        ty: ret.clone(),
    }
}

/// Lower `arena_free_all(arena) -> ()`.
pub fn gen_arena_free_all(ctx: &mut FnCtx<'_>, args: &[HirExpr]) -> GenValue {
    let arena = args
        .first()
        .map(|a| gen_expr(ctx, a))
        .filter(|v| !v.value.is_empty())
        .map(|v| v.value)
        .unwrap_or_else(|| "null".to_string());
    ctx.push(format!("call void @yashy_arena_free_all(i8* {arena})"));
    GenValue {
        value: String::new(),
        ty: TypeKind::Unit,
    }
}
