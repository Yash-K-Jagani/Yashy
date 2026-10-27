//! String operations that are runtime calls rather than instructions.
//!
//! A `string` value is a pointer to a refcounted `%yashy_str` (see
//! `crate::module` and the master plan's ledger L1), so there is no instruction
//! that concatenates or compares one. Each operator here is a call into the
//! runtime half of ledger L1, and each is a place a **temporary is consumed**:
//! the runtime copies the bytes it is given and keeps no pointer, so an operand
//! the expression itself built is released right after the call. A literal
//! (refcount -1) and a local's value are not owned names and are left alone.

use yashy_sema::{BinOp, HirExpr, TypeKind};

use crate::expr_gen::{gen_expr, FnCtx, GenValue, RefKind};

/// The IR spelling of a slice header, matching `crate::slices`.
const SLICE_REPR: &str = "{ i8*, i64, i64 }";

/// Lower one binary operator applied to two `string` operands.
///
/// `+` concatenates and `==`/`!=` compare contents. The checker permits no
/// other operator on two strings, so an unexpected one is a compiler bug: it
/// produces a comment and a `false` rather than a call to a symbol that does not
/// exist, which keeps the module loadable and the bug visible.
pub fn gen_string_binary(ctx: &mut FnCtx<'_>, op: BinOp, l: &GenValue, r: &GenValue) -> GenValue {
    match op {
        BinOp::Add => {
            // A left operand this expression built is a temporary with no other
            // reader, so the runtime can extend its buffer instead of copying
            // it into a fresh string. That is the shape of every left-grouped
            // chain -- `"a" + "b" + "c"`, and the tree a debug printer or an
            // interpolation builds -- so the chain stays linear in the bytes it
            // produces rather than copying the whole prefix at each step.
            //
            // `yashy_str_append` takes over the left reference: it returns the
            // same pointer when it can grow in place, and otherwise builds a
            // new string and releases the old one. Either way the temporary is
            // forgotten here and the result is what the caller owns.
            if ctx.is_owned(&l.value) {
                let appended = ctx.tmp();
                ctx.push(format!(
                    "{appended} = call i8* @yashy_str_append(i8* {}, i8* {})",
                    l.value, r.value
                ));
                ctx.forget_owned(&l.value);
                ctx.mark_owned(&appended, RefKind::Str);
                ctx.release_owned(&r.value);
                return GenValue {
                    value: appended,
                    ty: TypeKind::Str,
                };
            }
            let joined = ctx.tmp();
            ctx.push(format!(
                "{joined} = call i8* @yashy_str_concat(i8* {}, i8* {})",
                l.value, r.value
            ));
            // The result is a fresh string the caller owns; the operands have
            // been copied, so an owned one is released here.
            ctx.mark_owned(&joined, RefKind::Str);
            ctx.release_owned(&l.value);
            ctx.release_owned(&r.value);
            GenValue {
                value: joined,
                ty: TypeKind::Str,
            }
        }
        BinOp::Eq | BinOp::Ne => {
            let equal = ctx.tmp();
            ctx.push(format!(
                "{equal} = call i1 @yashy_str_eq(i8* {}, i8* {})",
                l.value, r.value
            ));
            // The comparison reads the bytes and keeps nothing.
            ctx.release_owned(&l.value);
            ctx.release_owned(&r.value);
            if op == BinOp::Eq {
                return GenValue {
                    value: equal,
                    ty: TypeKind::Bool,
                };
            }
            let negated = ctx.tmp();
            ctx.push(format!("{negated} = xor i1 {equal}, true"));
            GenValue {
                value: negated,
                ty: TypeKind::Bool,
            }
        }
        other => {
            ctx.push(format!(
                "; note: `{other:?}` on two strings has no runtime call"
            ));
            GenValue {
                value: "false".to_string(),
                ty: TypeKind::Bool,
            }
        }
    }
}

/// Lower `byte_at(s, i)`: the byte at `i`, or 0 when `i` is out of range.
///
/// A byte string, so the value is a byte and not a character: `byte_at("é", 0)`
/// is `0xC3`. Out of range is an answer rather than a fault, matching the
/// runtime (ledger L1). The string is read and nothing is kept, so a temporary
/// is consumed.
pub fn gen_str_byte_at(ctx: &mut FnCtx<'_>, args: &[yashy_sema::HirExpr]) -> GenValue {
    let value = args
        .first()
        .map(|a| crate::expr_gen::gen_expr(ctx, a))
        .filter(|v| !v.value.is_empty())
        .map(|v| v.value)
        .unwrap_or_else(|| "null".to_string());
    let index = args
        .get(1)
        .map(|a| crate::expr_gen::gen_expr(ctx, a))
        .filter(|v| !v.value.is_empty())
        .map(|v| v.value)
        .unwrap_or_else(|| "0".to_string());
    let byte = ctx.tmp();
    ctx.push(format!(
        "{byte} = call i8 @yashy_str_byte_at(i8* {value}, i64 {index})"
    ));
    ctx.release_owned(&value);
    GenValue {
        value: byte,
        ty: TypeKind::Char,
    }
}

/// Lower `len(s)`: the byte length of a string.
///
/// Bytes, not characters: a Yashy string is a byte string, so `len("é")` is 2.
/// The length is stored, so this reads a field rather than scanning -- which is
/// also why measuring a string does not keep it alive.
pub fn gen_str_len(ctx: &mut FnCtx<'_>, args: &[yashy_sema::HirExpr]) -> GenValue {
    let value = args
        .first()
        .map(|a| crate::expr_gen::gen_expr(ctx, a))
        .filter(|v| !v.value.is_empty())
        .map(|v| v.value)
        .unwrap_or_else(|| "null".to_string());
    let length = ctx.tmp();
    ctx.push(format!("{length} = call i64 @yashy_str_len(i8* {value})"));
    // The length is read out and nothing is kept, so a temporary is consumed.
    ctx.release_owned(&value);
    GenValue {
        value: length,
        ty: TypeKind::I64,
    }
}

fn unit() -> GenValue {
    GenValue {
        value: String::new(),
        ty: TypeKind::Unit,
    }
}

/// Lower `str_get(s, i) -> Option[i64]`: the byte at `i`, or `None` out of
/// range. The runtime writes the byte through an out slot because `Option`
/// needs a payload even on the `None` path.
pub fn gen_str_get(ctx: &mut FnCtx<'_>, args: &[HirExpr], option_ty: &TypeKind) -> GenValue {
    let TypeKind::Option(element) = option_ty else {
        return unit();
    };
    let (Some(value_expr), Some(index_expr)) = (args.first(), args.get(1)) else {
        return unit();
    };
    let value = gen_expr(ctx, value_expr);
    let index = gen_expr(ctx, index_expr);
    let out_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{out_slot} = alloca i64"));
    ctx.push(format!("store i64 0, ptr {out_slot}"));
    let flag = ctx.tmp();
    ctx.push(format!(
        "{flag} = call i64 @yashy_str_get(i8* {}, i64 {}, ptr {out_slot})",
        value.value, index.value
    ));
    ctx.release_owned(&value.value);
    crate::slices::option_from_flag(ctx, &flag, &out_slot, &element.kind, option_ty)
}

/// Lower `str_index_of(s, needle) -> Option[i64]`.
///
/// The runtime returns -1 for a miss, so the flag is the comparison against
/// zero and the same value is the payload for a hit.
pub fn gen_str_index_of(ctx: &mut FnCtx<'_>, args: &[HirExpr], option_ty: &TypeKind) -> GenValue {
    let TypeKind::Option(element) = option_ty else {
        return unit();
    };
    let (Some(value_expr), Some(needle_expr)) = (args.first(), args.get(1)) else {
        return unit();
    };
    let value = gen_expr(ctx, value_expr);
    let needle = gen_expr(ctx, needle_expr);
    let found_at = ctx.tmp();
    ctx.push(format!(
        "{found_at} = call i64 @yashy_str_index_of(i8* {}, i8* {})",
        value.value, needle.value
    ));
    ctx.release_owned(&value.value);
    ctx.release_owned(&needle.value);
    let found = ctx.tmp();
    ctx.push(format!("{found} = icmp sge i64 {found_at}, 0"));
    let flag = ctx.tmp();
    ctx.push(format!("{flag} = zext i1 {found} to i64"));
    let out_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{out_slot} = alloca i64"));
    ctx.push(format!("store i64 {found_at}, ptr {out_slot}"));
    crate::slices::option_from_flag(ctx, &flag, &out_slot, &element.kind, option_ty)
}

/// Lower `str_split(s, separator) -> []string`.
///
/// The runtime fills an out header; every element it puts in the result is a
/// fresh owned string, so freeing the slice releases the pieces.
pub fn gen_str_split(ctx: &mut FnCtx<'_>, args: &[HirExpr], slice_ty: &TypeKind) -> GenValue {
    if !matches!(slice_ty, TypeKind::Slice(_)) {
        return unit();
    }
    let (Some(value_expr), Some(separator_expr)) = (args.first(), args.get(1)) else {
        return unit();
    };
    let value = gen_expr(ctx, value_expr);
    let separator = gen_expr(ctx, separator_expr);
    let out_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{out_slot} = alloca {SLICE_REPR}"));
    ctx.push(format!("store {SLICE_REPR} zeroinitializer, ptr {out_slot}"));
    ctx.push(format!(
        "call void @yashy_str_split(ptr {out_slot}, i8* {}, i8* {})",
        value.value, separator.value
    ));
    ctx.release_owned(&value.value);
    ctx.release_owned(&separator.value);
    let result = ctx.tmp();
    ctx.push(format!("{result} = load {SLICE_REPR}, ptr {out_slot}"));
    GenValue {
        value: result,
        ty: slice_ty.clone(),
    }
}

/// Lower `str_lines(s) -> []string`: split on `\n`, strip a trailing `\r`,
/// and omit the final empty element after a trailing newline.
pub fn gen_str_lines(ctx: &mut FnCtx<'_>, args: &[HirExpr], slice_ty: &TypeKind) -> GenValue {
    if !matches!(slice_ty, TypeKind::Slice(_)) {
        return unit();
    }
    let Some(value_expr) = args.first() else {
        return unit();
    };
    let value = gen_expr(ctx, value_expr);
    let out_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{out_slot} = alloca {SLICE_REPR}"));
    ctx.push(format!("store {SLICE_REPR} zeroinitializer, ptr {out_slot}"));
    ctx.push(format!(
        "call void @yashy_str_lines(ptr {out_slot}, i8* {})",
        value.value
    ));
    ctx.release_owned(&value.value);
    let result = ctx.tmp();
    ctx.push(format!("{result} = load {SLICE_REPR}, ptr {out_slot}"));
    GenValue {
        value: result,
        ty: slice_ty.clone(),
    }
}
