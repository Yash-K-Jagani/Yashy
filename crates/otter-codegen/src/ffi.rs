//! The C boundary: copying, borrowing, and the `unsafe` opt-out.
//!
//! Spec §8.1: crossing into C **copies**. Otter's `string` is a refcounted
//! object, while C wants a NUL-terminated `char*` whose lifetime the callee can
//! extend past the call. So a safe call:
//!
//! 1. copies each string argument into a fresh buffer (`otter_str_cstr`),
//! 2. makes the call with those buffers,
//! 3. frees the buffers and releases any temporary strings it consumed.
//!
//! A call site marked `unsafe` waives the copy: the callee is lent the
//! string's own bytes (`otter_str_ptr`) and may keep them. Nothing is freed,
//! because the pointer now belongs to C as far as the compiler can prove.
//!
//! A raw pointer is an address, not a value that can be copied meaningfully, so
//! it always crosses unchanged. The checker requires `unsafe` on such a call
//! for the same reason: C may keep or free what the pointer names.
//!
//! Slices do not exist yet (M6); when they do, their elements are copied here
//! the same way, with their length.

use otter_sema::{HirExternDecl, TypeKind};

use crate::expr_gen::{FnCtx, GenValue};
use crate::fn_gen::fn_return_repr;
use crate::ir::repr_type;

/// Lower a call to a C declaration, applying the boundary rules.
///
/// `args` are already-lowered values: their instructions are in the module and
/// their types decide whether a copy is needed. The returned string is the SSA
/// name of the result, or empty when the declaration returns `void`.
pub fn gen_ffi_call(
    ctx: &mut FnCtx<'_>,
    decl: &HirExternDecl,
    args: &[GenValue],
    unsafe_call: bool,
) -> String {
    let mut operands = Vec::new();
    // Buffers the callee was given and that this call still owns.
    let mut copies: Vec<String> = Vec::new();
    // Temporary strings this call consumed (safe calls only).
    let mut consumed: Vec<String> = Vec::new();

    for (index, arg) in args.iter().enumerate() {
        let is_c_string_parameter = matches!(
            decl.params.get(index).map(|p| &p.ty.kind),
            Some(TypeKind::CPtr(_))
        );
        if matches!(arg.ty, TypeKind::Str) && is_c_string_parameter {
            let data = ctx.tmp();
            if unsafe_call {
                // Lend, do not copy. C may keep this pointer, so the string
                // that owns it is deliberately not released below.
                ctx.push(format!(
                    "{data} = call i8* @otter_str_ptr(i8* {})",
                    arg.value
                ));
            } else {
                ctx.push(format!(
                    "{data} = call i8* @otter_str_cstr(i8* {})",
                    arg.value
                ));
                copies.push(data.clone());
                consumed.push(arg.value.clone());
            }
            operands.push(format!("i8* {data}"));
        } else {
            // Everything else -- integers, floats, raw pointers -- is already
            // the machine value C wants.
            operands.push(format!("{} {}", repr_type(&arg.ty), arg.value));
        }
    }

    let ret = fn_return_repr(&decl.ret.kind);
    let result = if ret == "void" {
        ctx.push(format!(
            "call void @{}({})",
            decl.name,
            operands.join(", ")
        ));
        String::new()
    } else {
        let result = ctx.tmp();
        ctx.push(format!(
            "{result} = call {ret} @{}({})",
            decl.name,
            operands.join(", ")
        ));
        result
    };

    if !unsafe_call {
        for copy in &copies {
            ctx.push(format!("call void @otter_free(i8* {copy})"));
        }
        for value in &consumed {
            ctx.release_owned(value);
        }
    }

    result
}
