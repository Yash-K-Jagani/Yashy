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

use otter_sema::{HirExternDecl, HirExpr, TypeKind};

use crate::expr_gen::{gen_expr, FnCtx, GenValue};
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

/// Lower a builtin whose every argument is a `string` C wants to read:
/// `read_file(path)` and `getenv(name)`.
///
/// The conversion is the safe-call half of [`gen_ffi_call`]: one
/// `otter_str_cstr` copy per argument, the call, then the copy is freed and any
/// temporary string it consumed is released. The result is always an owned
/// `string` the runtime built, so it is marked as such -- a binding takes the
/// reference over and a temporary is released when its consumer is done.
///
/// The symbols return the empty string rather than a null on failure, so there is
/// no flag to test here: the builtin's contract is "the text, or nothing", and
/// the plan records that as the M6 limitation it is.
pub fn gen_cstr_string_builtin(
    ctx: &mut FnCtx<'_>,
    symbol: &str,
    args: &[HirExpr],
    ty: &TypeKind,
) -> GenValue {
    let mut operands = Vec::new();
    let mut copies: Vec<String> = Vec::new();
    let mut consumed: Vec<String> = Vec::new();
    for arg in args {
        let value = gen_expr(ctx, arg);
        let copy = ctx.tmp();
        ctx.push(format!(
            "{copy} = call i8* @otter_str_cstr(i8* {})",
            value.value
        ));
        copies.push(copy.clone());
        consumed.push(value.value);
        operands.push(format!("i8* {copy}"));
    }
    if operands.is_empty() {
        // The checker guarantees one argument; a short list is a bug here, and a
        // null string is the harmless answer that keeps the IR well formed.
        return GenValue {
            value: "null".to_string(),
            ty: ty.clone(),
        };
    }

    let result = ctx.tmp();
    ctx.push(format!(
        "{result} = call i8* @{symbol}({})",
        operands.join(", ")
    ));
    for copy in &copies {
        ctx.push(format!("call void @otter_free(i8* {copy})"));
    }
    for value in &consumed {
        ctx.release_owned(value);
    }
    ctx.mark_owned(&result);
    GenValue {
        value: result,
        ty: ty.clone(),
    }
}

/// Lower `write_file(path, text) -> i64`.
///
/// The path is copied to a C string, but the text is **lent**: a file body is a
/// byte range that may contain a NUL, so it crosses as `(pointer, length)` and
/// the callee must not keep either. The length comes from `otter_str_len` and the
/// pointer from `otter_str_ptr`, which is the same borrow an `unsafe` extern call
/// makes -- safe here because the runtime writes the bytes before it returns.
pub fn gen_write_file(ctx: &mut FnCtx<'_>, args: &[HirExpr], ty: &TypeKind) -> GenValue {
    let (Some(path_expr), Some(text_expr)) = (args.first(), args.get(1)) else {
        return GenValue {
            value: "0".to_string(),
            ty: ty.clone(),
        };
    };
    let path = gen_expr(ctx, path_expr);
    let text = gen_expr(ctx, text_expr);

    let path_copy = ctx.tmp();
    ctx.push(format!(
        "{path_copy} = call i8* @otter_str_cstr(i8* {})",
        path.value
    ));
    let bytes = ctx.tmp();
    ctx.push(format!(
        "{bytes} = call i8* @otter_str_ptr(i8* {})",
        text.value
    ));
    let len = ctx.tmp();
    ctx.push(format!(
        "{len} = call i64 @otter_str_len(i8* {})",
        text.value
    ));

    let result = ctx.tmp();
    ctx.push(format!(
        "{result} = call i64 @otter_write_file(i8* {path_copy}, i8* {bytes}, i64 {len})"
    ));
    ctx.push(format!("call void @otter_free(i8* {path_copy})"));
    ctx.release_owned(&path.value);
    ctx.release_owned(&text.value);
    GenValue {
        value: result,
        ty: ty.clone(),
    }
}
