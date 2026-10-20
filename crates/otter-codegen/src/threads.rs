//! Threads, mutexes and channels: runtime calls, plus the one function the
//! backend has to synthesize.
//!
//! The runtime's thread entry takes a single `void*` and knows nothing about
//! Otter's parameter lists, so a spawn cannot hand it `@work` directly. The
//! lowering instead:
//!
//! 1. builds a struct of the arguments in the caller's frame, retaining each
//!    `string` for the thread;
//! 2. passes the struct's address and size to `otter_thread_spawn`, which
//!    copies those bytes onto the heap;
//! 3. synthesizes `@otter_thread_entry_N(i8* %raw)`, which unpacks the copy,
//!    calls the body, and releases the string references the thread owned.
//!
//! The struct's size is read from LLVM's own layout (`getelementptr` on a null
//! pointer), not from hand arithmetic: the same rule then sizes a channel slot,
//! where an `Option[T]` element is wider than `size_of` says.
//!
//! Mutexes and channels are opaque pointers, so every operation is one call.

use otter_sema::{HirExpr, TypeKind};

use crate::expr_gen::{gen_expr, GenValue, FnCtx};
use crate::ir::repr_type;
use crate::slices::option_from_flag;

fn unit() -> GenValue {
    GenValue {
        value: String::new(),
        ty: TypeKind::Unit,
    }
}

/// The size in bytes of one value of `repr`, as LLVM lays it out.
///
/// `getelementptr` with a null base and index 1 is the classic `sizeof`: the
/// resulting address is exactly the alloc size, and LLVM folds it away. The
/// alternative — summing field sizes — is wrong for any struct with padding,
/// which includes `{ i8, i64 }` (`Option[i64]`) and every mixed argument list.
fn size_in_bytes(ctx: &mut FnCtx<'_>, repr: &str) -> String {
    let address = ctx.tmp();
    ctx.push(format!("{address} = getelementptr {repr}, ptr null, i32 1"));
    let size = ctx.tmp();
    ctx.push(format!("{size} = ptrtoint ptr {address} to i64"));
    size
}

/// Lower `thread_spawn(body, args...) -> i64`.
///
/// The checker has already rejected a body that returns anything but `()` and
/// any argument the copy cannot carry, so the only shape that reaches here is
/// one the synthesized entry can unpack.
pub fn gen_thread_spawn(ctx: &mut FnCtx<'_>, body: &HirExpr, args: &[HirExpr]) -> GenValue {
    let HirExpr::Ident(path, _, body_ty) = body else {
        return unit();
    };
    let Some(name) = path.last() else {
        return unit();
    };
    let TypeKind::Func(params, _) = &body_ty.kind else {
        return unit();
    };

    let struct_ty = if params.is_empty() {
        "{}".to_string()
    } else {
        let fields: Vec<String> = params.iter().map(|p| repr_type(&p.kind)).collect();
        format!("{{ {} }}", fields.join(", "))
    };

    // The caller's copy. It only has to live until `otter_thread_spawn`
    // returns, because the runtime copies these bytes onto its own heap.
    let args_slot = if params.is_empty() {
        String::new()
    } else {
        let slot = ctx.alloc_slot();
        ctx.push_alloca(format!("{slot} = alloca {struct_ty}"));
        slot
    };

    let mut lowered = Vec::with_capacity(args.len());
    for (index, (argument, parameter)) in args.iter().zip(params.iter()).enumerate() {
        let value = gen_expr(ctx, argument);
        let field_repr = repr_type(&parameter.kind);
        let field = ctx.tmp();
        ctx.push(format!(
            "{field} = getelementptr inbounds {struct_ty}, ptr {args_slot}, i32 0, i32 {index}"
        ));
        let stored = if matches!(parameter.kind, TypeKind::Str) {
            // The thread gets a reference of its own; the synthesized entry
            // releases it after the body returns. A static literal ignores the
            // retain, which is exactly why releasing it is always safe.
            let retained = ctx.tmp();
            ctx.push_str_retain_into(&retained, &value.value);
            retained
        } else {
            value.value.clone()
        };
        ctx.push(format!("store {field_repr} {stored}, ptr {field}"));
        lowered.push(value);
    }

    // One entry per spawn site: its parameter list is this site's argument
    // struct, so two spawns of the same body with different shapes each get an
    // entry that matches their own bytes.
    let index = ctx.module.next_thread_entry();
    let entry = format!("otter_thread_entry_{index}");
    let release = ctx.string_release_symbol();
    let mut lines = Vec::new();
    lines.push(format!("define void @{entry}(i8* %raw) {{"));
    lines.push("entry:".to_string());
    let mut call_args = Vec::new();
    let mut strings = Vec::new();
    for (index, parameter) in params.iter().enumerate() {
        let repr = repr_type(&parameter.kind);
        let field = format!("%arg{index}_p");
        let value = format!("%arg{index}_v");
        lines.push(format!(
            "  {field} = getelementptr inbounds {struct_ty}, ptr %raw, i32 0, i32 {index}"
        ));
        lines.push(format!("  {value} = load {repr}, ptr {field}"));
        if matches!(parameter.kind, TypeKind::Str) {
            strings.push(value.clone());
        }
        call_args.push(format!("{repr} {value}"));
    }
    lines.push(format!("  call void @{name}({})", call_args.join(", ")));
    for value in strings {
        lines.push(format!("  call void @{release}(i8* {value})"));
    }
    lines.push("  ret void".to_string());
    lines.push("}".to_string());
    ctx.module.add_synthesized_function(lines);

    let (pointer, size) = if params.is_empty() {
        ("i8* null".to_string(), "0".to_string())
    } else {
        (format!("i8* {args_slot}"), size_in_bytes(ctx, &struct_ty))
    };
    let handle32 = ctx.tmp();
    ctx.push(format!(
        "{handle32} = call i32 @otter_thread_spawn(i8* @{entry}, {pointer}, i64 {size})"
    ));
    let handle = ctx.tmp();
    ctx.push(format!("{handle} = sext i32 {handle32} to i64"));

    // The thread took its own reference; an argument that was an owned
    // temporary (a concatenation, say) still owes the caller's reference back.
    for value in &lowered {
        ctx.release_owned(&value.value);
    }

    GenValue {
        value: handle,
        ty: TypeKind::I64,
    }
}

/// Lower `thread_join(handle) -> ()`.
///
/// The runtime returns its own status, which the language does not surface: a
/// join that fails to find its handle is a backend bug, not a program outcome.
pub fn gen_thread_join(ctx: &mut FnCtx<'_>, args: &[HirExpr]) -> GenValue {
    let Some(handle) = args.first() else {
        return unit();
    };
    let value = gen_expr(ctx, handle);
    let narrow = ctx.tmp();
    ctx.push(format!("{narrow} = trunc i64 {} to i32", value.value));
    let status = ctx.tmp();
    ctx.push(format!(
        "{status} = call i32 @otter_thread_join(i32 {narrow})"
    ));
    unit()
}

/// Lower `mutex_new() -> Mutex`.
pub fn gen_mutex_new(ctx: &mut FnCtx<'_>) -> GenValue {
    let handle = ctx.tmp();
    ctx.push(format!("{handle} = call i8* @otter_mutex_new()"));
    GenValue {
        value: handle,
        ty: TypeKind::Mutex,
    }
}

/// Lower `mutex_lock(m)`, `mutex_unlock(m)` and `mutex_free(m)`.
pub fn gen_mutex_call(ctx: &mut FnCtx<'_>, args: &[HirExpr], symbol: &str) -> GenValue {
    let Some(mutex) = args.first() else {
        return unit();
    };
    let handle = gen_expr(ctx, mutex);
    ctx.push(format!("call void @{symbol}(i8* {})", handle.value));
    unit()
}

/// Lower `chan_new(capacity) -> Channel[T]`.
pub fn gen_chan_new(ctx: &mut FnCtx<'_>, args: &[HirExpr], ty: &TypeKind) -> GenValue {
    let Some(capacity) = args.first() else {
        return unit();
    };
    let capacity = gen_expr(ctx, capacity);
    let channel = ctx.tmp();
    ctx.push(format!(
        "{channel} = call i8* @otter_chan_new(i64 {})",
        capacity.value
    ));
    GenValue {
        value: channel,
        ty: ty.clone(),
    }
}

/// Lower `chan_send(c, value) -> bool`.
///
/// The runtime copies the element out of a slot, so the value is spilled
/// first. A full channel blocks the sender; the 0 answer means the channel was
/// closed, and that 0 is the function's `false`.
pub fn gen_chan_send(ctx: &mut FnCtx<'_>, args: &[HirExpr]) -> GenValue {
    let (Some(receiver), Some(value)) = (args.first(), args.get(1)) else {
        return unit();
    };
    let channel = gen_expr(ctx, receiver);
    let TypeKind::Channel(element) = &channel.ty else {
        return unit();
    };
    let slot = ctx.alloc_slot();
    let repr = repr_type(&element.kind);
    ctx.push_alloca(format!("{slot} = alloca {repr}"));
    let value = gen_expr(ctx, value);
    ctx.push(format!("store {repr} {}, ptr {slot}", value.value));
    let size = size_in_bytes(ctx, &repr);
    let flag = ctx.tmp();
    ctx.push(format!(
        "{flag} = call i8 @otter_chan_send(i8* {}, ptr {slot}, i64 {size})",
        channel.value
    ));
    let sent = ctx.tmp();
    ctx.push(format!("{sent} = icmp ne i8 {flag}, 0"));
    GenValue {
        value: sent,
        ty: TypeKind::Bool,
    }
}

/// Lower `chan_recv(c) -> Option[T]`.
///
/// The out slot is zeroed first: the runtime may leave it untouched when it
/// answers 0, and `option_from_flag` loads the payload unconditionally even
/// for `None`.
pub fn gen_chan_recv(ctx: &mut FnCtx<'_>, args: &[HirExpr], option_ty: &TypeKind) -> GenValue {
    let Some(receiver) = args.first() else {
        return unit();
    };
    let channel = gen_expr(ctx, receiver);
    let TypeKind::Channel(element) = &channel.ty else {
        return unit();
    };
    let repr = repr_type(&element.kind);
    let slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{slot} = alloca {repr}"));
    ctx.push(format!("store {repr} zeroinitializer, ptr {slot}"));
    let size = size_in_bytes(ctx, &repr);
    let flag = ctx.tmp();
    ctx.push(format!(
        "{flag} = call i8 @otter_chan_recv(i8* {}, ptr {slot}, i64 {size})",
        channel.value
    ));
    let flag = {
        let wide = ctx.tmp();
        ctx.push(format!("{wide} = zext i8 {flag} to i64"));
        wide
    };
    option_from_flag(ctx, &flag, &slot, &element.kind, option_ty)
}

/// Lower `chan_close(c)` and `chan_free(c)`.
pub fn gen_chan_call(ctx: &mut FnCtx<'_>, args: &[HirExpr], symbol: &str) -> GenValue {
    let Some(receiver) = args.first() else {
        return unit();
    };
    let channel = gen_expr(ctx, receiver);
    ctx.push(format!("call void @{symbol}(i8* {})", channel.value));
    unit()
}
