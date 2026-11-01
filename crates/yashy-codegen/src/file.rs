//! Byte-buffer builtins: `alloc_bytes`, `read_bytes`, `write_bytes`.
//!
//! `alloc_bytes` lives in [`crate::memory`] because it is pure heap allocation.
//! `read_bytes` and `write_bytes` cross the C boundary (a `string` path becomes
//! a NUL-terminated buffer) and build a `Result` whose tag is decided at run
//! time, so they lower here with a slot-plus-branch rather than a `select` on
//! an aggregate (LLVM cannot `select` an aggregate).

use yashy_common::Span;
use yashy_sema::{HirExpr, Type, TypeKind};

use crate::expr_gen::{gen_expr, FnCtx, GenValue};
use crate::ir::repr_type;

/// The IR spelling of a slice header, matching [`crate::slices::SLICE_REPR`].
const SLICE_REPR: &str = "{ i8*, i64, i64 }";

/// A type built at a synthetic span, for the builtin's fixed signature.
fn ty(kind: TypeKind) -> Type {
    Type {
        kind,
        span: Span::new(0, 0),
    }
}

/// The full type of `io.Error`.
fn io_error_ty() -> Type {
    ty(TypeKind::Enum("io.Error".to_string()))
}

/// The type `Result[T, E]`.
fn result_ty(ok: TypeKind, err: TypeKind) -> TypeKind {
    TypeKind::Result(Box::new(ty(ok)), Box::new(ty(err)))
}

/// Lower `read_bytes(path: string) -> Result[[]char, io.Error]`.
///
/// The runtime writes the slice header into an out slot and returns a small
/// integer: `0` is success and `1 + variant_index` is the failure. Field 1 of
/// the `Result` holds the slice on success; field 2 holds the `io.Error` tag on
/// failure. An `Ok` payload of `[]char` is a real slice value, so the
/// aggregate field is `{ i8*, i64, i64 }`, not `{}`.
pub fn gen_read_bytes(ctx: &mut FnCtx<'_>, args: &[HirExpr], ret: &TypeKind) -> GenValue {
    let Some(path_expr) = args.first() else {
        return zero_result(ctx, ret);
    };
    let path = gen_expr(ctx, path_expr);

    // Copy the path to a NUL-terminated buffer the C function may keep.
    let path_copy = ctx.tmp();
    ctx.push(format!(
        "{path_copy} = call i8* @yashy_str_cstr(i8* {})",
        path.value
    ));

    // The out slot where the runtime writes the slice header.
    let out_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{out_slot} = alloca {SLICE_REPR}"));
    ctx.push(format!("store {SLICE_REPR} zeroinitializer, ptr {out_slot}"));

    let code = ctx.tmp();
    ctx.push(format!(
        "{code} = call i64 @yashy_read_bytes(i8* {path_copy}, ptr {out_slot})"
    ));
    ctx.push(format!("call void @yashy_free(i8* {path_copy})"));
    ctx.release_owned(&path.value);

    let repr = repr_type(ret);
    let result_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{result_slot} = alloca {repr}"));
    let ok_label = ctx.alloc_label_base() + "_read_bytes_ok";
    let err_label = ctx.alloc_label_base() + "_read_bytes_err";
    let cont_label = ctx.alloc_label_base() + "_read_bytes_cont";
    let is_ok = ctx.tmp();
    ctx.push(format!("{is_ok} = icmp eq i64 {code}, 0"));
    ctx.push(format!("br i1 {is_ok}, label %{ok_label}, label %{err_label}"));

    // Success: tag 0, payload = the slice the runtime wrote.
    ctx.push(format!("{ok_label}:"));
    let slice_val = ctx.tmp();
    ctx.push(format!("{slice_val} = load {SLICE_REPR}, ptr {out_slot}"));
    let ok_tagged = ctx.tmp();
    ctx.push(format!("{ok_tagged} = insertvalue {repr} undef, i8 0, 0"));
    let ok_full = ctx.tmp();
    ctx.push(format!(
        "{ok_full} = insertvalue {repr} {ok_tagged}, {} {}, 1",
        repr_type(&TypeKind::Slice(Box::new(ty(TypeKind::Char)))),
        slice_val
    ));
    ctx.push(format!("store {repr} {ok_full}, ptr {result_slot}"));
    ctx.push(format!("br label %{cont_label}"));

    // Failure: tag 1, payload = the io.Error variant tag.
    ctx.push(format!("{err_label}:"));
    let err_tag = ctx.tmp();
    ctx.push(format!("{err_tag} = sub i64 {code}, 1"));
    let err_tag_i8 = ctx.tmp();
    ctx.push(format!("{err_tag_i8} = trunc i64 {err_tag} to i8"));
    let err_tagged = ctx.tmp();
    ctx.push(format!("{err_tagged} = insertvalue {repr} undef, i8 1, 0"));
    let err_enum = ctx.tmp();
    ctx.push(format!(
        "{err_enum} = insertvalue %enum.io.Error undef, i8 {err_tag_i8}, 0"
    ));
    let err_full = ctx.tmp();
    ctx.push(format!(
        "{err_full} = insertvalue {repr} {err_tagged}, %enum.io.Error {err_enum}, 2"
    ));
    ctx.push(format!("store {repr} {err_full}, ptr {result_slot}"));
    ctx.push(format!("br label %{cont_label}"));

    ctx.push(format!("{cont_label}:"));
    let result = ctx.tmp();
    ctx.push(format!("{result} = load {repr}, ptr {result_slot}"));
    GenValue {
        value: result,
        ty: ret.clone(),
    }
}

/// Lower `write_bytes(path: string, body: []char) -> Result[void, io.Error]`.
///
/// `body` is a slice: its data pointer is field 0 and its length field 1 of
/// the header. The `Ok` payload is `void`, which renders as `{}` in field
/// position, so the `Ok` arm inserts only the tag.
pub fn gen_write_bytes(ctx: &mut FnCtx<'_>, args: &[HirExpr]) -> GenValue {
    let Some(path_expr) = args.first() else {
        let ret = result_ty(TypeKind::Unit, io_error_ty().kind);
        return zero_result(ctx, &ret);
    };
    let Some(body_expr) = args.get(1) else {
        let ret = result_ty(TypeKind::Unit, io_error_ty().kind);
        return zero_result(ctx, &ret);
    };
    let path = gen_expr(ctx, path_expr);
    let body = gen_expr(ctx, body_expr);

    let path_copy = ctx.tmp();
    ctx.push(format!(
        "{path_copy} = call i8* @yashy_str_cstr(i8* {})",
        path.value
    ));
    ctx.release_owned(&path.value);

    let body_data = ctx.tmp();
    ctx.push(format!(
        "{body_data} = extractvalue {SLICE_REPR} {}, 0",
        body.value
    ));
    let body_len = ctx.tmp();
    ctx.push(format!(
        "{body_len} = extractvalue {SLICE_REPR} {}, 1",
        body.value
    ));

    let code = ctx.tmp();
    ctx.push(format!(
        "{code} = call i64 @yashy_write_bytes(i8* {path_copy}, i8* {body_data}, i64 {body_len})"
    ));
    ctx.push(format!("call void @yashy_free(i8* {path_copy})"));

    let ret = result_ty(TypeKind::Unit, io_error_ty().kind);
    let repr = repr_type(&ret);
    let result_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{result_slot} = alloca {repr}"));
    let ok_label = ctx.alloc_label_base() + "_write_bytes_ok";
    let err_label = ctx.alloc_label_base() + "_write_bytes_err";
    let cont_label = ctx.alloc_label_base() + "_write_bytes_cont";
    let is_ok = ctx.tmp();
    ctx.push(format!("{is_ok} = icmp eq i64 {code}, 0"));
    ctx.push(format!("br i1 {is_ok}, label %{ok_label}, label %{err_label}"));

    // Success: tag 0, payload = `void`, which is `{}` in field position.
    ctx.push(format!("{ok_label}:"));
    let ok_tagged = ctx.tmp();
    ctx.push(format!("{ok_tagged} = insertvalue {repr} undef, i8 0, 0"));
    ctx.push(format!("store {repr} {ok_tagged}, ptr {result_slot}"));
    ctx.push(format!("br label %{cont_label}"));

    // Failure: tag 1, payload = the io.Error variant tag.
    ctx.push(format!("{err_label}:"));
    let err_tag = ctx.tmp();
    ctx.push(format!("{err_tag} = sub i64 {code}, 1"));
    let err_tag_i8 = ctx.tmp();
    ctx.push(format!("{err_tag_i8} = trunc i64 {err_tag} to i8"));
    let err_tagged = ctx.tmp();
    ctx.push(format!("{err_tagged} = insertvalue {repr} undef, i8 1, 0"));
    let err_enum = ctx.tmp();
    ctx.push(format!(
        "{err_enum} = insertvalue %enum.io.Error undef, i8 {err_tag_i8}, 0"
    ));
    let err_full = ctx.tmp();
    ctx.push(format!(
        "{err_full} = insertvalue {repr} {err_tagged}, %enum.io.Error {err_enum}, 2"
    ));
    ctx.push(format!("store {repr} {err_full}, ptr {result_slot}"));
    ctx.push(format!("br label %{cont_label}"));

    ctx.push(format!("{cont_label}:"));
    let result = ctx.tmp();
    ctx.push(format!("{result} = load {repr}, ptr {result_slot}"));
    GenValue {
        value: result,
        ty: ret,
    }
}

/// A `Result` whose tag is `0` and whose payload fields are `undef`, for the
/// arity-mismatch fallback.
fn zero_result(ctx: &mut FnCtx<'_>, ret: &TypeKind) -> GenValue {
    let repr = repr_type(ret);
    let tmp = ctx.tmp();
    ctx.push(format!("{tmp} = insertvalue {repr} undef, i8 0, 0"));
    GenValue {
        value: tmp,
        ty: ret.clone(),
    }
}
