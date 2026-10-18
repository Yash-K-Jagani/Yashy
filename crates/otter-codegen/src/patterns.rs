//! Pattern lowering: a `switch` subject tested against its arms, with the
//! payload fields of matching variants bound into local slots.
//!
//! The shape of a lowered match is fixed: for each non-wildcard arm, load the
//! scrutinee's tag field, compare it against the variant's tag constant, and
//! branch into that arm's block; the arm's body writes its result into a
//! shared slot and jumps to the merge block. A wildcard or `default` arm gets
//! no tag test — it is the block the tests fall through to.
//!
//! # No input makes this module panic
//!
//! Tag and payload lookups return `Option` (see [`crate::ir::variant_tag`]).
//! An arm whose variant the scrutinee does not have lowers to an untested
//! branch plus a comment rather than an abort; sema has already rejected that
//! program, so the fallback exists only so codegen can never panic.

use otter_sema::{HirPattern, HirSwitchArm, TypeKind};

use crate::expr_gen::{gen_expr, FnCtx, GenValue};
use crate::ir::{repr_type, variant_payload_index, variant_tag};

/// Lower a `switch` on `subject` against `arms`. Returns the name of the
/// result alloca slot, or `""` when the result type is `()`.
pub fn gen_match(
    ctx: &mut FnCtx<'_>,
    subject: &GenValue,
    arms: &[HirSwitchArm],
    result_ty: &TypeKind,
) -> String {
    let result_repr = repr_type(result_ty);
    let result_slot = if result_repr == "void" {
        String::new()
    } else {
        let slot = ctx.alloc_slot();
        ctx.push(format!("{slot} = alloca {result_repr}"));
        slot
    };
    let result_done = ctx.alloc_label_base() + "_result";

    let last = arms.len().saturating_sub(1);
    for (index, arm) in arms.iter().enumerate() {
        let is_last = index == last;
        let next_block = if is_last {
            result_done.clone()
        } else {
            ctx.alloc_label_base() + "_next"
        };
        let arm_block = ctx.alloc_label_base() + "_arm";

        // Tag test: only concrete variant arms need one. A wildcard, a
        // binding, or `default` falls straight through to its own block.
        let variant_name = pattern_variant_name(&arm.pattern);
        let tag_value = if arm.is_default {
            None
        } else {
            variant_name.and_then(|name| variant_tag(&subject.ty, name))
        };

        match tag_value {
            Some(tag) => {
                let tag_tmp = ctx.tmp();
                let subject_repr = repr_type(&subject.ty);
                ctx.push(format!(
                    "{tag_tmp} = extractvalue {subject_repr} {}, 0",
                    subject.value
                ));
                let cmp = ctx.tmp();
                ctx.push(format!("{cmp} = icmp eq i8 {tag_tmp}, i8 {tag}"));
                ctx.push(format!("br i1 {cmp}, label %{arm_block}, label %{next_block}"));
            }
            None => {
                if !arm.is_default && variant_name.is_some() {
                    ctx.push(format!(
                        "; TODO: arm names a variant the scrutinee does not have (at {:?})",
                        arm.span
                    ));
                }
                ctx.push(format!("br label %{arm_block}"));
            }
        }

        ctx.push(format!("{arm_block}:"));
        bind_pattern(ctx, subject, &arm.pattern);

        let body_value = gen_expr(ctx, &arm.body);
        if result_repr != "void" && !body_value.value.is_empty() {
            ctx.push(format!(
                "store {result_repr} {}, ptr {result_slot}",
                body_value.value
            ));
        }
        ctx.push(format!("br label %{result_done}"));
    }

    ctx.push(format!("{result_done}:"));
    result_slot
}

/// The variant name a pattern names, when it names one.
pub fn pattern_variant_name(pattern: &HirPattern) -> Option<&str> {
    match pattern {
        HirPattern::UnitVariant { path, .. } => path.last().map(String::as_str),
        HirPattern::TupleVariant { path, .. } => path.last().map(String::as_str),
        HirPattern::StructVariant { path, .. } => path.last().map(String::as_str),
        _ => None,
    }
}

/// Bind every pattern variable into `ctx.locals` before an arm's body runs.
///
/// `Binding` (a `default` arm's catch-all) stores the whole subject value;
/// `TupleVariant` arms extract the payload field(s) per
/// [`variant_payload_index`].
pub fn bind_pattern(ctx: &mut FnCtx<'_>, subject: &GenValue, pattern: &HirPattern) {
    match pattern {
        HirPattern::Wildcard(_) => {}
        HirPattern::Binding { name, ty, .. } => {
            let slot = ctx.alloc_slot();
            let repr = repr_type(&ty.kind);
            ctx.push(format!("{slot} = alloca {repr}"));
            if !subject.value.is_empty() {
                ctx.push(format!("store {repr} {}, ptr {slot}", subject.value));
            }
            ctx.locals.insert(name.clone(), (slot, ty.kind.clone()));
        }
        HirPattern::TupleVariant { args, .. } => {
            let start = pattern_variant_name(pattern)
                .and_then(|name| variant_payload_index(&subject.ty, name));
            let subject_repr = repr_type(&subject.ty);
            for (offset, arg) in args.iter().enumerate() {
                let index = match start {
                    Some(start) => start + offset as u32,
                    None => continue,
                };
                let payload_tmp = ctx.tmp();
                ctx.push(format!(
                    "{payload_tmp} = extractvalue {subject_repr} {}, {index}",
                    subject.value
                ));
                if let HirPattern::Binding { name, ty, .. } = arg {
                    let slot = ctx.alloc_slot();
                    let repr = repr_type(&ty.kind);
                    ctx.push(format!("{slot} = alloca {repr}"));
                    ctx.push(format!("store {repr} {payload_tmp}, ptr {slot}"));
                    ctx.locals.insert(name.clone(), (slot, ty.kind.clone()));
                }
            }
        }
        HirPattern::UnitVariant { .. } => {} // no payload
        HirPattern::StructVariant { .. } => {} // Task 12 threads the field table
        HirPattern::Literal(_, _) => {} // literal arms bind nothing
    }
}
