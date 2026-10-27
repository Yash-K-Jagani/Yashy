//! Slices: the literal, `push`, `pop`, indexing, `len`, and `free_slice`.
//!
//! A slice value is the three-word header `{ i8*, i64, i64 }` — data pointer,
//! length, capacity — matching `yashy_slice` in the runtime header. The header
//! is a value; the backing array is one heap block owned by the binding that
//! holds the header. `push` takes the *address* of the header (the binding's
//! alloca) because a grow must update length and capacity in place.

use yashy_sema::{HirExpr, TypeKind};

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
/// One `yashy_alloc` for `n` elements, one store per element, then the three
/// `insertvalue`s that build the header. An empty literal allocates nothing and
/// is the null header, so `[]T {}` is a valid value of every slice type and its
/// first `push` starts from capacity zero.
///
/// A `string` element is retained by the slice: a local or literal is borrowed
/// and gets a new reference, while a temporary result (`a + b`) already owns
/// one and transfers it. The slice releases every element in `free_slice`.
pub fn gen_literal(
    ctx: &mut FnCtx<'_>,
    elem_ty: &TypeKind,
    elements: &[HirExpr],
    slice_ty: &TypeKind,
) -> GenValue {
    let elem_repr = repr_type(elem_ty);
    let elem_size = size_of(elem_ty);
    let count = elements.len();
    let is_string = matches!(elem_ty, TypeKind::Str);

    let data = if count == 0 || elem_size == 0 {
        "null".to_string()
    } else {
        let bytes = ctx.tmp();
        ctx.push(format!("{bytes} = mul i64 {count}, {elem_size}"));
        let data = ctx.tmp();
        ctx.push(format!(
            "{data} = call i8* @yashy_alloc(i64 {bytes}, i64 {SLICE_ALIGN})"
        ));
        data
    };

    for (index, element) in elements.iter().enumerate() {
        let mut value = gen_expr(ctx, element);
        if is_string {
            if ctx.is_owned(&value.value) {
                ctx.forget_owned(&value.value);
            } else if !value.value.is_empty() {
                let retained = ctx.tmp();
                ctx.push_str_retain_into(&retained, &value.value);
                value.value = retained;
            }
        }
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
/// A `string` is retained first -- or, when it is a temporary, its reference is
/// transferred -- so the slice owns one reference to every element it holds.
/// Compound elements are refused until their real stride is known, so no
/// aggregate copy can be under-sized here.
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
    let mut value = gen_expr(ctx, value_expr);
    if matches!(element.kind, TypeKind::Str) {
        if ctx.is_owned(&value.value) {
            ctx.forget_owned(&value.value);
        } else if !value.value.is_empty() {
            let retained = ctx.tmp();
            ctx.push_str_retain_into(&retained, &value.value);
            value.value = retained;
        }
    }
    push_value(ctx, &slot, &element.kind, &value.value);
    unit()
}

/// Append one value to a slice binding, with the capacity check in line.
///
/// The runtime's `yashy_slice_push` does the whole job, but a call per element
/// is the dominant cost of a `push` loop. The common case -- the backing array
/// exists and there is room -- is two loads, two compares, and a store. Only
/// the grow case calls the runtime, which keeps the growth policy (four, then
/// doubling), the copy, and the failure behavior in one place. A header that
/// carries a capacity without bytes also takes the call, so the runtime can
/// repair it.
///
/// The binding's header is never the pointer the call receives. A header
/// passed to a call is an address the compiler cannot promote, so `len`, `cap`
/// and `data` would be reloaded from the stack on every iteration, which is
/// most of the gap between this loop and the same loop in C. The cold path
/// copies the header out, calls the runtime on the copy, and writes the result
/// back.
pub(crate) fn push_value(ctx: &mut FnCtx<'_>, slot: &str, elem_kind: &TypeKind, value: &str) {
    let elem_repr = repr_type(elem_kind);
    let elem_size = size_of(elem_kind);
    let len_ptr = ctx.tmp();
    ctx.push(format!(
        "{len_ptr} = getelementptr {SLICE_REPR}, ptr {slot}, i32 0, i32 1"
    ));
    let len = ctx.tmp();
    ctx.push(format!("{len} = load i64, ptr {len_ptr}"));
    let cap_ptr = ctx.tmp();
    ctx.push(format!(
        "{cap_ptr} = getelementptr {SLICE_REPR}, ptr {slot}, i32 0, i32 2"
    ));
    let cap = ctx.tmp();
    ctx.push(format!("{cap} = load i64, ptr {cap_ptr}"));
    let data_ptr = ctx.tmp();
    ctx.push(format!(
        "{data_ptr} = getelementptr {SLICE_REPR}, ptr {slot}, i32 0, i32 0"
    ));
    let data = ctx.tmp();
    ctx.push(format!("{data} = load i8*, ptr {data_ptr}"));
    let has_room = ctx.tmp();
    ctx.push(format!("{has_room} = icmp slt i64 {len}, {cap}"));
    let has_data = ctx.tmp();
    ctx.push(format!("{has_data} = icmp ne i8* {data}, null"));
    let fast = ctx.tmp();
    ctx.push(format!("{fast} = and i1 {has_room}, {has_data}"));
    let store_label = ctx.alloc_label_base() + "_store";
    let grow_label = ctx.alloc_label_base() + "_grow";
    let done_label = ctx.alloc_label_base() + "_pushed";
    ctx.push(format!(
        "br i1 {fast}, label %{store_label}, label %{grow_label}"
    ));

    ctx.push(format!("{store_label}:"));
    let offset = ctx.tmp();
    ctx.push(format!("{offset} = mul i64 {len}, {elem_size}"));
    let address = ctx.tmp();
    ctx.push(format!(
        "{address} = getelementptr i8, ptr {data}, i64 {offset}"
    ));
    ctx.push(format!("store {elem_repr} {value}, ptr {address}"));
    let next = ctx.tmp();
    ctx.push(format!("{next} = add i64 {len}, 1"));
    ctx.push(format!("store i64 {next}, ptr {len_ptr}"));
    ctx.push(format!("br label %{done_label}"));

    ctx.push(format!("{grow_label}:"));
    // The runtime call takes a header, and a header handed to a call cannot be
    // promoted to registers: every `len`/`cap`/`data` load in the loop would
    // go back to memory, which is most of the cost of a push loop. The call is
    // cold, so it gets a copy of the header and the result is written back;
    // the binding's own header never escapes and the hot path stays in
    // registers.
    let header = ctx.alloc_slot();
    ctx.push_alloca(format!("{header} = alloca {SLICE_REPR}"));
    let snapshot = ctx.tmp();
    ctx.push(format!("{snapshot} = load {SLICE_REPR}, ptr {slot}"));
    ctx.push(format!("store {SLICE_REPR} {snapshot}, ptr {header}"));
    let temp = ctx.alloc_slot();
    ctx.push_alloca(format!("{temp} = alloca {elem_repr}"));
    ctx.push(format!("store {elem_repr} {value}, ptr {temp}"));
    ctx.push(format!(
        "call void @yashy_slice_push(ptr {header}, ptr {temp}, i64 {elem_size})"
    ));
    let updated = ctx.tmp();
    ctx.push(format!("{updated} = load {SLICE_REPR}, ptr {header}"));
    ctx.push(format!("store {SLICE_REPR} {updated}, ptr {slot}"));
    ctx.push(format!("br label %{done_label}"));

    ctx.push(format!("{done_label}:"));
}

/// Lower `len(slice) -> i64`.
///
/// The length is field 1 of the header value; no call is needed. A string
/// length is still `yashy_str_len` (see `strings::gen_str_len`).
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
        "{len} = extractvalue {SLICE_REPR} {}, 1",
        header.value
    ));
    GenValue {
        value: len,
        ty: TypeKind::I64,
    }
}

/// Lower `args() -> []string`.
///
/// The runtime fills a stack header with owned strings for the process's user
/// arguments, exactly as `split`/`lines` fill theirs. Building the slice
/// element by element in IR would duplicate the runtime's push and ownership
/// rules; one call keeps the contract in one place.
pub fn gen_args(ctx: &mut FnCtx<'_>, slice_ty: &TypeKind) -> GenValue {
    if !matches!(slice_ty, TypeKind::Slice(_)) {
        return unit();
    }
    let out_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{out_slot} = alloca {SLICE_REPR}"));
    ctx.push(format!(
        "store {SLICE_REPR} zeroinitializer, ptr {out_slot}"
    ));
    ctx.push(format!("call void @yashy_args_slice(ptr {out_slot})"));
    let result = ctx.tmp();
    ctx.push(format!("{result} = load {SLICE_REPR}, ptr {out_slot}"));
    GenValue {
        value: result,
        ty: slice_ty.clone(),
    }
}

/// Build `Option[element]` from a 0/1 flag and an initialized out slot.
///
/// The payload is loaded even on a miss. That is why every caller stores
/// `zeroinitializer` into the slot first: the loaded value is then defined, and
/// the tag is what says the payload must not be read.
pub(crate) fn option_from_flag(
    ctx: &mut FnCtx<'_>,
    flag: &str,
    out_slot: &str,
    element: &TypeKind,
    option_ty: &TypeKind,
) -> GenValue {
    let element_repr = repr_type(element);
    let option_repr = repr_type(option_ty);
    let tag = ctx.tmp();
    ctx.push(format!("{tag} = trunc i64 {flag} to i8"));
    let first = ctx.tmp();
    ctx.push(format!(
        "{first} = insertvalue {option_repr} undef, i8 {tag}, 0"
    ));
    let payload = ctx.tmp();
    ctx.push(format!("{payload} = load {element_repr}, ptr {out_slot}"));
    let done = ctx.tmp();
    ctx.push(format!(
        "{done} = insertvalue {option_repr} {first}, {element_repr} {payload}, 1"
    ));
    GenValue {
        value: done,
        ty: option_ty.clone(),
    }
}

/// Lower `xs[index] -> Option[T]`.
///
/// The header is copied into a slot because `yashy_slice_get` takes its
/// address; the copy is a read-only snapshot, so a binding and a temporary take
/// the same path. The runtime owns the bounds test: negative and out of range
/// are `None`, never a fault or a wraparound.
pub fn gen_index(
    ctx: &mut FnCtx<'_>,
    base: &HirExpr,
    index: &HirExpr,
    option_ty: &TypeKind,
) -> GenValue {
    let TypeKind::Option(element) = option_ty else {
        return unit();
    };
    let header = gen_expr(ctx, base);
    let position = gen_expr(ctx, index);
    let header_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{header_slot} = alloca {SLICE_REPR}"));
    ctx.push(format!(
        "store {SLICE_REPR} {}, ptr {header_slot}",
        header.value
    ));
    let element_repr = repr_type(&element.kind);
    let out_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{out_slot} = alloca {element_repr}"));
    ctx.push(format!(
        "store {element_repr} zeroinitializer, ptr {out_slot}"
    ));
    let flag = ctx.tmp();
    ctx.push(format!(
        "{flag} = call i64 @yashy_slice_get(ptr {header_slot}, i64 {}, ptr {out_slot}, i64 {})",
        position.value,
        size_of(&element.kind)
    ));
    option_from_flag(ctx, &flag, &out_slot, &element.kind, option_ty)
}

/// Lower `pop(xs) -> Option[T]`.
///
/// Unlike indexing, this updates the binding's header in place: the runtime
/// decrements `len` and copies the element it removed. The payload is owned by
/// the caller because it no longer belongs to the slice.
pub fn gen_pop(ctx: &mut FnCtx<'_>, args: &[HirExpr], option_ty: &TypeKind) -> GenValue {
    let Some((slot, _)) = binding_slot(ctx, args) else {
        return unit();
    };
    let TypeKind::Option(element) = option_ty else {
        return unit();
    };
    let element_repr = repr_type(&element.kind);
    let out_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{out_slot} = alloca {element_repr}"));
    ctx.push(format!(
        "store {element_repr} zeroinitializer, ptr {out_slot}"
    ));
    let flag = ctx.tmp();
    ctx.push(format!(
        "{flag} = call i64 @yashy_slice_pop(ptr {slot}, ptr {out_slot}, i64 {})",
        size_of(&element.kind)
    ));
    option_from_flag(ctx, &flag, &out_slot, &element.kind, option_ty)
}

/// Lower `slice_set(slice, index, value) -> bool`.
///
/// The value is copied into a temporary slot and the runtime copies it over the
/// element only when the index is in range. The receiver must be a binding, as
/// for `push` and `pop`, because the runtime takes the address of its header.
pub fn gen_set(ctx: &mut FnCtx<'_>, args: &[HirExpr]) -> GenValue {
    let Some((slot, ty)) = binding_slot(ctx, args) else {
        return GenValue {
            value: "false".to_string(),
            ty: TypeKind::Bool,
        };
    };
    let TypeKind::Slice(element) = &ty else {
        return GenValue {
            value: "false".to_string(),
            ty: TypeKind::Bool,
        };
    };
    let (Some(index_expr), Some(value_expr)) = (args.get(1), args.get(2)) else {
        return GenValue {
            value: "false".to_string(),
            ty: TypeKind::Bool,
        };
    };
    let index = gen_expr(ctx, index_expr);
    let value = gen_expr(ctx, value_expr);
    let elem_repr = repr_type(&element.kind);
    let elem_size = size_of(&element.kind);
    let temp = ctx.alloc_slot();
    ctx.push_alloca(format!("{temp} = alloca {elem_repr}"));
    ctx.push(format!("store {elem_repr} {}, ptr {temp}", value.value));
    let flag = ctx.tmp();
    ctx.push(format!(
        "{flag} = call i64 @yashy_slice_set(ptr {slot}, i64 {}, ptr {temp}, i64 {elem_size})",
        index.value
    ));
    let ok = ctx.tmp();
    ctx.push(format!("{ok} = icmp ne i64 {flag}, 0"));
    GenValue {
        value: ok,
        ty: TypeKind::Bool,
    }
}

/// Lower `slice_insert(slice, index, value) -> bool`.
///
/// Like `set`, the value goes through a temporary slot because the runtime
/// copies bytes. The runtime grows the array when the insert fills it and
/// shifts the tail with `memmove`; an index past `len` is a `false` answer.
pub fn gen_insert(ctx: &mut FnCtx<'_>, args: &[HirExpr]) -> GenValue {
    let Some((slot, ty)) = binding_slot(ctx, args) else {
        return false_value();
    };
    let TypeKind::Slice(element) = &ty else {
        return false_value();
    };
    let (Some(index_expr), Some(value_expr)) = (args.get(1), args.get(2)) else {
        return false_value();
    };
    let index = gen_expr(ctx, index_expr);
    let value = gen_expr(ctx, value_expr);
    let elem_repr = repr_type(&element.kind);
    let temp = ctx.alloc_slot();
    ctx.push_alloca(format!("{temp} = alloca {elem_repr}"));
    ctx.push(format!("store {elem_repr} {}, ptr {temp}", value.value));
    let flag = ctx.tmp();
    ctx.push(format!(
        "{flag} = call i64 @yashy_slice_insert(ptr {slot}, i64 {}, ptr {temp}, i64 {})",
        index.value,
        size_of(&element.kind)
    ));
    bool_from_flag(ctx, &flag)
}

/// Lower `slice_remove(slice, index) -> Option[T]`.
///
/// The runtime copies the removed element into the out slot before shifting
/// the tail left, so the answer owns the value that just left the array.
pub fn gen_remove(ctx: &mut FnCtx<'_>, args: &[HirExpr], option_ty: &TypeKind) -> GenValue {
    let Some((slot, _)) = binding_slot(ctx, args) else {
        return unit();
    };
    let TypeKind::Option(element) = option_ty else {
        return unit();
    };
    let Some(index_expr) = args.get(1) else {
        return unit();
    };
    let index = gen_expr(ctx, index_expr);
    let element_repr = repr_type(&element.kind);
    let out_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{out_slot} = alloca {element_repr}"));
    ctx.push(format!(
        "store {element_repr} zeroinitializer, ptr {out_slot}"
    ));
    let flag = ctx.tmp();
    ctx.push(format!(
        "{flag} = call i64 @yashy_slice_remove(ptr {slot}, i64 {}, ptr {out_slot}, i64 {})",
        index.value,
        size_of(&element.kind)
    ));
    option_from_flag(ctx, &flag, &out_slot, &element.kind, option_ty)
}

/// Lower `slice_clear(slice) -> ()`.
///
/// A `[]string` releases each element before dropping the length; every other
/// element type is a plain machine word or aggregate, so the runtime only
/// zeroes the length. The backing array and its capacity stay allocated.
pub fn gen_clear(ctx: &mut FnCtx<'_>, args: &[HirExpr]) -> GenValue {
    let Some((slot, ty)) = binding_slot(ctx, args) else {
        return unit();
    };
    let TypeKind::Slice(element) = &ty else {
        return unit();
    };
    let callee = if matches!(element.kind, TypeKind::Str) {
        "yashy_slice_clear_strings"
    } else {
        "yashy_slice_clear"
    };
    ctx.push(format!("call void @{callee}(ptr {slot})"));
    unit()
}

/// Lower `slice_reverse(slice) -> ()`.
pub fn gen_reverse(ctx: &mut FnCtx<'_>, args: &[HirExpr]) -> GenValue {
    let Some((slot, ty)) = binding_slot(ctx, args) else {
        return unit();
    };
    let TypeKind::Slice(element) = &ty else {
        return unit();
    };
    ctx.push(format!(
        "call void @yashy_slice_reverse(ptr {slot}, i64 {})",
        size_of(&element.kind)
    ));
    unit()
}

/// Lower `slice_sort(slice) -> ()`, `i64` elements only.
pub fn gen_sort(ctx: &mut FnCtx<'_>, args: &[HirExpr]) -> GenValue {
    let Some((slot, _)) = binding_slot(ctx, args) else {
        return unit();
    };
    ctx.push(format!("call void @yashy_slice_sort_i64(ptr {slot})"));
    unit()
}

/// Store a slice header in a fresh slot and return the slot.
fn header_slot(ctx: &mut FnCtx<'_>, value: &GenValue) -> String {
    let slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{slot} = alloca {SLICE_REPR}"));
    ctx.push(format!("store {SLICE_REPR} {}, ptr {slot}", value.value));
    slot
}

/// The runtime entry point and the lowered value for an equality search.
///
/// `bool` is stored as one byte, but the C ABI of the helper takes a machine
/// word, so the value is zero-extended at the call site. A `string` handle is
/// an `i8*` in the IR and an `yashy_str*` in C; both are the same pointer.
fn index_of_call(
    ctx: &mut FnCtx<'_>,
    element: &TypeKind,
    slot: &str,
    value: &GenValue,
) -> Option<String> {
    let symbol = match element {
        TypeKind::I64 => "yashy_slice_index_of_i64",
        TypeKind::F64 => "yashy_slice_index_of_f64",
        TypeKind::Bool => "yashy_slice_index_of_bool",
        TypeKind::Str => "yashy_slice_index_of_string",
        _ => return None,
    };
    let argument = if matches!(element, TypeKind::Bool) {
        let wide = ctx.tmp();
        ctx.push(format!("{wide} = zext i1 {} to i64", value.value));
        wide
    } else {
        value.value.clone()
    };
    let repr = if matches!(element, TypeKind::F64) {
        "double"
    } else if matches!(element, TypeKind::Str) {
        "i8*"
    } else {
        "i64"
    };
    let result = ctx.tmp();
    ctx.push(format!(
        "{result} = call i64 @{symbol}(ptr {slot}, {repr} {argument})"
    ));
    Some(result)
}

/// Lower `slice_contains(slice, value) -> bool`.
pub fn gen_contains(ctx: &mut FnCtx<'_>, args: &[HirExpr]) -> GenValue {
    let Some(receiver) = args.first() else {
        return false_value();
    };
    let Some(value_expr) = args.get(1) else {
        return false_value();
    };
    let header = gen_expr(ctx, receiver);
    let TypeKind::Slice(element) = &header.ty else {
        return false_value();
    };
    let slot = header_slot(ctx, &header);
    let value = gen_expr(ctx, value_expr);
    let Some(result) = index_of_call(ctx, &element.kind, &slot, &value) else {
        return false_value();
    };
    if matches!(element.kind, TypeKind::Str) {
        ctx.release_owned(&value.value);
    }
    let found = ctx.tmp();
    ctx.push(format!("{found} = icmp sge i64 {result}, 0"));
    GenValue {
        value: found,
        ty: TypeKind::Bool,
    }
}

/// Lower `slice_index_of(slice, value) -> Option[i64]`.
pub fn gen_index_of(
    ctx: &mut FnCtx<'_>,
    args: &[HirExpr],
    option_ty: &TypeKind,
) -> GenValue {
    let TypeKind::Option(element) = option_ty else {
        return unit();
    };
    let Some(receiver) = args.first() else {
        return unit();
    };
    let Some(value_expr) = args.get(1) else {
        return unit();
    };
    let header = gen_expr(ctx, receiver);
    let TypeKind::Slice(slice_element) = &header.ty else {
        return unit();
    };
    let slot = header_slot(ctx, &header);
    let value = gen_expr(ctx, value_expr);
    let Some(result) = index_of_call(ctx, &slice_element.kind, &slot, &value) else {
        return unit();
    };
    if matches!(slice_element.kind, TypeKind::Str) {
        ctx.release_owned(&value.value);
    }
    let found = ctx.tmp();
    ctx.push(format!("{found} = icmp sge i64 {result}, 0"));
    let flag = ctx.tmp();
    ctx.push(format!("{flag} = zext i1 {found} to i64"));
    let out_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{out_slot} = alloca i64"));
    ctx.push(format!("store i64 {result}, ptr {out_slot}"));
    option_from_flag(ctx, &flag, &out_slot, &element.kind, option_ty)
}

/// Lower `slice_extend(slice, other) -> ()`.
///
/// `other` is consumed: the runtime appends its elements and then frees its
/// backing array. For `[]string` the appended elements are retained before the
/// consumed array releases its own references.
pub fn gen_extend(ctx: &mut FnCtx<'_>, args: &[HirExpr]) -> GenValue {
    let Some((slot, ty)) = binding_slot(ctx, args) else {
        return unit();
    };
    let TypeKind::Slice(element) = &ty else {
        return unit();
    };
    let Some(other_expr) = args.get(1) else {
        return unit();
    };
    let other = gen_expr(ctx, other_expr);
    let other_slot = header_slot(ctx, &other);
    if matches!(element.kind, TypeKind::Str) {
        ctx.push(format!(
            "call void @yashy_slice_extend_strings(ptr {slot}, ptr {other_slot})"
        ));
    } else {
        ctx.push(format!(
            "call void @yashy_slice_extend(ptr {slot}, ptr {other_slot}, i64 {})",
            size_of(&element.kind)
        ));
    }
    unit()
}

/// Lower `slice_copy(slice, start, end) -> []T`.
///
/// The runtime clamps both bounds to the source length and copies the range
/// into a fresh array. A `[]string` result retains each element, so it owns one
/// reference per element independently of the source.
pub fn gen_copy(ctx: &mut FnCtx<'_>, args: &[HirExpr], slice_ty: &TypeKind) -> GenValue {
    let TypeKind::Slice(element) = slice_ty else {
        return unit();
    };
    let (Some(source_expr), Some(start_expr), Some(end_expr)) =
        (args.first(), args.get(1), args.get(2))
    else {
        return unit();
    };
    let source = gen_expr(ctx, source_expr);
    let source_slot = header_slot(ctx, &source);
    let start = gen_expr(ctx, start_expr);
    let end = gen_expr(ctx, end_expr);
    let out_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{out_slot} = alloca {SLICE_REPR}"));
    ctx.push(format!(
        "store {SLICE_REPR} zeroinitializer, ptr {out_slot}"
    ));
    if matches!(element.kind, TypeKind::Str) {
        ctx.push(format!(
            "call void @yashy_slice_slice_strings(ptr {out_slot}, ptr {source_slot}, i64 {}, i64 {})",
            start.value, end.value
        ));
    } else {
        ctx.push(format!(
            "call void @yashy_slice_slice(ptr {out_slot}, ptr {source_slot}, i64 {}, i64 {}, i64 {})",
            start.value,
            end.value,
            size_of(&element.kind)
        ));
    }
    let result = ctx.tmp();
    ctx.push(format!("{result} = load {SLICE_REPR}, ptr {out_slot}"));
    GenValue {
        value: result,
        ty: slice_ty.clone(),
    }
}

/// A `false` value for a slice method that cannot be lowered.
pub fn false_value() -> GenValue {
    GenValue {
        value: "false".to_string(),
        ty: TypeKind::Bool,
    }
}

/// Turn a runtime 0/1 flag into an LLVM `i1`.
pub fn bool_from_flag(ctx: &mut FnCtx<'_>, flag: &str) -> GenValue {
    let ok = ctx.tmp();
    ctx.push(format!("{ok} = icmp ne i64 {flag}, 0"));
    GenValue {
        value: ok,
        ty: TypeKind::Bool,
    }
}

/// Lower `free_slice(slice) -> ()`.
///
/// The runtime frees the backing array and zeroes the header it is given. The
/// checker has already moved the binding out, so the value cannot be read
/// again. A `[]string` goes through `yashy_slice_free_strings`, which releases
/// each element before freeing the array: those references belong to the slice,
/// not to the binding that held the header.
///
/// The runtime is handed a copy of the header, and the binding is zeroed here.
/// Freeing the binding's own header would make it an address the compiler
/// cannot promote, which would keep every `len`/`cap`/`data` load in a
/// surrounding loop in memory.
pub fn gen_free(ctx: &mut FnCtx<'_>, args: &[HirExpr]) -> GenValue {
    let Some((slot, ty)) = binding_slot(ctx, args) else {
        return unit();
    };
    let strings = match &ty {
        TypeKind::Slice(element) => matches!(element.kind, TypeKind::Str),
        _ => false,
    };
    let header = ctx.alloc_slot();
    ctx.push_alloca(format!("{header} = alloca {SLICE_REPR}"));
    let snapshot = ctx.tmp();
    ctx.push(format!("{snapshot} = load {SLICE_REPR}, ptr {slot}"));
    ctx.push(format!("store {SLICE_REPR} {snapshot}, ptr {header}"));
    let symbol = if strings {
        "yashy_slice_free_strings"
    } else {
        "yashy_slice_free"
    };
    ctx.push(format!("call void @{symbol}(ptr {header})"));
    ctx.push(format!("store {SLICE_REPR} zeroinitializer, ptr {slot}"));
    unit()
}
