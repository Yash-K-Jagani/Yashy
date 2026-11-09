//! Byte-buffer and honest-signature builtins.
//!
//! `alloc_bytes` lives in [`crate::memory`] because it is pure heap allocation.
//! The functions here cross the C boundary (a `string` path becomes a
//! NUL-terminated buffer) and build an `Option`/`Result` whose tag is decided at
//! run time, so they lower with a slot-plus-branch rather than a `select` on an
//! aggregate (LLVM cannot `select` an aggregate).
//!
//! Every failure-carrying runtime entry point uses one convention: `0` is
//! success and `1 + variant_index` is the `io.Error` variant, or `1`/`0` for an
//! `Option` and a NULL pointer for `read_line` at end of input. The lowering
//! turns that into the tag the language spells, and writes `null` into any
//! absent `string` payload field so releasing the aggregate is always safe.

use yashy_common::Span;
use yashy_sema::{HirExpr, Type, TypeKind};

use crate::expr_gen::{
    gen_expr, has_string_payload, string_payload_fields, FnCtx, GenValue, RefKind,
};
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

/// One variant of an `Option`/`Result`: the tag, `null` in every absent
/// `string` payload field, and the variant's own payload when it has one.
///
/// `field` is the payload field this variant uses (`None` for `None`), and it
/// is exactly the field that must *not* be nulled. Writing the absent string
/// fields as `null` is what makes releasing the aggregate unconditional-safe:
/// the runtime's release leaves `null` and static strings alone.
fn tagged_value(
    ctx: &mut FnCtx<'_>,
    ret: &TypeKind,
    tag: u8,
    field: Option<u32>,
    payload: Option<(&str, &str)>,
) -> String {
    let repr = repr_type(ret);
    let mut acc = ctx.tmp();
    ctx.push(format!("{acc} = insertvalue {repr} undef, i8 {tag}, 0"));
    for absent in string_payload_fields(ret) {
        if Some(absent) == field {
            continue;
        }
        let next = ctx.tmp();
        ctx.push(format!(
            "{next} = insertvalue {repr} {acc}, i8* null, {absent}"
        ));
        acc = next;
    }
    if let (Some(field), Some((payload_repr, payload_value))) = (field, payload) {
        let next = ctx.tmp();
        ctx.push(format!(
            "{next} = insertvalue {repr} {acc}, {payload_repr} {payload_value}, {field}"
        ));
        acc = next;
    }
    acc
}

/// Branch on a runtime status word and build the `Result` it describes.
///
/// `0` is success and `1 + variant_index` is the `io.Error` tag. `ok` is the
/// success payload the caller has already produced, or `None` for
/// `Result[void, io.Error]`. The runtime hands the payload's reference to the
/// caller, so the aggregate is marked owned.
fn finish_result(
    ctx: &mut FnCtx<'_>,
    ret: &TypeKind,
    code: &str,
    ok: Option<(&str, &str)>,
    label: &str,
) -> GenValue {
    let repr = repr_type(ret);
    let result_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{result_slot} = alloca {repr}"));
    let base = ctx.alloc_label_base();
    let ok_label = format!("{base}_{label}_ok");
    let err_label = format!("{base}_{label}_err");
    let cont_label = format!("{base}_{label}_cont");
    let is_ok = ctx.tmp();
    ctx.push(format!("{is_ok} = icmp eq i64 {code}, 0"));
    ctx.push(format!("br i1 {is_ok}, label %{ok_label}, label %{err_label}"));

    // Success: tag 0, the payload (when there is one).
    ctx.push(format!("{ok_label}:"));
    let ok_full = tagged_value(ctx, ret, 0, Some(1), ok);
    ctx.push(format!("store {repr} {ok_full}, ptr {result_slot}"));
    ctx.push(format!("br label %{cont_label}"));

    // Failure: tag 1, payload = the io.Error variant tag.
    ctx.push(format!("{err_label}:"));
    let err_tag = ctx.tmp();
    ctx.push(format!("{err_tag} = sub i64 {code}, 1"));
    let err_tag_i8 = ctx.tmp();
    ctx.push(format!("{err_tag_i8} = trunc i64 {err_tag} to i8"));
    let err_enum = ctx.tmp();
    ctx.push(format!(
        "{err_enum} = insertvalue %enum.io.Error undef, i8 {err_tag_i8}, 0"
    ));
    let err_full = tagged_value(ctx, ret, 1, Some(2), Some(("%enum.io.Error", &err_enum)));
    ctx.push(format!("store {repr} {err_full}, ptr {result_slot}"));
    ctx.push(format!("br label %{cont_label}"));

    ctx.push(format!("{cont_label}:"));
    let result = ctx.tmp();
    ctx.push(format!("{result} = load {repr}, ptr {result_slot}"));
    if has_string_payload(ret) {
        ctx.mark_owned_typed(&result, RefKind::Payload, Some(ret.clone()));
    }
    GenValue {
        value: result,
        ty: ret.clone(),
    }
}

/// Branch on a presence flag and build the `Option` it describes.
///
/// `Some` is tag 1 and `None` tag 0; `present` is an `i1` the caller computed.
/// A present payload the runtime built is an owned reference the caller now
/// holds, so the aggregate is marked owned.
fn finish_option(
    ctx: &mut FnCtx<'_>,
    ret: &TypeKind,
    present: &str,
    payload: Option<(&str, &str)>,
    label: &str,
) -> GenValue {
    let repr = repr_type(ret);
    let result_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{result_slot} = alloca {repr}"));
    let base = ctx.alloc_label_base();
    let some_label = format!("{base}_{label}_some");
    let none_label = format!("{base}_{label}_none");
    let cont_label = format!("{base}_{label}_cont");
    ctx.push(format!(
        "br i1 {present}, label %{some_label}, label %{none_label}"
    ));

    ctx.push(format!("{some_label}:"));
    let some_full = tagged_value(ctx, ret, 1, Some(1), payload);
    ctx.push(format!("store {repr} {some_full}, ptr {result_slot}"));
    ctx.push(format!("br label %{cont_label}"));

    ctx.push(format!("{none_label}:"));
    let none_full = tagged_value(ctx, ret, 0, None, None);
    ctx.push(format!("store {repr} {none_full}, ptr {result_slot}"));
    ctx.push(format!("br label %{cont_label}"));

    ctx.push(format!("{cont_label}:"));
    let result = ctx.tmp();
    ctx.push(format!("{result} = load {repr}, ptr {result_slot}"));
    if has_string_payload(ret) {
        ctx.mark_owned_typed(&result, RefKind::Payload, Some(ret.clone()));
    }
    GenValue {
        value: result,
        ty: ret.clone(),
    }
}

/// Lower `read_line() -> Option[string]`.
///
/// `NULL` is end of input; an empty line is a non-null empty string, so the two
/// are different answers. The string the runtime returns is owned.
pub fn gen_read_line(ctx: &mut FnCtx<'_>, ret: &TypeKind) -> GenValue {
    let line = ctx.tmp();
    ctx.push(format!("{line} = call i8* @yashy_read_line()"));
    let present = ctx.tmp();
    ctx.push(format!("{present} = icmp ne i8* {line}, null"));
    finish_option(ctx, ret, &present, Some(("i8*", &line)), "read_line")
}

/// Lower `read_file(path: string) -> Result[string, io.Error]`.
pub fn gen_read_file(ctx: &mut FnCtx<'_>, args: &[HirExpr], ret: &TypeKind) -> GenValue {
    let Some(path_expr) = args.first() else {
        return zero_result(ctx, ret);
    };
    let path = gen_expr(ctx, path_expr);
    let path_copy = ctx.tmp();
    ctx.push(format!(
        "{path_copy} = call i8* @yashy_str_cstr(i8* {})",
        path.value
    ));
    let out_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{out_slot} = alloca i8*"));
    ctx.push(format!("store i8* null, ptr {out_slot}"));
    let code = ctx.tmp();
    ctx.push(format!(
        "{code} = call i64 @yashy_read_file(i8* {path_copy}, ptr {out_slot})"
    ));
    ctx.push(format!("call void @yashy_free(i8* {path_copy})"));
    ctx.release_owned(&path.value);
    let contents = ctx.tmp();
    ctx.push(format!("{contents} = load i8*, ptr {out_slot}"));
    finish_result(ctx, ret, &code, Some(("i8*", &contents)), "read_file")
}

/// Lower `write_file(path: string, body: string) -> Result[void, io.Error]`.
///
/// The body is lent, not consumed: it crosses as `(pointer, length)` because a
/// file body may contain a NUL, and the runtime keeps neither.
pub fn gen_write_file(ctx: &mut FnCtx<'_>, args: &[HirExpr]) -> GenValue {
    let ret = result_ty(TypeKind::Unit, io_error_ty().kind);
    let (Some(path_expr), Some(body_expr)) = (args.first(), args.get(1)) else {
        return zero_result(ctx, &ret);
    };
    let path = gen_expr(ctx, path_expr);
    let body = gen_expr(ctx, body_expr);
    let path_copy = ctx.tmp();
    ctx.push(format!(
        "{path_copy} = call i8* @yashy_str_cstr(i8* {})",
        path.value
    ));
    let bytes = ctx.tmp();
    ctx.push(format!(
        "{bytes} = call i8* @yashy_str_ptr(i8* {})",
        body.value
    ));
    let len = ctx.tmp();
    ctx.push(format!(
        "{len} = call i64 @yashy_str_len(i8* {})",
        body.value
    ));
    let code = ctx.tmp();
    ctx.push(format!(
        "{code} = call i64 @yashy_write_file(i8* {path_copy}, i8* {bytes}, i64 {len})"
    ));
    ctx.push(format!("call void @yashy_free(i8* {path_copy})"));
    ctx.release_owned(&path.value);
    ctx.release_owned(&body.value);
    finish_result(ctx, &ret, &code, None, "write_file")
}

/// Lower `log_info(message)` / `log_error(message)`: a JSON line on stderr.
/// The runtime prints `{"ts":…,"level":"…","message":…}` and escapes the
/// message; the level is a static the sink already knows how to spell.
pub fn gen_log(ctx: &mut FnCtx<'_>, args: &[HirExpr], level: &str) -> GenValue {
    let value = gen_expr(ctx, &args[0]);
    // The message's bytes: `yashy_str_cstr` hands back the data pointer of
    // a `%yashy_str`, which is what the sink reads.
    let text = ctx.tmp();
    ctx.push(format!("{text} = call i8* @yashy_str_cstr(i8* {})", value.value));
    let level_global = ctx.module.intern_string(level.as_bytes());
    ctx.push(format!(
        "call void @yashy_log_event(i8* {level_global}.data, i8* {text})"
    ));
    GenValue {
        value: String::new(),
        ty: TypeKind::Unit,
    }
}

/// Lower `metrics_add(name, value)` / `metrics_set(name, value)`.
pub fn gen_metrics(ctx: &mut FnCtx<'_>, args: &[HirExpr], kind: &str) -> GenValue {
    let name = gen_expr(ctx, &args[0]);
    let value = gen_expr(ctx, &args[1]);
    let text = ctx.tmp();
    ctx.push(format!("{text} = call i8* @yashy_str_cstr(i8* {})", name.value));
    let kind_global = ctx.module.intern_string(kind.as_bytes());
    ctx.push(format!(
        "call void @yashy_metrics(i8* {kind_global}.data, i8* {text}, i64 {})",
        value.value
    ));
    GenValue {
        value: String::new(),
        ty: TypeKind::Unit,
    }
}

/// Lower `getenv(name: string) -> Option[string]`.
///
/// `1` is "set" and writes the value; an empty variable is `Some("")`, which is
/// why this is not a string that is empty on failure.
pub fn gen_getenv(ctx: &mut FnCtx<'_>, args: &[HirExpr], ret: &TypeKind) -> GenValue {
    let Some(name_expr) = args.first() else {
        return zero_option(ctx, ret);
    };
    let name = gen_expr(ctx, name_expr);
    let name_copy = ctx.tmp();
    ctx.push(format!(
        "{name_copy} = call i8* @yashy_str_cstr(i8* {})",
        name.value
    ));
    let out_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{out_slot} = alloca i8*"));
    ctx.push(format!("store i8* null, ptr {out_slot}"));
    let flag = ctx.tmp();
    ctx.push(format!(
        "{flag} = call i32 @yashy_getenv(i8* {name_copy}, ptr {out_slot})"
    ));
    ctx.push(format!("call void @yashy_free(i8* {name_copy})"));
    ctx.release_owned(&name.value);
    let value = ctx.tmp();
    ctx.push(format!("{value} = load i8*, ptr {out_slot}"));
    let present = ctx.tmp();
    ctx.push(format!("{present} = icmp ne i32 {flag}, 0"));
    finish_option(ctx, ret, &present, Some(("i8*", &value)), "getenv")
}

/// Lower `getcwd() -> Result[string, io.Error]`.
pub fn gen_getcwd(ctx: &mut FnCtx<'_>, ret: &TypeKind) -> GenValue {
    let out_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{out_slot} = alloca i8*"));
    ctx.push(format!("store i8* null, ptr {out_slot}"));
    let code = ctx.tmp();
    ctx.push(format!("{code} = call i64 @yashy_getcwd(ptr {out_slot})"));
    let value = ctx.tmp();
    ctx.push(format!("{value} = load i8*, ptr {out_slot}"));
    finish_result(ctx, ret, &code, Some(("i8*", &value)), "getcwd")
}

/// Lower `s.parse_i64() -> Option[int]`.
pub fn gen_parse_i64(ctx: &mut FnCtx<'_>, args: &[HirExpr], ret: &TypeKind) -> GenValue {
    let Some(text_expr) = args.first() else {
        return zero_option(ctx, ret);
    };
    let text = gen_expr(ctx, text_expr);
    let out_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{out_slot} = alloca i64"));
    ctx.push(format!("store i64 0, ptr {out_slot}"));
    let flag = ctx.tmp();
    ctx.push(format!(
        "{flag} = call i64 @yashy_parse_i64(i8* {}, ptr {out_slot})",
        text.value
    ));
    ctx.release_owned(&text.value);
    let value = ctx.tmp();
    ctx.push(format!("{value} = load i64, ptr {out_slot}"));
    let present = ctx.tmp();
    ctx.push(format!("{present} = icmp ne i64 {flag}, 0"));
    finish_option(ctx, ret, &present, Some(("i64", &value)), "parse_i64")
}

/// Lower `s.parse_f64() -> Option[float]`.
pub fn gen_parse_f64(ctx: &mut FnCtx<'_>, args: &[HirExpr], ret: &TypeKind) -> GenValue {
    let Some(text_expr) = args.first() else {
        return zero_option(ctx, ret);
    };
    let text = gen_expr(ctx, text_expr);
    let out_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{out_slot} = alloca double"));
    ctx.push(format!("store double 0.0, ptr {out_slot}"));
    let flag = ctx.tmp();
    ctx.push(format!(
        "{flag} = call i64 @yashy_parse_f64(i8* {}, ptr {out_slot})",
        text.value
    ));
    ctx.release_owned(&text.value);
    let value = ctx.tmp();
    ctx.push(format!("{value} = load double, ptr {out_slot}"));
    let present = ctx.tmp();
    ctx.push(format!("{present} = icmp ne i64 {flag}, 0"));
    finish_option(ctx, ret, &present, Some(("double", &value)), "parse_f64")
}

/// A `Result` whose tag is `0` and whose payload fields are `null`/`undef`, for
/// the arity-mismatch fallback.
fn zero_result(ctx: &mut FnCtx<'_>, ret: &TypeKind) -> GenValue {
    let value = tagged_value(ctx, ret, 0, None, None);
    GenValue {
        value,
        ty: ret.clone(),
    }
}

/// An `Option` whose tag is `0`, for the arity-mismatch fallback.
fn zero_option(ctx: &mut FnCtx<'_>, ret: &TypeKind) -> GenValue {
    let value = tagged_value(ctx, ret, 0, None, None);
    GenValue {
        value,
        ty: ret.clone(),
    }
}
