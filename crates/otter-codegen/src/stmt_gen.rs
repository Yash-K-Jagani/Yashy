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
//! # No input makes this module panic
//!
//! Every lookup is total; a `break` outside a loop, or a missing loop label,
//! lowers to a comment rather than an abort.

use otter_sema::{expr_ty, HirBlock, HirExpr, HirStmt, TypeKind};

use crate::expr_gen::{gen_expr, gen_expr_to_slot, FnCtx};
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

/// Lower every statement of `block` into the current basic block, stopping at
/// the first statement that terminates it.
pub fn gen_block(ctx: &mut FnCtx<'_>, block: &HirBlock, defers: &mut Vec<HirBlock>) -> Flow {
    for stmt in &block.stmts {
        let flow = gen_stmt(ctx, stmt, defers);
        if flow != Flow::Fallthrough {
            return flow;
        }
    }
    Flow::Fallthrough
}

/// Lower one statement.
pub fn gen_stmt(ctx: &mut FnCtx<'_>, stmt: &HirStmt, defers: &mut Vec<HirBlock>) -> Flow {
    match stmt {
        HirStmt::Expr(expr, _, _) => {
            let _ = gen_expr(ctx, expr);
            Flow::Fallthrough
        }
        HirStmt::Const { name, init, .. } | HirStmt::Var { name, init, .. } => {
            let slot = gen_expr_to_slot(ctx, init);
            ctx.locals
                .insert(name.clone(), (slot, expr_ty(init).kind.clone()));
            Flow::Fallthrough
        }
        HirStmt::Assign {
            name, value, ty, ..
        } => {
            // The checker proved `name` is a `var` binding, so its slot is in
            // `locals`. There is no `?` to reach for here: this is the backend,
            // which never fails on user input, and a name with no slot would be
            // a compiler bug rather than a bad program. Skipping the store
            // leaves valid IR and a wrong value, which is the right failure
            // mode for a case that cannot happen -- the alternative is a panic,
            // which the project forbids.
            let slot = ctx.locals.get(name).map(|(slot, _)| slot.clone());
            let value = gen_expr(ctx, value);
            let repr = repr_type(&ty.kind);
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
        HirStmt::Return(value, _) => {
            // Defers for this scope run before the block is terminated, so
            // they execute on the way out, not when the `defer` was reached.
            run_defers(ctx, defers);
            match value {
                Some(expr) => {
                    let v = gen_expr(ctx, expr);
                    let repr = repr_type(&ctx.ret_type);
                    if repr == "void" {
                        ctx.push("ret void");
                    } else {
                        ctx.push(format!("ret {repr} {}", v.value));
                    }
                }
                None => ctx.push("ret void"),
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
            defers,
        ),
        HirStmt::ForRange {
            name, start, end, body, ..
        } => gen_for_range(ctx, name, start, end, body),
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
            defers,
        ),
        HirStmt::Switch { subject, arms, .. } => {
            let scrutinee = gen_expr(ctx, subject);
            let _ = gen_match(ctx, &scrutinee, arms, &TypeKind::Unit);
            Flow::Fallthrough
        }
        HirStmt::Break(_) => {
            run_defers(ctx, defers);
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
            run_defers(ctx, defers);
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
            defers.push(body.clone());
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
    defers: &mut Vec<HirBlock>,
) -> Flow {
    if let Some(init) = init {
        let flow = gen_stmt(ctx, init, defers);
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
    let mut then_defers = Vec::new();
    if gen_block(ctx, then_block, &mut then_defers) == Flow::Fallthrough {
        run_defers(ctx, &mut then_defers);
        ctx.push(format!("br label %{merge_label}"));
    }

    ctx.push(format!("{else_label}:"));
    match else_block {
        Some(block) => {
            let mut else_defers = Vec::new();
            if gen_block(ctx, block, &mut else_defers) == Flow::Fallthrough {
                run_defers(ctx, &mut else_defers);
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
    let mut body_defers = Vec::new();
    let flow = gen_block(ctx, body, &mut body_defers);
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
        run_defers(ctx, &mut body_defers);
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
    defers: &mut Vec<HirBlock>,
) -> Flow {
    if let Some(init) = init {
        let flow = gen_stmt(ctx, init, defers);
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
    let mut body_defers = Vec::new();
    let flow = gen_block(ctx, body, &mut body_defers);
    ctx.loops.pop();
    if flow == Flow::Fallthrough {
        run_defers(ctx, &mut body_defers);
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
pub fn run_defers(ctx: &mut FnCtx<'_>, defers: &mut Vec<HirBlock>) {
    while let Some(body) = defers.pop() {
        let mut inner = Vec::new();
        let _ = gen_block(ctx, &body, &mut inner);
        run_defers(ctx, &mut inner);
    }
}
