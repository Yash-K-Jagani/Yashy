//! Pattern lowering: a `switch` subject tested against its arms, with the
//! payload fields of matching variants bound into local slots.
//!
//! # Shape
//!
//! A lowered match is a linear test chain followed by the arm bodies:
//!
//! ```text
//! ; current block
//! test0:  %tag = extractvalue …; %c = icmp eq …; br i1 %c, %arm0, %test1
//! test1:  …                                              br i1 %c, %arm1, %fallthrough
//! fallthrough:                                           br label %<first untested arm>
//! arm0:   <bindings> <body> br %result
//! arm1:   <bindings> <body> br %result
//! result:
//! ```
//!
//! The chain is what makes the IR well formed. Emitting each test immediately
//! before its own body, the obvious alternative, appends the *next* arm's
//! branch to a block the previous arm already terminated with `br %result` —
//! two terminators in one block, which LLVM rejects. Testing first and
//! lowering bodies afterwards keeps every block's terminator unique.
//!
//! An arm with no tag test (a `default`, a wildcard, or a bare binding) is the
//! chain's fall-through: the first such arm is the block the tests bottom out
//! in.
//!
//! # No input makes this module panic
//!
//! Tag lookups return `Option` (see [`crate::ir::variant_tag`]); an arm whose
//! variant the scrutinee does not have becomes an untested arm rather than an
//! abort. Sema has already rejected that program, so the fallback exists only
//! so codegen can never panic.

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

    // The tag each arm tests against, or `None` when the arm has no test.
    let tests: Vec<Option<u8>> = arms
        .iter()
        .map(|arm| {
            if arm.is_default {
                None
            } else {
                pattern_variant_name(&arm.pattern)
                    .and_then(|name| variant_tag(&subject.ty, name))
            }
        })
        .collect();
    let arm_labels: Vec<String> = arms
        .iter()
        .map(|_| ctx.alloc_label_base() + "_arm")
        .collect();
    let test_labels: Vec<String> = arms
        .iter()
        .map(|_| ctx.alloc_label_base() + "_test")
        .collect();
    let fallthrough = tests.iter().position(|test| test.is_none());

    // Phase 1: the test chain. The first test lands in the current block; each
    // later one opens the block the previous test branched to.
    let mut opened = false;
    for (index, test) in tests.iter().enumerate() {
        let Some(tag) = test else { continue };
        if opened {
            ctx.push(format!("{}:", test_labels[index]));
        }
        opened = true;

        let subject_repr = repr_type(&subject.ty);
        let tag_field = ctx.tmp();
        ctx.push(format!(
            "{tag_field} = extractvalue {subject_repr} {}, 0",
            subject.value
        ));
        let is_match = ctx.tmp();
        ctx.push(format!("{is_match} = icmp eq i8 {tag_field}, i8 {tag}"));

        let next = next_test(&tests, index)
            .map(|next| test_labels[next].clone())
            .or_else(|| fallthrough.map(|arm| arm_labels[arm].clone()))
            .unwrap_or_else(|| result_done.clone());
        ctx.push(format!(
            "br i1 {is_match}, label %{}, label %{next}",
            arm_labels[index]
        ));
    }
    if !opened {
        // No arm tests anything: fall straight through to the untested arm, or
        // to the merge point if every arm was untested (only the first is
        // reachable, the rest are unreachable but structurally required).
        let target = fallthrough
            .map(|arm| arm_labels[arm].clone())
            .unwrap_or_else(|| result_done.clone());
        ctx.push(format!("br label %{target}"));
    }

    // Phase 2: the arm bodies. Each binds its pattern, lowers its body, and
    // writes the result into the shared slot.
    for (index, arm) in arms.iter().enumerate() {
        ctx.push(format!("{}:", arm_labels[index]));
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

/// The index of the next arm that tests a tag, after `from`.
fn next_test(tests: &[Option<u8>], from: usize) -> Option<usize> {
    tests
        .iter()
        .enumerate()
        .skip(from + 1)
        .find(|(_, test)| test.is_some())
        .map(|(index, _)| index)
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
                let payload = ctx.tmp();
                ctx.push(format!(
                    "{payload} = extractvalue {subject_repr} {}, {index}",
                    subject.value
                ));
                if let HirPattern::Binding { name, ty, .. } = arg {
                    let slot = ctx.alloc_slot();
                    let repr = repr_type(&ty.kind);
                    ctx.push(format!("{slot} = alloca {repr}"));
                    ctx.push(format!("store {repr} {payload}, ptr {slot}"));
                    ctx.locals.insert(name.clone(), (slot, ty.kind.clone()));
                }
            }
        }
        HirPattern::UnitVariant { .. } => {} // no payload
        HirPattern::StructVariant { .. } => {} // Task 12 threads the field table
        HirPattern::Literal(_, _) => {} // literal arms bind nothing
    }
}
