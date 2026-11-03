//! Heap builtins and ownership moves.
//!
//! Three constructs reach the backend as ordinary expressions and are rewritten
//! here rather than in [`crate::expr_gen`]: `alloc`, `dealloc` and `move`. Each
//! one is a thin wrapper over the runtime C ABI in ledger L1 — every symbol
//! emitted here has a `declare` in [`crate::module`] and a definition in
//! `yashy-runtime`, and those three places change together.

use yashy_common::Span;
use yashy_sema::{linearity_of, HirExpr, Linearity, Literal, Type, TypeKind};

use crate::escape::{AllocSite, Placement};
use crate::expr_gen::{gen_expr, FnCtx, GenValue};
use crate::ir::repr_type;

/// The size, in bytes, of one `i64` element of an allocation.
pub const I64_SIZE: i64 = 8;

/// The alignment, in bytes, `yashy_alloc` is asked for.
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
pub fn gen_alloc(ctx: &mut FnCtx<'_>, args: &[HirExpr], ret: &TypeKind, span: Span) -> GenValue {
    match ctx.placement(AllocSite::of(span)) {
        Placement::Stack => {
            // Escape analysis only chooses the stack for a positive literal
            // count, so the array's length is known here. A site that somehow
            // arrives without one falls back to the allocator rather than
            // emitting an unsized `alloca`.
            if let Some(count) = constant_count(args) {
                let slot = ctx.alloc_slot();
                ctx.push_alloca(format!("{slot} = alloca [{count} x i64], align 8"));
                let ptr = ctx.tmp();
                ctx.push(format!(
                    "{ptr} = getelementptr [{count} x i64], ptr {slot}, i64 0, i64 0"
                ));
                return GenValue {
                    value: ptr,
                    ty: ret.clone(),
                };
            }
        }
        Placement::Arena(id) => {
            let arena = ctx.arena_handle(id);
            let count = element_count(ctx, args);
            let bytes = ctx.tmp();
            ctx.push(format!("{bytes} = mul i64 {count}, {I64_SIZE}"));
            let ptr = ctx.tmp();
            ctx.push(format!(
                "{ptr} = call i8* @yashy_arena_alloc(i8* {arena}, i64 {bytes}, i64 {I64_ALIGN})"
            ));
            return GenValue {
                value: ptr,
                ty: ret.clone(),
            };
        }
        Placement::Heap => {}
    }
    let count = element_count(ctx, args);
    let bytes = ctx.tmp();
    ctx.push(format!("{bytes} = mul i64 {count}, {I64_SIZE}"));
    let ptr = ctx.tmp();
    ctx.push(format!(
        "{ptr} = call i8* @yashy_alloc(i64 {bytes}, i64 {I64_ALIGN})"
    ));
    GenValue {
        value: ptr,
        ty: ret.clone(),
    }
}

/// The literal element count of an `alloc`, when the source gave one.
fn constant_count(args: &[HirExpr]) -> Option<i64> {
    match args.first() {
        Some(HirExpr::Literal(Literal::I64(count), ..)) if *count > 0 => Some(*count),
        _ => None,
    }
}

/// Lower the count expression to an SSA name, defaulting to zero.
fn element_count(ctx: &mut FnCtx<'_>, args: &[HirExpr]) -> String {
    args.first()
        .map(|a| gen_expr(ctx, a))
        .filter(|v| !v.value.is_empty())
        .map(|v| v.value)
        .unwrap_or_else(|| "0".to_string())
}

/// Lower `dealloc(p) -> ()`.
///
/// One call and no value. `yashy_free` tolerates a null pointer, which is what
/// lets [`gen_move`] poison a moved slot without a guard here.
pub fn gen_dealloc(ctx: &mut FnCtx<'_>, args: &[HirExpr]) -> GenValue {
    // A promoted allocation lives in the frame or in a block arena, so the
    // allocator never owned it. Calling `free` on it would be a hard error and
    // not just a leak, which is why the matching `dealloc` disappears with the
    // allocation.
    if let Some(HirExpr::Ident(path, _, _)) = args.first() {
        if let [name] = path.as_slice() {
            if let Some((slot, _)) = ctx.locals.get(name) {
                if ctx.is_promoted(slot) {
                    return GenValue {
                        value: String::new(),
                        ty: TypeKind::Unit,
                    };
                }
            }
        }
    }
    let ptr = args
        .first()
        .map(|a| gen_expr(ctx, a))
        .filter(|v| !v.value.is_empty())
        .map(|v| v.value)
        .unwrap_or_else(|| "null".to_string());
    ctx.push(format!("call void @yashy_free(i8* {ptr})"));
    GenValue {
        value: String::new(),
        ty: TypeKind::Unit,
    }
}

/// Lower `alloc_bytes(n) -> *mut char`.
///
/// `n` counts *bytes*, not elements, and the runtime's alignment is 1: a byte
/// buffer has no element-size constraint. The result is `*mut char`, so
/// `buf[i]` is a `char` and a program that wants an `int` must convert.
pub fn gen_alloc_bytes(ctx: &mut FnCtx<'_>, args: &[HirExpr]) -> GenValue {
    let count = args
        .first()
        .map(|a| gen_expr(ctx, a))
        .filter(|v| !v.value.is_empty())
        .map(|v| v.value)
        .unwrap_or_else(|| "0".to_string());
    let ptr = ctx.tmp();
    ctx.push(format!("{ptr} = call i8* @yashy_alloc_bytes(i64 {count})"));
    GenValue {
        value: ptr,
        ty: TypeKind::Ptr(Box::new(Type {
            kind: TypeKind::Char,
            span: Span::new(0, 0),
        })),
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
                    // An aggregate is emptied with a zero value, not with
                    // `null`: `null` is a pointer, and an aggregate is not
                    // one.
                    let empty = if repr.starts_with('{') {
                        "zeroinitializer"
                    } else {
                        "null"
                    };
                    ctx.push(format!("store {repr} {empty}, ptr {slot}"));
                }
            }
        }
    }
    value
}
