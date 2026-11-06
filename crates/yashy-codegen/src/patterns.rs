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
//! An arm with no test (a `default`, a wildcard, or a bare binding) is the
//! chain's fall-through: the first such arm is the block the tests bottom out
//! in.
//!
//! A pattern is a boolean question, not a single tag comparison: a nested
//! payload (`A(Some(1))`) ands one comparison per column, an or-pattern ors
//! its alternatives, and a range compares both ends. Binding is separate
//! because an or-pattern has to bind through the alternative that matched;
//! [`bind_pattern`] emits that little dispatch when it meets one.
//!
//! # No input makes this module panic
//!
//! Tag lookups return `Option` (see [`crate::ir::variant_tag`]); an arm whose
//! variant the scrutinee does not have becomes an untested arm rather than an
//! abort. Sema has already rejected that program, so the fallback exists only
//! so codegen can never panic.

use yashy_parser::ast::Literal;
use yashy_sema::{HirBlock, HirExpr, HirPattern, HirSwitchArm, TypeKind};

use crate::expr_gen::{gen_expr, gen_literal, has_string_payload, retain_payload, FnCtx, GenValue};
use crate::ir::{repr_type, variant_payload_index, variant_tag};
use crate::stmt_gen::Flow;

/// If a pattern can fail, its test is a boolean expression; when it cannot,
/// the arm is a fall-through unless a guard gives it a decision to make.
fn needs_test(pattern: &HirPattern) -> bool {
    match pattern {
        HirPattern::Wildcard(_) | HirPattern::Binding { .. } => false,
        // An or-pattern matches everything when one alternative does.
        HirPattern::Or { alternatives, .. } => alternatives.iter().all(needs_test),
        HirPattern::UnitVariant { .. }
        | HirPattern::TupleVariant { .. }
        | HirPattern::StructVariant { .. }
        | HirPattern::Literal(..)
        | HirPattern::Range { .. } => true,
    }
}

/// Emit the instructions for `pattern` against `subject`, returning the
/// computed `i1` value, or `None` when the pattern matches everything.
fn emit_pattern_test(
    ctx: &mut FnCtx<'_>,
    pattern: &HirPattern,
    subject: &GenValue,
) -> Option<String> {
    match pattern {
        HirPattern::Wildcard(_) | HirPattern::Binding { .. } => None,
        HirPattern::Literal(literal, _) => Some(emit_literal_test(ctx, literal, subject)),
        HirPattern::Range {
            lo,
            hi,
            inclusive,
            ..
        } => Some(emit_range_test(ctx, lo, hi, *inclusive, subject)),
        HirPattern::UnitVariant { .. } => emit_tag_test(ctx, pattern, subject),
        HirPattern::TupleVariant { args, .. } => {
            let mut conditions = Vec::new();
            if let Some(tag) = emit_tag_test(ctx, pattern, subject) {
                conditions.push(tag);
            }
            let fields = payload_fields(ctx, &subject.ty, pattern);
            let start = payload_start(ctx, &subject.ty, pattern);
            if let Some(start) = start {
                for (index, arg) in args.iter().enumerate() {
                    if !needs_test(arg) {
                        continue;
                    }
                    let Some((_, field_ty)) = fields.get(index) else {
                        continue;
                    };
                    let payload = extract_field(ctx, subject, start + index as u32);
                    let value = GenValue {
                        value: payload,
                        ty: field_ty.clone(),
                    };
                    if let Some(condition) = emit_pattern_test(ctx, arg, &value) {
                        conditions.push(condition);
                    }
                }
            }
            and_all(ctx, conditions)
        }
        HirPattern::StructVariant { fields, .. } => {
            let mut conditions = Vec::new();
            if let Some(tag) = emit_tag_test(ctx, pattern, subject) {
                conditions.push(tag);
            }
            let declared = payload_fields(ctx, &subject.ty, pattern);
            let start = payload_start(ctx, &subject.ty, pattern);
            if let Some(start) = start {
                for (field_name, subpattern, _) in fields {
                    if !needs_test(subpattern) {
                        continue;
                    }
                    let Some(offset) = declared
                        .iter()
                        .position(|(name, _)| name.as_deref() == Some(field_name.as_str()))
                    else {
                        continue;
                    };
                    let Some((_, field_ty)) = declared.get(offset) else {
                        continue;
                    };
                    let payload = extract_field(ctx, subject, start + offset as u32);
                    let value = GenValue {
                        value: payload,
                        ty: field_ty.clone(),
                    };
                    if let Some(condition) = emit_pattern_test(ctx, subpattern, &value) {
                        conditions.push(condition);
                    }
                }
            }
            and_all(ctx, conditions)
        }
        HirPattern::Or { alternatives, .. } => {
            let mut conditions = Vec::new();
            for alternative in alternatives {
                match emit_pattern_test(ctx, alternative, subject) {
                    Some(condition) => conditions.push(condition),
                    None => return None,
                }
            }
            or_all(ctx, conditions)
        }
    }
}

fn emit_tag_test(
    ctx: &mut FnCtx<'_>,
    pattern: &HirPattern,
    subject: &GenValue,
) -> Option<String> {
    let user = ctx.module.enum_variants(&subject.ty);
    let name = pattern_variant_name(pattern)?;
    let tag = variant_tag(&subject.ty, name, user)?;
    let subject_repr = repr_type(&subject.ty);
    let tag_type = ctx.module.tag_type(&subject.ty);
    let tag_field = ctx.tmp();
    ctx.push(format!(
        "{tag_field} = extractvalue {subject_repr} {}, 0",
        subject.value
    ));
    let is_match = ctx.tmp();
    ctx.push(format!(
        "{is_match} = icmp eq {tag_type} {tag_field}, {tag}"
    ));
    Some(is_match)
}

fn emit_literal_test(ctx: &mut FnCtx<'_>, literal: &Literal, subject: &GenValue) -> String {
    let value = gen_literal(ctx, literal, &subject.ty);
    let is_match = ctx.tmp();
    if matches!(subject.ty, TypeKind::Str) {
        ctx.push(format!(
            "{is_match} = call i1 @yashy_str_eq(i8* {}, i8* {})",
            subject.value, value.value
        ));
    } else if matches!(subject.ty, TypeKind::F64) {
        ctx.push(format!(
            "{is_match} = fcmp oeq double {}, {}",
            subject.value, value.value
        ));
    } else {
        ctx.push(format!(
            "{is_match} = icmp eq {} {}, {}",
            repr_type(&subject.ty),
            subject.value,
            value.value
        ));
    }
    is_match
}

fn emit_range_test(
    ctx: &mut FnCtx<'_>,
    lo: &Literal,
    hi: &Literal,
    inclusive: bool,
    subject: &GenValue,
) -> String {
    let lo_value = gen_literal(ctx, lo, &subject.ty);
    let hi_value = gen_literal(ctx, hi, &subject.ty);
    let repr = repr_type(&subject.ty);
    // A `char` is an unsigned byte; an `int` is signed. Choosing the compare
    // here keeps `'\u{80}'..` from being read as negative.
    let unsigned = matches!(subject.ty, TypeKind::Char | TypeKind::CUInt(_));
    let lower_op = if unsigned { "uge" } else { "sge" };
    let upper_op = match (unsigned, inclusive) {
        (true, true) => "ule",
        (true, false) => "ult",
        (false, true) => "sle",
        (false, false) => "slt",
    };
    let at_least = ctx.tmp();
    ctx.push(format!(
        "{at_least} = icmp {lower_op} {repr} {}, {}",
        subject.value, lo_value.value
    ));
    let at_most = ctx.tmp();
    ctx.push(format!(
        "{at_most} = icmp {upper_op} {repr} {}, {}",
        subject.value, hi_value.value
    ));
    let is_match = ctx.tmp();
    ctx.push(format!("{is_match} = and i1 {at_least}, {at_most}"));
    is_match
}

fn and_all(ctx: &mut FnCtx<'_>, conditions: Vec<String>) -> Option<String> {
    let mut iter = conditions.into_iter();
    let mut acc = iter.next()?;
    for condition in iter {
        let next = ctx.tmp();
        ctx.push(format!("{next} = and i1 {acc}, {condition}"));
        acc = next;
    }
    Some(acc)
}

fn or_all(ctx: &mut FnCtx<'_>, conditions: Vec<String>) -> Option<String> {
    let mut iter = conditions.into_iter();
    let mut acc = iter.next()?;
    for condition in iter {
        let next = ctx.tmp();
        ctx.push(format!("{next} = or i1 {acc}, {condition}"));
        acc = next;
    }
    Some(acc)
}

/// Extract one field of the subject aggregate.
fn extract_field(ctx: &mut FnCtx<'_>, subject: &GenValue, index: u32) -> String {
    let payload = ctx.tmp();
    ctx.push(format!(
        "{payload} = extractvalue {} {}, {index}",
        repr_type(&subject.ty),
        subject.value
    ));
    payload
}

/// The payload fields of the variant a pattern names, in declaration order.
fn payload_fields(
    ctx: &FnCtx<'_>,
    subject_ty: &TypeKind,
    pattern: &HirPattern,
) -> Vec<(Option<String>, TypeKind)> {
    let Some(name) = pattern_variant_name(pattern) else {
        return Vec::new();
    };
    match subject_ty {
        TypeKind::Option(inner) => {
            if name == "Some" {
                vec![(None, (**inner).kind.clone())]
            } else {
                Vec::new()
            }
        }
        TypeKind::Result(ok, err) => {
            if name == "Ok" {
                vec![(None, (**ok).kind.clone())]
            } else if name == "Err" {
                vec![(None, (**err).kind.clone())]
            } else {
                Vec::new()
            }
        }
        TypeKind::Enum(_) => ctx
            .module
            .enum_variants(subject_ty)
            .and_then(|variants| variants.iter().find(|variant| variant.name == name))
            .map(|variant| variant.fields.clone())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn payload_start(ctx: &FnCtx<'_>, subject_ty: &TypeKind, pattern: &HirPattern) -> Option<u32> {
    let name = pattern_variant_name(pattern)?;
    let user = ctx.module.enum_variants(subject_ty);
    variant_payload_index(subject_ty, name, user)
}

/// Lower a `switch` on `subject` against `arms`. Returns the name of the
/// result alloca slot, or `""` when the result type is `void`.
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
        ctx.push_alloca(format!("{slot} = alloca {result_repr}"));
        slot
    };
    let result_done = ctx.alloc_label_base() + "_result";

    let tested: Vec<bool> = arms
        .iter()
        .map(|arm| {
            if arm.is_default {
                arm.guard.is_some()
            } else {
                needs_test(&arm.pattern) || arm.guard.is_some()
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
    let fallthrough = tested.iter().position(|test| !test);

    // Phase 1: the test chain. The first test lands in the current block; each
    // later one opens the block the previous test branched to.
    //
    // `fail_targets[index]` is where arm `index` goes when its *guard* says no.
    // That is the same place its test would have gone on a non-match: the next
    // test, or the chain's fall-through arm. A guard that fails is a pattern
    // that did not match, which is exactly the semantics Yashy gives it.
    let mut fail_targets: Vec<Option<String>> = vec![None; arms.len()];
    let mut opened = false;
    for (index, arm) in arms.iter().enumerate() {
        if !tested[index] {
            continue;
        }
        if opened {
            ctx.push(format!("{}:", test_labels[index]));
        }
        opened = true;

        let condition = if arm.is_default {
            None
        } else {
            emit_pattern_test(ctx, &arm.pattern, subject)
        };
        let next = next_test(&tested, index)
            .map(|next| test_labels[next].clone())
            .or_else(|| fallthrough.map(|arm| arm_labels[arm].clone()))
            .unwrap_or_else(|| result_done.clone());
        fail_targets[index] = Some(next.clone());
        match condition {
            Some(condition) => ctx.push(format!(
                "br i1 {condition}, label %{}, label %{next}",
                arm_labels[index]
            )),
            None => ctx.push(format!("br label %{}", arm_labels[index])),
        }
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
    // writes the result into the shared slot. An arm with a guard evaluates it
    // first: true falls into the body, false goes where a failed test would
    // have gone. An unguarded arm has no extra block, so its IR is untouched.
    //
    // A `string` result (or an `Option[string]`/`Result[string, E]` result) is a
    // reference the match owns: a fresh one is taken over from the arm, and a
    // borrowed one -- a binding, a parameter -- is retained, because the subject
    // is released after the merge point and the result may point into it.
    let owns_result = matches!(result_ty, TypeKind::Str) || has_string_payload(result_ty);
    for (index, arm) in arms.iter().enumerate() {
        ctx.push(format!("{}:", arm_labels[index]));
        bind_pattern(ctx, subject, &arm.pattern);
        if let Some(guard) = &arm.guard {
            let condition = gen_expr(ctx, guard);
            let accepted = ctx.alloc_label_base() + "_accept";
            let rejected = fail_targets[index]
                .clone()
                .unwrap_or_else(|| result_done.clone());
            ctx.push(format!(
                "br i1 {}, label %{accepted}, label %{rejected}",
                condition.value
            ));
            ctx.push(format!("{accepted}:"));
        }
        let body_value = gen_expr(ctx, &arm.body);
        if result_repr != "void" && !body_value.value.is_empty() {
            let mut stored = body_value.value.clone();
            if owns_result {
                if ctx.is_owned(&body_value.value) {
                    ctx.forget_owned(&body_value.value);
                } else if matches!(result_ty, TypeKind::Str) {
                    let retained = ctx.tmp();
                    ctx.push_str_retain_into(&retained, &body_value.value);
                    stored = retained;
                } else {
                    stored = retain_payload(ctx, &body_value.value, result_ty);
                }
            }
            ctx.push(format!("store {result_repr} {stored}, ptr {result_slot}"));
        }
        ctx.push(format!("br label %{result_done}"));
    }

    ctx.push(format!("{result_done}:"));
    // A subject the program built just for the match (`switch read_line() { … }`)
    // owns a reference, and nothing else will release it. Every result that
    // pointed into it took its own reference above, so the subject is released
    // here, after every arm has run. A subject that is a copy of a binding or a
    // parameter owns nothing, and is left alone.
    if ctx.is_owned(&subject.value) {
        ctx.release_owned(&subject.value);
    }
    result_slot
}

/// The index of the next arm that tests anything, after `from`.
fn next_test(tested: &[bool], from: usize) -> Option<usize> {
    tested
        .iter()
        .enumerate()
        .skip(from + 1)
        .find(|(_, test)| **test)
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

/// Collect the names `pattern` binds, in the order it binds them.
fn pattern_names(pattern: &HirPattern, out: &mut Vec<String>) {
    match pattern {
        HirPattern::Binding { name, .. } => out.push(name.clone()),
        HirPattern::TupleVariant { args, .. } => {
            for arg in args {
                pattern_names(arg, out);
            }
        }
        HirPattern::StructVariant { fields, .. } => {
            for (_, sub, _) in fields {
                pattern_names(sub, out);
            }
        }
        HirPattern::Or { alternatives, .. } => {
            // Every alternative binds the same names by the checker's rule, so
            // one of them is the complete list.
            if let Some(first) = alternatives.first() {
                pattern_names(first, out);
            }
        }
        HirPattern::Wildcard(_)
        | HirPattern::UnitVariant { .. }
        | HirPattern::Literal(..)
        | HirPattern::Range { .. } => {}
    }
}

/// Lower `if let <pattern> = <subject> { then } else { else }`.
///
/// The subject is evaluated once and tested the way a `switch` arm tests its
/// pattern: a match binds the pattern's names and enters `then`, a miss enters
/// `else`. Both arms are blocks, so `return`, `break` and `continue` inside one
/// terminate the current block and skip the merge label, exactly as they do in
/// an `if`.
///
/// A subject the program built just for the test (`if let Some(x) = next()`)
/// owns a reference and is released after the merge point, where every branch
/// has already run. A subject that is a copy of a binding or a parameter owns
/// nothing and is left alone.
pub fn gen_if_let(
    ctx: &mut FnCtx<'_>,
    pattern: &HirPattern,
    subject_expr: &HirExpr,
    then_block: &HirBlock,
    else_block: Option<&HirBlock>,
) -> Flow {
    let subject = gen_expr(ctx, subject_expr);
    let then_label = ctx.alloc_label_base() + "_then";
    let else_label = ctx.alloc_label_base() + "_else";
    let merge_label = ctx.alloc_label_base() + "_merge";

    match emit_pattern_test(ctx, pattern, &subject) {
        Some(condition) => ctx.push(format!(
            "br i1 {condition}, label %{then_label}, label %{else_label}"
        )),
        // A wildcard or a binding matches everything. The checker rejects the
        // form as E0603, so no valid program reaches here; taking the `then`
        // branch keeps the module well formed if one somehow does.
        None => ctx.push(format!("br label %{then_label}")),
    }

    // The bindings are scoped to the `if let`, so whatever they shadowed comes
    // back when the statement ends. The statement's own names cannot be read
    // after it -- the checker popped their scope -- but an outer binding of the
    // same name can.
    let mut names = Vec::new();
    pattern_names(pattern, &mut names);
    let shadowed: Vec<(String, Option<(String, TypeKind)>)> = names
        .into_iter()
        .map(|name| {
            let previous = ctx.locals.get(&name).cloned();
            (name, previous)
        })
        .collect();

    ctx.push(format!("{then_label}:"));
    bind_pattern(ctx, &subject, pattern);
    if crate::stmt_gen::gen_block(ctx, then_block) == Flow::Fallthrough {
        ctx.push(format!("br label %{merge_label}"));
    }

    ctx.push(format!("{else_label}:"));
    match else_block {
        Some(block) => {
            if crate::stmt_gen::gen_block(ctx, block) == Flow::Fallthrough {
                ctx.push(format!("br label %{merge_label}"));
            }
        }
        None => ctx.push(format!("br label %{merge_label}")),
    }

    ctx.push(format!("{merge_label}:"));
    for (name, previous) in shadowed {
        match previous {
            Some(binding) => {
                ctx.locals.insert(name, binding);
            }
            None => {
                ctx.locals.remove(&name);
            }
        }
    }
    if ctx.is_owned(&subject.value) {
        ctx.release_owned(&subject.value);
    }
    Flow::Fallthrough
}

/// Bind every pattern variable into `ctx.locals` before an arm's body runs.
///
/// `Binding` (a `default` arm's catch-all) stores the whole subject value;
/// `TupleVariant` arms extract the payload field(s) per
/// [`variant_payload_index`].
///
/// An `Or` pattern has to know which alternative matched, because that decides
/// where its payload lives. It emits a small test chain over its alternatives
/// and binds inside the matching block, then continues at a join label. Every
/// other form is straight-line.
///
/// A binding is a **borrow**: the slot holds the subject's pointer, and the
/// subject outlives every arm (it is released after the merge point, or not at
/// all when it belongs to a parameter). Only a value that escapes the arm -- the
/// match's own result -- takes its own reference, which [`gen_match`] does at
/// the store. Retaining here would double-count and, worse, release a slot on a
/// path where a different arm ran and never initialised it.
pub fn bind_pattern(ctx: &mut FnCtx<'_>, subject: &GenValue, pattern: &HirPattern) {
    match pattern {
        HirPattern::Wildcard(_) => {}
        HirPattern::Binding { name, ty, .. } => {
            bind_value(ctx, name, &subject.value, &ty.kind);
        }
        HirPattern::Or { alternatives, .. } => bind_or(ctx, subject, alternatives),
        HirPattern::TupleVariant { args, .. } => {
            let fields = payload_fields(ctx, &subject.ty, pattern);
            let start = payload_start(ctx, &subject.ty, pattern);
            let Some(start) = start else {
                return;
            };
            for (offset, arg) in args.iter().enumerate() {
                let Some((_, field_ty)) = fields.get(offset) else {
                    continue;
                };
                let payload = extract_field(ctx, subject, start + offset as u32);
                let value = GenValue {
                    value: payload,
                    ty: field_ty.clone(),
                };
                bind_pattern(ctx, &value, arg);
            }
        }
        HirPattern::UnitVariant { .. } => {} // no payload
        HirPattern::StructVariant { fields, .. } => {
            // A named payload is flattened into the enum's aggregate in source
            // order, so a field's index is its position in its own variant
            // plus the fields every earlier variant contributed.
            let declared = payload_fields(ctx, &subject.ty, pattern);
            let start = payload_start(ctx, &subject.ty, pattern);
            let Some(start) = start else {
                return;
            };
            for (field_name, subpattern, _) in fields {
                let Some(offset) = declared
                    .iter()
                    .position(|(name, _)| name.as_deref() == Some(field_name.as_str()))
                else {
                    continue;
                };
                let Some((_, field_ty)) = declared.get(offset) else {
                    continue;
                };
                let payload = extract_field(ctx, subject, start + offset as u32);
                let value = GenValue {
                    value: payload,
                    ty: field_ty.clone(),
                };
                bind_pattern(ctx, &value, subpattern);
            }
        }
        HirPattern::Literal(_, _) | HirPattern::Range { .. } => {} // bind nothing
    }
}

/// Bind an or-pattern through the alternative that matched.
fn bind_or(ctx: &mut FnCtx<'_>, subject: &GenValue, alternatives: &[HirPattern]) {
    if alternatives.is_empty() {
        return;
    }
    let join = ctx.alloc_label_base() + "_or_join";
    for (index, alternative) in alternatives.iter().enumerate() {
        let last = index + 1 == alternatives.len();
        if last {
            bind_pattern(ctx, subject, alternative);
            ctx.push(format!("br label %{join}"));
            break;
        }
        let hit = ctx.alloc_label_base() + "_or_hit";
        let miss = ctx.alloc_label_base() + "_or_miss";
        match emit_pattern_test(ctx, alternative, subject) {
            Some(condition) => {
                ctx.push(format!("br i1 {condition}, label %{hit}, label %{miss}"))
            }
            None => ctx.push(format!("br label %{hit}")),
        }
        ctx.push(format!("{hit}:"));
        bind_pattern(ctx, subject, alternative);
        ctx.push(format!("br label %{join}"));
        ctx.push(format!("{miss}:"));
    }
    ctx.push(format!("{join}:"));
}

/// Store `value` in a fresh slot bound to `name`.
///
/// The binding borrows: the slot holds the pointer the subject already owns and
/// the slot is not tracked in any scope, so nothing releases it. The subject is
/// released after the merge point (when the match owns it), which is after every
/// use of the binding.
fn bind_value(ctx: &mut FnCtx<'_>, name: &str, value: &str, ty: &TypeKind) {
    let repr = repr_type(ty);
    let slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{slot} = alloca {repr}"));
    if !value.is_empty() {
        ctx.push(format!("store {repr} {value}, ptr {slot}"));
    }
    ctx.locals.insert(name.to_string(), (slot, ty.clone()));
}
