//! Maps: `map[K]V` lowered to the runtime's open-addressing table.
//!
//! A map value is the runtime's opaque pointer (`i8*`), so every operation is a
//! call and the table's layout lives entirely in `hashmap.c`. The key kind is
//! passed once, at `map_new`, because it decides how the runtime hashes and
//! compares entries from then on.

use otter_sema::{HirExpr, TypeKind};

use crate::expr_gen::{gen_expr, gen_expr_to_slot, FnCtx, GenValue};
use crate::ir::{repr_type, size_of};
use crate::slices::{bool_from_flag, option_from_flag};

/// The slice header the runtime fills: data pointer, length, capacity.
///
/// The runtime's `otter_slice` has exactly this layout, which is what lets a
/// map hand back an array through the same header `str_split` uses.
const SLICE_REPR: &str = "{ i8*, i64, i64 }";

/// The key tags shared with `hashmap.c`: `i64` = 0, `bool` = 1, `string` = 2.
///
/// The checker restricts map keys to these three kinds, so `None` here means
/// the tree and the backend disagree; callers emit nothing rather than pass a
/// tag the runtime would not understand.
fn key_kind(kind: &TypeKind) -> Option<i64> {
    match kind {
        TypeKind::I64 => Some(0),
        TypeKind::Bool => Some(1),
        TypeKind::Str => Some(2),
        _ => None,
    }
}

fn unit() -> GenValue {
    GenValue {
        value: String::new(),
        ty: TypeKind::Unit,
    }
}

/// Lower `map_new() -> map[K]V` to one call carrying the key kind and the two
/// element strides.
///
/// The new table allocates nothing until the first insert, so an empty map is
/// one small allocation and no arrays.
pub fn gen_new(ctx: &mut FnCtx<'_>, ty: &TypeKind) -> GenValue {
    let TypeKind::Map(key, value) = ty else {
        return unit();
    };
    let Some(kind) = key_kind(&key.kind) else {
        return unit();
    };
    let table = ctx.tmp();
    ctx.push(format!(
        "{table} = call i8* @otter_map_new(i64 {kind}, i64 {}, i64 {})",
        size_of(&key.kind),
        size_of(&value.kind)
    ));
    GenValue {
        value: table,
        ty: ty.clone(),
    }
}

/// Lower `map_insert(map, key, value) -> ()`.
///
/// Both the key and the value go through a slot so the runtime can copy
/// `key_size`/`value_size` bytes. For a `string` key that copy is a handle: the
/// runtime retains it, so the map owns its own reference and the caller's
/// binding is unaffected.
pub fn gen_insert(ctx: &mut FnCtx<'_>, args: &[HirExpr]) -> GenValue {
    let mut args = args.iter();
    let (Some(map), Some(key), Some(value)) = (args.next(), args.next(), args.next()) else {
        return unit();
    };
    let table = gen_expr(ctx, map);
    let key_slot = gen_expr_to_slot(ctx, key);
    let value_slot = gen_expr_to_slot(ctx, value);
    if key_slot.is_empty() || value_slot.is_empty() {
        return unit();
    }
    ctx.push(format!(
        "call void @otter_map_insert(i8* {}, ptr {key_slot}, ptr {value_slot})",
        table.value
    ));
    unit()
}

/// Lower `map_get(map, key) -> Option[V]` and `map_remove(map, key) -> Option[V]`.
///
/// Both hand the runtime a zeroed out slot and turn its 0/1 answer into an
/// `Option`, the same shape slice indexing uses. The difference is what the
/// runtime does to the entry; lowering cannot tell them apart.
fn gen_lookup(
    ctx: &mut FnCtx<'_>,
    args: &[HirExpr],
    option_ty: &TypeKind,
    function: &str,
) -> GenValue {
    let TypeKind::Option(value) = option_ty else {
        return unit();
    };
    let Some(map) = args.first() else {
        return unit();
    };
    let Some(key) = args.get(1) else {
        return unit();
    };
    let table = gen_expr(ctx, map);
    let key_slot = gen_expr_to_slot(ctx, key);
    if key_slot.is_empty() {
        return unit();
    }
    let value_repr = repr_type(&value.kind);
    let out_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{out_slot} = alloca {value_repr}"));
    ctx.push(format!(
        "store {value_repr} zeroinitializer, ptr {out_slot}"
    ));
    let flag = ctx.tmp();
    ctx.push(format!(
        "{flag} = call i64 @{function}(i8* {}, ptr {key_slot}, ptr {out_slot})",
        table.value
    ));
    option_from_flag(ctx, &flag, &out_slot, &value.kind, option_ty)
}

pub fn gen_get(ctx: &mut FnCtx<'_>, args: &[HirExpr], option_ty: &TypeKind) -> GenValue {
    gen_lookup(ctx, args, option_ty, "otter_map_get")
}

pub fn gen_remove(ctx: &mut FnCtx<'_>, args: &[HirExpr], option_ty: &TypeKind) -> GenValue {
    gen_lookup(ctx, args, option_ty, "otter_map_remove")
}

/// Lower `map_len(map) -> i64`.
pub fn gen_len(ctx: &mut FnCtx<'_>, args: &[HirExpr]) -> GenValue {
    let Some(map) = args.first() else {
        return unit();
    };
    let table = gen_expr(ctx, map);
    let len = ctx.tmp();
    ctx.push(format!(
        "{len} = call i64 @otter_map_len(i8* {})",
        table.value
    ));
    GenValue {
        value: len,
        ty: TypeKind::I64,
    }
}

/// Lower `map_free(map) -> ()`.
///
/// The checker has already moved the binding out, so the table cannot be
/// reached again. The runtime releases every retained string key before it
/// frees the arrays.
pub fn gen_free(ctx: &mut FnCtx<'_>, args: &[HirExpr]) -> GenValue {
    let Some(map) = args.first() else {
        return unit();
    };
    let table = gen_expr(ctx, map);
    ctx.push(format!("call void @otter_map_free(i8* {})", table.value));
    unit()
}

/// Lower `map_keys(map) -> []K` and `map_values(map) -> []V`.
///
/// The runtime copies the stored bytes into a fresh array and fills a slice
/// header, the same shape `str_split` uses. The array's element stride is
/// the table's own stride, so the two agree without the backend repeating
/// what the runtime already knows.
fn gen_entries(
    ctx: &mut FnCtx<'_>,
    args: &[HirExpr],
    slice_ty: &TypeKind,
    function: &str,
) -> GenValue {
    if !matches!(slice_ty, TypeKind::Slice(_)) {
        return unit();
    }
    let Some(map) = args.first() else {
        return unit();
    };
    let table = gen_expr(ctx, map);
    let out_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{out_slot} = alloca {SLICE_REPR}"));
    ctx.push(format!("store {SLICE_REPR} zeroinitializer, ptr {out_slot}"));
    ctx.push(format!(
        "call void @{function}(ptr {out_slot}, i8* {})",
        table.value
    ));
    let result = ctx.tmp();
    ctx.push(format!("{result} = load {SLICE_REPR}, ptr {out_slot}"));
    GenValue {
        value: result,
        ty: slice_ty.clone(),
    }
}

/// Lower `map_keys(map) -> []K`.
///
/// The array is a copy of the table's keys, in the table's own order, and it
/// owns a reference of its own for every `string` key it holds.
pub fn gen_keys(ctx: &mut FnCtx<'_>, args: &[HirExpr], slice_ty: &TypeKind) -> GenValue {
    gen_entries(ctx, args, slice_ty, "otter_map_keys")
}

/// Lower `map_values(map) -> []V`.
///
/// The array is a copy of the table's values, in the same order as the keys.
pub fn gen_values(ctx: &mut FnCtx<'_>, args: &[HirExpr], slice_ty: &TypeKind) -> GenValue {
    gen_entries(ctx, args, slice_ty, "otter_map_values")
}

/// Lower `map_contains_key(map, key) -> bool`.
///
/// The runtime answers 0 or 1 without copying the value out; the comparison
/// turns that into the language's one-bit `bool`.
pub fn gen_contains_key(ctx: &mut FnCtx<'_>, args: &[HirExpr]) -> GenValue {
    let (Some(map), Some(key)) = (args.first(), args.get(1)) else {
        return crate::slices::false_value();
    };
    let table = gen_expr(ctx, map);
    let key_slot = gen_expr_to_slot(ctx, key);
    if key_slot.is_empty() {
        return crate::slices::false_value();
    }
    let flag = ctx.tmp();
    ctx.push(format!(
        "{flag} = call i64 @otter_map_contains_key(i8* {}, ptr {key_slot})",
        table.value
    ));
    bool_from_flag(ctx, &flag)
}
