//! The C boundary: copying, borrowing, and the `unsafe` opt-out.
//!
//! Spec §8.1: crossing into C **copies**. Yashy's `string` is a refcounted
//! object, while C wants a NUL-terminated `char*` whose lifetime the callee can
//! extend past the call. So a safe call:
//!
//! 1. copies each string argument into a fresh buffer (`yashy_str_cstr`),
//! 2. makes the call with those buffers,
//! 3. frees the buffers and releases any temporary strings it consumed.
//!
//! A call site marked `unsafe` waives the copy: the callee is lent the
//! string's own bytes (`yashy_str_ptr`) and may keep them. Nothing is freed,
//! because the pointer now belongs to C as far as the compiler can prove.
//!
//! A raw pointer is an address, not a value that can be copied meaningfully, so
//! it always crosses unchanged. The checker requires `unsafe` on such a call
//! for the same reason: C may keep or free what the pointer names.
//!
//! Slices do not exist yet (M6); when they do, their elements are copied here
//! the same way, with their length.

use yashy_sema::{HirExternDecl, TypeKind};

use crate::expr_gen::{FnCtx, GenValue};
use crate::ir::repr_type;

/// The C type of a declared parameter or return. An `Option` whose payload is
/// a pointer crosses as that pointer: null is `None`, a non-null address is
/// `Some`. The aggregate Yashy uses internally never appears in a C signature.
fn c_type(ty: &TypeKind) -> String {
    if let TypeKind::Option(inner) = ty {
        if matches!(inner.kind, TypeKind::Ptr(_) | TypeKind::CPtr(_)) {
            return "i8*".to_string();
        }
    }
    repr_type(ty)
}

fn is_ptr_option(ty: &TypeKind) -> bool {
    matches!(ty, TypeKind::Option(inner) if matches!(inner.kind, TypeKind::Ptr(_) | TypeKind::CPtr(_)))
}

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
    expected: &TypeKind,
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
        ) || (decl.variadic && index >= decl.params.len());
        if matches!(arg.ty, TypeKind::Str) && is_c_string_parameter {
            let data = ctx.tmp();
            if unsafe_call {
                // Lend, do not copy. C may keep this pointer, so the string
                // that owns it is deliberately not released below.
                ctx.push(format!(
                    "{data} = call i8* @yashy_str_ptr(i8* {})",
                    arg.value
                ));
            } else {
                ctx.push(format!(
                    "{data} = call i8* @yashy_str_cstr(i8* {})",
                    arg.value
                ));
                copies.push(data.clone());
                consumed.push(arg.value.clone());
            }
            operands.push(format!("i8* {data}"));
        } else if is_ptr_option(&arg.ty)
            && decl
                .params
                .get(index)
                .is_some_and(|param| is_ptr_option(&param.ty.kind))
        {
            // Yashy's `{ i8, i8* }` becomes one C pointer: `None` is null and
            // `Some(p)` is p. The null check selects between them without a
            // branch, so the aggregate never reaches C.
            let tag = ctx.tmp();
            let payload = ctx.tmp();
            let not_none = ctx.tmp();
            let pointer = ctx.tmp();
            let option_repr = repr_type(&arg.ty);
            ctx.push(format!("{tag} = extractvalue {option_repr} {}, 0", arg.value));
            ctx.push(format!(
                "{payload} = extractvalue {option_repr} {}, 1",
                arg.value
            ));
            ctx.push(format!("{not_none} = icmp ne i8 {tag}, 0"));
            ctx.push(format!(
                "{pointer} = select i1 {not_none}, i8* {payload}, i8* null"
            ));
            operands.push(format!("i8* {pointer}"));
        } else {
            // Everything else -- integers, floats, raw pointers -- is already
            // the machine value C wants.
            operands.push(format!("{} {}", repr_type(&arg.ty), arg.value));
        }
    }

    let ret = c_type(&decl.ret.kind);
    // LLVM needs the full function type at a vararg call site, because the
    // arguments after the fixed ones carry no types of their own:
    // `call i32 (i8*, ...) @printf(i8* %fmt, i64 %n)`. A non-variadic call
    // names the symbol alone.
    let callee = if decl.variadic {
        let fixed: Vec<String> = decl.params.iter().map(|p| repr_type(&p.ty.kind)).collect();
        let type_params = if fixed.is_empty() {
            "...".to_string()
        } else {
            format!("{}, ...", fixed.join(", "))
        };
        format!("{ret} ({type_params}) @{}", decl.symbol())
    } else {
        format!("{ret} @{}", decl.symbol())
    };
    let result = if ret == "void" {
        ctx.push(format!("call {callee}({})", operands.join(", ")));
        String::new()
    } else {
        let result = ctx.tmp();
        ctx.push(format!("{result} = call {callee}({})", operands.join(", ")));
        result
    };

    if !unsafe_call {
        for copy in &copies {
            ctx.push(format!("call void @yashy_free(i8* {copy})"));
        }
        for value in &consumed {
            ctx.release_owned(value);
        }
    }

    if !result.is_empty() && is_ptr_option(&decl.ret.kind) && is_ptr_option(expected) {
        // The C callee returned a bare pointer. null is `None`; any other
        // address is `Some`. Rebuild the aggregate the Yashy caller expects.
        let present = ctx.tmp();
        let tag = ctx.tmp();
        let tagged = ctx.tmp();
        let option = ctx.tmp();
        let repr = repr_type(expected);
        ctx.push(format!("{present} = icmp ne i8* {result}, null"));
        ctx.push(format!("{tag} = zext i1 {present} to i8"));
        ctx.push(format!("{tagged} = insertvalue {repr} undef, i8 {tag}, 0"));
        ctx.push(format!(
            "{option} = insertvalue {repr} {tagged}, i8* {result}, 1"
        ));
        return option;
    }

    result
}

