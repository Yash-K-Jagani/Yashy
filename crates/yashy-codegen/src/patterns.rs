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

use yashy_parser::ast::Literal;
use yashy_sema::{HirBlock, HirExpr, HirPattern, HirSwitchArm, TypeKind};

use crate::expr_gen::{gen_expr, gen_literal, has_string_payload, retain_payload, FnCtx, GenValue};
use crate::ir::{repr_type, variant_payload_index, variant_tag};
use crate::stmt_gen::Flow;

/// What one arm tests, when it tests anything.
///
/// A variant arm tests the scrutinee's tag; a literal arm compares the
/// scrutinee itself. They are one enum rather than two parallel tables so an
/// arm can never get a tag test and a literal test at once.
enum Test {
    Tag(u8),
    Literal(Literal),
    /// A guard on a pattern that matches everything.
    ///
    /// A bare `case _ if c` has no tag to test and still has two ways to go:
    /// the guard says yes, or the search continues. Giving it a place in the
    /// chain -- a block that branches straight to the arm -- is what keeps the
    /// arm at its source position instead of turning it into the chain's
    /// fall-through, where nothing after it could be tried.
    Always,
}

/// The test an arm performs, or `None` for an arm that matches everything and
/// has no guard.
fn test_for(ctx: &FnCtx<'_>, arm: &HirSwitchArm, subject: &TypeKind) -> Option<Test> {
    if arm.is_default {
        return arm.guard.as_ref().map(|_| Test::Always);
    }
    match test_for_pattern(ctx, &arm.pattern, subject) {
        Some(test) => Some(test),
        // A wildcard or a binding matches everything, so normally it is the
        // fall-through. A guard changes that: the arm can decline the value,
        // and the arms after it still have to be tried.
        None if arm.guard.is_some() => Some(Test::Always),
        None => None,
    }
}

/// The test one pattern performs against a subject, ignoring guards.
///
/// Shared by `switch` arms and `if let`: a pattern is tested the same way
/// wherever it appears, and the only difference between the two forms is what
/// happens on a match.
fn test_for_pattern(ctx: &FnCtx<'_>, pattern: &HirPattern, subject: &TypeKind) -> Option<Test> {
    if let HirPattern::Literal(literal, _) = pattern {
        return Some(Test::Literal(literal.clone()));
    }
    let user = ctx.module.enum_variants(subject);
    pattern_variant_name(pattern)
        .and_then(|name| variant_tag(subject, name, user))
        .map(Test::Tag)
}

/// Emit the instructions for one test, returning the computed `i1` value.
///
/// Each branch allocates its temps in the order the pinned goldens record: the
/// tag extraction first, then the comparison. `Always` allocates nothing --
/// there is no question to ask -- and answers `None`.
fn emit_test(ctx: &mut FnCtx<'_>, subject: &GenValue, test: &Test) -> Option<String> {
    match test {
        Test::Tag(tag) => {
            let subject_repr = repr_type(&subject.ty);
            let tag_type = ctx.module.tag_type(&subject.ty);
            let tag_field = ctx.tmp();
            ctx.push(format!(
                "{tag_field} = extractvalue {subject_repr} {}, 0",
                subject.value
            ));
            let is_match = ctx.tmp();
            // The compared operand is a bare integer: modern LLVM has no
            // typed constants, so `i8 0` here is a parse error ("expected
            // value token").
            ctx.push(format!(
                "{is_match} = icmp eq {tag_type} {tag_field}, {tag}"
            ));
            Some(is_match)
        }
        Test::Literal(literal) => {
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
            Some(is_match)
        }
        Test::Always => None,
    }
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

    let tests: Vec<Option<Test>> = arms
        .iter()
        .map(|arm| test_for(ctx, arm, &subject.ty))
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
    //
    // `fail_targets[index]` is where arm `index` goes when its *guard* says no.
    // That is the same place its test would have gone on a non-match: the next
    // test, or the chain's fall-through arm. A guard that fails is a pattern
    // that did not match, which is exactly the semantics Yashy gives it.
    let mut fail_targets: Vec<Option<String>> = vec![None; arms.len()];
    let mut opened = false;
    for (index, test) in tests.iter().enumerate() {
        let Some(test) = test else { continue };
        if opened {
            ctx.push(format!("{}:", test_labels[index]));
        }
        opened = true;

        // Each branch allocates its temps in the order the pinned goldens
        // record: the tag extraction first, then the comparison. `Always`
        // allocates nothing -- there is no question to ask.
        let is_match = emit_test(ctx, subject, test);

        let next = next_test(&tests, index)
            .map(|next| test_labels[next].clone())
            .or_else(|| fallthrough.map(|arm| arm_labels[arm].clone()))
            .unwrap_or_else(|| result_done.clone());
        fail_targets[index] = Some(next.clone());
        match is_match {
            Some(is_match) => ctx.push(format!(
                "br i1 {is_match}, label %{}, label %{next}",
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
fn next_test(tests: &[Option<Test>], from: usize) -> Option<usize> {
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
        HirPattern::Wildcard(_) | HirPattern::UnitVariant { .. } | HirPattern::Literal(..) => {}
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

    match test_for_pattern(ctx, pattern, &subject.ty) {
        Some(test) => match emit_test(ctx, &subject, &test) {
            Some(is_match) => ctx.push(format!(
                "br i1 {is_match}, label %{then_label}, label %{else_label}"
            )),
            // `Always` is what a guarded `switch` arm asks for; an `if let`
            // pattern has no guard, so this is unreachable and the branch is
            // written as an unconditional one.
            None => ctx.push(format!("br label %{then_label}")),
        },
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
        HirPattern::TupleVariant { args, .. } => {
            let start = pattern_variant_name(pattern).and_then(|name| {
                let user = ctx.module.enum_variants(&subject.ty);
                variant_payload_index(&subject.ty, name, user)
            });
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
                    bind_value(ctx, name, &payload, &ty.kind);
                }
            }
        }
        HirPattern::UnitVariant { .. } => {} // no payload
        HirPattern::StructVariant { fields, .. } => {
            // A named payload is flattened into the enum's aggregate in source
            // order, so a field's index is its position in its own variant
            // plus the fields every earlier variant contributed.
            let Some(variant_name) = pattern_variant_name(pattern) else {
                return;
            };
            let Some(variants) = ctx
                .module
                .enum_variants(&subject.ty)
                .map(<[_]>::to_vec)
            else {
                return;
            };
            let mut start = 1u32;
            let mut declared_fields = None;
            for declared in &variants {
                if declared.name == variant_name {
                    declared_fields = Some(declared.fields.clone());
                    break;
                }
                start += declared.fields.len() as u32;
            }
            let Some(declared_fields) = declared_fields else {
                return;
            };
            let subject_repr = repr_type(&subject.ty);
            for (field_name, subpattern, _) in fields {
                let Some(offset) = declared_fields
                    .iter()
                    .position(|(name, _)| name.as_deref() == Some(field_name.as_str()))
                else {
                    continue;
                };
                let payload = ctx.tmp();
                ctx.push(format!(
                    "{payload} = extractvalue {subject_repr} {}, {}",
                    subject.value,
                    start + offset as u32
                ));
                if let HirPattern::Binding { name, ty, .. } = subpattern {
                    bind_value(ctx, name, &payload, &ty.kind);
                }
            }
        }
        HirPattern::Literal(_, _) => {} // literal arms bind nothing
    }
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
