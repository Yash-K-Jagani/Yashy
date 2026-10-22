//! Fixed arrays `[T; N]` and borrowed views `[T]`.
//!
//! A fixed array is an LLVM aggregate `[N x T]` that lives wherever its value
//! lives: a local's `alloca` holds the elements themselves, so there is no
//! allocation, no owner, and nothing to free. A literal is an `insertvalue`
//! chain and indexing takes the address of the aggregate, checks the index the
//! same way a slice does, and loads the element out of it.
//!
//! A view is the two-word header `{ i8*, i64 }` -- a pointer to the first
//! element covered and a length. It borrows storage that something else owns,
//! so creating one only reads its source; the checker guarantees the source
//! outlives the view. Range bounds are clamped to the source's length, exactly
//! as `slice(start, end)` clamps, because a range is not a place a program can
//! fail usefully: `a[2..99]` on three elements is the two-element tail.

use otter_sema::{expr_ty, HirExpr, TypeKind};

use crate::expr_gen::{gen_expr, gen_expr_to_slot, FnCtx, GenValue};
use crate::ir::{repr_type, size_of};
use crate::slices::option_from_flag;

/// The IR spelling of a view header, repeated in `ir.rs`.
const VIEW_REPR: &str = "{ i8*, i64 }";

fn unit() -> GenValue {
    GenValue {
        value: String::new(),
        ty: TypeKind::Unit,
    }
}

/// Lower `[a, b, c]`.
///
/// Every element is a value of `elem_ty`, and the aggregate is built in SSA
/// registers with one `insertvalue` per element. No memory is touched, so a
/// literal can be used anywhere a value is expected and a copy of the resulting
/// array is just a new use of the same aggregate.
pub fn gen_literal(
    ctx: &mut FnCtx<'_>,
    elem_ty: &TypeKind,
    elements: &[HirExpr],
    array_ty: &TypeKind,
) -> GenValue {
    let elem_repr = repr_type(elem_ty);
    let array_repr = repr_type(array_ty);
    let mut aggregate = "undef".to_string();
    for (index, element) in elements.iter().enumerate() {
        let value = gen_expr(ctx, element);
        let next = ctx.tmp();
        ctx.push(format!(
            "{next} = insertvalue {array_repr} {aggregate}, {elem_repr} {}, {index}",
            value.value
        ));
        aggregate = next;
    }
    GenValue {
        value: aggregate,
        ty: array_ty.clone(),
    }
}

/// The address of an array value's storage, and its length in elements.
///
/// A binding has a slot already; a temporary is stored in a fresh one. Either
/// way the result is a pointer to the aggregate, which is what a GEP into the
/// elements needs. The length of a fixed array is in its type, so the caller
/// gets the constant it needs without reading memory.
fn array_storage(ctx: &mut FnCtx<'_>, base: &HirExpr) -> Option<(String, usize)> {
    let TypeKind::FixedArray(_, length) = &expr_ty(base).kind else {
        return None;
    };
    let length = *length;
    if let HirExpr::Ident(path, _, _) = base {
        if let Some(name) = path.last() {
            if let Some((slot, _)) = ctx.locals.get(name) {
                if !slot.is_empty() {
                    return Some((slot.clone(), length));
                }
            }
        }
    }
    let slot = gen_expr_to_slot(ctx, base);
    if slot.is_empty() {
        return None;
    }
    Some((slot, length))
}

/// The data pointer and length of a view value.
///
/// A slice header has the same first two words plus a capacity, so the repr
/// has to come from the base's own type: extracting a `{ i8*, i64 }` out of a
/// `{ i8*, i64, i64 }` is not a detail LLVM will forgive.
fn view_parts(ctx: &mut FnCtx<'_>, base: &HirExpr) -> Option<(String, String)> {
    let repr = repr_type(&expr_ty(base).kind);
    let header = gen_expr(ctx, base);
    let data = ctx.tmp();
    ctx.push(format!("{data} = extractvalue {repr} {}, 0", header.value));
    let len = ctx.tmp();
    ctx.push(format!("{len} = extractvalue {repr} {}, 1", header.value));
    Some((data, len))
}

/// Lower `v[i]` on a fixed array or a view: an `Option[T]`.
///
/// The bounds test is the same for both: `0 <= i < len`. The load is guarded by
/// selecting index 0 when the index is out of range, so the pointer handed to
/// `load` is always inside the storage even though the answer is `None`. That
/// keeps the promise indexing makes everywhere else in Otter -- out of range is
/// a value, never a fault.
pub fn gen_index(
    ctx: &mut FnCtx<'_>,
    base: &HirExpr,
    index: &HirExpr,
    option_ty: &TypeKind,
) -> GenValue {
    let TypeKind::Option(element) = option_ty else {
        return unit();
    };
    let (data, len) = match expr_ty(base).kind.clone() {
        TypeKind::FixedArray(_, _) => {
            let Some((slot, length)) = array_storage(ctx, base) else {
                return unit();
            };
            // The first element of the aggregate: index 0 of the array, then
            // index 0 of whatever the element type is.
            let repr = repr_type(&expr_ty(base).kind);
            let data = ctx.tmp();
            ctx.push(format!(
                "{data} = getelementptr {repr}, ptr {slot}, i64 0, i64 0"
            ));
            let len = ctx.tmp();
            ctx.push(format!(
                "{len} = add i64 0, {}",
                i64::try_from(length).unwrap_or(i64::MAX)
            ));
            (data, len)
        }
        TypeKind::View(_) | TypeKind::Slice(_) => {
            let Some(parts) = view_parts(ctx, base) else {
                return unit();
            };
            parts
        }
        _ => return unit(),
    };
    let position = gen_expr(ctx, index);
    let element_repr = repr_type(&element.kind);
    let out_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{out_slot} = alloca {element_repr}"));
    ctx.push(format!(
        "store {element_repr} zeroinitializer, ptr {out_slot}"
    ));

    let negative = ctx.tmp();
    ctx.push(format!("{negative} = icmp slt i64 {}, 0", position.value));
    let too_far = ctx.tmp();
    ctx.push(format!(
        "{too_far} = icmp sge i64 {}, {len}",
        position.value
    ));
    let out_of_range = ctx.tmp();
    ctx.push(format!("{out_of_range} = or i1 {negative}, {too_far}"));
    let in_range = ctx.tmp();
    ctx.push(format!("{in_range} = xor i1 {out_of_range}, true"));
    // Load from element 0 when the index is out of range: the address is in
    // bounds, and the tag says the payload is not the answer.
    let safe_index = ctx.tmp();
    ctx.push(format!(
        "{safe_index} = select i1 {in_range}, i64 {}, i64 0",
        position.value
    ));
    let address = ctx.tmp();
    ctx.push(format!(
        "{address} = getelementptr {element_repr}, ptr {data}, i64 {safe_index}"
    ));
    let loaded = ctx.tmp();
    ctx.push(format!("{loaded} = load {element_repr}, ptr {address}"));
    ctx.push(format!("store {element_repr} {loaded}, ptr {out_slot}"));

    let flag = ctx.tmp();
    ctx.push(format!("{flag} = zext i1 {in_range} to i64"));
    option_from_flag(ctx, &flag, &out_slot, &element.kind, option_ty)
}

/// Lower `a[start..end]`: a view over part of an array, a slice, or another
/// view.
///
/// Both bounds are clamped into `[0, len]`, and `end` is then raised to `start`
/// if it was below it, so the length is never negative and the pointer is never
/// past the end. The source's own header is read, never changed: a view owns
/// nothing.
pub fn gen_view(
    ctx: &mut FnCtx<'_>,
    base: &HirExpr,
    start: &HirExpr,
    end: &HirExpr,
    view_ty: &TypeKind,
) -> GenValue {
    let TypeKind::View(element) = view_ty else {
        return unit();
    };
    let base_ty = expr_ty(base);
    let (data, len) = match &base_ty.kind {
        TypeKind::FixedArray(_, _) => {
            let Some((slot, length)) = array_storage(ctx, base) else {
                return unit();
            };
            let repr = repr_type(&base_ty.kind);
            let data = ctx.tmp();
            ctx.push(format!(
                "{data} = getelementptr {repr}, ptr {slot}, i64 0, i64 0"
            ));
            let len = ctx.tmp();
            ctx.push(format!(
                "{len} = add i64 0, {}",
                i64::try_from(length).unwrap_or(i64::MAX)
            ));
            (data, len)
        }
        TypeKind::Slice(_) | TypeKind::View(_) => {
            let Some(parts) = view_parts(ctx, base) else {
                return unit();
            };
            parts
        }
        _ => return unit(),
    };
    let raw_start = gen_expr(ctx, start);
    let raw_end = gen_expr(ctx, end);
    let begin = clamp(ctx, &raw_start.value, &len);
    let finish = clamp(ctx, &raw_end.value, &len);
    // `end < start` would be a negative length; a range like `3..1` is the
    // empty range starting at 3, which is what `slice` does too.
    let backwards = ctx.tmp();
    ctx.push(format!("{backwards} = icmp slt i64 {finish}, {begin}"));
    let last = ctx.tmp();
    ctx.push(format!(
        "{last} = select i1 {backwards}, i64 {begin}, i64 {finish}"
    ));
    let count = ctx.tmp();
    ctx.push(format!("{count} = sub i64 {last}, {begin}"));
    let stride = size_of(&element.kind);
    let offset = ctx.tmp();
    ctx.push(format!("{offset} = mul i64 {begin}, {stride}"));
    let first = ctx.tmp();
    ctx.push(format!("{first} = getelementptr i8, ptr {data}, i64 {offset}"));
    let with_data = ctx.tmp();
    ctx.push(format!(
        "{with_data} = insertvalue {VIEW_REPR} undef, i8* {first}, 0"
    ));
    let header = ctx.tmp();
    ctx.push(format!(
        "{header} = insertvalue {VIEW_REPR} {with_data}, i64 {count}, 1"
    ));
    GenValue {
        value: header,
        ty: view_ty.clone(),
    }
}

/// Clamp `value` into `[0, len]` with two selects.
fn clamp(ctx: &mut FnCtx<'_>, value: &str, len: &str) -> String {
    let negative = ctx.tmp();
    ctx.push(format!("{negative} = icmp slt i64 {value}, 0"));
    let at_least_zero = ctx.tmp();
    ctx.push(format!(
        "{at_least_zero} = select i1 {negative}, i64 0, i64 {value}"
    ));
    let past_end = ctx.tmp();
    ctx.push(format!("{past_end} = icmp sgt i64 {at_least_zero}, {len}"));
    let result = ctx.tmp();
    ctx.push(format!(
        "{result} = select i1 {past_end}, i64 {len}, i64 {at_least_zero}"
    ));
    result
}

/// Lower `view_len(v) -> i64`: the length field of the header.
pub fn gen_len(ctx: &mut FnCtx<'_>, args: &[HirExpr]) -> GenValue {
    let Some(argument) = args.first() else {
        return GenValue {
            value: "0".to_string(),
            ty: TypeKind::I64,
        };
    };
    let header = gen_expr(ctx, argument);
    let len = ctx.tmp();
    ctx.push(format!(
        "{len} = extractvalue {VIEW_REPR} {}, 1",
        header.value
    ));
    GenValue {
        value: len,
        ty: TypeKind::I64,
    }
}
