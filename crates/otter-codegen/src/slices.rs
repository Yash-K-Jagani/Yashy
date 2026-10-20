//! Slices: the literal, `push`, and `free_slice`.
//!
//! A slice value is the three-word header `{ i8*, i64, i64 }` — data pointer,
//! length, capacity — matching `otter_slice` in the runtime header. The header
//! is a value; the backing array is one heap block owned by the binding that
//! holds the header. `push` takes the *address* of the header (the binding's
//! alloca) because a grow must update length and capacity in place.

use otter_sema::{HirExpr, TypeKind};

use crate::expr_gen::{gen_expr, FnCtx, GenValue};
use crate::ir::{repr_type, size_of};

/// The IR spelling of a slice header, repeated in `ir.rs`, here and in the
/// runtime declarations in `module.rs`.
const SLICE_REPR: &str = "{ i8*, i64, i64 }";

/// The alignment every slice backing array is allocated with.
const SLICE_ALIGN: u64 = 8;

fn unit() -> GenValue {
    GenValue {
        value: String::new(),
        ty: TypeKind::Unit,
    }
}

/// Lower `[]T { a, b, ... }`.
///
/// One `otter_alloc` for `n` elements, one store per element, then the three
/// `insertvalue`s that build the header. An empty literal allocates nothing and
/// is the null header, so `[]T {}` is a valid value of every slice type and its
/// first `push` starts from capacity zero.
pub fn gen_literal(
    ctx: &mut FnCtx<'_>,
    elem_ty: &TypeKind,
    elements: &[HirExpr],
    slice_ty: &TypeKind,
) -> GenValue {
    let elem_repr = repr_type(elem_ty);
    let elem_size = size_of(elem_ty);
    let count = elements.len();

    let data = if count == 0 || elem_size == 0 {
        "null".to_string()
    } else {
        let bytes = ctx.tmp();
        ctx.push(format!("{bytes} = mul i64 {count}, {elem_size}"));
        let data = ctx.tmp();
        ctx.push(format!(
            "{data} = call i8* @otter_alloc(i64 {bytes}, i64 {SLICE_ALIGN})"
        ));
        data
    };

    for (index, element) in elements.iter().enumerate() {
        let value = gen_expr(ctx, element);
        let address = ctx.tmp();
        ctx.push(format!(
            "{address} = getelementptr {elem_repr}, ptr {data}, i64 {index}"
        ));
        ctx.push(format!(
            "store {elem_repr} {}, ptr {address}",
            value.value
        ));
    }

    let aggregate = ctx.tmp();
    ctx.push(format!(
        "{aggregate} = insertvalue {SLICE_REPR} undef, i8* {data}, 0"
    ));
    let with_len = ctx.tmp();
    ctx.push(format!(
        "{with_len} = insertvalue {SLICE_REPR} {aggregate}, i64 {count}, 1"
    ));
    let with_cap = ctx.tmp();
    ctx.push(format!(
        "{with_cap} = insertvalue {SLICE_REPR} {with_len}, i64 {count}, 2"
    ));

    GenValue {
        value: with_cap,
        ty: slice_ty.clone(),
    }
}

/// The binding a slice operation names, or `None` when the shape is not one
/// the checker lets through.
///
/// The checker requires a binding for both operations, so anything else here
/// means the tree and the backend disagree; the caller emits nothing rather
/// than guessing at an address.
fn binding_slot(ctx: &FnCtx<'_>, args: &[HirExpr]) -> Option<(String, TypeKind)> {
    let HirExpr::Ident(path, _, _) = args.first()? else {
        return None;
    };
    let name = path.last()?;
    let (slot, ty) = ctx.locals.get(name)?;
    Some((slot.clone(), ty.clone()))
}

/// Lower `push(slice, value) -> ()`.
///
/// The value is stored in a fresh slot and its address handed to the runtime,
/// which copies `elem_size` bytes. That works for every element representation,
/// including aggregates, without a second lowering path per type.
pub fn gen_push(ctx: &mut FnCtx<'_>, args: &[HirExpr]) -> GenValue {
    let Some((slot, ty)) = binding_slot(ctx, args) else {
        return unit();
    };
    let TypeKind::Slice(element) = &ty else {
        return unit();
    };
    let Some(value_expr) = args.get(1) else {
        return unit();
    };
    let value = gen_expr(ctx, value_expr);
    let elem_repr = repr_type(&element.kind);
    let elem_size = size_of(&element.kind);
    let temp = ctx.alloc_slot();
    ctx.push_alloca(format!("{temp} = alloca {elem_repr}"));
    ctx.push(format!("store {elem_repr} {}, ptr {temp}", value.value));
    ctx.push(format!(
        "call void @otter_slice_push(ptr {slot}, ptr {temp}, i64 {elem_size})"
    ));
    unit()
}

/// Lower `free_slice(slice) -> ()`.
///
/// The runtime frees the backing array and zeroes the header in the binding's
/// slot. The checker has already moved the binding out, so the value cannot be
/// read again.
pub fn gen_free(ctx: &mut FnCtx<'_>, args: &[HirExpr]) -> GenValue {
    let Some((slot, _)) = binding_slot(ctx, args) else {
        return unit();
    };
    ctx.push(format!("call void @otter_slice_free(ptr {slot})"));
    unit()
}
