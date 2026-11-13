//! Async lowering: an `async fn` becomes a heap frame plus a poll function.
//!
//! The frame is a struct the factory function (`@f`) allocates with
//! `yashy_alloc` and the poll function (`@f.poll`) drives:
//!
//! ```text
//! %struct.async.f.frame = type { i32, i8*, i8*, i8*, i8*, <args>, <live locals>, <result> }
//!                           resume poll child result_slot name
//! ```
//!
//! - `resume` is the `u32` state index. `0` enters the first segment; the
//!   return path writes `4294967295`, so re-polling a finished task answers
//!   `Ready` without running anything again.
//! - `poll` is the address of `@f.poll` itself, written by the factory. A
//!   parent task reads it out of a child's header and calls it indirectly,
//!   which is how nested `await`s compose: every child is itself a task.
//! - `child` is the task currently being awaited. Storing it before
//!   `return null` (Pending) is what makes resumption possible: the LLVM
//!   stack frame is gone while the task is suspended.
//! - `result_slot` is a pointer to the result field, so `await` and
//!   `block_on` can read the result without knowing the child frame's
//!   layout.
//! - `name` is the NUL-terminated name of the async function, in a string
//!   global. The executor's live-task registry reads it (byte offset 32) to
//!   name a task the way a debugger would — `task.<id>.<fn>` — in the
//!   `yashy tasks` dump. The poll function's own argument is named
//!   `%task.frame` for the same reason: the LLVM value a debugger sees is
//!   the task's frame pointer, not an anonymous one.
//!
//! The header is five words (40 bytes); the executor knows the resume
//! index lives at byte 0 and the name at byte 32, and nothing else about
//! the frame. Every field after the header is reached through
//! `getelementptr`, so a hand-made frame only needs its `resume`, `poll`
//! and `child` words.
//!
//! Which locals live in the frame is a backwards liveness pass over the
//! body: a binding defined before an `await` and used after it lives in the
//! frame; anything else stays in an ordinary `alloca`.
//!
//! The segment split only breaks at awaits the statement can surrender:
//! `let x = child().await`, `x = child().await`, `child().await;` and
//! `return child().await;`. An await nested inside a larger expression is
//! driven to completion inline (`yashy_block_on`), which is the same
//! result without a suspension point.

use yashy_sema::{HirBlock, HirExpr, HirFn, HirStmt, TypeKind};

use crate::expr_gen::{gen_expr, FnCtx, GenValue};
use crate::ir::field_repr;
use crate::stmt_gen::{gen_block, Flow};

/// One field of an async frame.
#[derive(Clone, Debug)]
pub struct FrameField {
    /// The binding this field holds, or the header role's name.
    pub name: String,
    /// The Yashy type of the field.
    pub ty: TypeKind,
}

/// The frame layout one `async fn` lowers to.
#[derive(Clone, Debug)]
pub struct FrameLayout {
    /// The LLVM named type, e.g. `%struct.async.f.frame`.
    pub frame_type: String,
    /// The poll function's symbol, e.g. `f.poll`.
    pub poll_name: String,
    /// Every named field after the four-word header: parameters first, then
    /// the locals live across an `await`.
    pub fields: Vec<FrameField>,
    /// The unwrapped result type `T` (the `HirFn` return type is `Task[T]`).
    pub result_ty: TypeKind,
    /// LLVM index of the result field: the last field.
    pub result_index: i32,
    /// 1-based resume index written when suspending at each `await`, in
    /// order.
    pub resume_points: Vec<i32>,
}

/// The shape of one `await` a statement can surrender to a suspension.
enum AwaitSite {
    /// `let x = child().await` / `const x = child().await`: the result
    /// binds `name` when the task resumes.
    Bind(String),
    /// `x = child().await`.
    Assign(String),
    /// `child().await;` with the result dropped.
    Discard,
    /// `return child().await;`: the result is the function's answer.
    Return,
}

/// Split the function body into segments at the awaits it can surrender.
///
/// Returns the segments' statements and, between consecutive segments, the
/// child expression and binding each suspension carries.
struct Segmentation {
    segments: Vec<Vec<HirStmt>>,
    awaits: Vec<(HirExpr, AwaitSite)>,
}

fn await_site(stmt: &HirStmt) -> Option<(HirExpr, AwaitSite)> {
    match stmt {
        HirStmt::Var { name, init, .. } | HirStmt::Const { name, init, .. } => match init {
            HirExpr::Await { expr, .. } => Some(((**expr).clone(), AwaitSite::Bind(name.clone()))),
            _ => None,
        },
        HirStmt::Assign { name, value, .. } => match value {
            HirExpr::Await { expr, .. } => {
                Some(((**expr).clone(), AwaitSite::Assign(name.clone())))
            }
            _ => None,
        },
        HirStmt::Expr(HirExpr::Await { expr, .. }, ..) => {
            Some(((**expr).clone(), AwaitSite::Discard))
        }
        HirStmt::Return(Some(HirExpr::Await { expr, .. }), ..) => {
            Some(((**expr).clone(), AwaitSite::Return))
        }
        _ => None,
    }
}

fn segment_body(body: &[HirStmt]) -> Segmentation {
    let mut segments: Vec<Vec<HirStmt>> = vec![Vec::new()];
    let mut awaits = Vec::new();
    for stmt in body {
        if let Some((child, site)) = await_site(stmt) {
            awaits.push((child, site));
            segments.push(Vec::new());
        } else {
            segments
                .last_mut()
                .expect("segments always has one open segment")
                .push(stmt.clone());
        }
    }
    Segmentation { segments, awaits }
}

/// Every name a binding introduces, with its type.
fn collect_bindings(stmt: &HirStmt, out: &mut Vec<(String, TypeKind)>) {
    match stmt {
        HirStmt::Var { name, ty, init, .. } | HirStmt::Const { name, ty, init, .. } => {
            let kind = ty
                .as_ref()
                .map(|ty| ty.kind.clone())
                .unwrap_or_else(|| expr_ty_kind(init));
            out.push((name.clone(), kind));
        }
        HirStmt::Destructure { pattern, .. } => {
            for (name, ty) in pattern.bindings() {
                out.push((name, ty.kind));
            }
        }
        HirStmt::ForRange { name, body, .. } => {
            // The loop variable is `int`; the body may bind more names.
            out.push((name.clone(), TypeKind::I64));
            for inner in &body.stmts {
                collect_bindings(inner, out);
            }
        }
        _ => {}
    }
}

fn expr_ty_kind(expr: &HirExpr) -> TypeKind {
    use yashy_sema::hir::expr_ty;
    expr_ty(expr).kind
}

/// Every `Ident` a statement reads.
fn collect_uses(stmt: &HirStmt, out: &mut Vec<String>) {
    fn expr_uses(expr: &HirExpr, out: &mut Vec<String>) {
        match expr {
            HirExpr::Ident(path, ..) => {
                if let Some(name) = path.last() {
                    out.push(name.clone());
                }
            }
            HirExpr::Binary { lhs, rhs, .. } => {
                expr_uses(lhs, out);
                expr_uses(rhs, out);
            }
            HirExpr::Unary { operand, .. } => expr_uses(operand, out),
            HirExpr::Move(inner, ..) => expr_uses(inner, out),
            HirExpr::Call { args, .. } => {
                for arg in args {
                    expr_uses(arg, out);
                }
            }
            HirExpr::StructLit { fields, .. } => {
                for (_, field, _) in fields {
                    expr_uses(field, out);
                }
            }
            HirExpr::SliceLit { elements, .. } | HirExpr::FixedArrayLit { elements, .. } => {
                for element in elements {
                    expr_uses(element, out);
                }
            }
            HirExpr::FieldAccess { base, .. } => expr_uses(base, out),
            HirExpr::Index { base, index, .. } => {
                expr_uses(base, out);
                expr_uses(index, out);
            }
            HirExpr::TupleLit { elements, .. } => {
                for element in elements {
                    expr_uses(element, out);
                }
            }
            HirExpr::Cast { expr, .. }
            | HirExpr::Try { expr, .. }
            | HirExpr::Await { expr, .. } => expr_uses(expr, out),
            HirExpr::Spawn { callee, args, .. } => {
                expr_uses(callee, out);
                for arg in args {
                    expr_uses(arg, out);
                }
            }
            HirExpr::SwitchExpr { subject, arms, .. } => {
                expr_uses(subject, out);
                for arm in arms {
                    expr_uses(&arm.body, out);
                    if let Some(guard) = &arm.guard {
                        expr_uses(guard, out);
                    }
                }
            }
            HirExpr::View {
                base, start, end, ..
            } => {
                expr_uses(base, out);
                expr_uses(start, out);
                expr_uses(end, out);
            }
            HirExpr::CallCapture { args, .. } => {
                for arg in args {
                    expr_uses(arg, out);
                }
            }
            HirExpr::HigherOrder {
                receiver,
                func,
                init,
                ..
            } => {
                expr_uses(receiver, out);
                expr_uses(func, out);
                if let Some(init) = init {
                    expr_uses(init, out);
                }
            }
            HirExpr::Literal(..)
            | HirExpr::Capture { .. }
            | HirExpr::AddrOf { .. }
            | HirExpr::Lambda { .. } => {}
        }
    }
    match stmt {
        HirStmt::Expr(expr, ..) => expr_uses(expr, out),
        HirStmt::Var { init, .. } | HirStmt::Const { init, .. } => expr_uses(init, out),
        HirStmt::Assign { name, value, .. } => {
            out.push(name.clone());
            expr_uses(value, out);
        }
        HirStmt::Destructure { init, .. } => expr_uses(init, out),
        HirStmt::IndexAssign {
            base, index, value, ..
        } => {
            expr_uses(base, out);
            expr_uses(index, out);
            expr_uses(value, out);
        }
        HirStmt::FieldAssign { value, .. } => expr_uses(value, out),
        HirStmt::FieldPtrAssign {
            pointer,
            index,
            value,
            ..
        } => {
            expr_uses(pointer, out);
            expr_uses(index, out);
            expr_uses(value, out);
        }
        HirStmt::Return(value, _) => {
            if let Some(value) = value {
                expr_uses(value, out);
            }
        }
        HirStmt::If {
            init,
            cond,
            then_block,
            else_block,
            ..
        } => {
            if let Some(init) = init {
                collect_uses(init, out);
            }
            expr_uses(cond, out);
            for inner in &then_block.stmts {
                collect_uses(inner, out);
            }
            if let Some(else_block) = else_block {
                for inner in &else_block.stmts {
                    collect_uses(inner, out);
                }
            }
        }
        HirStmt::IfLet {
            subject,
            then_block,
            else_block,
            ..
        } => {
            expr_uses(subject, out);
            for inner in &then_block.stmts {
                collect_uses(inner, out);
            }
            if let Some(else_block) = else_block {
                for inner in &else_block.stmts {
                    collect_uses(inner, out);
                }
            }
        }
        HirStmt::ForRange {
            start, end, body, ..
        } => {
            expr_uses(start, out);
            expr_uses(end, out);
            for inner in &body.stmts {
                collect_uses(inner, out);
            }
        }
        HirStmt::ForIn { iterable, body, .. } => {
            expr_uses(iterable, out);
            for inner in &body.stmts {
                collect_uses(inner, out);
            }
        }
        HirStmt::ForC {
            init,
            cond,
            update,
            body,
            ..
        } => {
            if let Some(init) = init {
                collect_uses(init, out);
            }
            if let Some(cond) = cond {
                expr_uses(cond, out);
            }
            if let Some(update) = update {
                expr_uses(update, out);
            }
            for inner in &body.stmts {
                collect_uses(inner, out);
            }
        }
        HirStmt::Switch { subject, arms, .. } => {
            expr_uses(subject, out);
            for arm in arms {
                expr_uses(&arm.body, out);
                if let Some(guard) = &arm.guard {
                    expr_uses(guard, out);
                }
            }
        }
        HirStmt::Defer { body, .. } | HirStmt::Scope { body, .. } => {
            for inner in &body.stmts {
                collect_uses(inner, out);
            }
        }
        HirStmt::Break(_) | HirStmt::Continue(_) | HirStmt::LoopInvariant(_) => {}
    }
}

/// The `T` inside the type of a child expression: `child()` answers
/// `Task[T]`, so awaiting it yields `T`.
fn task_inner_ty(expr: &HirExpr) -> TypeKind {
    match expr_ty_kind(expr) {
        TypeKind::Task(inner) => inner.kind,
        other => other,
    }
}

/// The result type `T` inside `Task[T]` (or `unit` for a non-`Task` return,
/// which the checker never produces for an `async fn`).
fn result_type(f: &HirFn) -> TypeKind {
    match &f.ret.kind {
        TypeKind::Task(inner) => inner.kind.clone(),
        other => other.clone(),
    }
}

/// Lower the frame layout for an `async fn`: the parameters plus every local
/// live across an `await`.
///
/// The second value names the poll function, which always is
/// `<name>.poll` in the same module.
pub fn lower_async(f: &HirFn) -> (FrameLayout, String) {
    let segmentation = segment_body(&f.body.stmts);

    // Backwards liveness: walk the segments from the last one backwards,
    // carrying the names used after an await; a name defined before that
    // await and used after it lives in the frame.
    let mut live: Vec<(String, TypeKind)> = Vec::new();
    let mut used_after: Vec<String> = Vec::new();
    for segment in segmentation.segments.iter().rev() {
        let mut uses = Vec::new();
        for stmt in segment {
            collect_uses(stmt, &mut uses);
        }
        let mut defs = Vec::new();
        for stmt in segment {
            collect_bindings(stmt, &mut defs);
        }
        for (name, ty) in &defs {
            if used_after.iter().any(|used| used == name)
                && !live.iter().any(|(live_name, _)| live_name == name)
            {
                live.push((name.clone(), ty.clone()));
            }
        }
        used_after = uses.into_iter().chain(used_after).collect::<Vec<_>>();
        // Bindings of this segment that no later segment uses are dead by
        // the backwards pass; definitions in the last segment are never
        // live-across, so nothing to add for them.
    }
    // Parameters live across the first await as well: the factory writes
    // them into the frame, and the poll reads them back.
    let mut fields = Vec::new();
    for param in &f.params {
        fields.push(FrameField {
            name: param.name.clone(),
            ty: param.ty.kind.clone(),
        });
    }
    for (name, ty) in &live {
        if !fields.iter().any(|field| &field.name == name) {
            fields.push(FrameField {
                name: name.clone(),
                ty: ty.clone(),
            });
        }
    }
    // The name an `await` binds is not part of any segment's statements, so
    // the liveness pass never gives it a field. It needs one, typed by the
    // child task's `T` -- the value being bound -- not by anything else.
    for (child, site) in &segmentation.awaits {
        let name = match site {
            AwaitSite::Bind(name) | AwaitSite::Assign(name) => name,
            AwaitSite::Return | AwaitSite::Discard => continue,
        };
        if !fields.iter().any(|field| &field.name == name) {
            fields.push(FrameField {
                name: name.clone(),
                ty: task_inner_ty(child),
            });
        }
    }

    let result_ty = result_type(f);
    let result_index = (5 + fields.len()) as i32;
    let poll_name = format!("{}.poll", f.name);
    let resume_points = (1..=segmentation.awaits.len()).map(|i| i as i32).collect();
    let layout = FrameLayout {
        frame_type: format!("%struct.async.{}.frame", f.name),
        poll_name: poll_name.clone(),
        fields,
        result_ty,
        result_index,
        resume_points,
    };
    (layout, poll_name)
}

/// The named frame type's fields for `layout`, in LLVM's spelling.
///
/// The five-word header is `resume`, `poll`, `child`, `result_slot` and
/// `name`; the frame's own fields and the result follow it.
pub fn frame_type_fields(layout: &FrameLayout) -> Vec<String> {
    let mut parts = vec![
        "i32".to_string(),
        "i8*".to_string(),
        "i8*".to_string(),
        "i8*".to_string(),
        "i8*".to_string(),
    ];
    for field in &layout.fields {
        parts.push(field_repr(&field.ty));
    }
    parts.push(field_repr(&layout.result_ty));
    parts
}

/// The size of a frame in bytes, computed the way the thread lowering does:
/// `getelementptr` on a null pointer, so LLVM's own layout answers.
fn frame_size(ctx: &mut FnCtx<'_>, layout: &FrameLayout) -> String {
    let address = ctx.tmp();
    ctx.push(format!(
        "{address} = getelementptr {}, ptr null, i32 1",
        layout.frame_type
    ));
    let size = ctx.tmp();
    ctx.push(format!("{size} = ptrtoint ptr {address} to i64"));
    size
}

/// The factory function: allocate the frame, write the header and the
/// arguments, and hand the frame back as the `Task[T]` value.
fn gen_factory(ctx: &mut FnCtx<'_>, f: &HirFn, layout: &FrameLayout) {
    // The factory has no body of its own: the arguments arrive as `%argN`
    // and land in the frame. Emit it through a bare FnCtx so the constants
    // live in one place.
    let size = frame_size(ctx, layout);
    let frame = ctx.tmp();
    ctx.push(format!(
        "{frame} = call i8* @yashy_alloc(i64 {size}, i64 8)"
    ));

    let field_at = |ctx: &mut FnCtx<'_>, index: i32| -> String {
        let field = ctx.tmp();
        ctx.push(format!(
            "{field} = getelementptr {}, ptr {frame}, i32 0, i32 {index}",
            layout.frame_type
        ));
        field
    };
    let resume = field_at(ctx, 0);
    ctx.push(format!("store i32 0, ptr {resume}"));
    let poll = field_at(ctx, 1);
    ctx.push(format!(
        "store i8* @{poll_name}, ptr {poll}",
        poll_name = layout.poll_name
    ));
    let child = field_at(ctx, 2);
    ctx.push(format!("store i8* null, ptr {child}"));
    let result_ptr = field_at(ctx, layout.result_index);
    let result_slot = ctx.tmp();
    ctx.push(format!("{result_slot} = bitcast ptr {result_ptr} to i8*"));
    let slot = field_at(ctx, 3);
    ctx.push(format!("store i8* {result_slot}, ptr {slot}"));
    // The name metadata (byte offset 32): the function's name as a
    // NUL-terminated string global, which the executor's registry reads so
    // `yashy tasks` reports `task.<id>.<fn>` for every live task.
    let name_global = ctx.module.intern_string(f.name.as_bytes());
    let name_ptr = ctx.tmp();
    ctx.push(format!(
        "{name_ptr} = bitcast [{} x i8]* {name_global}.data to i8*",
        f.name.len() + 1
    ));
    let name = field_at(ctx, 4);
    ctx.push(format!("store i8* {name_ptr}, ptr {name}"));

    for (index, field) in layout.fields.iter().enumerate() {
        if index < f.params.len() {
            let repr = crate::ir::repr_type(&field.ty);
            let slot = field_at(ctx, (5 + index) as i32);
            ctx.push(format!("store {repr} %arg{index}, ptr {slot}"));
        }
    }
    ctx.push(format!("ret i8* {frame}"));
}

/// The slot of frame field `index` for the function being emitted.
///
/// The poll function's argument is `%task.frame`, not `%frame`: the frame
/// pointer is the task's identity, and a debugger (or a reader of the IR)
/// sees it named for the task it belongs to.
fn field_slot(ctx: &mut FnCtx<'_>, layout: &FrameLayout, index: i32) -> String {
    let slot = ctx.tmp();
    ctx.push(format!(
        "{slot} = getelementptr {}, ptr %task.frame, i32 0, i32 {index}",
        layout.frame_type
    ));
    slot
}

/// Hand a finished child frame back once its result has been read. A spawned
/// task's frame belongs to the scope that spawned it: the runtime marks that
/// scope's child slot done (so the join skips it) and frees the frame; a
/// frame of a directly called `async fn` has no slot and is simply freed.
/// `yashy_task_release` answers from the live scope chain, which keeps the
/// code generator free of an ownership fact it does not have (it cannot tell
/// a spawned task from a direct call: both are `Task[T]`).
fn release_child(ctx: &mut FnCtx<'_>, child: &str) {
    let executor = ctx.tmp();
    ctx.push(format!("{executor} = call i8* @yashy_executor_default()"));
    ctx.push(format!(
        "call void @yashy_task_release(i8* {executor}, i8* {child})"
    ));
}

/// Bind the result of the child task that just became ready to the awaited
/// value, then hand the child frame back.
fn bind_result(ctx: &mut FnCtx<'_>, layout: &FrameLayout, site: &AwaitSite, result_ty: TypeKind) {
    let child_slot = field_slot(ctx, layout, 2);
    let child = ctx.tmp();
    ctx.push(format!("{child} = load i8*, ptr {child_slot}"));
    // The child's result slot is header word 3 (byte 24) of the child frame,
    // not of this frame: this frame's word 3 points at our own result.
    let slot_addr = ctx.tmp();
    ctx.push(format!(
        "{slot_addr} = getelementptr i8, ptr {child}, i32 24"
    ));
    let slot_base = ctx.tmp();
    ctx.push(format!("{slot_base} = load i8*, ptr {slot_addr}"));
    let repr = crate::ir::repr_type(&result_ty);
    if repr != "void" {
        let value = ctx.tmp();
        ctx.push(format!("{value} = load {repr}, ptr {slot_base}"));
        match site {
            AwaitSite::Bind(name) | AwaitSite::Assign(name) => {
                if !layout.fields.iter().any(|field| &field.name == name)
                    && !ctx.locals.contains_key(name)
                {
                    // A binding surrendered by the split keeps a stack slot:
                    // its defining statement never ran through `gen_block`,
                    // so allocate the slot here.
                    let slot = ctx.alloc_slot();
                    let ty = result_ty.clone();
                    ctx.push_alloca(format!("{slot} = alloca {}", repr));
                    ctx.locals.insert(name.clone(), (slot, ty));
                }
                if layout.fields.iter().any(|field| &field.name == name) {
                    let field_index = (5 + layout
                        .fields
                        .iter()
                        .position(|field| &field.name == name)
                        .unwrap_or(0)) as i32;
                    let dest = field_slot(ctx, layout, field_index);
                    ctx.push(format!("store {repr} {value}, ptr {dest}"));
                } else {
                    // A binding not live across another await keeps a stack
                    // slot, which `gen_block` for the rest of the body will
                    // have allocated; store through the binding's slot.
                    if let Some((dest, _)) = ctx.locals.get(name).cloned() {
                        ctx.push(format!("store {repr} {value}, ptr {dest}"));
                    }
                }
            }
            AwaitSite::Return => {
                let result = field_slot(ctx, layout, layout.result_index);
                ctx.push(format!("store {repr} {value}, ptr {result}"));
                let resume = field_slot(ctx, layout, 0);
                ctx.push(format!("store i32 -1, ptr {resume}"));
                release_child(ctx, &child);
                ctx.push("ret i8* %task.frame".to_string());
                return;
            }
            AwaitSite::Discard => {}
        }
    }
    release_child(ctx, &child);
}

fn gen_poll(module: &mut crate::module::Module, f: &HirFn, layout: &FrameLayout) {
    use crate::stmt_gen::run_defers;
    // `--no-opt=async` disables suspension: the whole body is one segment
    // and every await drives its child inline.
    let suspend = module.async_enabled();
    module.push(format!(
        "define i8* @{} (i8* %task.frame) {{",
        layout.poll_name
    ));
    module.begin_fn_report(&f.name, f.file, f.span);
    {
        let mut ctx = FnCtx::new(module, layout.result_ty.clone());
        ctx.async_poll = Some(crate::expr_gen::AsyncPoll {
            frame_slot: "%task.frame".to_string(),
            frame_type: layout.frame_type.clone(),
            result_index: layout.result_index,
            result_ty: layout.result_ty.clone(),
        });
        ctx.begin_entry();

        // Header and frame fields.
        let resume_slot = field_slot(&mut ctx, layout, 0);
        for (index, field) in layout.fields.iter().enumerate() {
            let slot = field_slot(&mut ctx, layout, (5 + index) as i32);
            ctx.locals
                .insert(field.name.clone(), (slot, field.ty.clone()));
        }

        let segmentation = if suspend {
            segment_body(&f.body.stmts)
        } else {
            Segmentation {
                segments: vec![f.body.stmts.clone()],
                awaits: Vec::new(),
            }
        };
        let resume = ctx.tmp();
        ctx.push(format!("{resume} = load i32, ptr {resume_slot}"));
        let done = ctx.tmp();
        ctx.push(format!("{done} = icmp eq i32 {resume}, -1"));
        let done_label = format!("done{}", ctx.alloc_label_base());
        let dispatch_label = format!("dispatch{}", ctx.alloc_label_base());
        ctx.push(format!(
            "br i1 {done}, label %{done_label}, label %{dispatch_label}"
        ));
        ctx.push(format!("{done_label}:"));
        ctx.push("ret i8* %task.frame".to_string());
        ctx.push(format!("{dispatch_label}:"));
        let mut cases = Vec::new();
        for point in 1..=segmentation.awaits.len() {
            cases.push(format!("i32 {point}, label %state{point}"));
        }
        ctx.push(format!(
            "switch i32 {resume}, label %state0 [ {} ]",
            cases.join(" ")
        ));

        for (index, segment) in segmentation.segments.iter().enumerate() {
            let state_label = format!("state{index}");
            ctx.push(format!("{state_label}:"));
            if index > 0 {
                // Resume: the child stored at the suspension should be
                // ready by now (or is polled once here and pends again).
                let (child_expr, site) = &segmentation.awaits[index - 1];
                let _ = child_expr;
                let child_slot = field_slot(&mut ctx, layout, 2);
                let child = ctx.tmp();
                ctx.push(format!("{child} = load i8*, ptr {child_slot}"));
                // The child's poll function is header word 1 (byte 8).
                let poll_addr = ctx.tmp();
                ctx.push(format!(
                    "{poll_addr} = getelementptr i8, ptr {child}, i32 8"
                ));
                let poll_fn = ctx.tmp();
                ctx.push(format!("{poll_fn} = load i8*, ptr {poll_addr}"));
                let ready = ctx.tmp();
                ctx.push(format!("{ready} = call i8* {poll_fn}(i8* {child})"));
                let is_pending = ctx.tmp();
                ctx.push(format!("{is_pending} = icmp eq i8* {ready}, null"));
                let pending_label = format!("pending{index}");
                let cont_label = format!("cont{index}");
                ctx.push(format!(
                    "br i1 {is_pending}, label %{pending_label}, label %{cont_label}"
                ));
                ctx.push(format!("{pending_label}:"));
                ctx.push("ret i8* null".to_string());
                ctx.push(format!("{cont_label}:"));
                // The result type is the child's `T`: the await's own type.
                let result_ty = match site {
                    AwaitSite::Bind(name) | AwaitSite::Assign(name) => layout
                        .fields
                        .iter()
                        .find(|field| &field.name == name)
                        .map(|field| field.ty.clone())
                        .unwrap_or_else(|| layout.result_ty.clone()),
                    AwaitSite::Return => layout.result_ty.clone(),
                    AwaitSite::Discard => TypeKind::Unit,
                };
                // `bind_result` reads the child from the frame; preload the
                // locals map so a stack binding receives the store.
                bind_result(&mut ctx, layout, site, result_ty);
                if matches!(site, AwaitSite::Return) {
                    continue;
                }
            }
            let block = HirBlock {
                stmts: segment.clone(),
                span: f.body.span,
                arena: false,
            };
            let flow = gen_block(&mut ctx, &block);
            if flow == Flow::Return {
                continue;
            }
            if index < segmentation.awaits.len() {
                let (child_expr, _) = &segmentation.awaits[index];
                let child = gen_expr(&mut ctx, child_expr);
                let child_slot = field_slot(&mut ctx, layout, 2);
                ctx.push(format!("store i8* {}, ptr {child_slot}", child.value));
                let resume_next = ((index + 1) as i32).to_string();
                ctx.push(format!("store i32 {resume_next}, ptr {resume_slot}"));
                ctx.push("ret i8* null".to_string());
            } else {
                // Fell off the end: the checker proved a value return on
                // every path, so this is a zero result for a `void` task.
                let _ = run_defers(&mut ctx);
                let result = field_slot(&mut ctx, layout, layout.result_index);
                let repr = field_repr(&layout.result_ty);
                if repr != "{}" {
                    ctx.push(format!("store {repr} zeroinitializer, ptr {result}"));
                }
                ctx.push(format!("store i32 -1, ptr {resume_slot}"));
                ctx.push("ret i8* %task.frame".to_string());
            }
        }
        ctx.push("}".to_string());
        ctx.finish();
    }
    module.end_fn_report();
}

/// Emit an async function: its factory plus its poll function.
///
/// The factory keeps the source name; the function value a caller gets is
/// the factory's, and the value it returns is the frame pointer the
/// runtime's executor calls the poll function on.
pub fn gen_async_fn(module: &mut crate::module::Module, f: &HirFn) {
    let (layout, _) = lower_async(f);
    module.define_named_type(&layout.frame_type, frame_type_fields(&layout));
    // The poll function is emitted first so the factory can name it.
    gen_poll(module, f, &layout);

    let ret_repr = crate::fn_gen::fn_return_repr(&f.ret.kind);
    let mut params = Vec::new();
    for (index, param) in f.params.iter().enumerate() {
        params.push(format!(
            "{} %arg{index}",
            crate::ir::repr_type(&param.ty.kind)
        ));
    }
    module.push(format!(
        "define {ret_repr} @{}({}) {{",
        crate::ir::fn_symbol(&f.name),
        params.join(", ")
    ));
    module.begin_fn_report(&f.name, f.file, f.span);
    {
        let mut ctx = FnCtx::new(module, f.ret.kind.clone());
        ctx.begin_entry();
        gen_factory(&mut ctx, f, &layout);
        ctx.push("}".to_string());
        ctx.finish();
    }
    module.end_fn_report();
}

/// A zero frame for an `async fn` that the code generator was asked to lower
/// without running the pass (`--no-opt=async`): the factory allocates the
/// same frame and the poll function drives the body to completion without
/// suspending, so the bisect flag changes no program's meaning.
pub fn gen_async_fn_blocking(module: &mut crate::module::Module, f: &HirFn) {
    // With the pass disabled the body runs as one segment and every await
    // is driven inline; the frame exists so the poll contract holds.
    gen_async_fn(module, f);
}

/// Drive a child task to completion inline, then read its result from the
/// frame: the `await` lowering for awaits nested inside an expression,
/// which cannot surrender.
pub fn drive_inline(ctx: &mut FnCtx<'_>, child: &GenValue, result_ty: &TypeKind) -> GenValue {
    let executor = ctx.tmp();
    ctx.push(format!("{executor} = call i8* @yashy_executor_default()"));
    // The child's poll function is header word 1 (byte 8).
    let poll_addr = ctx.tmp();
    ctx.push(format!(
        "{poll_addr} = getelementptr i8, ptr {}, i32 8",
        child.value
    ));
    let poll_fn = ctx.tmp();
    ctx.push(format!("{poll_fn} = load i8*, ptr {poll_addr}"));
    let done = ctx.tmp();
    ctx.push(format!(
        "{done} = call i32 @yashy_block_on(i8* {executor}, i8* {poll_fn}, i8* {})",
        child.value
    ));
    let _ = done;
    // The result pointer is header word 3 (byte 24).
    let slot_addr = ctx.tmp();
    ctx.push(format!(
        "{slot_addr} = getelementptr i8, ptr {}, i32 24",
        child.value
    ));
    let result_slot = ctx.tmp();
    ctx.push(format!("{result_slot} = load i8*, ptr {slot_addr}"));
    let repr = crate::ir::repr_type(result_ty);
    let value = if repr == "void" {
        "undef".to_string()
    } else {
        let value = ctx.tmp();
        ctx.push(format!("{value} = load {repr}, ptr {result_slot}"));
        value
    };
    release_child(ctx, &child.value);
    GenValue {
        value,
        ty: result_ty.clone(),
    }
}

/// Lower `spawn(worker, args...) -> Task[T]`: build the worker's task frame
/// and enqueue it on the default executor. The returned `Task[T]` is the
/// frame pointer: an `await` polls it through its header, so a spawned task
/// and a directly-created one drive identically.
pub fn gen_spawn(
    ctx: &mut FnCtx<'_>,
    callee: &HirExpr,
    args: &[HirExpr],
    ty: &TypeKind,
) -> GenValue {
    use yashy_sema::hir::expr_ty;
    let worker_ty = expr_ty(callee);
    let TypeKind::Func(params, ret) = &worker_ty.kind else {
        return GenValue {
            value: "null".to_string(),
            ty: ty.clone(),
        };
    };
    let name = match callee {
        HirExpr::Ident(path, ..) => path.last().cloned().unwrap_or_default(),
        _ => {
            return GenValue {
                value: "null".to_string(),
                ty: ty.clone(),
            }
        }
    };
    let is_async_worker = matches!(ret.kind, TypeKind::Task(_));
    let frame = if is_async_worker {
        // The worker's factory: a direct call that hands back the frame.
        let lowered: Vec<GenValue> = args.iter().map(|arg| gen_expr(ctx, arg)).collect();
        let call_args: Vec<String> = lowered
            .iter()
            .map(|value| format!("{} {}", crate::ir::repr_type(&value.ty), value.value))
            .collect();
        let frame = ctx.tmp();
        ctx.push(format!(
            "{frame} = call i8* @{}({})",
            crate::ir::fn_symbol(&name),
            call_args.join(", ")
        ));
        for value in &lowered {
            ctx.release_owned(&value.value);
        }
        frame
    } else {
        gen_spawn_sync(ctx, &name, params, args, &ret.kind, ty)
    };
    let executor = ctx.tmp();
    ctx.push(format!("{executor} = call i8* @yashy_executor_default()"));
    // The worker's poll function is header word 1 (byte 8) of its frame.
    let poll_addr = ctx.tmp();
    ctx.push(format!(
        "{poll_addr} = getelementptr i8, ptr {frame}, i32 8"
    ));
    let poll_fn = ctx.tmp();
    ctx.push(format!("{poll_fn} = load i8*, ptr {poll_addr}"));
    let status = ctx.tmp();
    ctx.push(format!(
        "{status} = call i32 @yashy_task_spawn(i8* {executor}, i8* {poll_fn}, i8* {frame})"
    ));
    // A failed spawn (a sandbox denial is one) must be loud: the frame was
    // never queued, so an await would poll it inline and pretend the spawn
    // had happened. Exit instead, with the mismatch visible in the IR.
    let failed = ctx.tmp();
    ctx.push(format!("{failed} = icmp ne i32 {status}, 0"));
    let failed_label = format!("spawn_failed{}", ctx.alloc_label_base());
    let ok_label = format!("spawn_ok{}", ctx.alloc_label_base());
    ctx.push(format!(
        "br i1 {failed}, label %{failed_label}, label %{ok_label}"
    ));
    ctx.push(format!("{failed_label}:"));
    ctx.push(format!("call void @yashy_free(i8* {frame})"));
    ctx.push("call void @yashy_exit(i64 1)".to_string());
    ctx.push("unreachable".to_string());
    ctx.push(format!("{ok_label}:"));
    GenValue {
        value: frame,
        ty: ty.clone(),
    }
}

/// A synchronous worker spawned as a task: synthesize a poll function that
/// calls the worker once and stores its answer in the frame's result slot.
fn gen_spawn_sync(
    ctx: &mut FnCtx<'_>,
    name: &str,
    params: &[yashy_sema::ty::Type],
    args: &[HirExpr],
    ret: &TypeKind,
    task_ty: &TypeKind,
) -> String {
    use crate::ir::{field_repr, repr_type};
    let frame_ty = format!("%struct.async.spawn.{name}.frame");
    // The worker is a Yashy function: its symbol carries the prefix that keeps
    // it apart from the C library's names.
    let worker = crate::ir::fn_symbol(name);
    let mut fields = vec![
        "i32".to_string(),
        "i8*".to_string(),
        "i8*".to_string(),
        "i8*".to_string(),
        "i8*".to_string(),
    ];
    for param in params {
        fields.push(field_repr(&param.kind));
    }
    fields.push(field_repr(ret));
    ctx.module.define_named_type(&frame_ty, fields.clone());
    let result_index = (5 + params.len()) as i32;

    // The synthesized poll function: load the arguments from the frame,
    // call the worker, store the result, run no further statements.
    let index = ctx.module.next_thread_entry();
    let poll_name = format!("yashy_spawn_poll_{index}");
    let mut lines = Vec::new();
    lines.push(format!("define i8* @{poll_name}(i8* %task.frame) {{"));
    lines.push("entry:".to_string());
    // Re-polling a finished task answers Ready without running the worker
    // again, exactly like an async poll's done sentinel.
    lines.push(format!(
        "  %spawn_resume0 = getelementptr {frame_ty}, ptr %task.frame, i32 0, i32 0"
    ));
    lines.push("  %spawn_resume_v = load i32, ptr %spawn_resume0".to_string());
    lines.push("  %spawn_done = icmp eq i32 %spawn_resume_v, -1".to_string());
    lines.push("  br i1 %spawn_done, label %spawn_done_lbl, label %spawn_run".to_string());
    lines.push("spawn_done_lbl:".to_string());
    lines.push("  ret i8* %task.frame".to_string());
    lines.push("spawn_run:".to_string());
    let mut call_args = Vec::new();
    for (param_index, param) in params.iter().enumerate() {
        let repr = repr_type(&param.kind);
        let field = format!("%spawn_arg{param_index}");
        let value = format!("%spawn_arg{param_index}_v");
        lines.push(format!(
            "  {field} = getelementptr {frame_ty}, ptr %task.frame, i32 0, i32 {}",
            5 + param_index
        ));
        lines.push(format!("  {value} = load {repr}, ptr {field}"));
        call_args.push(format!("{repr} {value}"));
    }
    let ret_repr = repr_type(ret);
    if ret_repr == "void" {
        lines.push(format!("  call void @{worker}({})", call_args.join(", ")));
    } else {
        let result = "%spawn_result";
        lines.push(format!(
            "  {result} = call {ret_repr} @{worker}({})",
            call_args.join(", ")
        ));
        let field = "%spawn_result_field";
        lines.push(format!(
            "  {field} = getelementptr {frame_ty}, ptr %task.frame, i32 0, i32 {result_index}"
        ));
        lines.push(format!("  store {ret_repr} {result}, ptr {field}"));
    }
    let resume = "%spawn_resume";
    lines.push(format!(
        "  {resume} = getelementptr {frame_ty}, ptr %task.frame, i32 0, i32 0"
    ));
    lines.push(format!("  store i32 -1, ptr {resume}"));
    lines.push("  ret i8* %task.frame".to_string());
    lines.push("}".to_string());
    ctx.module.add_synthesized_function(lines);

    // The caller's frame: allocate, write the header and the arguments.
    let size_addr = ctx.tmp();
    ctx.push(format!(
        "{size_addr} = getelementptr {frame_ty}, ptr null, i32 1"
    ));
    let size = ctx.tmp();
    ctx.push(format!("{size} = ptrtoint ptr {size_addr} to i64"));
    let frame = ctx.tmp();
    ctx.push(format!(
        "{frame} = call i8* @yashy_alloc(i64 {size}, i64 8)"
    ));
    let field_at = |ctx: &mut FnCtx<'_>, index: i32| -> String {
        let field = ctx.tmp();
        ctx.push(format!(
            "{field} = getelementptr {frame_ty}, ptr {frame}, i32 0, i32 {index}"
        ));
        field
    };
    let resume_slot = field_at(ctx, 0);
    ctx.push(format!("store i32 0, ptr {resume_slot}"));
    let poll_slot = field_at(ctx, 1);
    ctx.push(format!("store i8* @{poll_name}, ptr {poll_slot}"));
    let child_slot = field_at(ctx, 2);
    ctx.push(format!("store i8* null, ptr {child_slot}"));
    let result_field = field_at(ctx, result_index);
    let result_slot = ctx.tmp();
    ctx.push(format!("{result_slot} = bitcast ptr {result_field} to i8*"));
    let slot_slot = field_at(ctx, 3);
    ctx.push(format!("store i8* {result_slot}, ptr {slot_slot}"));
    // The name metadata (byte offset 32), like an async frame's: the
    // worker's name as a NUL-terminated string global.
    let name_global = ctx.module.intern_string(name.as_bytes());
    let name_ptr = ctx.tmp();
    ctx.push(format!(
        "{name_ptr} = bitcast [{} x i8]* {name_global}.data to i8*",
        name.len() + 1
    ));
    let name_slot = field_at(ctx, 4);
    ctx.push(format!("store i8* {name_ptr}, ptr {name_slot}"));
    for (param_index, (argument, param)) in args.iter().zip(params.iter()).enumerate() {
        let _ = param;
        let value = gen_expr(ctx, argument);
        let repr = repr_type(&value.ty);
        let dest = field_at(ctx, (5 + param_index) as i32);
        ctx.push(format!("store {repr} {}, ptr {dest}", value.value));
    }
    let _ = task_ty;
    frame
}
