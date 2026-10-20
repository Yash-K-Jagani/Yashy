//! String operations that are runtime calls rather than instructions.
//!
//! A `string` value is a pointer to a refcounted `%otter_str` (see
//! `crate::module` and the master plan's ledger L1), so there is no instruction
//! that concatenates or compares one. Each operator here is a call into the
//! runtime half of ledger L1, and each is a place a **temporary is consumed**:
//! the runtime copies the bytes it is given and keeps no pointer, so an operand
//! the expression itself built is released right after the call. A literal
//! (refcount -1) and a local's value are not owned names and are left alone.

use otter_sema::{BinOp, TypeKind};

use crate::expr_gen::{FnCtx, GenValue};

/// Lower one binary operator applied to two `string` operands.
///
/// `+` concatenates and `==`/`!=` compare contents. The checker permits no
/// other operator on two strings, so an unexpected one is a compiler bug: it
/// produces a comment and a `false` rather than a call to a symbol that does not
/// exist, which keeps the module loadable and the bug visible.
pub fn gen_string_binary(ctx: &mut FnCtx<'_>, op: BinOp, l: &GenValue, r: &GenValue) -> GenValue {
    match op {
        BinOp::Add => {
            let joined = ctx.tmp();
            ctx.push(format!(
                "{joined} = call i8* @otter_str_concat(i8* {}, i8* {})",
                l.value, r.value
            ));
            // The result is a fresh string the caller owns; the operands have
            // been copied, so an owned one is released here.
            ctx.mark_owned(&joined);
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
                "{equal} = call i1 @otter_str_eq(i8* {}, i8* {})",
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
pub fn gen_str_byte_at(ctx: &mut FnCtx<'_>, args: &[otter_sema::HirExpr]) -> GenValue {
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
        "{byte} = call i64 @otter_str_byte_at(i8* {value}, i64 {index})"
    ));
    ctx.release_owned(&value);
    GenValue {
        value: byte,
        ty: TypeKind::I64,
    }
}

/// Lower `len(s)`: the byte length of a string.
///
/// Bytes, not characters: an Otter string is a byte string, so `len("é")` is 2.
/// The length is stored, so this reads a field rather than scanning -- which is
/// also why measuring a string does not keep it alive.
pub fn gen_str_len(ctx: &mut FnCtx<'_>, args: &[otter_sema::HirExpr]) -> GenValue {
    let value = args
        .first()
        .map(|a| crate::expr_gen::gen_expr(ctx, a))
        .filter(|v| !v.value.is_empty())
        .map(|v| v.value)
        .unwrap_or_else(|| "null".to_string());
    let length = ctx.tmp();
    ctx.push(format!("{length} = call i64 @otter_str_len(i8* {value})"));
    // The length is read out and nothing is kept, so a temporary is consumed.
    ctx.release_owned(&value);
    GenValue {
        value: length,
        ty: TypeKind::I64,
    }
}
