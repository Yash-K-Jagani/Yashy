//! String operations that are runtime calls rather than instructions.
//!
//! A `string` value is an `i8*` pointing at NUL-terminated bytes (M0–M2's
//! representation; the interim is recorded in the master plan's ledger L4), so
//! there is no instruction that concatenates or compares one. Each operator
//! here is a call into the runtime half of ledger L1.

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

/// Lower `len(s)`: the byte length of a string.
///
/// Bytes, not characters: an Otter string is a byte string, so `len("é")` is 2.
pub fn gen_str_len(ctx: &mut FnCtx<'_>, args: &[otter_sema::HirExpr]) -> GenValue {
    let value = args
        .first()
        .map(|a| crate::expr_gen::gen_expr(ctx, a))
        .filter(|v| !v.value.is_empty())
        .map(|v| v.value)
        .unwrap_or_else(|| "null".to_string());
    let length = ctx.tmp();
    ctx.push(format!("{length} = call i64 @otter_str_len(i8* {value})"));
    GenValue {
        value: length,
        ty: TypeKind::I64,
    }
}
