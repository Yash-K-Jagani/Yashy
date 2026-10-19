//! The fault path: where an integer operation that cannot be carried out reports
//! itself, and the checks that send it there.
//!
//! # Checked arithmetic
//!
//! Integer arithmetic is **checked**. `a + b` that overflows, `a / 0`, and
//! `-i64::MIN` stop the program with a message naming the operands and the
//! position, rather than wrapping to a number that is wrong, dividing by zero
//! into whatever the hardware does, or -- in the case of `i64::MIN / -1`, which
//! has no answer in `i64` at all -- leaving the program's behaviour undefined.
//! Silent overflow is worse than a loud fault, and a language with no exceptions
//! has nowhere else to put the error (spec §6.1).
//!
//! # Three instructions on the succeeding path
//!
//! Every check here is arranged so the common case pays as little as possible
//! and the cold case pays for itself:
//!
//! - `add`/`sub`/`mul` go through LLVM's `*.with.overflow` intrinsics, which
//!   produce the value and the overflow flag together. The succeeding path is
//!   three instructions and a branch, and LLVM removes all four when it can
//!   prove the operation cannot overflow -- which for the plumbing arithmetic of
//!   a `for` loop it usually can.
//! - `/` and `%` compute their value *inside* the continuation block, after the
//!   branch, so a division that is about to fault never executes: on x86 a
//!   `sdiv` by zero does not return a wrong answer, it raises `#DE`.
//! - Everything the fault needs -- the operator's symbol, the file name, the
//!   position -- is built inside the fault block. None of it is on the path that
//!   succeeds.
//!
//! # Where the position comes from
//!
//! A fault names the file, line and column it happened at, and those are facts
//! only the checker can read out of the source map. It resolves them once and the
//! tree carries them ([`otter_sema::FaultSite`]); this module prints the numbers
//! as constants, which is the only thing an IR emitter can do with a position.

use otter_sema::{BinOp, FaultSite, TypeKind, UnOp};

use crate::expr_gen::{FnCtx, GenValue};
use crate::ir::{repr_type, variant_payload_index, variant_tag};

/// The smallest `i64`, as LLVM wants it written.
const I64_MIN: &str = "-9223372036854775808";

/// The largest `i64`.
const I64_MAX: &str = "9223372036854775807";

/// The three ways a program can ask for something other than a fault, when the
/// run-time answer is what it wants and stopping is not.
#[derive(Clone, Copy)]
enum Family {
    /// Two's-complement wraparound: the bare operation, no check at all.
    Wrapping,
    /// `None` instead of stopping, so the caller decides what to do.
    Checked,
    /// Clamp to `i64::MIN`/`i64::MAX`.
    Saturating,
}

/// Lower one of the nine arithmetic builtins -- `wrapping_add`, `checked_sub`,
/// `saturating_mul`, and the rest of the family.
///
/// The family and the operation come out of the name, which is why the names are
/// spelled out: this function is the only place in the compiler that knows all
/// nine, and the list is short enough to read. A name that is not one of them
/// cannot arrive from the checker (the builtin table declares exactly these) and
/// is answered with a zero rather than a `panic!` all the same.
///
/// The arguments arrive already lowered, and a call with fewer than two of them
/// -- which the checker has already rejected -- is padded with zeroes rather than
/// indexed into.
pub(crate) fn gen_arithmetic_builtin(
    ctx: &mut FnCtx<'_>,
    name: &str,
    args: &[GenValue],
    ty: &TypeKind,
) -> GenValue {
    let i64_zero = || GenValue {
        value: "0".to_string(),
        ty: TypeKind::I64,
    };
    let lhs = args.first().cloned().unwrap_or_else(i64_zero);
    let rhs = args.get(1).cloned().unwrap_or_else(i64_zero);
    let (family, op) = match builtin_of(name) {
        Some(pair) => pair,
        None => return i64_zero(),
    };
    // `sadd`/`ssub`/`smul` name the operation in three of the four intrinsics this
    // function uses, so the family picks the suffix and the operation picks the
    // first half.
    let stem = match op {
        BinOp::Add => "add",
        BinOp::Sub => "sub",
        _ => "mul",
    };
    match family {
        Family::Wrapping => {
            // The bare operation, which is what C, Go and Rust-in-release emit
            // for `+`. Nothing else: no flag, no branch, no cold block.
            let tmp = ctx.tmp();
            ctx.push(format!(
                "{tmp} = {stem} i64 {}, {}",
                lhs.value, rhs.value
            ));
            GenValue {
                value: tmp,
                ty: TypeKind::I64,
            }
        }
        Family::Saturating => {
            if stem == "mul" {
                // `sadd.sat` and `ssub.sat` are LLVM intrinsics that clang lowers
                // on sight; `smul.sat` is in the language reference but not
                // implemented -- clang emits a call to an undefined symbol named
                // `llvm.smul.sat.i64` and linking fails. So the multiply
                // saturates by hand: saturate on the sign of the *true* product,
                // which is negative exactly when the operands' signs differ.
                // (Neither operand is zero when the multiply overflowed, so the
                // sign of the true product is never a question of zero.)
                let pair = ctx.tmp();
                ctx.push(format!(
                    "{pair} = call {{ i64, i1 }} @llvm.smul.with.overflow.i64(i64 {}, i64 {})",
                    lhs.value, rhs.value
                ));
                let product = ctx.tmp();
                ctx.push(format!("{product} = extractvalue {{ i64, i1 }} {pair}, 0"));
                let overflowed = ctx.tmp();
                ctx.push(format!("{overflowed} = extractvalue {{ i64, i1 }} {pair}, 1"));
                let lhs_neg = ctx.tmp();
                ctx.push(format!("{lhs_neg} = icmp slt i64 {}, 0", lhs.value));
                let rhs_neg = ctx.tmp();
                ctx.push(format!("{rhs_neg} = icmp slt i64 {}, 0", rhs.value));
                let negative = ctx.tmp();
                ctx.push(format!("{negative} = xor i1 {lhs_neg}, {rhs_neg}"));
                let limit = ctx.tmp();
                ctx.push(format!(
                    "{limit} = select i1 {negative}, i64 {I64_MIN}, i64 {I64_MAX}"
                ));
                let saturated = ctx.tmp();
                ctx.push(format!(
                    "{saturated} = select i1 {overflowed}, i64 {limit}, i64 {product}"
                ));
                GenValue {
                    value: saturated,
                    ty: TypeKind::I64,
                }
            } else {
                let tmp = ctx.tmp();
                ctx.push(format!(
                    "{tmp} = call i64 @llvm.s{stem}.sat.i64(i64 {}, i64 {})",
                    lhs.value, rhs.value
                ));
                GenValue {
                    value: tmp,
                    ty: TypeKind::I64,
                }
            }
        }
        Family::Checked => {
            // The same intrinsic the checked default uses, with the flag spent on
            // a `select` instead of a branch: `Some(value)` when it did not
            // overflow, `None` when it did. A program that handles overflow pays
            // a select for the privilege of deciding.
            //
            // Both arms are built and one is selected, which is why `None`'s
            // payload stays `undef` -- every match tests the tag first, the same
            // contract the `None` builtin keeps.
            let pair = ctx.tmp();
            ctx.push(format!(
                "{pair} = call {{ i64, i1 }} @llvm.s{stem}.with.overflow.i64(i64 {}, i64 {})",
                lhs.value, rhs.value
            ));
            let value = ctx.tmp();
            ctx.push(format!("{value} = extractvalue {{ i64, i1 }} {pair}, 0"));
            let flag = ctx.tmp();
            ctx.push(format!("{flag} = extractvalue {{ i64, i1 }} {pair}, 1"));
            // The tag and the payload index come from the tree's own layout of
            // `Option`, rather than being written down twice; the fallbacks are
            // the values that layout has today.
            let tag = variant_tag(ty, "Some").unwrap_or(1);
            let field = variant_payload_index(ty, "Some").unwrap_or(1);
            let repr = repr_type(ty);
            let some = ctx.tmp();
            ctx.push(format!("{some} = insertvalue {repr} undef, i8 {tag}, 0"));
            let some_full = ctx.tmp();
            ctx.push(format!(
                "{some_full} = insertvalue {repr} {some}, i64 {value}, {field}"
            ));
            let none = ctx.tmp();
            ctx.push(format!("{none} = insertvalue {repr} undef, i8 0, 0"));
            let selected = ctx.tmp();
            ctx.push(format!(
                "{selected} = select i1 {flag}, {repr} {none}, {repr} {some_full}"
            ));
            GenValue {
                value: selected,
                ty: ty.clone(),
            }
        }
    }
}

/// The family and operation a builtin name names, if it is one of the nine.
fn builtin_of(name: &str) -> Option<(Family, BinOp)> {
    let (family, op) = match name {
        "wrapping_add" => (Family::Wrapping, BinOp::Add),
        "wrapping_sub" => (Family::Wrapping, BinOp::Sub),
        "wrapping_mul" => (Family::Wrapping, BinOp::Mul),
        "checked_add" => (Family::Checked, BinOp::Add),
        "checked_sub" => (Family::Checked, BinOp::Sub),
        "checked_mul" => (Family::Checked, BinOp::Mul),
        "saturating_add" => (Family::Saturating, BinOp::Add),
        "saturating_sub" => (Family::Saturating, BinOp::Sub),
        "saturating_mul" => (Family::Saturating, BinOp::Mul),
        _ => return None,
    };
    Some((family, op))
}

/// A fresh label for the block a check branches to on failure.
pub(crate) fn fault_label(ctx: &mut FnCtx<'_>) -> String {
    format!("{}_fault", ctx.alloc_label_base())
}

/// Emit the block `label` names: the reporting call, then `unreachable`.
///
/// The label is passed in rather than allocated here because the branch that
/// targets it is emitted first and has to name it. The call is built by
/// `report`, which runs *inside* the block: the string constants it needs are
/// cold, so they belong here rather than on the path that succeeds. Nothing
/// follows `unreachable`, and the caller must open its own continuation block if
/// it has more to emit -- which is what makes this safe to call from any
/// expression position.
pub(crate) fn close_fault(
    ctx: &mut FnCtx<'_>,
    label: &str,
    report: impl FnOnce(&mut FnCtx<'_>) -> String,
) {
    ctx.push(format!("{label}:"));
    let call = report(ctx);
    ctx.push(call);
    ctx.push("unreachable".to_string());
}

/// Lower an integer operation with the check that makes it total.
///
/// `None` when this operation has no check, which the caller answers with the
/// ordinary lowering: a comparison, a boolean `and`/`or`, and float arithmetic
/// (an infinity is a float answer, not a fault) all come out here, as does
/// anything asked of a non-integer operand -- a guard, not a case the caller is
/// expected to have excluded, so that a checked float `add` is impossible rather
/// than merely not emitted.
///
/// The value returned is the result of the operation; the module is left
/// positioned at the block that follows the check, so the caller may carry on
/// emitting there.
pub(crate) fn gen_checked_binary(
    ctx: &mut FnCtx<'_>,
    op: BinOp,
    l: &GenValue,
    r: &GenValue,
    site: &FaultSite,
) -> Option<GenValue> {
    if !matches!(l.ty, TypeKind::I64) {
        return None;
    }
    match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul => Some(gen_overflow(ctx, op, l, r, site)),
        BinOp::Div | BinOp::Rem => Some(gen_division(ctx, op, l, r, site)),
        _ => None,
    }
}

/// Lower the negation of an integer, with the check for the one value it cannot
/// negate.
///
/// `-i64::MIN` is `i64::MIN` when it wraps and has no answer in `i64` at all
/// mathematically, so it is the same fault as an overflow and reports itself
/// through the same runtime function. `None` for `!`, for float negation (which
/// is exact: negating any `f64`, including an infinity, is correct), and for a
/// non-integer operand.
pub(crate) fn gen_checked_neg(
    ctx: &mut FnCtx<'_>,
    op: UnOp,
    value: &GenValue,
    site: &FaultSite,
) -> Option<GenValue> {
    if !matches!(op, UnOp::Neg) || !matches!(value.ty, TypeKind::I64) {
        return None;
    }
    let smallest = ctx.tmp();
    ctx.push(format!("{smallest} = icmp eq i64 {}, {I64_MIN}", value.value));
    let fault = fault_label(ctx);
    let merge = format!("{}_ok", ctx.alloc_label_base());
    ctx.push(format!(
        "br i1 {smallest}, label %{fault}, label %{merge}"
    ));
    close_fault(ctx, &fault, |ctx| {
        // `-x` is a subtraction from zero, and the message says so: the runtime
        // prints `<lhs> <symbol> <rhs>`, so passing zero as the left operand
        // produces `0 - -9223372036854775808`, which is exactly the operation
        // that has no answer.
        crate::stmt_gen::run_all_scopes_defers(ctx);
        let args = operands(ctx, "-", "0", &value.value, site);
        format!("call void @otter_panic_overflow({args})")
    });
    ctx.push(format!("{merge}:"));
    let negated = ctx.tmp();
    ctx.push(format!("{negated} = sub i64 0, {}", value.value));
    Some(GenValue {
        value: negated,
        ty: TypeKind::I64,
    })
}

/// Lower `add`/`sub`/`mul` through the intrinsic that reports overflow.
///
/// The intrinsic computes the value and the flag together, so the succeeding path
/// is one `call`, two `extractvalue`s and a branch. The value is extracted
/// *before* the branch, in a block that dominates the continuation, so the
/// continuation can use it without another `phi`.
fn gen_overflow(
    ctx: &mut FnCtx<'_>,
    op: BinOp,
    l: &GenValue,
    r: &GenValue,
    site: &FaultSite,
) -> GenValue {
    // Every `BinOp` that reaches this function is `+`, `-` or `*`; the fallback
    // is the one the caller's match already excluded, written as a value rather
    // than as an `unreachable!` because a compiler that panics on its own logic
    // is a compiler that panics on someone's program one day.
    let intrinsic = match op {
        BinOp::Add => "sadd",
        BinOp::Sub => "ssub",
        _ => "smul",
    };
    let pair = ctx.tmp();
    ctx.push(format!(
        "{pair} = call {{ i64, i1 }} @llvm.{intrinsic}.with.overflow.i64(i64 {}, i64 {})",
        l.value, r.value
    ));
    let value = ctx.tmp();
    ctx.push(format!("{value} = extractvalue {{ i64, i1 }} {pair}, 0"));
    let flag = ctx.tmp();
    ctx.push(format!("{flag} = extractvalue {{ i64, i1 }} {pair}, 1"));

    let fault = fault_label(ctx);
    let merge = format!("{}_ok", ctx.alloc_label_base());
    ctx.push(format!("br i1 {flag}, label %{fault}, label %{merge}"));
    let symbol = symbol_of(op);
    close_fault(ctx, &fault, |ctx| {
        crate::stmt_gen::run_all_scopes_defers(ctx);
        let args = operands(ctx, symbol, &l.value, &r.value, site);
        format!("call void @otter_panic_overflow({args})")
    });
    ctx.push(format!("{merge}:"));
    GenValue {
        value,
        ty: TypeKind::I64,
    }
}

/// Lower `sdiv`/`srem` with the check for the two ways they have no answer.
///
/// A divisor of zero is the obvious one. The other is `i64::MIN / -1`: the
/// mathematical answer is one past `i64::MAX`, so it has no representation, and
/// `srem` is undefined there too even though its answer -- zero -- would fit.
/// One branch covers both, and the runtime decides which message to print from
/// the operands it is handed.
///
/// The division itself is emitted in the continuation block, not before the
/// check: a `sdiv` that is going to fault must not run, and reaching
/// `otter_panic_division` is the whole point.
fn gen_division(
    ctx: &mut FnCtx<'_>,
    op: BinOp,
    l: &GenValue,
    r: &GenValue,
    site: &FaultSite,
) -> GenValue {
    let mnemonic = if matches!(op, BinOp::Div) { "sdiv" } else { "srem" };
    let by_zero = ctx.tmp();
    ctx.push(format!("{by_zero} = icmp eq i64 {}, 0", r.value));
    let smallest = ctx.tmp();
    ctx.push(format!("{smallest} = icmp eq i64 {}, {I64_MIN}", l.value));
    let minus_one = ctx.tmp();
    ctx.push(format!("{minus_one} = icmp eq i64 {}, -1", r.value));
    let overflows = ctx.tmp();
    ctx.push(format!("{overflows} = and i1 {smallest}, {minus_one}"));
    let fails = ctx.tmp();
    ctx.push(format!("{fails} = or i1 {by_zero}, {overflows}"));

    let fault = fault_label(ctx);
    let merge = format!("{}_ok", ctx.alloc_label_base());
    ctx.push(format!("br i1 {fails}, label %{fault}, label %{merge}"));
    let symbol = symbol_of(op);
    close_fault(ctx, &fault, |ctx| {
        crate::stmt_gen::run_all_scopes_defers(ctx);
        let args = operands(ctx, symbol, &l.value, &r.value, site);
        format!("call void @otter_panic_division({args})")
    });
    ctx.push(format!("{merge}:"));
    let value = ctx.tmp();
    ctx.push(format!(
        "{value} = {mnemonic} i64 {}, {}",
        l.value, r.value
    ));
    GenValue {
        value,
        ty: TypeKind::I64,
    }
}

/// The arguments of a fault call: the operator, the two operands, and where it
/// happened.
///
/// The operator is a symbol the runtime prints between the operands, and the file
/// name is a string constant; both need the `getelementptr` that
/// [`crate::module::Module::intern_bytes_ptr`] emits, and it is emitted here --
/// inside the fault block -- because nothing on the succeeding path needs it.
///
/// The two numbers are the operands *by SSA name*, which is what makes the
/// message show the values that were actually being combined rather than
/// anything reconstructed from the source.
fn operands(
    ctx: &mut FnCtx<'_>,
    symbol: &str,
    lhs: &str,
    rhs: &str,
    site: &FaultSite,
) -> String {
    let op = ctx.module.intern_bytes_ptr(symbol.as_bytes());
    let file = ctx.module.intern_bytes_ptr(site.file.as_bytes());
    format!(
        "i8* {op}, i64 {lhs}, i64 {rhs}, i8* {file}, i64 {}, i64 {}",
        site.line, site.column
    )
}

/// The symbol the runtime prints between the operands of `op`.
///
/// Total: the fallback is `+`, and reaching it means a `BinOp` that is not
/// arithmetic, which the caller's match has already excluded.
fn symbol_of(op: BinOp) -> &'static str {
    match op {
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Rem => "%",
        _ => "+",
    }
}
