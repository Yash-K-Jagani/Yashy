//! Statement lowering: the control-flow backbone of a function body.
//!
//! Every statement returns a [`Flow`] telling the caller whether the current
//! basic block was **terminated** by the statement. A terminated block must
//! not receive more instructions, so [`gen_block`] stops at the first
//! non-`Fallthrough` statement — that is what keeps the emitted IR well formed
//! (LLVM allows exactly one terminator per block, and nothing after it).
//!
//! # Deferred bodies (`defer`)
//!
//! A `defer` body runs when its enclosing scope exits: on fall-through, on
//! `return`, and on `break`/`continue`. The bodies are collected as a stack in
//! the `defers` parameter — one `Vec` per scope, owned by whoever opens the
//! scope — and [`run_defers`] lowers them last-in-first-out. A `return`
//! therefore runs the current scope's defers before emitting `ret`, and a
//! nested `if`/`for` passes its own `Vec` so only its own defers fire at its
//! own exit.
//!
//! A deferred body is validated by the checker when the `defer` is **reached**
//! but runs at **exit**, so each registration snapshots the linear bindings in
//! scope into slots of their own ([`Deferred`]). Without that, the state the
//! checker approved and the state the body executes against can disagree, and
//! the disagreement is a double free rather than a diagnostic.
//!
//! # No input makes this module panic
//!
//! Every lookup is total; a `break` outside a loop, or a missing loop label,
//! lowers to a comment rather than an abort.

use otter_sema::{expr_ty, linearity_of, HirBlock, HirExpr, HirStmt, Linearity, TypeKind};

use crate::expr_gen::{gen_expr, gen_expr_to_slot, FnCtx, GenValue};
use crate::ir::repr_type;
use crate::patterns::gen_match;

/// How a statement left its basic block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    /// The block continues; the next statement emits into it.
    Fallthrough,
    /// The block ended with a branch to the enclosing loop's exit.
    Break,
    /// The block ended with a branch to the enclosing loop's increment.
    Continue,
    /// The block ended with `ret`.
    Return,
}

/// A registered `defer` body, plus the values it must see when it runs.
///
/// The checker validates a deferred body when the `defer` statement is
/// *reached*; the body itself *runs* at scope exit, and the ownership state can
/// differ between those two moments. `captured` is what makes the two agree: at
/// registration every linear binding in scope is copied into a slot of its own,
/// and while the body runs those slots replace the bindings in `FnCtx::locals`.
///
/// Without it, this program compiles and corrupts the heap:
///
/// ```text
/// var buf = alloc(16);
/// defer { dealloc(buf); }   // the checker sees `buf` live here
/// buf = alloc(32);          // ... and does not free the first block here,
/// dealloc(buf);             // because the deferred `dealloc` already moved it
///                           // at registration — so the deferred body frees the
///                           // same pointer a second time at exit.
/// ```
///
/// With the capture, the deferred `dealloc` releases the block that was live at
/// registration (the one still owned), and the body's own `dealloc` releases the
/// rebind. One free per allocation, which is what the source reads like.
///
/// The snapshot is limited to **linear** bindings, which are the ones whose
/// *ownership* is at stake. A deferred body that reads a copy-typed binding
/// still sees its value at exit rather than at registration; that differs from
/// the design's "captures at registration" for those reads, but it cannot free
/// anything twice, and copying every integer in scope at every `defer` would
/// cost a load and a store per binding per registration — much more than the
/// rule is worth inside a loop.
#[derive(Clone)]
pub struct Deferred {
    /// The body, as written.
    pub body: HirBlock,
    /// `(name, slot, type)` for each captured binding, sorted by name so the
    /// emitted stores do not depend on `HashMap` iteration order.
    pub captured: Vec<(String, String, TypeKind)>,
}

/// Snapshot every linear binding in scope into a fresh slot.
///
/// The stores are emitted here, at the `defer` statement, so the snapshot is
/// taken when the `defer` is *reached* — which is what "captures at
/// registration" means. The slots themselves are allocas, so they are hoisted
/// into the entry block like every other slot.
fn capture_linear_locals(ctx: &mut FnCtx<'_>) -> Vec<(String, String, TypeKind)> {
    let mut names: Vec<String> = ctx
        .locals
        .iter()
        .filter(|(_, (slot, ty))| {
            !slot.is_empty()
                && repr_type(ty) != "void"
                && matches!(linearity_of(ty), Linearity::Linear)
        })
        .map(|(name, _)| name.clone())
        .collect();
    names.sort();

    let mut captured = Vec::new();
    for name in names {
        let current = match ctx.locals.get(&name) {
            Some((slot, ty)) => (slot.clone(), ty.clone()),
            None => continue,
        };
        let repr = repr_type(&current.1);
        let snapshot = ctx.alloc_slot();
        ctx.push_alloca(format!("{snapshot} = alloca {repr}"));
        let value = ctx.tmp();
        ctx.push(format!("{value} = load {repr}, ptr {}", current.0));
        ctx.push(format!("store {repr} {value}, ptr {snapshot}"));
        captured.push((name, snapshot, current.1));
    }
    captured
}

/// Lower every statement of `block` into the current basic block, stopping at
/// the first statement that terminates it.
pub fn gen_block(ctx: &mut FnCtx<'_>, block: &HirBlock) -> Flow {
    // Each scope owns its own defer list and its own string locals, so a
    // `defer` registers into the scope it is written in and only that scope's
    // defers run (and only that scope's strings are released) at its exit.
    ctx.defers.push(Vec::new());
    ctx.open_string_scope();
    for stmt in &block.stmts {
        let flow = gen_stmt(ctx, stmt);
        if flow != Flow::Fallthrough {
            // The terminating statement (return/break/continue) already ran
            // this scope's defers and released its string locals before
            // branching, so both lists are empty; popping keeps the stacks
            // honest for whatever scope is below.
            ctx.defers.pop();
            ctx.close_string_scope();
            return flow;
        }
    }
    // Fall-through: run this scope's defers -- which may read the string locals
    // -- then release them and close the scope. A caller that has just lowered a
    // nested block must not run *its own* defers here: this function already did
    // that for the block it was given, and doing it again from the `if` or loop
    // would move the body to the end of the branch or of one iteration.
    run_defers(ctx);
    ctx.defers.pop();
    ctx.close_string_scope_released();
    Flow::Fallthrough
}

/// Replace the field named by `steps` inside the aggregate `base` of type
/// `aggregate`, returning the new aggregate value.
///
/// A single step is one `insertvalue`. A nested chain (`p.inner.x = v`) is the
/// same shape repeated: `extractvalue` the sub-aggregate, rebuild it, then
/// `insertvalue` the rebuilt one back. Every type along the way comes from
/// `steps`, which the checker filled in from the declarations.
///
/// An empty chain cannot reach here -- the checker requires at least one field --
/// and returning `base` unchanged keeps the module loadable if it somehow does.
fn insert_field(
    ctx: &mut FnCtx<'_>,
    base: &str,
    aggregate: &TypeKind,
    steps: &[otter_sema::FieldStep],
    value: &GenValue,
) -> String {
    let repr = repr_type(aggregate);
    let Some(step) = steps.first() else {
        return base.to_string();
    };
    let field_repr = repr_type(&step.ty.kind);
    if steps.len() == 1 {
        let updated = ctx.tmp();
        ctx.push(format!(
            "{updated} = insertvalue {repr} {base}, {field_repr} {}, {}",
            value.value, step.index
        ));
        return updated;
    }
    let sub = ctx.tmp();
    ctx.push(format!("{sub} = extractvalue {repr} {base}, {}", step.index));
    let rebuilt = insert_field(ctx, &sub, &step.ty.kind, &steps[1..], value);
    let updated = ctx.tmp();
    ctx.push(format!(
        "{updated} = insertvalue {repr} {base}, {field_repr} {rebuilt}, {}",
        step.index
    ));
    updated
}

/// Emit the terminator for a `return` with value `v`.
///
/// Split out of the `return` statement so the string case can lower the value
/// early -- before the scope's locals are released -- and still share the
/// truncation, `void`, and `Never` rules.
fn emit_return(ctx: &mut FnCtx<'_>, v: &GenValue) {
    let repr = repr_type(&ctx.ret_type);
    if matches!(v.ty, TypeKind::Never) {
        // The expression never produces a value, so the block that would have
        // returned is unreachable. `unreachable` is the honest terminator for
        // it: `ret i64 ` with no operand is not valid IR.
        ctx.push("unreachable".to_string());
    } else if ctx.entry_point && repr == "i64" {
        // The entry point's signature is `i32`: truncate the Otter result to
        // the width the operating system reads.
        let narrowed = ctx.tmp();
        ctx.push(format!("{narrowed} = trunc i64 {} to i32", v.value));
        ctx.push(format!("ret i32 {narrowed}"));
    } else if repr == "void" {
        ctx.push("ret void");
    } else {
        ctx.push(format!("ret {repr} {}", v.value));
    }
}

/// Lower one statement.
pub fn gen_stmt(ctx: &mut FnCtx<'_>, stmt: &HirStmt) -> Flow {
    match stmt {
        HirStmt::Expr(expr, _, _) => {
            let value = gen_expr(ctx, expr);
            // An expression statement discards its value, so an owned string
            // the runtime just built has no other consumer and is released
            // here. A call kept for its side effect (`print_str(...)`) has an
            // empty value and owns nothing, and a name bound to a local is the
            // local's reference, so both are left alone.
            ctx.release_owned(&value.value);
            Flow::Fallthrough
        }
        HirStmt::Const { name, init, .. } | HirStmt::Var { name, init, .. } => {
            let ty = expr_ty(init).kind.clone();
            let slot = if matches!(ty, TypeKind::Str) {
                // A `string` binding owns a reference: the slot holds one for as
                // long as the scope lasts, and the scope releases it at exit.
                // The value is either taken over (a temporary the runtime built)
                // or kept (a literal, whose retain is a no-op; a local; a user
                // call's result) -- never both, so the reference is counted once.
                let value = gen_expr(ctx, init);
                let slot = ctx.alloc_slot();
                ctx.push_alloca(format!("{slot} = alloca i8*"));
                if !value.value.is_empty() {
                    if ctx.is_owned(&value.value) {
                        ctx.forget_owned(&value.value);
                    } else {
                        ctx.push(format!("call i8* @otter_str_retain(i8* {})", value.value));
                    }
                    ctx.push(format!("store i8* {}, ptr {slot}", value.value));
                    ctx.track_string_slot(&slot);
                }
                slot
            } else {
                gen_expr_to_slot(ctx, init)
            };
            ctx.locals.insert(name.clone(), (slot, ty));
            Flow::Fallthrough
        }
        HirStmt::Assign {
            name,
            value,
            ty,
            release_previous,
            ..
        } => {
            // The checker proved `name` is a `var` binding, so its slot is in
            // `locals`. There is no `?` to reach for here: this is the backend,
            // which never fails on user input, and a name with no slot would be
            // a compiler bug rather than a bad program. Skipping the store
            // leaves valid IR and a wrong value, which is the right failure
            // mode for a case that cannot happen -- the alternative is a panic,
            // which the project forbids.
            let slot = ctx.locals.get(name).map(|(slot, _)| slot.clone());
            let repr = repr_type(&ty.kind);
            let is_string = matches!(ty.kind, TypeKind::Str);
            // Overwriting a linear binding discards the pointer that was there,
            // so it is released first. `release_previous` is the checker's
            // verdict on whether that pointer was still owned: after
            // `dealloc(buf)` the slot still holds the address, but the address
            // was already freed, so freeing it again here would be a double
            // free. Only the checker knows the difference, which is why the
            // flag is in the HIR rather than recomputed from the slot. A
            // `string` is a copy type with its own rule below, not a linear one.
            if *release_previous && repr != "void" && !is_string {
                if let Some(slot) = &slot {
                    let old = ctx.tmp();
                    ctx.push(format!("{old} = load {repr}, ptr {slot}"));
                    ctx.push(format!("call void @otter_free(i8* {old})"));
                }
            }
            let value = gen_expr(ctx, value);
            if is_string && !value.value.is_empty() {
                // The new reference is secured *before* the old one is dropped,
                // so `s = s + "x"` reads the old value and the store that
                // follows cannot leave the slot pointing at freed bytes.
                if ctx.is_owned(&value.value) {
                    ctx.forget_owned(&value.value);
                } else {
                    ctx.push(format!("call i8* @otter_str_retain(i8* {})", value.value));
                }
                // Dropping the old value is only right when this slot owns it.
                // A parameter's reference is the caller's, so it is left alone.
                if let Some(slot) = &slot {
                    if ctx.is_tracked_string_slot(slot) {
                        let old = ctx.tmp();
                        ctx.push(format!("{old} = load i8*, ptr {slot}"));
                        ctx.push(format!("call void @otter_str_release(i8* {old})"));
                    }
                }
            }
            match slot {
                Some(slot) if repr != "void" => {
                    ctx.push(format!("store {repr} {}, ptr {slot}", value.value));
                }
                Some(_) => {}
                None => ctx.push(format!(
                    "; note: no slot for `{name}`, which the checker said was assignable"
                )),
            }
            Flow::Fallthrough
        }
        HirStmt::IndexAssign { base, index, value, .. } => {
            // `p[i] = v`: getelementptr to the element's address, then store.
            // The checker proved `base` is a `*mut i64` and both `index` and
            // `value` are `i64`, so the types are known and correct here.
            let ptr = gen_expr(ctx, base);
            let idx = gen_expr(ctx, index);
            let val = gen_expr(ctx, value);
            let addr = ctx.tmp();
            ctx.push(format!(
                "{addr} = getelementptr i64, ptr {}, i64 {}",
                ptr.value, idx.value
            ));
            ctx.push(format!("store i64 {}, ptr {addr}", val.value));
            Flow::Fallthrough
        }
        HirStmt::FieldAssign {
            root, steps, value, ..
        } => {
            // `p.x = v`: a struct value is an aggregate, not an address, so this
            // is a read-modify-write -- load the binding's aggregate, replace the
            // field with `insertvalue` (rebuilding the enclosing aggregates on
            // the way out of a nested chain), and store the result back. The
            // checker resolved `steps`, so no declaration is looked up twice.
            //
            // Nothing is released or retained here, on purpose: a struct's field
            // references are not tracked, and `P { name: s }` stores `s`'s
            // pointer *without* a retain, so dropping the old field would drop a
            // reference the aggregate may not own. That is a leak, which is the
            // failure mode chosen wherever ownership cannot be proved.
            let value = gen_expr(ctx, value);
            let slot = ctx.locals.get(root).cloned();
            let Some((slot, root_ty)) = slot else {
                ctx.push(format!(
                    "; note: no slot for `{root}`, which the checker said owned the field"
                ));
                return Flow::Fallthrough;
            };
            let repr = repr_type(&root_ty);
            let loaded = ctx.tmp();
            ctx.push(format!("{loaded} = load {repr}, ptr {slot}"));
            let updated = insert_field(ctx, &loaded, &root_ty, steps, &value);
            ctx.push(format!("store {repr} {updated}, ptr {slot}"));
            Flow::Fallthrough
        }
        HirStmt::Return(value, _) => {
            // A returned `string` is taken *before* the scope's locals are
            // released: the block is leaving, and a local the `return` names is
            // still in its slot. Only for a string return type, so every other
            // function's semantics (and its IR) are unchanged.
            let taken = match (value, matches!(ctx.ret_type, TypeKind::Str)) {
                (Some(expr), true) => Some(gen_expr(ctx, expr)),
                _ => None,
            };
            // Defers for this scope run before the block is terminated, so
            // they execute on the way out, not when the `defer` was reached --
            // and before the string locals below are released, so a deferred
            // body still sees them.
            run_defers(ctx);
            if let Some(taken) = &taken {
                // The returned reference has to survive the release: a local's
                // value is kept here and given to the caller, while a
                // temporary's reference is transferred as-is.
                if !ctx.is_owned(&taken.value) && !taken.value.is_empty() {
                    ctx.push(format!(
                        "call i8* @otter_str_retain(i8* {})",
                        taken.value
                    ));
                }
            }
            ctx.release_current_string_scope();
            match (value, taken) {
                (Some(_), Some(v)) => emit_return(ctx, &v),
                (Some(expr), None) => {
                    let v = gen_expr(ctx, expr);
                    emit_return(ctx, &v);
                }
                (None, _) => {
                    let terminator = ctx.unit_terminator.clone();
                    ctx.push(terminator);
                }
            }
            Flow::Return
        }
        HirStmt::If {
            init,
            cond,
            then_block,
            else_block,
            ..
        } => gen_if(
            ctx,
            init.as_deref(),
            cond,
            then_block,
            else_block.as_ref(),
        ),
        HirStmt::ForRange {
            name, start, end, body, ..
        } => gen_for_range(ctx, name, start, end, body),
        HirStmt::ForIn {
            name,
            iterable,
            body,
            ..
        } => gen_for_in(ctx, name, iterable, body),
        HirStmt::ForC {
            init,
            cond,
            update,
            body,
            ..
        } => gen_for_c(
            ctx,
            init.as_deref(),
            cond.as_ref(),
            update.as_ref(),
            body,
        ),
        HirStmt::Switch { subject, arms, .. } => {
            let scrutinee = gen_expr(ctx, subject);
            let _ = gen_match(ctx, &scrutinee, arms, &TypeKind::Unit);
            Flow::Fallthrough
        }
        HirStmt::Break(_) => {
            run_defers(ctx);
            ctx.release_current_string_scope();
            match ctx.loops.last().cloned() {
                Some((break_label, _)) => {
                    ctx.push(format!("br label %{break_label}"));
                    Flow::Break
                }
                None => {
                    ctx.push("; `break` outside a loop; the type checker rejected this");
                    Flow::Fallthrough
                }
            }
        }
        HirStmt::Continue(_) => {
            run_defers(ctx);
            ctx.release_current_string_scope();
            match ctx.loops.last().cloned() {
                Some((_, continue_label)) => {
                    ctx.push(format!("br label %{continue_label}"));
                    Flow::Continue
                }
                None => {
                    ctx.push("; `continue` outside a loop; the type checker rejected this");
                    Flow::Fallthrough
                }
            }
        }
        HirStmt::Defer { body, .. } => {
            // The capture happens *here*, not when the body runs: that is what
            // makes the deferred body see the ownership state the checker
            // approved. See [`Deferred`].
            let captured = capture_linear_locals(ctx);
            let defers = ctx.defers.last_mut().expect("a scope is always active");
            defers.push(Deferred {
                body: body.clone(),
                captured,
            });
            Flow::Fallthrough
        }
    }
}

/// Lower `if [init;] cond { then } else { else }`.
fn gen_if(
    ctx: &mut FnCtx<'_>,
    init: Option<&HirStmt>,
    cond: &HirExpr,
    then_block: &HirBlock,
    else_block: Option<&HirBlock>,
) -> Flow {
    if let Some(init) = init {
        let flow = gen_stmt(ctx, init);
        if flow != Flow::Fallthrough {
            return flow;
        }
    }
    let condition = gen_expr(ctx, cond);
    let then_label = ctx.alloc_label_base() + "_then";
    let else_label = ctx.alloc_label_base() + "_else";
    let merge_label = ctx.alloc_label_base() + "_merge";
    ctx.push(format!(
        "br i1 {}, label %{then_label}, label %{else_label}",
        condition.value
    ));

    ctx.push(format!("{then_label}:"));
    if gen_block(ctx, then_block) == Flow::Fallthrough {
        ctx.push(format!("br label %{merge_label}"));
    }

    ctx.push(format!("{else_label}:"));
    match else_block {
        Some(block) => {
            if gen_block(ctx, block) == Flow::Fallthrough {
                run_defers(ctx);
                ctx.push(format!("br label %{merge_label}"));
            }
        }
        None => ctx.push(format!("br label %{merge_label}")),
    }

    ctx.push(format!("{merge_label}:"));
    Flow::Fallthrough
}

/// Lower `for name in start..end { body }`. The range is half-open, so the
/// loop variable runs `start`, `start+1`, …, `end-1`.
fn gen_for_range(
    ctx: &mut FnCtx<'_>,
    name: &str,
    start: &HirExpr,
    end: &HirExpr,
    body: &HirBlock,
) -> Flow {
    let start_value = gen_expr(ctx, start);
    let end_value = gen_expr(ctx, end);

    let end_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{end_slot} = alloca i64"));
    ctx.push(format!("store i64 {}, ptr {end_slot}", end_value.value));
    let index_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{index_slot} = alloca i64"));
    ctx.push(format!("store i64 {}, ptr {index_slot}", start_value.value));

    let cond_label = ctx.alloc_label_base() + "_cond";
    let body_label = ctx.alloc_label_base() + "_body";
    let inc_label = ctx.alloc_label_base() + "_inc";
    let done_label = ctx.alloc_label_base() + "_done";

    ctx.push(format!("br label %{cond_label}"));
    ctx.push(format!("{cond_label}:"));
    let current = ctx.tmp();
    ctx.push(format!("{current} = load i64, ptr {index_slot}"));
    let limit = ctx.tmp();
    ctx.push(format!("{limit} = load i64, ptr {end_slot}"));
    let in_range = ctx.tmp();
    ctx.push(format!("{in_range} = icmp slt i64 {current}, {limit}"));
    ctx.push(format!(
        "br i1 {in_range}, label %{body_label}, label %{done_label}"
    ));

    ctx.push(format!("{body_label}:"));
    let shadowed = ctx
        .locals
        .insert(name.to_string(), (index_slot.clone(), TypeKind::I64));
    ctx.loops.push((done_label.clone(), inc_label.clone()));
    let flow = gen_block(ctx, body);
    ctx.loops.pop();
    match shadowed {
        Some(previous) => {
            ctx.locals.insert(name.to_string(), previous);
        }
        None => {
            ctx.locals.remove(name);
        }
    }
    if flow == Flow::Fallthrough {
        ctx.push(format!("br label %{inc_label}"));
    }

    ctx.push(format!("{inc_label}:"));
    let previous = ctx.tmp();
    ctx.push(format!("{previous} = load i64, ptr {index_slot}"));
    let next = ctx.tmp();
    ctx.push(format!("{next} = add i64 {previous}, 1"));
    ctx.push(format!("store i64 {next}, ptr {index_slot}"));
    ctx.push(format!("br label %{cond_label}"));

    ctx.push(format!("{done_label}:"));
    Flow::Fallthrough
}

/// Lower `for x in slice { .. }`.
///
/// The iterable is a binding (the checker requires it), so the loop can read the
/// header through the binding's slot on every iteration. That is what makes a
/// `push` in the body visible: a grow updates `len`, `cap` and `data` in that
/// slot, and caching any of them before the loop would either stop early or
/// read freed memory.
///
/// The element is copied into a fresh slot that the body sees as `name`. Element
/// types the checker admits are numbers and `bool`, so the copy owns nothing and
/// needs no retain/release at either end of the iteration.
fn gen_for_in(ctx: &mut FnCtx<'_>, name: &str, iterable: &HirExpr, body: &HirBlock) -> Flow {
    let HirExpr::Ident(path, _, _) = iterable else {
        return Flow::Fallthrough;
    };
    let Some(binding) = path.last() else {
        return Flow::Fallthrough;
    };
    let Some((slice_slot, slice_ty)) = ctx.locals.get(binding).cloned() else {
        return Flow::Fallthrough;
    };
    let TypeKind::Slice(element) = &slice_ty else {
        return Flow::Fallthrough;
    };
    let element_kind = element.kind.clone();
    let element_repr = repr_type(&element_kind);
    let slice_repr = repr_type(&slice_ty);

    let index_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{index_slot} = alloca i64"));
    ctx.push(format!("store i64 0, ptr {index_slot}"));

    let cond_label = ctx.alloc_label_base() + "_cond";
    let body_label = ctx.alloc_label_base() + "_body";
    let inc_label = ctx.alloc_label_base() + "_inc";
    let done_label = ctx.alloc_label_base() + "_done";

    ctx.push(format!("br label %{cond_label}"));
    ctx.push(format!("{cond_label}:"));
    let len_ptr = ctx.tmp();
    ctx.push(format!(
        "{len_ptr} = getelementptr {slice_repr}, ptr {slice_slot}, i32 0, i32 1"
    ));
    let len = ctx.tmp();
    ctx.push(format!("{len} = load i64, ptr {len_ptr}"));
    let current = ctx.tmp();
    ctx.push(format!("{current} = load i64, ptr {index_slot}"));
    let in_range = ctx.tmp();
    ctx.push(format!("{in_range} = icmp slt i64 {current}, {len}"));
    ctx.push(format!(
        "br i1 {in_range}, label %{body_label}, label %{done_label}"
    ));

    ctx.push(format!("{body_label}:"));
    let data_ptr = ctx.tmp();
    ctx.push(format!(
        "{data_ptr} = getelementptr {slice_repr}, ptr {slice_slot}, i32 0, i32 0"
    ));
    let data = ctx.tmp();
    ctx.push(format!("{data} = load i8*, ptr {data_ptr}"));
    let position = ctx.tmp();
    ctx.push(format!("{position} = load i64, ptr {index_slot}"));
    let element_ptr = ctx.tmp();
    ctx.push(format!(
        "{element_ptr} = getelementptr {element_repr}, ptr {data}, i64 {position}"
    ));
    let value = ctx.tmp();
    ctx.push(format!("{value} = load {element_repr}, ptr {element_ptr}"));
    let element_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{element_slot} = alloca {element_repr}"));
    ctx.push(format!(
        "store {element_repr} {value}, ptr {element_slot}"
    ));
    let shadowed = ctx
        .locals
        .insert(name.to_string(), (element_slot, element_kind));
    ctx.loops.push((done_label.clone(), inc_label.clone()));
    let flow = gen_block(ctx, body);
    ctx.loops.pop();
    match shadowed {
        Some(previous) => {
            ctx.locals.insert(name.to_string(), previous);
        }
        None => {
            ctx.locals.remove(name);
        }
    }
    if flow == Flow::Fallthrough {
        ctx.push(format!("br label %{inc_label}"));
    }

    ctx.push(format!("{inc_label}:"));
    let previous = ctx.tmp();
    ctx.push(format!("{previous} = load i64, ptr {index_slot}"));
    let next = ctx.tmp();
    ctx.push(format!("{next} = add i64 {previous}, 1"));
    ctx.push(format!("store i64 {next}, ptr {index_slot}"));
    ctx.push(format!("br label %{cond_label}"));

    ctx.push(format!("{done_label}:"));
    Flow::Fallthrough
}

/// Lower `for [init;] [cond;] [update] { body }` — the C-style loop. A missed
/// condition is the constant `true`.
fn gen_for_c(
    ctx: &mut FnCtx<'_>,
    init: Option<&HirStmt>,
    cond: Option<&HirExpr>,
    update: Option<&HirExpr>,
    body: &HirBlock,
) -> Flow {
    if let Some(init) = init {
        let flow = gen_stmt(ctx, init);
        if flow != Flow::Fallthrough {
            return flow;
        }
    }

    let cond_label = ctx.alloc_label_base() + "_cond";
    let body_label = ctx.alloc_label_base() + "_body";
    let inc_label = ctx.alloc_label_base() + "_inc";
    let done_label = ctx.alloc_label_base() + "_done";

    ctx.push(format!("br label %{cond_label}"));
    ctx.push(format!("{cond_label}:"));
    match cond {
        Some(condition) => {
            let value = gen_expr(ctx, condition);
            ctx.push(format!(
                "br i1 {}, label %{body_label}, label %{done_label}",
                value.value
            ));
        }
        None => ctx.push(format!("br label %{body_label}")),
    }

    ctx.push(format!("{body_label}:"));
    ctx.loops.push((done_label.clone(), inc_label.clone()));
    let flow = gen_block(ctx, body);
    ctx.loops.pop();
    if flow == Flow::Fallthrough {
        ctx.push(format!("br label %{inc_label}"));
    }

    ctx.push(format!("{inc_label}:"));
    if let Some(update) = update {
        let _ = gen_expr(ctx, update);
    }
    ctx.push(format!("br label %{cond_label}"));

    ctx.push(format!("{done_label}:"));
    Flow::Fallthrough
}

/// Lower every pending deferred body, last registered first, plus any bodies
/// those bodies themselves defer.
///
/// Each body runs with the bindings it captured installed in `FnCtx::locals`, so
/// a name in a deferred body resolves to the value the binding held when the
/// `defer` was reached rather than to whatever is in the slot at exit. The
/// previous entries are restored afterwards, so the code *after* a `defer`
/// statement still sees the real binding — only the deferred body reads the
/// snapshot.
pub fn run_defers(ctx: &mut FnCtx<'_>) {
    let Some(defers) = ctx.defers.last_mut() else {
        return;
    };
    let pending = std::mem::take(defers);
    lower_defers(ctx, pending);
}

/// Run every pending deferred body in every active scope, innermost first.
/// Used on the panic/exit/fault path, where no scope exits normally and every
/// defer that was registered must still fire.
///
/// Unlike [`run_defers`], this does **not** consume [`crate::FnCtx::defers`].
/// The fault block it emits into is a *second* path through code that is still
/// being lowered, so the scope stack must survive the call: a `defer` written
/// after a checked operation has to register into the scope that is still open.
/// The cost is that a fault site in a scope with pending defers also emits those
/// bodies into its cold block, which runs only when the process is already
/// ending.
///
/// Re-entry is refused. While a deferred body is being lowered, a fault *inside*
/// it sets [`crate::FnCtx::lowering_defers`], so it does not try to emit the same
/// bodies again — which would not terminate. Its fault reaches the runtime
/// directly, the same "a `defer` that panics during unwinding aborts" rule the
/// design states.
pub fn run_all_scopes_defers(ctx: &mut FnCtx<'_>) {
    if ctx.lowering_defers {
        return;
    }
    let levels: Vec<Vec<Deferred>> = ctx.defers.iter().rev().cloned().collect();
    for pending in levels {
        lower_defers(ctx, pending);
    }
}

/// Lower `pending` (already taken out of whichever scope owns it), last
/// registered first, plus any bodies those bodies themselves defer.
///
/// Split out of [`run_defers`] so the fault path can lower a *clone* of a
/// scope's list while leaving the compile-time stack untouched.
fn lower_defers(ctx: &mut FnCtx<'_>, mut pending: Vec<Deferred>) {
    while let Some(deferred) = pending.pop() {
        // Install the captured slots for the duration of this body.
        let mut restore: Vec<(String, Option<(String, TypeKind)>)> = Vec::new();
        for (name, slot, ty) in &deferred.captured {
            let previous = ctx
                .locals
                .insert(name.clone(), (slot.clone(), ty.clone()));
            restore.push((name.clone(), previous));
        }

        // A fault in this body must not re-enter the fault lowering; see
        // [`run_all_scopes_defers`].
        let was_lowering = ctx.lowering_defers;
        ctx.lowering_defers = true;
        ctx.defers.push(Vec::new());
        let _ = gen_block(ctx, &deferred.body);
        run_defers(ctx);
        ctx.defers.pop();
        ctx.lowering_defers = was_lowering;

        for (name, previous) in restore {
            match previous {
                Some(entry) => {
                    ctx.locals.insert(name, entry);
                }
                None => {
                    ctx.locals.remove(&name);
                }
            }
        }
    }
}
