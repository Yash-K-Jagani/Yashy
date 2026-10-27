//! `map`, `filter`, and `fold`: the array methods that call a function.
//!
//! The three are one shape -- an array, a function, and a loop that calls the
//! function -- so they share the loop here and differ in what the loop does
//! with each element. The checker has already said what the whole expression
//! is worth; this is the machine part.
//!
//! The function is called through its value, the same way any call through a
//! function value is: the code pointer comes out of the pair, and the
//! environment goes with it as the first argument.

use yashy_sema::{expr_ty, HirExpr, HigherOrderOp, TypeKind};

use crate::expr_gen::{gen_expr, FnCtx, GenValue, RefKind};
use crate::fn_gen::fn_return_repr;
use crate::ir::{repr_type, size_of};

/// The IR spelling of an array header.
const SLICE_REPR: &str = "{ i8*, i64, i64 }";

/// The IR spelling of a function value.
const FUNC_REPR: &str = "{ i8*, i8* }";

fn unit() -> GenValue {
    GenValue {
        value: String::new(),
        ty: TypeKind::Unit,
    }
}

/// Mark the value a call returned as owned, when it is a reference.
///
/// A `string` the callback built is owned by the call's result, and a closure
/// it returned is owned by the environment it carries. Whoever takes the value
/// next -- the new array, or the accumulator -- has to know that, or the
/// reference is counted twice and released once.
fn mark_result_owned(ctx: &mut FnCtx<'_>, value: &str, ret: &TypeKind) {
    match ret {
        TypeKind::Str => ctx.mark_owned(value, RefKind::Str),
        TypeKind::Func(_, _) => ctx.mark_owned(value, RefKind::Env),
        _ => {}
    }
}

/// Lower `xs.map(f)`, `xs.filter(f)` or `xs.fold(init, f)`.
pub fn gen(
    ctx: &mut FnCtx<'_>,
    op: HigherOrderOp,
    receiver: &HirExpr,
    func: &HirExpr,
    init: Option<&HirExpr>,
    ty: &TypeKind,
) -> GenValue {
    // The receiver is a binding holding an array: the loop reads the header
    // out of the binding's slot on every iteration.
    let HirExpr::Ident(path, _, _) = receiver else {
        return unit();
    };
    let Some(name) = path.last() else {
        return unit();
    };
    let Some((slot, receiver_ty)) = ctx.locals.get(name).cloned() else {
        return unit();
    };
    let TypeKind::Slice(element) = &receiver_ty else {
        return unit();
    };
    let element_kind = element.kind.clone();
    let element_repr = repr_type(&element_kind);

    // The source header is read once, before the loop. The callback is an
    // unknown function as far as the optimizer is concerned -- a call may write
    // any memory -- so a length or data-pointer load inside the loop cannot be
    // hoisted past it. Reading both here keeps them in registers for the whole
    // walk. It is also what the borrow means: the array is only read while the
    // loop runs, so nothing can move the buffer under it.
    let source_len_ptr = ctx.tmp();
    ctx.push(format!(
        "{source_len_ptr} = getelementptr {SLICE_REPR}, ptr {slot}, i32 0, i32 1"
    ));
    let source_len = ctx.tmp();
    ctx.push(format!("{source_len} = load i64, ptr {source_len_ptr}"));
    let source_data_ptr = ctx.tmp();
    ctx.push(format!(
        "{source_data_ptr} = getelementptr {SLICE_REPR}, ptr {slot}, i32 0, i32 0"
    ));
    let source_data = ctx.tmp();
    ctx.push(format!("{source_data} = load i8*, ptr {source_data_ptr}"));

    // The function is a value: the call goes through its code pointer.
    let func_value = gen_expr(ctx, func);
    let code = ctx.tmp();
    ctx.push(format!(
        "{code} = extractvalue {FUNC_REPR} {}, 0",
        func_value.value
    ));
    let env = ctx.tmp();
    ctx.push(format!(
        "{env} = extractvalue {FUNC_REPR} {}, 1",
        func_value.value
    ));
    let func_ty = expr_ty(func);
    let TypeKind::Func(_, ret) = &func_ty.kind else {
        return unit();
    };
    let ret_repr = fn_return_repr(&ret.kind);

    // The index the loop walks.
    let index_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{index_slot} = alloca i64"));
    ctx.push(format!("store i64 0, ptr {index_slot}"));

    // `map` and `filter` build a new array; `fold` carries one value.
    let mut out_data: Option<String> = None;
    let mut written_slot: Option<String> = None;
    let out_slot = match op {
        HigherOrderOp::Map | HigherOrderOp::Filter => {
            let out = ctx.alloc_slot();
            ctx.push_alloca(format!("{out} = alloca {SLICE_REPR}"));
            ctx.push(format!("store {SLICE_REPR} zeroinitializer, ptr {out}"));
            // Reserve room for the whole result before the loop. A `map` returns
            // one element per input element, and a `filter` at most that many,
            // so the source length is the exact upper bound: one allocation and
            // no reallocation as the loop writes.
            let out_kind = match op {
                HigherOrderOp::Map => match ty {
                    TypeKind::Slice(result) => result.kind.clone(),
                    _ => TypeKind::Unit,
                },
                _ => element_kind.clone(),
            };
            let out_size = size_of(&out_kind);
            ctx.push(format!(
                "call void @yashy_slice_reserve(ptr {out}, i64 {source_len}, i64 {out_size})"
            ));
            // The reserved buffer, read once. It is null only when the
            // reservation could not allocate, which is the guard the write
            // path uses to fall back to the runtime's grow path.
            let data_ptr = ctx.tmp();
            ctx.push(format!(
                "{data_ptr} = getelementptr {SLICE_REPR}, ptr {out}, i32 0, i32 0"
            ));
            let data = ctx.tmp();
            ctx.push(format!("{data} = load i8*, ptr {data_ptr}"));
            out_data = Some(data);
            // How many elements the loop has put in the result. The header's
            // length is written from this once the loop is done, so the write
            // path does not have to touch the header per element.
            let written = ctx.alloc_slot();
            ctx.push_alloca(format!("{written} = alloca i64"));
            ctx.push(format!("store i64 0, ptr {written}"));
            written_slot = Some(written);
            Some(out)
        }
        HigherOrderOp::Fold => None,
    };
    let accumulator = match (op, init) {
        (HigherOrderOp::Fold, Some(init_expr)) => {
            let value = gen_expr(ctx, init_expr);
            let acc_repr = repr_type(ty);
            let slot = ctx.alloc_slot();
            ctx.push_alloca(format!("{slot} = alloca {acc_repr}"));
            ctx.push(format!("store {acc_repr} {}, ptr {slot}", value.value));
            Some(slot)
        }
        _ => None,
    };

    let cond_label = ctx.alloc_label_base() + "_cond";
    let body_label = ctx.alloc_label_base() + "_body";
    let inc_label = ctx.alloc_label_base() + "_inc";
    let done_label = ctx.alloc_label_base() + "_done";

    ctx.push(format!("br label %{cond_label}"));
    ctx.push(format!("{cond_label}:"));
    let current = ctx.tmp();
    ctx.push(format!("{current} = load i64, ptr {index_slot}"));
    let in_range = ctx.tmp();
    ctx.push(format!("{in_range} = icmp slt i64 {current}, {source_len}"));
    ctx.push(format!(
        "br i1 {in_range}, label %{body_label}, label %{done_label}"
    ));

    ctx.push(format!("{body_label}:"));
    // The element the callback is given.
    let element_ptr = ctx.tmp();
    ctx.push(format!(
        "{element_ptr} = getelementptr {element_repr}, ptr {source_data}, i64 {current}"
    ));
    let element_value = ctx.tmp();
    ctx.push(format!(
        "{element_value} = load {element_repr}, ptr {element_ptr}"
    ));

    match op {
        HigherOrderOp::Map => {
            // The callback's result is the element the new array holds.
            let mapped = if ret_repr == "void" {
                ctx.push(format!(
                    "call void {code}(i8* {env}, {element_repr} {element_value})"
                ));
                String::new()
            } else {
                let mapped = ctx.tmp();
                ctx.push(format!(
                    "{mapped} = call {ret_repr} {code}(i8* {env}, {element_repr} {element_value})"
                ));
                mark_result_owned(ctx, &mapped, &ret.kind);
                mapped
            };
            let out = out_slot.clone().unwrap_or_default();
            let result_kind = match ty {
                TypeKind::Slice(result) => (**result).kind.clone(),
                _ => TypeKind::Unit,
            };
            store_into(
                ctx,
                &out,
                out_data.as_deref().unwrap_or("null"),
                written_slot.as_deref().unwrap_or("null"),
                &result_kind,
                &mapped,
            );
            ctx.push(format!("br label %{inc_label}"));
        }
        HigherOrderOp::Filter => {
            // The callback says whether the element stays.
            let keep_label = ctx.alloc_label_base() + "_keep";
            let keep = ctx.tmp();
            ctx.push(format!(
                "{keep} = call i1 {code}(i8* {env}, {element_repr} {element_value})"
            ));
            ctx.push(format!(
                "br i1 {keep}, label %{keep_label}, label %{inc_label}"
            ));
            ctx.push(format!("{keep_label}:"));
            let out = out_slot.clone().unwrap_or_default();
            store_into(
                ctx,
                &out,
                out_data.as_deref().unwrap_or("null"),
                written_slot.as_deref().unwrap_or("null"),
                &element_kind,
                &element_value,
            );
            // The body branched here, so the block ends with this branch and
            // the loop's increment is the only way back.
            ctx.push(format!("br label %{inc_label}"));
        }
        HigherOrderOp::Fold => {
            // The accumulator goes in, the next one comes out.
            let acc_slot = accumulator.clone().unwrap_or_default();
            let acc_repr = repr_type(ty);
            let carried = ctx.tmp();
            ctx.push(format!("{carried} = load {acc_repr}, ptr {acc_slot}"));
            let next = if ret_repr == "void" {
                ctx.push(format!(
                    "call void {code}(i8* {env}, {acc_repr} {carried}, {element_repr} {element_value})"
                ));
                String::new()
            } else {
                let next = ctx.tmp();
                ctx.push(format!(
                    "{next} = call {ret_repr} {code}(i8* {env}, {acc_repr} {carried}, {element_repr} {element_value})"
                ));
                mark_result_owned(ctx, &next, &ret.kind);
                next
            };
            if !next.is_empty() {
                ctx.push(format!("store {acc_repr} {next}, ptr {acc_slot}"));
            }
            ctx.push(format!("br label %{inc_label}"));
        }
    }

    ctx.push(format!("{inc_label}:"));
    let next = ctx.tmp();
    ctx.push(format!("{next} = add i64 {current}, 1"));
    ctx.push(format!("store i64 {next}, ptr {index_slot}"));
    ctx.push(format!("br label %{cond_label}"));

    ctx.push(format!("{done_label}:"));
    // The function value was made for this call: if it was a closure, its
    // environment belongs to the call and goes with it.
    ctx.release_owned(&func_value.value);

    match op {
        HigherOrderOp::Map | HigherOrderOp::Filter => {
            let out = out_slot.clone().unwrap_or_default();
            // The write path stores elements without touching the header, so
            // publish the count once here. The fallback path's `push` kept the
            // same count as it went, so this writes the value it already holds.
            if let Some(written) = &written_slot {
                let written_count = ctx.tmp();
                ctx.push(format!("{written_count} = load i64, ptr {written}"));
                let len_ptr = ctx.tmp();
                ctx.push(format!(
                    "{len_ptr} = getelementptr {SLICE_REPR}, ptr {out}, i32 0, i32 1"
                ));
                ctx.push(format!("store i64 {written_count}, ptr {len_ptr}"));
            }
            let result = ctx.tmp();
            ctx.push(format!("{result} = load {SLICE_REPR}, ptr {out}"));
            GenValue {
                value: result,
                ty: ty.clone(),
            }
        }
        HigherOrderOp::Fold => {
            let acc_slot = accumulator.clone().unwrap_or_default();
            let acc_repr = repr_type(ty);
            let result = ctx.tmp();
            ctx.push(format!("{result} = load {acc_repr}, ptr {acc_slot}"));
            GenValue {
                value: result,
                ty: ty.clone(),
            }
        }
    }
}

/// Put a value into the new array.
///
/// The output was reserved with room for the source length, so in the normal
/// case the value is written straight into the buffer: one store, no call. The
/// guard is the data pointer read before the loop; it is null only when the
/// reservation could not allocate, and then this falls back to the runtime's
/// grow path. Both paths count the value in `written`, whose value becomes the
/// header's length once the loop is done.
///
/// A `string` is retained first -- or, when it is a temporary, its reference is
/// transferred -- so the new array owns one reference to every element it
/// holds.
fn store_into(
    ctx: &mut FnCtx<'_>,
    out: &str,
    data: &str,
    written: &str,
    elem_kind: &TypeKind,
    value: &str,
) {
    let mut value = value.to_string();
    if matches!(elem_kind, TypeKind::Str) {
        if ctx.is_owned(&value) {
            ctx.forget_owned(&value);
        } else if !value.is_empty() {
            let retained = ctx.tmp();
            ctx.push_str_retain_into(&retained, &value);
            value = retained;
        }
    }
    let elem_repr = repr_type(elem_kind);
    let elem_size = size_of(elem_kind);
    let store_label = ctx.alloc_label_base() + "_store";
    let push_label = ctx.alloc_label_base() + "_push";
    let done_label = ctx.alloc_label_base() + "_stored";
    let has_room = ctx.tmp();
    ctx.push(format!("{has_room} = icmp ne i8* {data}, null"));
    ctx.push(format!(
        "br i1 {has_room}, label %{store_label}, label %{push_label}"
    ));

    ctx.push(format!("{store_label}:"));
    let at = ctx.tmp();
    ctx.push(format!("{at} = load i64, ptr {written}"));
    let offset = ctx.tmp();
    ctx.push(format!("{offset} = mul i64 {at}, {elem_size}"));
    let address = ctx.tmp();
    ctx.push(format!(
        "{address} = getelementptr i8, ptr {data}, i64 {offset}"
    ));
    ctx.push(format!("store {elem_repr} {value}, ptr {address}"));
    let next = ctx.tmp();
    ctx.push(format!("{next} = add i64 {at}, 1"));
    ctx.push(format!("store i64 {next}, ptr {written}"));
    ctx.push(format!("br label %{done_label}"));

    ctx.push(format!("{push_label}:"));
    let temp = ctx.alloc_slot();
    ctx.push_alloca(format!("{temp} = alloca {elem_repr}"));
    ctx.push(format!("store {elem_repr} {value}, ptr {temp}"));
    ctx.push(format!(
        "call void @yashy_slice_push(ptr {out}, ptr {temp}, i64 {elem_size})"
    ));
    // `push` counted the element in the header, but the length is published
    // from `written` after the loop, so keep the two in step here too.
    let at = ctx.tmp();
    ctx.push(format!("{at} = load i64, ptr {written}"));
    let next = ctx.tmp();
    ctx.push(format!("{next} = add i64 {at}, 1"));
    ctx.push(format!("store i64 {next}, ptr {written}"));
    ctx.push(format!("br label %{done_label}"));

    ctx.push(format!("{done_label}:"));
}
