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

use yashy_sema::{
    expr_ty, linearity_of, BinOp, HirBlock, HirExpr, HirStmt, Linearity, Type, TypeKind,
};

use crate::expr_gen::retain_struct_fields;
use crate::expr_gen::{gen_expr, gen_expr_to_slot, FnCtx, GenValue, LoopTarget, RefKind};
use crate::ir::repr_type;
use crate::patterns::gen_match;

/// Whether an expression reads a value that already exists.
///
/// A name or a field of one is a *copy* of what it names, and a copy of a
/// struct that owns references needs its own. A literal or a call's result is
/// a fresh value, and brings the references it already has.
fn is_copy_of_a_value(expr: &HirExpr) -> bool {
    matches!(expr, HirExpr::Ident(..) | HirExpr::FieldAccess { .. } | HirExpr::Index { .. })
}

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
/// let buf = alloc(16);
/// defer { buf.free(); }     // the checker sees `buf` live here
/// buf = alloc(32);          // ... and does not free the first block here,
/// buf.free();               // because the deferred `free` already moved it
///                           // at registration — so the deferred body frees the
///                           // same pointer a second time at exit.
/// ```
///
/// With the capture, the deferred `free` releases the block that was live at
/// registration (the one still owned), and the body's own `free` releases the
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
    ctx.open_ref_scope();
    for stmt in &block.stmts {
        let flow = gen_stmt(ctx, stmt);
        if flow != Flow::Fallthrough {
            // The terminating statement (return/break/continue) already ran
            // this scope's defers and released its string locals before
            // branching, so both lists are empty; popping keeps the stacks
            // honest for whatever scope is below.
            ctx.defers.pop();
            ctx.close_ref_scope();
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
    ctx.close_ref_scope_released();
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
    steps: &[yashy_sema::FieldStep],
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
        // The entry point's signature is `i32`: truncate the Yashy result to
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
            // here. A call kept for its side effect (`print(...)`) has an
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
                        ctx.push_str_retain(&value.value);
                    }
                    ctx.push(format!("store i8* {}, ptr {slot}", value.value));
                    ctx.track_ref_slot(&slot, RefKind::Str);
                }
                slot
            } else if matches!(ty, TypeKind::Func(_, _)) {
                // A closure binding owns a reference to its environment, the
                // same way a `string` binding owns a reference to its bytes.
                // The value is either taken over (a closure the program just
                // made, which owns the only reference) or kept (a copy of
                // another binding, which has to be counted again).
                let value = gen_expr(ctx, init);
                let slot = ctx.alloc_slot();
                ctx.push_alloca(format!("{slot} = alloca {{ i8*, i8* }}"));
                if !value.value.is_empty() {
                    if ctx.is_owned(&value.value) {
                        ctx.forget_owned(&value.value);
                    } else {
                        ctx.retain_env(&value.value);
                    }
                    ctx.push(format!(
                        "store {{ i8*, i8* }} {}, ptr {slot}",
                        value.value
                    ));
                    ctx.track_ref_slot(&slot, RefKind::Env);
                }
                slot
            } else if let TypeKind::Struct(_) = ty {
                // A struct binding owns the references its `string` fields
                // hold. A value that is a *copy* of another struct's needs
                // its own references; a fresh one -- a literal, a call's
                // result -- brings the references it already has.
                let value = gen_expr(ctx, init);
                if is_copy_of_a_value(init) {
                    retain_struct_fields(ctx, &value.value, &ty);
                }
                let repr = repr_type(&ty);
                let slot = ctx.alloc_slot();
                ctx.push_alloca(format!("{slot} = alloca {repr}"));
                ctx.push(format!("store {repr} {}, ptr {slot}", value.value));
                ctx.track_ref_slot_typed(&slot, RefKind::StructFields, Some(ty.clone()));
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
            // The checker proved `name` is a `let` binding, so its slot is in
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
                    ctx.push(format!("call void @yashy_free(i8* {old})"));
                }
            }
            // Fast path for `s = s + piece`: append in place instead of
            // allocating a fresh string and releasing the old one. The slot
            // owns its reference, so `yashy_str_append` either mutates it or
            // takes over the reference and releases the old bytes itself.
            if is_string {
                if let Some(slot) = &slot {
                    // Only a slot the scope owns may be mutated in place. A
                    // parameter's reference belongs to the caller, and its slot
                    // is not tracked, so an assignment to a parameter keeps the
                    // general path (which builds a new string).
                    if ctx.is_tracked_ref_slot(slot) {
                        if let HirExpr::Binary {
                            op: BinOp::Add,
                            lhs,
                            rhs,
                            ..
                        } = value
                        {
                            if let HirExpr::Ident(path, _, _) = &**lhs {
                                if path.last().map(String::as_str) == Some(name.as_str()) {
                                    let old = ctx.tmp();
                                    ctx.push(format!("{old} = load i8*, ptr {slot}"));
                                    let piece = gen_expr(ctx, rhs);
                                    let appended = ctx.tmp();
                                    ctx.push(format!(
                                        "{appended} = call i8* @yashy_str_append(i8* {old}, i8* {})",
                                        piece.value
                                    ));
                                    ctx.release_owned(&piece.value);
                                    ctx.push(format!("store i8* {appended}, ptr {slot}"));
                                    return Flow::Fallthrough;
                                }
                            }
                        }
                    }
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
                    ctx.push_str_retain(&value.value);
                }
                // Dropping the old value is only right when this slot owns it.
                // A parameter's reference is the caller's, so it is left alone.
                if let Some(slot) = &slot {
                    if ctx.is_tracked_ref_slot(slot) {
                        let old = ctx.tmp();
                        ctx.push(format!("{old} = load i8*, ptr {slot}"));
                        ctx.push_str_release(&old);
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
        HirStmt::Destructure {
            init, bindings, ..
        } => {
            // The tuple value is an aggregate, so each element is one
            // `extractvalue` -- no address, no copy of the whole, and nothing
            // to free: the checker only admits elements that own nothing.
            let value = gen_expr(ctx, init);
            let repr = repr_type(&value.ty);
            for (index, binding) in bindings.iter().enumerate() {
                let Some((name, ty)) = binding else { continue };
                let element_repr = repr_type(&ty.kind);
                let slot = ctx.alloc_slot();
                ctx.push_alloca(format!("{slot} = alloca {element_repr}"));
                let element = ctx.tmp();
                ctx.push(format!(
                    "{element} = extractvalue {repr} {}, {index}",
                    value.value
                ));
                if element_repr != "void" {
                    ctx.push(format!("store {element_repr} {element}, ptr {slot}"));
                }
                ctx.locals.insert(name.clone(), (slot, ty.kind.clone()));
            }
            Flow::Fallthrough
        }
        HirStmt::IndexAssign {
            base,
            index,
            value,
            span,
        } => {
            // `p[i] = v`: getelementptr to the element's address, then store.
            // The element is whatever the pointer points at -- the checker
            // proved the value fits it -- so the width comes from the
            // representation rather than from a fixed `i64`.
            let ptr = gen_expr(ctx, base);
            let idx = gen_expr(ctx, index);
            let val = gen_expr(ctx, value);
            let element = match &ptr.ty {
                TypeKind::Ptr(inner) => (**inner).clone(),
                other => Type {
                    kind: other.clone(),
                    span: *span,
                },
            };
            let repr = repr_type(&element.kind);
            let addr = ctx.tmp();
            ctx.push(format!(
                "{addr} = getelementptr {repr}, ptr {}, i64 {}",
                ptr.value, idx.value
            ));
            ctx.push(format!("store {repr} {}, ptr {addr}", val.value));
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
        HirStmt::FieldPtrAssign {
            pointer,
            index,
            steps,
            value,
            ty,
            ..
        } => {
            // `p[i].field = v`: the pointer is an address, so the write is a
            // store through it -- take the element's address, step to the
            // field's offset, store. No load, no rebuild: a pointer is not an
            // aggregate that has to be copied back.
            let ptr = gen_expr(ctx, pointer);
            let idx = gen_expr(ctx, index);
            let element = match &ptr.ty {
                TypeKind::Ptr(inner) => (**inner).clone(),
                _ => ty.clone(),
            };
            let element_repr = repr_type(&element.kind);
            let mut addr = ctx.tmp();
            ctx.push(format!(
                "{addr} = getelementptr {element_repr}, ptr {}, i64 {}",
                ptr.value, idx.value
            ));
            let mut current = element;
            for step in steps {
                let repr = repr_type(&current.kind);
                let next = ctx.tmp();
                ctx.push(format!(
                    "{next} = getelementptr {repr}, ptr {addr}, i32 0, i32 {}",
                    step.index
                ));
                addr = next;
                current = step.ty.clone();
            }
            let val = gen_expr(ctx, value);
            ctx.push(format!(
                "store {} {}, ptr {addr}",
                repr_type(&current.kind),
                val.value
            ));
            Flow::Fallthrough
        }
        HirStmt::Return(value, _) => {
            // A returned `string` is taken *before* the scope's locals are
            // released: the block is leaving, and a local the `return` names is
            // still in its slot. Only for a string return type, so every other
            // function's semantics (and its IR) are unchanged.
            //
            // A returned closure is taken for the same reason, and for one
            // more: the closure's environment must survive the scope's
            // release, or the value the caller gets would point at a block
            // that is already gone.
            let returned_kind = match ctx.ret_type {
                TypeKind::Str => Some(RefKind::Str),
                TypeKind::Func(_, _) => Some(RefKind::Env),
                TypeKind::Struct(_) => Some(RefKind::StructFields),
                _ => None,
            };
            let taken = match (value, returned_kind) {
                (Some(expr), Some(_)) => Some(gen_expr(ctx, expr)),
                _ => None,
            };
            // Defers for every scope between this `return` and the function
            // body run before the block is terminated, innermost first, so they
            // execute on the way out, not when the `defer` was reached -- and
            // before the string locals below are released, so a deferred body
            // still sees them. A `return` inside an `if` leaves the enclosing
            // scope too; running only the innermost one leaked its bindings.
            run_all_scopes_defers(ctx);
            if let (Some(taken), Some(kind)) = (&taken, returned_kind) {
                // The returned reference has to survive the release: a local's
                // value is kept here and given to the caller, while a
                // temporary's reference is transferred as-is.
                if !ctx.is_owned(&taken.value) && !taken.value.is_empty() {
                    match kind {
                        RefKind::Str => ctx.push_str_retain(&taken.value),
                        RefKind::Env => ctx.retain_env(&taken.value),
                        RefKind::StructFields => {
                            let ret_type = ctx.ret_type.clone();
                            retain_struct_fields(ctx, &taken.value, &ret_type);
                        }
                    }
                }
            }
            ctx.release_all_ref_scopes();
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
        HirStmt::Break(_) => match ctx.loops.last().cloned() {
            Some(target) => {
                // A `break` nested in a block leaves the block *and* the loop
                // body. Release the body's scopes, and run its defers, before
                // branching; the enclosing scopes stay for the code after the
                // loop.
                run_defers_from(ctx, target.defer_base);
                ctx.release_ref_scopes_from(target.scope_base);
                ctx.push(format!("br label %{}", target.break_label));
                Flow::Break
            }
            None => {
                run_defers(ctx);
                ctx.release_current_ref_scope();
                ctx.push("; `break` outside a loop; the type checker rejected this");
                Flow::Fallthrough
            }
        },
        HirStmt::Continue(_) => match ctx.loops.last().cloned() {
            Some(target) => {
                // The body's scopes are released before the jump to the
                // increment, so the next iteration starts clean.
                run_defers_from(ctx, target.defer_base);
                ctx.release_ref_scopes_from(target.scope_base);
                ctx.push(format!("br label %{}", target.continue_label));
                Flow::Continue
            }
            None => {
                run_defers(ctx);
                ctx.release_current_ref_scope();
                ctx.push("; `continue` outside a loop; the type checker rejected this");
                Flow::Fallthrough
            }
        },
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
        HirStmt::LoopInvariant(_) => {
            // An invariant is a promise to the verifier, not work the program
            // does. It has no lowering: the compiler spends it, it does not run
            // it.
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
    let (scope_base, defer_base) = ctx.unwind_depths();
    ctx.loops.push(LoopTarget {
        break_label: done_label.clone(),
        continue_label: inc_label.clone(),
        scope_base,
        defer_base,
    });
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
    let Some((slot, iterable_ty)) = ctx.locals.get(binding).cloned() else {
        return Flow::Fallthrough;
    };
    // All three array forms iterate the same way -- walk a pointer and a
    // length -- but they must not read those two words at the same time. A
    // slice's header is *live*: a `push` inside the loop grows the backing
    // array, and the loop has to see the new length and pointer, so both are
    // re-read on every iteration. A view behaves the same way, because a `let`
    // view can be rebound. A fixed array cannot change at all: its length is a
    // constant in its type and its elements are the binding's own slot, so
    // those two are computed once, before the loop.
    let element_kind = match &iterable_ty {
        TypeKind::Slice(element) | TypeKind::View(element) => element.kind.clone(),
        TypeKind::FixedArray(element, _) => element.kind.clone(),
        _ => return Flow::Fallthrough,
    };
    let repr = repr_type(&iterable_ty);
    let fixed_data = match &iterable_ty {
        TypeKind::FixedArray(_, _) => {
            let data = ctx.tmp();
            ctx.push(format!(
                "{data} = getelementptr {repr}, ptr {slot}, i64 0, i64 0"
            ));
            Some(data)
        }
        _ => None,
    };
    let fixed_len = match &iterable_ty {
        TypeKind::FixedArray(_, length) => {
            Some(i64::try_from(*length).unwrap_or(i64::MAX).to_string())
        }
        _ => None,
    };
    let element_repr = repr_type(&element_kind);

    let index_slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{index_slot} = alloca i64"));
    ctx.push(format!("store i64 0, ptr {index_slot}"));

    let cond_label = ctx.alloc_label_base() + "_cond";
    let body_label = ctx.alloc_label_base() + "_body";
    let inc_label = ctx.alloc_label_base() + "_inc";
    let done_label = ctx.alloc_label_base() + "_done";

    ctx.push(format!("br label %{cond_label}"));
    ctx.push(format!("{cond_label}:"));
    let len = match &fixed_len {
        Some(len) => len.clone(),
        None => {
            let len_ptr = ctx.tmp();
            ctx.push(format!(
                "{len_ptr} = getelementptr {repr}, ptr {slot}, i32 0, i32 1"
            ));
            let len = ctx.tmp();
            ctx.push(format!("{len} = load i64, ptr {len_ptr}"));
            len
        }
    };
    let current = ctx.tmp();
    ctx.push(format!("{current} = load i64, ptr {index_slot}"));
    let in_range = ctx.tmp();
    ctx.push(format!("{in_range} = icmp slt i64 {current}, {len}"));
    ctx.push(format!(
        "br i1 {in_range}, label %{body_label}, label %{done_label}"
    ));

    ctx.push(format!("{body_label}:"));
    let data = match &fixed_data {
        Some(data) => data.clone(),
        None => {
            let data_ptr = ctx.tmp();
            ctx.push(format!(
                "{data_ptr} = getelementptr {repr}, ptr {slot}, i32 0, i32 0"
            ));
            let data = ctx.tmp();
            ctx.push(format!("{data} = load i8*, ptr {data_ptr}"));
            data
        }
    };
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
    let (scope_base, defer_base) = ctx.unwind_depths();
    ctx.loops.push(LoopTarget {
        break_label: done_label.clone(),
        continue_label: inc_label.clone(),
        scope_base,
        defer_base,
    });
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
    let (scope_base, defer_base) = ctx.unwind_depths();
    ctx.loops.push(LoopTarget {
        break_label: done_label.clone(),
        continue_label: inc_label.clone(),
        scope_base,
        defer_base,
    });
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

/// Run every pending deferred body in the scopes at or above `base`, innermost
/// first, without consuming the stacks.
///
/// `base` is the depth recorded when a loop body was entered. A
/// `break`/`continue` written in a block nested inside the body leaves the body
/// and the nested blocks, but not the scopes around the loop, so only those
/// levels fire. Like [`run_all_scopes_defers`], the lists survive because the
/// blocks that own them still pop their own entries as the flow propagates.
fn run_defers_from(ctx: &mut FnCtx<'_>, base: usize) {
    if ctx.lowering_defers || base >= ctx.defers.len() {
        return;
    }
    let levels: Vec<Vec<Deferred>> = ctx.defers[base..].iter().rev().cloned().collect();
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
