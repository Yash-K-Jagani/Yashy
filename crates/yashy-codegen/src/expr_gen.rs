//! Expression lowering: typed HIR expressions to LLVM IR instructions.
//!
//! Every value-producing expression is lowered once, into a fresh SSA
//! temporary or alloca slot — never twice. [`gen_expr`] returns the value;
//! [`gen_expr_to_slot`] additionally spills it into an alloca, which is the
//! single entry point statements, `return` and match arms use.
//!
//! # Checked integer arithmetic (ledger L4, replaced)
//!
//! `Add Sub Mul` and `Div Rem` on `i64` are **checked**: an operation that
//! overflows, divides by zero, or negates the smallest `i64` branches to a block
//! that reports the operands and the position and ends the process. The ledger's
//! M0–M2 row said these emitted bare `add`/`sub`/`mul` — no `nsw` — and a bare
//! `sdiv`/`srem` with no zero guard, which made wraparound defined and division
//! by zero UB; that row is what M4 replaced, and `tests/golden/ops.expected.ll`
//! was rewritten with it rather than left to drift.
//!
//! The lowering lives in [`crate::faults`], and the position it reports comes
//! from the checker (see [`yashy_sema::FaultSite`]): the backend is handed IR
//! text and a path, and a span would mean nothing to it.
//!
//! Everything else is untouched: a comparison, a boolean `and`/`or`, a string
//! concatenation and float arithmetic have no fault to report and take the
//! ordinary path.
//!
//! # Interim semantics
//!
//! A construct not lowered yet becomes a comment plus a zero of its type:
//! visibly incomplete IR, never a silently wrong answer.
//!
//! `FieldAccess` was the last of those and is no longer: a struct value is an
//! aggregate, so a field read is `extractvalue` with the index the checker
//! resolved against the declaration order.
//!
//! # String ownership
//!
//! A `string` is a pointer to a refcounted `%yashy_str`; a literal is *static*
//! (refcount -1, never freed) and everything the runtime builds comes back owned
//! (refcount 1). The backend tracks the owned ones in a per-function set of SSA
//! names, [`FnCtx::owned_strings`], and emits a release where an operation has
//! *consumed* one — see [`FnCtx::mark_owned`] and [`FnCtx::release_owned`] for
//! why it is a name set and which operations count as consumers. A name that is
//! never released is a leak; a name released while the program still holds it
//! would be a use-after-free, so the set is what keeps the two apart.
//!
//! # No input makes this module panic
//!
//! No `unwrap`, no `expect`, no `panic!`, no unchecked indexing. Every
//! identifier lookup, variant tag and payload index is an `Option`, and a
//! missing one lowers to a comment plus a zero rather than aborting the
//! compile.

use std::collections::HashMap;

use yashy_common::Span;
use yashy_parser::ast::Literal;
use yashy_sema::{expr_ty, BinOp, CaptureSource, FaultSite, HirCapture, HirExpr, HirFn, TypeKind, UnOp};

use crate::ir::repr_type;
use crate::module::Module;

/// Run every pending deferred body in every active scope, innermost first, then
/// clear them all. Used on the panic/exit path, where no scope exits normally.
#[derive(Clone, Debug)]
pub struct GenValue {
    /// An LLVM SSA name (`%t3`) or a literal (`42`, `1.0`, `true`, `null`).
    /// The empty string means "no value" — a `()`-valued expression.
    pub value: String,
    /// The Yashy type of the value.
    pub ty: TypeKind,
}

/// The state of one function being emitted.
///
/// The module is **borrowed**, not owned (ledger ruling on Task 10 → 12):
/// `emit_program` owns one [`Module`] and creates a `FnCtx` per
/// function against it, so every function's body lands in the same module.
/// `locals` and `next_label` are additions the plan's three-field sketch did
/// not foresee: a body cannot resolve an identifier or emit a second `switch`
/// without them.
pub struct FnCtx<'a> {
    /// The module every instruction is appended to.
    pub module: &'a mut Module,
    /// The enclosing function's return type, so a `return` can coerce.
    pub ret_type: TypeKind,
    /// The index the next `%slotN` alloca will use.
    pub next_slot: u32,
    /// The index the next `matchN` block-name base will use.
    next_label: u32,
    /// `alloca` lines waiting to be spliced into the entry block, in the order
    /// they were allocated. See [`FnCtx::push_alloca`].
    hoisted_allocas: Vec<String>,
    /// The module line index of the entry block's label, once
    /// [`FnCtx::begin_entry`] has declared it.
    entry_line: Option<usize>,
    /// Bound names (`locals`, parameters, pattern bindings) to their slot and
    /// declared type.
    pub locals: HashMap<String, (String, TypeKind)>,
    /// Active loop labels, innermost last: `(break label, continue label)`.
    /// `break`/`continue` read the top entry, so nested loops need no extra
    /// plumbing through the statement tree.
    pub loops: Vec<(String, String)>,
    /// Deferred bodies registered in the active scopes, innermost scope last.
    ///
    /// Each scope pushes its own `Vec` on entry and pops it on exit, so a
    /// `defer` registers into the scope it is written in and the right set runs
    /// on `return`/`break`/`continue`. On a panic every level runs, innermost
    /// first, which is what makes `defer` useful on the fault path even though
    /// the process still ends.
    pub defers: Vec<Vec<crate::stmt_gen::Deferred>>,
    /// Whether this function's string refcounts are atomic.
    ///
    /// Read from the module at construction; see
    /// [`Module::set_atomic_strings`](crate::Module::set_atomic_strings). A
    /// program with one spawn uses the atomic pair in every function, because
    /// any string in it can reach the thread.
    pub atomic_strings: bool,
    /// SSA names that hold an **owned** `string` reference: one the runtime
    /// produced, that no binding has taken over and no consumer has released.
    ///
    /// A name in here is exactly a value a `release` is owed. It is a name set
    /// rather than a field on [`GenValue`] because the set *is* the whole fact
    /// -- "this exact value is owned" -- and because a temporary's owner is
    /// still the value by the time its consumer runs, not the expression that
    /// built it. A literal's static string and a name bound to a local are
    /// never in here: a literal is never freed, and a local's reference is the
    /// local's, so releasing one would drop a reference the program still
    /// holds.
    ///
    /// See [`FnCtx::release_owned`] for where the release is emitted. A name
    /// that is never released is a leak and not a corruption, which is why the
    /// sites that *do* release are the ones the consumer provably does not
    /// keep the pointer: the concatenations, the comparisons, `len`, the
    /// parsers and `print_str`.
    owned_refs: HashMap<String, OwnedRef>,
    /// Locals that own a reference, one `Vec` per active scope, innermost
    /// last, pushed and popped alongside [`FnCtx::defers`].
    ///
    /// Each entry is an `alloca` slot holding an owned value, and it is
    /// released when its scope exits — on fall-through, and on a
    /// `return`/`break`/`continue` from a statement in that scope. The release
    /// happens *after* the scope's deferred bodies have run, because
    /// `defer print_str(s)` reads `s` at exit and a `string` is a copy type, so
    /// the deferred body is not given a snapshot of it the way a linear binding
    /// is.
    ///
    /// This is why a *binding* is kept even though the temporary release would
    /// have freed it: a slot's value outlives the expression that produced it,
    /// so the reference has to belong to the slot and be dropped when the slot
    /// goes away. A scope that exits without any statement to hang the release
    /// on (a `break` from a doubly nested block) leaks, the same reading of the
    /// same limitation the defer list has.
    scope_refs: Vec<Vec<(String, RefKind, Option<TypeKind>)>>,
    /// The terminator for a `return` with no value, and for falling off the end
    /// of a body. `ret void` everywhere except the program's entry point, which
    /// has to yield an `i32` — see [`crate::fn_gen::gen_main`].
    pub unit_terminator: String,
    /// True for a function emitted as the process entry point, where `i32` is
    /// the return type the C runtime expects and an `i64` result is truncated
    /// to match it.
    pub entry_point: bool,
    /// Whether a deferred body is currently being lowered.
    ///
    /// A fault *inside* a deferred body would otherwise ask the fault lowering
    /// to emit that same body again, which does not terminate. Set around the
    /// body in `stmt_gen::lower_defers` and read by
    /// `stmt_gen::run_all_scopes_defers`, so a fault in a deferred body goes
    /// straight to the runtime.
    pub(crate) lowering_defers: bool,
    /// The values this function captured, in the order the environment holds
    /// them. Empty for a function that is not a closure.
    pub(crate) captures: Vec<HirCapture>,
    /// The offset of each captured value in the environment.
    pub(crate) capture_offsets: Vec<u64>,
    /// The environment pointer this function reads its captures from, when it
    /// is a closure. The name is the SSA value the parameter was bound to.
    pub(crate) env: Option<String>,
}

impl<'a> FnCtx<'a> {
    /// A context for one function, borrowing the shared module.
    pub fn new(module: &'a mut Module, ret_type: TypeKind) -> Self {
        let atomic_strings = module.atomic_strings();
        Self {
            module,
            ret_type,
            next_slot: 0,
            next_label: 0,
            hoisted_allocas: Vec::new(),
            entry_line: None,
            locals: HashMap::new(),
            loops: Vec::new(),
            defers: Vec::new(),
            owned_refs: HashMap::new(),
            scope_refs: Vec::new(),
            unit_terminator: "ret void".to_string(),
            entry_point: false,
            lowering_defers: false,
            captures: Vec::new(),
            capture_offsets: Vec::new(),
            env: None,
            atomic_strings,
        }
    }

    /// The retain symbol this function calls: plain, or the atomic pair when
    /// the module contains a spawn.
    pub(crate) fn string_retain_symbol(&self) -> &'static str {
        if self.atomic_strings {
            "yashy_str_retain_atomic"
        } else {
            "yashy_str_retain"
        }
    }

    /// The release symbol this function calls; the other half of
    /// [`FnCtx::string_retain_symbol`].
    pub(crate) fn string_release_symbol(&self) -> &'static str {
        if self.atomic_strings {
            "yashy_str_release_atomic"
        } else {
            "yashy_str_release"
        }
    }

    /// Emit `yashy_str_retain` (or its atomic form) for `value` and keep the
    /// result nowhere: the caller's own reference count is what changed.
    pub(crate) fn push_str_retain(&mut self, value: &str) {
        let symbol = self.string_retain_symbol();
        self.push(format!("call i8* @{symbol}(i8* {value})"));
    }

    /// Emit `yashy_str_release` (or its atomic form) for `value`.
    pub(crate) fn push_str_release(&mut self, value: &str) {
        let symbol = self.string_release_symbol();
        self.push(format!("call void @{symbol}(i8* {value})"));
    }

    /// Emit a retain whose result is bound to `dest`, for the places that keep
    /// the retained pointer rather than only bumping the count.
    pub(crate) fn push_str_retain_into(&mut self, dest: &str, value: &str) {
        let symbol = self.string_retain_symbol();
        self.push(format!("{dest} = call i8* @{symbol}(i8* {value})"));
    }

    /// Record that `name` holds an owned reference, so a consumer owes it a
    /// `release`. See [`FnCtx::release_owned`].
    pub(crate) fn mark_owned(&mut self, name: &str, kind: RefKind) {
        self.mark_owned_typed(name, kind, None);
    }

    /// Record that `name` holds an owned reference, and the type of the value
    /// that holds it.
    pub(crate) fn mark_owned_typed(&mut self, name: &str, kind: RefKind, ty: Option<TypeKind>) {
        if !name.is_empty() {
            self.owned_refs
                .insert(name.to_string(), OwnedRef { kind, ty });
        }
    }

    /// Release `name` if it holds an owned reference, and forget it.
    ///
    /// A name that is not owned is left alone: a literal is a static string
    /// (releasing it is a no-op, but there is nothing to say), a name bound to
    /// a local belongs to that local, and an integer or `()` value is not a
    /// reference at all. Emitting the release only for an owned name is what
    /// keeps this from dropping a reference the program still holds.
    pub(crate) fn release_owned(&mut self, name: &str) {
        let Some(owned) = self.owned_refs.remove(name) else {
            return;
        };
        match owned.kind {
            RefKind::Str => {
                let symbol = self.string_release_symbol();
                self.push(format!("call void @{symbol}(i8* {name})"));
            }
            RefKind::Env => {
                // The value is a `{ i8* code, i8* env }` pair: the environment
                // is its second half.
                let env = self.tmp();
                self.push(format!(
                    "{env} = extractvalue {{ i8*, i8* }} {name}, 1"
                ));
                self.push(format!("call void @yashy_env_release(i8* {env})"));
            }
            RefKind::StructFields => {
                if let Some(ty) = &owned.ty {
                    release_struct_fields(self, name, ty);
                }
            }
        }
    }

    /// Whether `name` holds an owned reference. A `return` asks so it can tell a
    /// string it is transferring from one it has to keep.
    pub(crate) fn is_owned(&self, name: &str) -> bool {
        self.owned_refs.contains_key(name)
    }

    /// Forget an owned reference because something else has taken it over: a
    /// slot that stores a temporary's value now owns that reference, so no
    /// consumer will release it.
    pub(crate) fn forget_owned(&mut self, name: &str) {
        self.owned_refs.remove(name);
    }

    /// Add a reference to the environment a function value carries.
    ///
    /// The value is a `{ i8* code, i8* env }` pair, and the environment is its
    /// second half. A value with no environment -- a named function -- has
    /// nothing to count, and the runtime's retain leaves a null alone.
    pub(crate) fn retain_env(&mut self, value: &str) {
        let env = self.tmp();
        self.push(format!(
            "{env} = extractvalue {{ i8*, i8* }} {value}, 1"
        ));
        self.push(format!("call i8* @yashy_env_retain(i8* {env})"));
    }

    /// Open a scope for owned locals, pushed and popped alongside
    /// [`FnCtx::defers`].
    pub(crate) fn open_ref_scope(&mut self) {
        self.scope_refs.push(Vec::new());
    }

    /// Record that `slot` holds an owned reference in the innermost scope.
    pub(crate) fn track_ref_slot(&mut self, slot: &str, kind: RefKind) {
        self.track_ref_slot_typed(slot, kind, None);
    }

    /// Record that `slot` holds an owned reference, and the type of the value
    /// in it.
    pub(crate) fn track_ref_slot_typed(&mut self, slot: &str, kind: RefKind, ty: Option<TypeKind>) {
        if let Some(scope) = self.scope_refs.last_mut() {
            scope.push((slot.to_string(), kind, ty));
        }
    }

    /// Whether `slot` is a local that owns a reference -- the test the
    /// assignment lowering uses to decide whether the old value has to be
    /// released. A parameter's slot is not one: its reference is the caller's.
    pub(crate) fn is_tracked_ref_slot(&self, slot: &str) -> bool {
        self.scope_refs
            .iter()
            .any(|scope| scope.iter().any(|(tracked, _, _)| tracked == slot))
    }

    /// Release every owned local of the innermost scope, innermost first, and
    /// close it. Used where the scope exits on its own: the block fell through.
    pub(crate) fn close_ref_scope_released(&mut self) {
        let Some(scope) = self.scope_refs.pop() else {
            return;
        };
        for (slot, kind, ty) in scope.iter().rev() {
            self.release_slot(slot, *kind, ty.as_ref());
        }
    }

    /// Release one slot's owned reference.
    fn release_slot(&mut self, slot: &str, kind: RefKind, ty: Option<&TypeKind>) {
        match kind {
            RefKind::Str => {
                let loaded = self.tmp();
                self.push(format!("{loaded} = load i8*, ptr {slot}"));
                self.push_str_release(&loaded);
            }
            RefKind::Env => {
                let pair = self.tmp();
                self.push(format!("{pair} = load {{ i8*, i8* }}, ptr {slot}"));
                let env = self.tmp();
                self.push(format!("{env} = extractvalue {{ i8*, i8* }} {pair}, 1"));
                self.push(format!("call void @yashy_env_release(i8* {env})"));
            }
            RefKind::StructFields => {
                let repr = ty.map(repr_type).unwrap_or_default();
                let pair = self.tmp();
                self.push(format!("{pair} = load {repr}, ptr {slot}"));
                if let Some(ty) = ty {
                    release_struct_fields(self, &pair, ty);
                }
            }
        }
    }

    /// Close the innermost scope without releasing it. Used where the
    /// block was already terminated by the statement that exited it: a release
    /// emitted after a terminator is not well-formed IR.
    pub(crate) fn close_ref_scope(&mut self) {
        self.scope_refs.pop();
    }

    /// Release the innermost scope's owned locals where a terminating statement
    /// is about to branch, leaving the (now empty) scope for the block's own
    /// close to pop. The scope's defers have already run.
    pub(crate) fn release_current_ref_scope(&mut self) {
        let slots = match self.scope_refs.last_mut() {
            Some(scope) => std::mem::take(scope),
            None => return,
        };
        for (slot, kind, ty) in slots.iter().rev() {
            self.release_slot(slot, *kind, ty.as_ref());
        }
    }

    /// Open the function's entry block.
    ///
    /// Emits the `entry:` label and remembers where it landed, so
    /// [`FnCtx::finish`] can place the allocas `push_alloca` diverted.
    /// A function must call this before lowering its body and close with
    /// [`FnCtx::finish`]; the pair is what keeps an `alloca` from being emitted
    /// mid-body.
    pub fn begin_entry(&mut self) {
        self.entry_line = Some(self.module.line_count());
        self.push("entry:");
    }

    /// Allocate the parameters of `f` into fresh slots so an identifier read
    /// is a `load` from a stable address. The slots are not initialised here;
    /// [`crate::gen_fn`] stores the incoming SSA arguments into them.
    pub fn alloc_params(&mut self, f: &HirFn) -> Vec<String> {
        let mut slots = Vec::new();
        for param in &f.params {
            let slot = self.alloc_slot();
            let repr = repr_type(&param.ty.kind);
            self.push_alloca(format!("{slot} = alloca {repr}"));
            self.locals
                .insert(param.name.clone(), (slot.clone(), param.ty.kind.clone()));
            slots.push(slot);
        }
        slots
    }

    /// Record an `alloca` for the function's entry block instead of emitting it
    /// here.
    ///
    /// LLVM wants every `alloca` in the entry block. One emitted mid-body — a
    /// loop's counter slots, a `switch`'s result slot — is a *dynamic*
    /// allocation: it runs again on every pass of the enclosing block, which at
    /// `-O0` means the stack pointer is adjusted inside the loop and the slot is
    /// re-created each iteration. Diverting them here and splicing the buffer
    /// into `entry:` at the end of the function gives one static slot per
    /// function, and the entry block dominates every use so the presence of the
    /// slot at its use site is never in question.
    pub(crate) fn push_alloca(&mut self, line: impl Into<String>) {
        self.hoisted_allocas.push(line.into());
    }

    /// Splice the allocas diverted during body lowering into the entry block.
    ///
    /// The counterpart of [`FnCtx::begin_entry`]. One static slot per function
    /// ends up at the top of `entry:`, which dominates every use, so the slot's
    /// presence at its use site is never in question. Allocas gathered without
    /// a `begin_entry` to place them are dropped — there is no valid position
    /// for an `alloca` outside the entry block, so emitting them late would be
    /// worse than not emitting them.
    pub fn finish(&mut self) {
        let allocas = std::mem::take(&mut self.hoisted_allocas);
        if let Some(line) = self.entry_line {
            self.module.insert_lines(line + 1, allocas);
        }
    }

    /// A fresh alloca name, `%slot0`, `%slot1`, …
    pub fn alloc_slot(&mut self) -> String {
        let name = format!("%slot{}", self.next_slot);
        self.next_slot += 1;
        name
    }

    /// A fresh label base, `bb0`, `bb1`, … so nested blocks do not collide.
    pub(crate) fn alloc_label_base(&mut self) -> String {
        let name = format!("bb{}", self.next_label);
        self.next_label += 1;
        name
    }

    /// Append one IR line to the module.
    pub(crate) fn push(&mut self, line: impl Into<String>) {
        self.module.push(line);
    }

    /// The next SSA temporary, `%t0`, `%t1`, …
    pub(crate) fn tmp(&mut self) -> String {
        self.module.next_value()
    }
}

/// What kind of reference a value owns.
///
/// A `string` owns a reference to its bytes; a closure owns a reference to
/// the environment its captured values live in; a struct owns one reference to
/// each of the strings in its fields. The three are released by different
/// calls, so the owner has to say which one it is holding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefKind {
    /// A `string`: released with `yashy_str_release`.
    Str,
    /// A closure's environment: released with `yashy_env_release`.
    Env,
    /// A struct that owns the references in its `string` fields.
    StructFields,
}

/// An owned reference, and the type of the value that owns it.
///
/// The type is only needed by the kinds that have to look inside the value --
/// a struct's fields, for instance -- so it is carried only when there is one.
#[derive(Clone, Debug)]
struct OwnedRef {
    kind: RefKind,
    ty: Option<TypeKind>,
}

/// Retain the references a struct value owns, so a copy of it has its own.
///
/// The value is the copy's: the original still owns the references it had,
/// and the copy owns new ones. Only the `string` fields are references; the
/// others are numbers.
pub(crate) fn retain_struct_fields(ctx: &mut FnCtx<'_>, value: &str, ty: &TypeKind) {
    let TypeKind::Struct(name) = ty else {
        return;
    };
    let repr = repr_type(ty);
    let fields: Vec<TypeKind> = ctx.module.struct_fields(name).to_vec();
    for (index, field_ty) in fields.iter().enumerate() {
        if !matches!(field_ty, TypeKind::Str) {
            continue;
        }
        let extracted = ctx.tmp();
        ctx.push(format!("{extracted} = extractvalue {repr} {value}, {index}"));
        let retained = ctx.tmp();
        ctx.push_str_retain_into(&retained, &extracted);
    }
}

/// Release the references a struct value owns.
fn release_struct_fields(ctx: &mut FnCtx<'_>, value: &str, ty: &TypeKind) {
    let TypeKind::Struct(name) = ty else {
        return;
    };
    let repr = repr_type(ty);
    let fields: Vec<TypeKind> = ctx.module.struct_fields(name).to_vec();
    for (index, field_ty) in fields.iter().enumerate() {
        if !matches!(field_ty, TypeKind::Str) {
            continue;
        }
        let extracted = ctx.tmp();
        ctx.push(format!("{extracted} = extractvalue {repr} {value}, {index}"));
        ctx.push_str_release(&extracted);
    }
}

/// Lower `expr` to a value, pushing its instructions into the module.
pub fn gen_expr(ctx: &mut FnCtx<'_>, expr: &HirExpr) -> GenValue {
    match expr {
        HirExpr::Literal(lit, _, ty) => gen_literal(ctx, lit, &ty.kind),
        HirExpr::AddrOf { name, ty, .. } => match ctx.locals.get(name).cloned() {
            // The local's slot *is* the storage the pointer designates, so the
            // address is the alloca's own name -- there is nothing to emit.
            Some((slot, _)) if !slot.is_empty() => GenValue {
                value: slot,
                ty: ty.kind.clone(),
            },
            // A pointer receiver can only be called on a binding the backend
            // has a slot for; the checker has already refused anything else.
            // A null here would be a dereference waiting to happen, so the
            // value says so rather than pretending.
            _ => GenValue {
                value: "null".to_string(),
                ty: ty.kind.clone(),
            },
        },
        HirExpr::Ident(path, _, ty) => {
            let name = path.last().cloned().unwrap_or_default();
            match ctx.locals.get(&name).cloned() {
                Some((slot, slot_ty))
                    if !slot.is_empty() && repr_type(&slot_ty) != "void" =>
                {
                    let tmp = ctx.tmp();
                    let repr = repr_type(&slot_ty);
                    ctx.push(format!("{tmp} = load {repr}, ptr {slot}"));
                    GenValue {
                        value: tmp,
                        ty: slot_ty,
                    }
                }
                // A top-level function name is not in `locals`; it becomes a
                // function value by taking the address of a synthesized thunk.
                // The thunk adds the environment parameter every indirect call
                // passes, and this value pairs it with a null environment.
                _ if matches!(ty.kind, TypeKind::Func(_, _)) => {
                    match ctx.module.fn_value_thunk(&name) {
                        Some(thunk) => {
                            let repr = "{ i8*, i8* }";
                            let with_code = ctx.tmp();
                            ctx.push(format!(
                                "{with_code} = insertvalue {repr} undef, i8* @{thunk}, 0"
                            ));
                            let with_env = ctx.tmp();
                            ctx.push(format!(
                                "{with_env} = insertvalue {repr} {with_code}, i8* null, 1"
                            ));
                            GenValue {
                                value: with_env,
                                ty: ty.kind.clone(),
                            }
                        }
                        None => zero_of(&ty.kind, ctx),
                    }
                }
                // `None` (and any other value-position variant name) has no
                // slot, and a `()` binding has no loadable value; both take a
                // zero of the type the expression already carries.
                _ => zero_of(&ty.kind, ctx),
            }
        }
        HirExpr::Binary {
            op,
            lhs,
            rhs,
            fault,
            ..
        } => gen_binary(ctx, *op, lhs, rhs, fault.as_ref()),
        HirExpr::Unary {
            op, operand, fault, ..
        } => gen_unary(ctx, *op, operand, fault.as_ref()),
        HirExpr::Move(inner, _, ty) => crate::memory::gen_move(ctx, inner, &ty.kind),
        HirExpr::Call {
            callee,
            args,
            unsafe_call,
            ty,
            ..
        } => gen_call(ctx, callee, args, &ty.kind, *unsafe_call),
        HirExpr::StructLit {
            path, fields, ty, ..
        } => gen_struct_lit(ctx, path, fields, &ty.kind),
        HirExpr::Lambda { f, ty, .. } => gen_lambda(ctx, f, &ty.kind),
        HirExpr::Capture { index, ty, .. } => gen_capture_read(ctx, *index, &ty.kind),
        HirExpr::CallCapture { index, args, ty, .. } => {
            gen_capture_call(ctx, *index, args, &ty.kind)
        }
        HirExpr::HigherOrder {
            op,
            receiver,
            func,
            init,
            ty,
            ..
        } => crate::higher_order::gen(ctx, *op, receiver, func, init.as_deref(), &ty.kind),
        HirExpr::SliceLit {
            elem_ty,
            elements,
            ty,
            ..
        } => crate::slices::gen_literal(ctx, &elem_ty.kind, elements, &ty.kind),
        HirExpr::FixedArrayLit {
            elem_ty,
            elements,
            ty,
            ..
        } => crate::fixed::gen_literal(ctx, &elem_ty.kind, elements, &ty.kind),
        // A tuple is built the way a fixed array is: one `insertvalue` per
        // element onto `undef`. Nothing is allocated, so the value is the
        // aggregate itself and a copy is a store.
        HirExpr::TupleLit { elements, ty, .. } => {
            let repr = repr_type(&ty.kind);
            let mut value = "undef".to_string();
            for (index, element) in elements.iter().enumerate() {
                let element_value = gen_expr(ctx, element);
                let element_repr = repr_type(&element_value.ty);
                let next = ctx.tmp();
                ctx.push(format!(
                    "{next} = insertvalue {repr} {value}, {element_repr} {}, {index}",
                    element_value.value
                ));
                value = next;
            }
            GenValue {
                value,
                ty: ty.kind.clone(),
            }
        }
        HirExpr::View {
            base, start, end, ty, ..
        } => crate::fixed::gen_view(ctx, base, start, end, &ty.kind),
        // A struct value is an aggregate, so reading one field of it is
        // `extractvalue` -- no address, no alloca, no GEP. The index was resolved
        // against the declaration order in the checker (see `HirExpr::FieldAccess`).
        HirExpr::FieldAccess {
            base, index, ty, ..
        } => {
            let value = gen_expr(ctx, base);
            let repr = repr_type(&value.ty);
            let tmp = ctx.tmp();
            ctx.push(format!("{tmp} = extractvalue {repr} {}, {index}", value.value));
            GenValue {
                value: tmp,
                ty: ty.kind.clone(),
            }
        }
        HirExpr::Index { base, index, ty, .. } => {
            match &expr_ty(base).kind {
                // `xs[i]` is an `Option`: the runtime bounds test decides.
                TypeKind::Slice(_) => {
                    return crate::slices::gen_index(ctx, base, index, &ty.kind);
                }
                // A fixed array or a view is checked against a length the
                // backend already has: a constant in the type, or the word the
                // view carries.
                TypeKind::FixedArray(_, _) | TypeKind::View(_) => {
                    return crate::fixed::gen_index(ctx, base, index, &ty.kind);
                }
                _ => {}
            }
            // `p[i]` reads the element at an address: getelementptr to the
            // element's address, then load. The element is whatever the
            // pointer points at, so the width comes from its representation.
            let ptr = gen_expr(ctx, base);
            let idx = gen_expr(ctx, index);
            let element = match &ptr.ty {
                TypeKind::Ptr(inner) => (**inner).clone(),
                _ => ty.clone(),
            };
            let repr = crate::ir::repr_type(&element.kind);
            let addr = ctx.tmp();
            ctx.push(format!(
                "{addr} = getelementptr {repr}, ptr {}, i64 {}",
                ptr.value, idx.value
            ));
            let value = ctx.tmp();
            ctx.push(format!("{value} = load {repr}, ptr {addr}"));
            GenValue {
                value,
                ty: ty.kind.clone(),
            }
        }
        HirExpr::Cast { expr, ty, .. } => gen_cast(ctx, expr, &ty.kind),
        HirExpr::Try { expr, ty, .. } => gen_try(ctx, expr, &ty.kind),
        HirExpr::SwitchExpr {
            subject, arms, ty, ..
        } => {
            let subject_value = gen_expr(ctx, subject);
            let result_slot = crate::patterns::gen_match(ctx, &subject_value, arms, &ty.kind);
            let repr = repr_type(&ty.kind);
            if repr == "void" || result_slot.is_empty() {
                return GenValue {
                    value: String::new(),
                    ty: ty.kind.clone(),
                };
            }
            let tmp = ctx.tmp();
            ctx.push(format!("{tmp} = load {repr}, ptr {result_slot}"));
            GenValue {
                value: tmp,
                ty: ty.kind.clone(),
            }
        }
    }
}

/// Lower `expr` into a fresh alloca slot, returning the slot's name.
///
/// A `()`-valued expression emits no slot and returns `""`: `void` cannot be
/// allocated, and its "value" is the effect of the instructions already
/// pushed.
pub fn gen_expr_to_slot(ctx: &mut FnCtx<'_>, expr: &HirExpr) -> String {
    let value = gen_expr(ctx, expr);
    let repr = repr_type(&value.ty);
    if repr == "void" || value.value.is_empty() {
        return String::new();
    }
    let slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{slot} = alloca {repr}"));
    ctx.push(format!("store {repr} {}, ptr {slot}", value.value));
    slot
}

/// A zero of the given type, used where a value is needed but the construct
/// that would produce it is not lowered yet.
///
/// Scalars get their literal zero; pointers get `null`; aggregates get
/// `undef`. No instruction is emitted, so a fallback can never produce
/// invalid IR.
fn zero_of(ty: &TypeKind, _ctx: &mut FnCtx<'_>) -> GenValue {
    let repr = repr_type(ty);
    let value = match repr.as_str() {
        "void" => String::new(),
        "i64" => "0".to_string(),
        "double" => "0.0".to_string(),
        "i1" => "false".to_string(),
        other if other.ends_with('*') => "null".to_string(),
        "float" => "0.0".to_string(),
        other if other.starts_with('i') => "0".to_string(),
        _ => "undef".to_string(),
    };
    GenValue {
        value,
        ty: ty.clone(),
    }
}

/// Lower a literal. Strings are interned as globals and decay to their data
/// pointer (see [`repr_type`]).
pub(crate) fn gen_literal(ctx: &mut FnCtx<'_>, lit: &Literal, ty: &TypeKind) -> GenValue {
    match lit {
        Literal::I64(v) => GenValue {
            value: v.to_string(),
            ty: ty.clone(),
        },
        Literal::F64(v) => GenValue {
            value: llvm_float(*v),
            ty: ty.clone(),
        },
        Literal::Bool(b) => GenValue {
            value: if *b { "1".to_string() } else { "0".to_string() },
            ty: ty.clone(),
        },
        Literal::Str(s) => GenValue {
            value: ctx.module.intern_bytes_ptr(&s.bytes),
            ty: ty.clone(),
        },
    }
}

/// Format an `f64` as an LLVM float literal.
///
/// Rust's `Debug` output is nearly right, but LLVM's float grammar requires a
/// decimal point in the mantissa, and `Debug` omits it in two cases: whole
/// numbers (`2` -> needs `2.0`) and exponent notation (`1e-7` -> needs
/// `1.0e-7`). A literal like `1e-7` is not merely unconventional, it is
/// rejected: LLVM lexes the leading `1` as an integer constant and then
/// reports "integer/byte constant must have integer/byte type". Non-finite
/// values are spelled as bit patterns, since LLVM has no `inf`/`NaN` words.
fn llvm_float(v: f64) -> String {
    if v.is_nan() {
        return "0x7FF8000000000000".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 {
            "0x7FF0000000000000".to_string()
        } else {
            "0xFFF0000000000000".to_string()
        };
    }
    let text = format!("{v:?}");
    let (mantissa, exponent) = match text.find(['e', 'E']) {
        Some(at) => text.split_at(at),
        None => (text.as_str(), ""),
    };
    if mantissa.contains('.') {
        text
    } else {
        format!("{mantissa}.0{exponent}")
    }
}

/// Lower a binary operation. Comparisons pick `icmp`/`fcmp` with the
/// signed/ordered predicates, `&&`/`||` are `and`/`or` on `i1`, and float
/// arithmetic needs its own mnemonics for *every* operator, not just division:
/// `add`/`sub`/`mul` accept only integer operands, so a malformed `add double`
/// is a parse error from LLVM rather than something that silently works. The
/// `is_float` test is therefore applied uniformly below.
///
/// Floating-point arithmetic also carries the `contract` fast-math flag when the
/// module allows contraction; see [`crate::CodegenOptions::fp_contract`].
///
/// # Integer arithmetic is checked
///
/// `fault` is the position the checker resolved for an operation that can go
/// wrong, and it is `Some` for exactly the integer arithmetic (`+ - * / %`,
/// negation). It is handled first, in [`crate::faults`], because the check
/// decides the *shape* of the block rather than just the mnemonic: the check is
/// emitted, the operation is emitted behind it, and the continuation is a new
/// block. A float `add` or a comparison cannot fault and takes the table below,
/// unchanged by any of this.
fn gen_binary(
    ctx: &mut FnCtx<'_>,
    op: BinOp,
    lhs: &HirExpr,
    rhs: &HirExpr,
    fault: Option<&FaultSite>,
) -> GenValue {
    // `&&` and `||` short-circuit: the right operand is evaluated only when
    // the left one does not decide the answer. That is a branch, not an
    // instruction, so these two are lowered before the operands are.
    if matches!(op, BinOp::And | BinOp::Or) {
        return gen_short_circuit(ctx, op, lhs, rhs);
    }
    let l = gen_expr(ctx, lhs);
    let r = gen_expr(ctx, rhs);
    // A `string` has no instruction-level operator: `+` concatenates and
    // `==`/`!=` compare bytes, both through the runtime. Handling it before the
    // numeric table below keeps that table about numbers.
    if matches!(l.ty, TypeKind::Str) {
        return crate::strings::gen_string_binary(ctx, op, &l, &r);
    }
    if let Some(site) = fault {
        if let Some(checked) = crate::faults::gen_checked_binary(ctx, op, &l, &r, site) {
            return checked;
        }
    }
    let is_float = matches!(l.ty, TypeKind::F64);
    let ty = l.ty.clone();
    let (mnemonic, result_ty) = match op {
        BinOp::Add => (if is_float { "fadd" } else { "add" }, ty.clone()),
        BinOp::Sub => (if is_float { "fsub" } else { "sub" }, ty.clone()),
        BinOp::Mul => (if is_float { "fmul" } else { "mul" }, ty.clone()),
        BinOp::Div => (if is_float { "fdiv" } else { "sdiv" }, ty.clone()),
        BinOp::Rem => (if is_float { "frem" } else { "srem" }, ty.clone()),
        BinOp::And => ("and", TypeKind::Bool),
        BinOp::Or => ("or", TypeKind::Bool),
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            let pred = cmp_predicate(op, is_float);
            let cmp_ty = repr_type(&l.ty);
            let tmp = ctx.tmp();
            ctx.push(format!("{tmp} = {pred} {cmp_ty} {}, {}", l.value, r.value));
            return GenValue {
                value: tmp,
                ty: TypeKind::Bool,
            };
        }
    };
    let op_ty = repr_type(&ty);
    let tmp = ctx.tmp();
    // A multiply and an add or subtract are the one pair LLVM's `contract` flag
    // can fuse, and only when the caller has allowed it. `contract` is a valid
    // flag on `fdiv` and `frem` too, but neither can fuse with anything, so
    // adding it there would be noise in the IR rather than a decision.
    let contract = if is_float
        && ctx.module.fp_contract()
        && matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul)
    {
        " contract"
    } else {
        ""
    };
    ctx.push(format!(
        "{tmp} = {mnemonic}{contract} {op_ty} {}, {}",
        l.value, r.value
    ));
    GenValue {
        value: tmp,
        ty: result_ty,
    }
}

/// The compare predicate for a comparison operator. Comparisons are the only
/// ops that reach here; the fallback arm is unreachable but total.
fn cmp_predicate(op: BinOp, is_float: bool) -> String {
    let (int, float) = match op {
        BinOp::Eq => ("eq", "oeq"),
        BinOp::Ne => ("ne", "one"),
        BinOp::Lt => ("slt", "olt"),
        BinOp::Le => ("sle", "ole"),
        BinOp::Gt => ("sgt", "ogt"),
        BinOp::Ge => ("sge", "oge"),
        _ => ("eq", "oeq"),
    };
    if is_float {
        format!("fcmp {float}")
    } else {
        format!("icmp {int}")
    }
}

/// Lower a unary operation: `Neg` is `sub`/`fsub` from zero, `Not` is `xor`
/// with `true` on `i1`.
///
/// Integer negation is checked, like every other integer operation: `-i64::MIN`
/// has no answer in `i64`. `fault` carries the position the checker resolved
/// when the operation can go wrong, and [`crate::faults`] lowers it; `!` and
/// float negation have nothing to check.
fn gen_unary(
    ctx: &mut FnCtx<'_>,
    op: UnOp,
    operand: &HirExpr,
    fault: Option<&FaultSite>,
) -> GenValue {
    let value = gen_expr(ctx, operand);
    if let Some(site) = fault {
        if let Some(checked) = crate::faults::gen_checked_neg(ctx, op, &value, site) {
            return checked;
        }
    }
    let tmp = ctx.tmp();
    match op {
        UnOp::Neg => {
            let repr = repr_type(&value.ty);
            let (mnemonic, zero) = if repr == "double" {
                ("fsub", "0.0".to_string())
            } else {
                ("sub", "0".to_string())
            };
            ctx.push(format!(
                "{tmp} = {mnemonic} {repr} {zero}, {}",
                value.value
            ));
        }
        UnOp::Not => {
            ctx.push(format!("{tmp} = xor i1 {}, true", value.value));
        }
    }
    GenValue {
        value: tmp,
        ty: value.ty,
    }
}

/// Lower a lambda: emit it as a private function and yield its value.
///
/// The value is the same pair every function value is -- the code pointer and
/// the environment -- so a lambda can be passed, bound and called exactly like
/// a named function. A lambda that captures nothing has a null environment: it
/// has nothing to carry.
fn gen_lambda(ctx: &mut FnCtx<'_>, f: &HirFn, ty: &TypeKind) -> GenValue {
    let index = ctx.module.next_lambda();
    let name = format!("__lambda_{index}");
    // The environment: the captured values, copied into a block the closure
    // carries. It is built before the body, because the body is lowered into
    // its own function and the two have to agree on the layout.
    let env = gen_env(ctx, f);
    // The body is written into a buffer: it is lowered while the function
    // that contains the lambda is still open, and a `define` block cannot be
    // written inside a function.
    ctx.module.begin_capture();
    crate::fn_gen::gen_lambda(ctx.module, f, &name);
    let lines = ctx.module.end_capture();
    ctx.module.add_synthesized_function(lines);

    let repr = "{ i8*, i8* }";
    let with_code = ctx.tmp();
    ctx.push(format!("{with_code} = insertvalue {repr} undef, i8* @{name}, 0"));
    let with_env = ctx.tmp();
    ctx.push(format!("{with_env} = insertvalue {repr} {with_code}, i8* {env}, 1"));
    if f.captures.is_empty() {
        GenValue {
            value: with_env,
            ty: ty.clone(),
        }
    } else {
        // The value owns the environment: the block was created with one
        // reference, and this is that reference.
        ctx.mark_owned(&with_env, RefKind::Env);
        GenValue {
            value: with_env,
            ty: ty.clone(),
        }
    }
}

/// Build the environment a closure carries, and return its pointer.
///
/// The captured values are copied out of the scope the closure is written in
/// and into the block, and the block's table says which of them the
/// environment owns a reference to.
fn gen_env(ctx: &mut FnCtx<'_>, f: &HirFn) -> String {
    if f.captures.is_empty() {
        return "null".to_string();
    }
    let offsets = crate::env::capture_offsets(&f.captures);
    let payload = crate::env::payload_size(&f.captures);
    let env = ctx.tmp();
    ctx.push(format!(
        "{env} = call i8* @yashy_env_new(i64 {}, i64 {payload}, i64 16)",
        f.captures.len()
    ));

    for (index, capture) in f.captures.iter().enumerate() {
        let repr = repr_type(&capture.ty.kind);
        // Read the captured value from the scope the closure is written in.
        let value = match &capture.source {
            CaptureSource::Scope => {
                let slot = ctx
                    .locals
                    .get(&capture.name)
                    .map(|(slot, _)| slot.clone())
                    .unwrap_or_default();
                if slot.is_empty() {
                    "null".to_string()
                } else {
                    let loaded = ctx.tmp();
                    ctx.push(format!("{loaded} = load {repr}, ptr {slot}"));
                    loaded
                }
            }
            CaptureSource::Enclosing(parent) => {
                // A capture of the enclosing closure: the value is in that
                // closure's environment, and the closure being made is inside
                // that closure's body.
                let env_name = ctx.env.clone().unwrap_or_default();
                let offset = ctx
                    .capture_offsets
                    .get(*parent)
                    .copied()
                    .unwrap_or_default();
                let address = ctx.tmp();
                ctx.push(format!(
                    "{address} = getelementptr i8, ptr {env_name}, i64 {offset}"
                ));
                let loaded = ctx.tmp();
                ctx.push(format!("{loaded} = load {repr}, ptr {address}"));
                loaded
            }
        };

        // Store it into the environment's payload.
        let address = ctx.tmp();
        ctx.push(format!(
            "{address} = getelementptr i8, ptr {env}, i64 {}",
            offsets[index]
        ));
        ctx.push(format!("store {repr} {value}, ptr {address}"));

        // The table entry: what the environment releases when it dies.
        let slot = ctx.tmp();
        ctx.push(format!(
            "{slot} = getelementptr i8, ptr {env}, i64 {}",
            crate::env::slot_offset(index)
        ));
        if crate::env::is_string(capture) {
            // The environment owns a reference to the string it captured.
            ctx.push_str_retain(&value);
            ctx.push(format!(
                "store i64 {}, ptr {slot}",
                crate::env::SLOT_STRING
            ));
            let pointer = ctx.tmp();
            ctx.push(format!("{pointer} = getelementptr i8, ptr {slot}, i64 8"));
            ctx.push(format!("store i8* {value}, ptr {pointer}"));
        } else {
            ctx.push(format!(
                "store i64 {}, ptr {slot}",
                crate::env::SLOT_NONE
            ));
        }
    }
    env
}

/// Lower a read of a captured value: the value is in the environment, not in
/// a slot.
fn gen_capture_read(ctx: &mut FnCtx<'_>, index: usize, ty: &TypeKind) -> GenValue {
    let Some(env_name) = ctx.env.clone() else {
        return zero_of(ty, ctx);
    };
    let Some(offset) = ctx.capture_offsets.get(index).copied() else {
        return zero_of(ty, ctx);
    };
    let repr = repr_type(ty);
    let address = ctx.tmp();
    ctx.push(format!(
        "{address} = getelementptr i8, ptr {env_name}, i64 {offset}"
    ));
    let value = ctx.tmp();
    ctx.push(format!("{value} = load {repr}, ptr {address}"));
    GenValue {
        value,
        ty: ty.clone(),
    }
}

/// Lower a call through a captured function value.
fn gen_capture_call(
    ctx: &mut FnCtx<'_>,
    index: usize,
    args: &[HirExpr],
    ty: &TypeKind,
) -> GenValue {
    let Some(capture) = ctx.captures.get(index).cloned() else {
        return zero_of(ty, ctx);
    };
    let TypeKind::Func(_, ret) = capture.ty.kind.clone() else {
        return zero_of(ty, ctx);
    };
    let value = gen_capture_read(ctx, index, &capture.ty.kind);
    gen_indirect_call(ctx, &value.value, &ret.kind, args, ty)
}

/// Lower a call: the three runtime builtins, the `Option`/`Result`
/// constructors, and user functions.
/// Lower `Enum.Variant(args)` for a user-declared enum.
///
/// The checker has already matched the arguments to the variant's fields and
/// ordered them; the backend only needs the tag and where the payload starts.
/// `None` means "not an enum construction", so the ordinary call path can
/// report an unknown callee exactly as before.
fn gen_enum_ctor(
    ctx: &mut FnCtx<'_>,
    callee: &[String],
    args: &[HirExpr],
    ty: &TypeKind,
) -> Option<GenValue> {
    let TypeKind::Enum(enum_name) = ty else {
        return None;
    };
    if callee.len() != 2 || &callee[0] != enum_name {
        return None;
    }
    let variants = ctx.module.enum_variants(ty)?.to_vec();
    let tag = u8::try_from(variants.iter().position(|v| v.name == callee[1])?).ok()?;
    let mut field = 1u32;
    for variant in &variants {
        if variant.name == callee[1] {
            break;
        }
        field += variant.fields.len() as u32;
    }
    let repr = repr_type(ty);
    let mut acc = ctx.tmp();
    ctx.push(format!("{acc} = insertvalue {repr} undef, i8 {tag}, 0"));
    for (offset, arg) in args.iter().enumerate() {
        let value = gen_expr(ctx, arg);
        let next = ctx.tmp();
        ctx.push(format!(
            "{next} = insertvalue {repr} {acc}, {} {}, {}",
            repr_type(&value.ty),
            value.value,
            field + offset as u32
        ));
        acc = next;
    }
    Some(GenValue {
        value: acc,
        ty: ty.clone(),
    })
}

/// Lower `a && b` and `a || b`, which evaluate the right operand only when the
/// left one does not decide the answer.
///
/// The value is a phi of the two ways out: the left operand's answer when it
/// decides, and the right operand's when it does not. The right operand is
/// evaluated in a block of its own, so the branch around it is what keeps it
/// from running when it should not.
fn gen_short_circuit(
    ctx: &mut FnCtx<'_>,
    op: BinOp,
    lhs: &HirExpr,
    rhs: &HirExpr,
) -> GenValue {
    let l = gen_expr(ctx, lhs);
    let rhs_label = ctx.alloc_label_base() + "_rhs";
    let short_label = ctx.alloc_label_base() + "_short";
    let end_label = ctx.alloc_label_base() + "_end";
    // The answer lives in a slot rather than in a phi: the right operand may
    // be a checked operation with a fault path of its own, and the block the
    // value comes from is then not the block the branch names. A slot does not
    // care which block the value was made in.
    let slot = ctx.alloc_slot();
    ctx.push_alloca(format!("{slot} = alloca i1"));
    // `a && b` is false when `a` is false; `a || b` is true when `a` is true.
    let (short_value, branch) = match op {
        BinOp::And => (
            "false",
            format!(
                "br i1 {}, label %{rhs_label}, label %{short_label}",
                l.value
            ),
        ),
        _ => (
            "true",
            format!(
                "br i1 {}, label %{short_label}, label %{rhs_label}",
                l.value
            ),
        ),
    };
    ctx.push(branch);
    ctx.push(format!("{short_label}:"));
    ctx.push(format!("store i1 {short_value}, ptr {slot}"));
    ctx.push(format!("br label %{end_label}"));
    ctx.push(format!("{rhs_label}:"));
    let r = gen_expr(ctx, rhs);
    ctx.push(format!("store i1 {}, ptr {slot}", r.value));
    ctx.push(format!("br label %{end_label}"));
    ctx.push(format!("{end_label}:"));
    let result = ctx.tmp();
    ctx.push(format!("{result} = load i1, ptr {slot}"));
    GenValue {
        value: result,
        ty: TypeKind::Bool,
    }
}

/// Lower a call through a function value.
///
/// The value is `{ i8* code, i8* env }`; the call passes `env` as its first
/// argument and then the lowered arguments. String arguments are borrowed for
/// the duration of the call, exactly as in [`gen_borrowed_call`], so an owned
/// temporary is released once the callee returns.
fn gen_indirect_call(
    ctx: &mut FnCtx<'_>,
    pair: &str,
    ret: &TypeKind,
    args: &[HirExpr],
    ty: &TypeKind,
) -> GenValue {
    let fn_repr = "{ i8*, i8* }";
    let code = ctx.tmp();
    ctx.push(format!("{code} = extractvalue {fn_repr} {pair}, 0"));
    let env = ctx.tmp();
    ctx.push(format!("{env} = extractvalue {fn_repr} {pair}, 1"));

    let mut call_args = vec![format!("i8* {env}")];
    let mut lowered = Vec::with_capacity(args.len());
    for argument in args {
        let value = gen_expr(ctx, argument);
        call_args.push(format!("{} {}", repr_type(&value.ty), value.value));
        lowered.push(value);
    }

    let ret_repr = if matches!(ret, TypeKind::Unit | TypeKind::Never) {
        "void".to_string()
    } else {
        repr_type(ret)
    };
    let result = if ret_repr == "void" {
        ctx.push(format!("call void {code}({})", call_args.join(", ")));
        String::new()
    } else {
        let result = ctx.tmp();
        ctx.push(format!(
            "{result} = call {ret_repr} {code}({})",
            call_args.join(", ")
        ));
        result
    };
    for argument in &lowered {
        ctx.release_owned(&argument.value);
    }
    if ret_repr != "void" && matches!(ret, TypeKind::Str) {
        ctx.mark_owned(&result, RefKind::Str);
    }
    if ret_repr != "void" && matches!(ret, TypeKind::Func(_, _)) {
        // A returned closure: the caller owns the reference the callee had.
        ctx.mark_owned(&result, RefKind::Env);
    }
    GenValue {
        value: result,
        ty: ty.clone(),
    }
}

fn gen_call(
    ctx: &mut FnCtx<'_>,
    callee: &[String],
    args: &[HirExpr],
    ty: &TypeKind,
    unsafe_call: bool,
) -> GenValue {
    if let Some(constructed) = gen_enum_ctor(ctx, callee, args, ty) {
        return constructed;
    }
    // A one-segment callee that names a local binding of function type is an
    // indirect call. A top-level function is not in `locals`, so it keeps the
    // direct-call path below.
    if callee.len() == 1 {
        if let Some(name) = callee.first() {
            if let Some((slot, TypeKind::Func(_, ret))) = ctx.locals.get(name).cloned() {
                let pair = ctx.tmp();
                ctx.push(format!("{pair} = load {{ i8*, i8* }}, ptr {slot}"));
                return gen_indirect_call(ctx, &pair, &ret.kind, args, ty);
            }
        }
    }
    let name = callee.last().cloned().unwrap_or_default();
    match name.as_str() {
        // `alloc` and `dealloc` are calls to the runtime's heap (ledger L1),
        // not C functions with matching names: `alloc(n)` counts elements and
        // the runtime counts bytes, so the shape changes here.
        "alloc" => crate::memory::gen_alloc(ctx, args, ty),
        "dealloc" => crate::memory::gen_dealloc(ctx, args),
        "push" => crate::slices::gen_push(ctx, args),
        "pop" => crate::slices::gen_pop(ctx, args, ty),
        "slice_set" => crate::slices::gen_set(ctx, args),
        "slice_insert" => crate::slices::gen_insert(ctx, args),
        "slice_remove" => crate::slices::gen_remove(ctx, args, ty),
        "slice_clear" => crate::slices::gen_clear(ctx, args),
        "slice_reverse" => crate::slices::gen_reverse(ctx, args),
        "slice_sort" => crate::slices::gen_sort(ctx, args),
        "slice_contains" => crate::slices::gen_contains(ctx, args),
        "slice_index_of" => crate::slices::gen_index_of(ctx, args, ty),
        "slice_extend" => crate::slices::gen_extend(ctx, args),
        "slice_copy" => crate::slices::gen_copy(ctx, args, ty),
        "free_slice" => crate::slices::gen_free(ctx, args),
        "view_len" => crate::fixed::gen_len(ctx, args),
        "map_new" => crate::maps::gen_new(ctx, ty),
        "map_insert" => crate::maps::gen_insert(ctx, args),
        "map_get" => crate::maps::gen_get(ctx, args, ty),
        "map_remove" => crate::maps::gen_remove(ctx, args, ty),
        "map_len" => crate::maps::gen_len(ctx, args),
        "map_free" => crate::maps::gen_free(ctx, args),
        "map_keys" => crate::maps::gen_keys(ctx, args, ty),
        "map_values" => crate::maps::gen_values(ctx, args, ty),
        "map_contains_key" => crate::maps::gen_contains_key(ctx, args),
        // Threads, mutexes and channels (M7). `thread_spawn` is the only one
        // that synthesizes a function; the rest are one runtime call each.
        "thread_spawn" => {
            let Some((body, rest)) = args.split_first() else {
                return unit();
            };
            crate::threads::gen_thread_spawn(ctx, body, rest)
        }
        "thread_join" => crate::threads::gen_thread_join(ctx, args),
        "mutex_new" => crate::threads::gen_mutex_new(ctx),
        "mutex_lock" => crate::threads::gen_mutex_call(ctx, args, "yashy_mutex_lock"),
        "mutex_unlock" => crate::threads::gen_mutex_call(ctx, args, "yashy_mutex_unlock"),
        "mutex_free" => crate::threads::gen_mutex_call(ctx, args, "yashy_mutex_free"),
        "chan_new" => crate::threads::gen_chan_new(ctx, args, ty),
        "chan_send" => crate::threads::gen_chan_send(ctx, args),
        "chan_recv" => crate::threads::gen_chan_recv(ctx, args, ty),
        "chan_close" => crate::threads::gen_chan_call(ctx, args, "yashy_chan_close"),
        "chan_free" => crate::threads::gen_chan_call(ctx, args, "yashy_chan_free"),
        "arena_new" => crate::arena::gen_arena_new(ctx, args, ty),
        "arena_alloc" => crate::arena::gen_arena_alloc(ctx, args, ty),
        "arena_free_all" => crate::arena::gen_arena_free_all(ctx, args),
        // `panic` never returns, so everything after it in the enclosing block
        // is dead. The call and its `unreachable` go into a block of their own
        // rather than into the current one: emitting `unreachable` here would
        // terminate the block and leave the caller appending instructions after
        // a terminator, which is not well-formed IR -- and the caller can be
        // anywhere, including the argument list of another call. So this block
        // branches to the panic block, and a fresh block is opened for whatever
        // follows. That continuation is unreachable -- the process ends inside
        // `yashy_panic` -- but it is where the rest of the enclosing expression
        // belongs, and no later stage has to know a panic happened at all. The
        // same shape is what every checked arithmetic operation uses; see
        // [`crate::faults`].
        //
        // The four arguments are the *checker's*, not the source's: it resolved
        // the message plus the file, line and column the panic was written at,
        // because the backend has no source map to attribute a runtime failure
        // with. See `Checker::panic_call`.
        "panic" => {
            let lowered: Vec<GenValue> = args.iter().map(|a| gen_expr(ctx, a)).collect();
            let call = panic_call(&lowered, 0);
            let fault = crate::faults::fault_label(ctx);
            let after_label = format!("{}_after_panic", ctx.alloc_label_base());
            ctx.push(format!("br label %{fault}"));
            crate::faults::close_fault(ctx, &fault, |ctx| {
                crate::stmt_gen::run_all_scopes_defers(ctx);
                call.clone()
            });
            ctx.push(format!("{after_label}:"));
            GenValue {
                value: "undef".to_string(),
                ty: TypeKind::Never,
            }
        }
        // `assert`, `assert_eq` and `assert_ne` (spec 6.2) are the same shape as
        // `panic` with a branch in front of the reporting block, so the passing
        // path pays one test and the reporting path is cold.
        "assert" => gen_assert(ctx, args),
        "assert_eq" | "assert_ne" => gen_assert_cmp(ctx, &name, args),
        // `os.exit(code)` (spec 6.3): the same shape as `panic` -- the call ends
        // the process, so it goes in a block of its own and the caller continues
        // in an unreachable one -- without the message. `exit` is `!` in the
        // checker, so `undef` is the right answer for a value that never exists.
        "exit" => {
            let lowered: Vec<GenValue> = args.iter().map(|a| gen_expr(ctx, a)).collect();
            let code = lowered
                .first()
                .map(operand_of)
                .unwrap_or_else(|| "i64 0".to_string());
            let call = format!("call void @yashy_exit({code})");
            let fault = crate::faults::fault_label(ctx);
            let after_label = format!("{}_after_exit", ctx.alloc_label_base());
            ctx.push(format!("br label %{fault}"));
            crate::faults::close_fault(ctx, &fault, |ctx| {
                crate::stmt_gen::run_all_scopes_defers(ctx);
                call.clone()
            });
            ctx.push(format!("{after_label}:"));
            GenValue {
                value: "undef".to_string(),
                ty: TypeKind::Never,
            }
        }
        // `debug_assert` in a release build: the checker dropped the condition
        // and handed down a call with nothing in it, so there is nothing to
        // lower. The name is kept rather than replaced by a unit literal because
        // the tree has no literal for "no value", and because a reader of the IR
        // can then see that an assertion was written and compiled away.
        "debug_assert" if args.is_empty() => unit(),
        // The opt-outs from the checked default, and the same operation the
        // checked lowering guards: `wrapping_*` is the bare opcode, `saturating_*`
        // is an LLVM saturating intrinsic, and `checked_*` spends the overflow
        // flag on a `select` instead of a fault. All nine names are one family,
        // so they are listed here once and lowered there. See [`crate::faults`].
        "wrapping_add" | "wrapping_sub" | "wrapping_mul" | "checked_add" | "checked_sub"
        | "checked_mul" | "saturating_add" | "saturating_sub" | "saturating_mul" => {
            let lowered: Vec<GenValue> = args.iter().map(|a| gen_expr(ctx, a)).collect();
            crate::faults::gen_arithmetic_builtin(ctx, &name, &lowered, ty)
        }
        // `len` is the byte length of a string and the element count of a slice.
        // The argument's checked type decides which; both are `i64`.
        "len" => {
            let is_slice = args
                .first()
                .map(|argument| matches!(expr_ty(argument).kind, TypeKind::Slice(_)))
                .unwrap_or(false);
            if is_slice {
                crate::slices::gen_len(ctx, args)
            } else {
                crate::strings::gen_str_len(ctx, args)
            }
        }
        "byte_at" => crate::strings::gen_str_byte_at(ctx, args),
        "str_get" => crate::strings::gen_str_get(ctx, args, ty),
        "str_index_of" => crate::strings::gen_str_index_of(ctx, args, ty),
        "str_split" => crate::strings::gen_str_split(ctx, args, ty),
        "str_lines" => crate::strings::gen_str_lines(ctx, args, ty),
        "str_contains" | "str_starts_with" | "str_ends_with" | "str_trim"
        | "str_trim_start" | "str_trim_end" | "str_to_upper" | "str_to_lower"
        | "str_replace" | "str_repeat" => {
            gen_borrowed_call(ctx, &format!("yashy_{name}"), args, ty)
        }
        // `str_from_bool` is the one conversion whose runtime signature is wider
        // than the language's type. C has no one-bit type, so
        // `yashy_str_from_bool` takes an `i64` and the header argues the case;
        // the `i1` is zero-extended here to meet it. (`str_from_i64` and
        // `str_from_f64` need no reshaping and go through the rename arm.)
        "str_from_bool" => {
            let value = match args.first() {
                Some(arg) => gen_expr(ctx, arg),
                None => return zero_of(ty, ctx),
            };
            let as_i64 = if repr_type(&value.ty) == "i1" {
                let widened = ctx.tmp();
                ctx.push(format!("{widened} = zext i1 {} to i64", value.value));
                widened
            } else {
                value.value
            };
            let converted = ctx.tmp();
            ctx.push(format!(
                "{converted} = call i8* @yashy_str_from_bool(i64 {as_i64})"
            ));
            // A conversion builds the string, so the result is owned.
            ctx.mark_owned(&converted, RefKind::Str);
            GenValue {
                value: converted,
                ty: ty.clone(),
            }
        }
        "print_i64" | "print_f64" | "print_str" => {
            let lowered: Vec<GenValue> = args.iter().map(|a| gen_expr(ctx, a)).collect();
            let arg_list: Vec<String> = lowered
                .iter()
                .map(|v| format!("{} {}", repr_type(&v.ty), v.value))
                .collect();
            ctx.push(format!(
                "call void @yashy_print_{suffix}({args})",
                suffix = name.trim_start_matches("print_"),
                args = arg_list.join(", ")
            ));
            // `print_str` copies the bytes into stdout and keeps no pointer, so
            // a temporary it was handed is consumed here. Only owned names are
            // released, so a literal or a local is untouched.
            for value in &lowered {
                ctx.release_owned(&value.value);
            }
            GenValue {
                value: String::new(),
                ty: TypeKind::Unit,
            }
        }
        "None" => {
            // `None` has no payload to insert, so only the tag is written; the
            // payload field stays `undef` and is never read, because every
            // match tests the tag first. Emitting the insertvalue regardless
            // (rather than skipping straight to `undef`) is what makes the
            // value a well-formed `{ i8, T }`.
            let repr = repr_type(ty);
            let tmp = ctx.tmp();
            ctx.push(format!("{tmp} = insertvalue {repr} undef, i8 0, 0"));
            GenValue {
                value: tmp,
                ty: ty.clone(),
            }
        }
        "Some" | "Ok" | "Err" => {
            let payload = match args.first() {
                Some(a) => gen_expr(ctx, a),
                None => zero_of(&TypeKind::I64, ctx),
            };
            let (tag, field) = match name.as_str() {
                "Some" => (1u8, 1u32),
                "Ok" => (0, 1),
                _ => (1, 2),
            };
            let repr = repr_type(ty);
            let with_tag = ctx.tmp();
            ctx.push(format!("{with_tag} = insertvalue {repr} undef, i8 {tag}, 0"));
            let full = ctx.tmp();
            ctx.push(format!(
                "{full} = insertvalue {repr} {with_tag}, {} {}, {field}",
                repr_type(&payload.ty),
                payload.value
            ));
            GenValue {
                value: full,
                ty: ty.clone(),
            }
        }
        _ => {
            // A C declaration is not a user function: its arguments may be
            // copied and its temporaries freed, which needs the lowered values
            // rather than the expression tree. The checker has already put the
            // declaration in scope under this name.
            if let Some(decl) = ctx.module.extern_decl(&name).cloned() {
                let lowered: Vec<GenValue> = args.iter().map(|a| gen_expr(ctx, a)).collect();
                let value = crate::ffi::gen_ffi_call(ctx, &decl, &lowered, unsafe_call);
                return GenValue {
                    value,
                    ty: ty.clone(),
                };
            }
            // Builtins whose runtime symbol is the `yashy_`-prefixed name and
            // whose arguments and result need no reshaping: only the name
            // differs, so the common path handles them.
            match name.as_str() {
                "read_line" | "parse_i64" | "parse_f64" | "str_from_i64" | "str_from_f64"
                | "str_slice" | "str_debug" => {
                    gen_borrowed_call(ctx, &format!("yashy_{name}"), args, ty)
                }
                // These three cross the C boundary: a `string` argument becomes a
                // NUL-terminated copy for the callee, and the copy is freed when
                // the call returns. `write_file` additionally lends the text's
                // bytes, because a file body is not a C string (it may contain a
                // NUL) and its length is passed beside the pointer.
                "read_file" | "getenv" => {
                    crate::ffi::gen_cstr_string_builtin(ctx, &format!("yashy_{name}"), args, ty)
                }
                "write_file" => crate::ffi::gen_write_file(ctx, args, ty),
                _ => gen_borrowed_call(ctx, &name, args, ty),
            }
        }
    }
}

/// Lower `assert(condition, message, file, line, col)`.
///
/// The message and the position are the checker's: it has the source text and
/// the map, and this stage does not. Everything here is the branch and the cold
/// block, which is the shape every checked operation uses. See [`crate::faults`].
fn gen_assert(ctx: &mut FnCtx<'_>, args: &[HirExpr]) -> GenValue {
    // Every argument is lowered before the branch, in the caller's own block.
    // The message is an interned string global and needs a `getelementptr`;
    // emitted after the branch it would land in a block the fault path cannot
    // reach.
    let lowered: Vec<GenValue> = args.iter().map(|a| gen_expr(ctx, a)).collect();
    let Some(condition) = lowered.first() else {
        // The checker hands down exactly these arguments, so this is a guard
        // against a short list rather than a case. Emitting nothing keeps a bug
        // in this stage from becoming invalid IR attributed to the program.
        return unit();
    };
    // `lowered` is [condition, message, file, line, col], and `yashy_panic`
    // takes the last four in that order.
    let call = panic_call(&lowered, 1);
    let verdict = condition.value.clone();
    branch_to_report(ctx, &verdict, |_| call);
    unit()
}

/// Lower `assert_eq(left, right, prefix, middle, file, line, col)` and the
/// `assert_ne` spelling.
///
/// The operands arrive as expressions rather than as message text because the
/// values can only be printed here, where they are registers -- and because
/// lowering each of them once and using the result twice is what keeps
/// `assert_eq(f(), 0)` from calling `f` a second time on the failure path, where
/// it could produce a different number than the one that failed.
fn gen_assert_cmp(ctx: &mut FnCtx<'_>, name: &str, args: &[HirExpr]) -> GenValue {
    let lowered: Vec<GenValue> = args.iter().map(|a| gen_expr(ctx, a)).collect();
    let (Some(left), Some(right), Some(prefix), Some(middle)) = (
        lowered.first(),
        lowered.get(1),
        lowered.get(2),
        lowered.get(3),
    ) else {
        // The checker hands down exactly these arguments. A shorter list means
        // the name resolved to a call this lowering does not describe -- a
        // dotted spelling the checker left alone, since it resolves by the last
        // segment -- so the call is passed through instead. That fails at link
        // time with the symbol's name in it, which is a better answer than the
        // program compiling to nothing.
        return gen_plain_call(ctx, name, args, &TypeKind::Unit);
    };
    // `assert_eq` passes when the operands compare equal; `assert_ne` is the
    // negation of that, so one predicate covers both.
    let verdict = compared(ctx, name == "assert_ne", left, right);
    branch_to_report(ctx, &verdict, |ctx| {
        let message = comparison_message(ctx, prefix, middle, left, right);
        panic_call_with(&message, &lowered, 4)
    });
    // Back on the path where the assertion held, the operands have been read
    // for the last time, so a temporary the expression built is released here.
    // The failure path never reaches this point -- it ends the process inside
    // the report -- so nothing below depends on the operands being alive.
    ctx.release_owned(&left.value);
    ctx.release_owned(&right.value);
    unit()
}

/// Branch on `verdict` and, in a cold block, assemble the report and end the
/// program; execution continues in a fresh block.
///
/// `report` runs *inside* that block, because what a failure message costs --
/// three concatenations, a conversion per operand -- should be paid only when
/// there is a failure to report.
fn branch_to_report(
    ctx: &mut FnCtx<'_>,
    verdict: &str,
    report: impl FnOnce(&mut FnCtx<'_>) -> String,
) {
    let pass = format!("{}_assert_ok", ctx.alloc_label_base());
    let fault = crate::faults::fault_label(ctx);
    ctx.push(format!("br i1 {verdict}, label %{pass}, label %{fault}"));
    crate::faults::close_fault(ctx, &fault, |ctx| {
        crate::stmt_gen::run_all_scopes_defers(ctx);
        report(ctx)
    });
    ctx.push(format!("{pass}:"));
}

/// The `i1` a comparison assertion tests.
///
/// `inverted` is the `assert_ne` sense. The instruction is chosen by the operand
/// type: `fcmp` for `f64` -- `une`, because a `NaN` is not equal to anything and
/// an `assert_eq` on one should fail -- the runtime's equality for a `string`,
/// and `icmp` for `i64` and `bool`.
fn compared(ctx: &mut FnCtx<'_>, inverted: bool, left: &GenValue, right: &GenValue) -> String {
    let equal = !inverted;
    let verdict = ctx.tmp();
    match &left.ty {
        TypeKind::F64 => {
            ctx.push(format!(
                "{verdict} = fcmp {} double {}, {}",
                if equal { "oeq" } else { "une" },
                left.value,
                right.value
            ));
            verdict
        }
        // The runtime answers "equal" whichever name was written, so the `ne`
        // case negates its answer rather than asking a second question.
        TypeKind::Str => {
            let same = ctx.tmp();
            ctx.push(format!(
                "{same} = call i1 @yashy_str_eq(i8* {}, i8* {})",
                left.value, right.value
            ));
            if equal {
                return same;
            }
            ctx.push(format!("{verdict} = xor i1 {same}, true"));
            verdict
        }
        // `i64` and `bool`, both integers to LLVM.
        _ => {
            ctx.push(format!(
                "{verdict} = icmp {} {} {}, {}",
                if equal { "eq" } else { "ne" },
                repr_type(&left.ty),
                left.value,
                right.value
            ));
            verdict
        }
    }
}

/// The message a failed `assert_eq`/`assert_ne` reports: the two halves the
/// checker wrote, with each operand's value printed between them.
///
/// ```text
/// assertion failed: n == 0, left = 3, right = 0
/// ```
fn comparison_message(
    ctx: &mut FnCtx<'_>,
    prefix: &GenValue,
    middle: &GenValue,
    left: &GenValue,
    right: &GenValue,
) -> String {
    let left_text = value_as_string(ctx, left);
    let right_text = value_as_string(ctx, right);
    let head = concat(ctx, &prefix.value, &left_text);
    let tail = concat(ctx, &head, &middle.value);
    concat(ctx, &tail, &right_text)
}

/// `left ++ right`, both `i8*`.
///
/// This is the assertion-message builder, and it is deliberately *not* one of
/// the sites that releases: the only caller runs inside the reporting block,
/// which ends the process, so every temporary it builds dies with the process
/// rather than being reclaimed. The operands come from the failure path too,
/// and the pass path releases them where it is created.
fn concat(ctx: &mut FnCtx<'_>, left: &str, right: &str) -> String {
    let joined = ctx.tmp();
    ctx.push(format!(
        "{joined} = call i8* @yashy_str_concat(i8* {left}, i8* {right})"
    ));
    joined
}

/// A value as the string a message prints: the runtime's conversion for the
/// three scalar types, and a `string` itself, which needs none.
///
/// Nothing here is marked owned, for the reason [`concat`] gives: the result
/// exists only to be printed by a program that is already ending.
fn value_as_string(ctx: &mut FnCtx<'_>, value: &GenValue) -> String {
    match &value.ty {
        TypeKind::I64 => {
            let text = ctx.tmp();
            ctx.push(format!(
                "{text} = call i8* @yashy_str_from_i64(i64 {})",
                value.value
            ));
            text
        }
        TypeKind::F64 => {
            let text = ctx.tmp();
            ctx.push(format!(
                "{text} = call i8* @yashy_str_from_f64(double {})",
                value.value
            ));
            text
        }
        // C has no one-bit type, so `yashy_str_from_bool` takes an `i64` and the
        // `i1` is zero-extended to meet it -- the same widening `str_from_bool`
        // does in [`gen_call`].
        TypeKind::Bool => {
            let widened = ctx.tmp();
            ctx.push(format!("{widened} = zext i1 {} to i64", value.value));
            let text = ctx.tmp();
            ctx.push(format!(
                "{text} = call i8* @yashy_str_from_bool(i64 {widened})"
            ));
            text
        }
        // `string` is already a pointer to bytes, and anything else the checker
        // lets through reads the same way.
        _ => value.value.clone(),
    }
}

/// `call void @yashy_panic(<message>, <file>, <line>, <col>)`, the message being
/// the register that holds one: `panic`'s own lowering, and `assert`'s, where
/// the checker wrote the text.
fn panic_call(lowered: &[GenValue], message_at: usize) -> String {
    let args: Vec<String> = lowered
        .iter()
        .skip(message_at)
        .map(operand_of)
        .collect();
    format!("call void @yashy_panic({})", args.join(", "))
}

/// The same call with the message given as a name rather than an argument: the
/// assembled message of a comparison assertion, followed by the position.
fn panic_call_with(message: &str, lowered: &[GenValue], first_position: usize) -> String {
    let mut args = vec![format!("i8* {message}")];
    args.extend(lowered.iter().skip(first_position).map(operand_of));
    format!("call void @yashy_panic({})", args.join(", "))
}

/// One lowered value as an IR operand: its type's representation, then its name.
fn operand_of(value: &GenValue) -> String {
    format!("{} {}", repr_type(&value.ty), value.value)
}

/// The result of a statement-shaped builtin: no register, and `()` for a type.
fn unit() -> GenValue {
    GenValue {
        value: String::new(),
        ty: TypeKind::Unit,
    }
}

/// Emit a direct call to `symbol`, passing the arguments through unchanged.
///
/// This is the common case: a runtime builtin or a user function whose declared
/// parameter and return types are already the IR types, so the call *is* the
/// whole lowering.
fn gen_plain_call(ctx: &mut FnCtx<'_>, symbol: &str, args: &[HirExpr], ty: &TypeKind) -> GenValue {
    let lowered: Vec<GenValue> = args.iter().map(|a| gen_expr(ctx, a)).collect();
    gen_lowered_call(ctx, symbol, &lowered, ty)
}

/// Emit one call from arguments that are already lowered.
///
/// The callee's return type decides whether the call produces a value at all; a
/// `void` result is an empty [`GenValue`], which every consumer already treats as
/// "nothing to use".
fn gen_lowered_call(
    ctx: &mut FnCtx<'_>,
    symbol: &str,
    lowered: &[GenValue],
    ty: &TypeKind,
) -> GenValue {
    let arg_list: Vec<String> = lowered
        .iter()
        .map(|v| format!("{} {}", repr_type(&v.ty), v.value))
        .collect();
    let ret_repr = repr_type(ty);
    if ret_repr == "void" {
        ctx.push(format!("call void @{symbol}({})", arg_list.join(", ")));
        return GenValue {
            value: String::new(),
            ty: ty.clone(),
        };
    }
    let tmp = ctx.tmp();
    ctx.push(format!(
        "{tmp} = call {ret_repr} @{symbol}({})",
        arg_list.join(", ")
    ));
    GenValue {
        value: tmp,
        ty: ty.clone(),
    }
}

/// Lower a call whose `string` arguments are **borrowed**: a function the program
/// wrote, or one of the reading builtins (`parse_i64`, `str_slice`, ...).
///
/// The callee may not free an argument, so a temporary the caller owns is
/// released once the call returns, exactly as `print_str`'s argument is. A
/// `string` result is marked owned: `return` retains a local on the way out (see
/// `gen_return`) and the runtime built ones are fresh, so either way the caller
/// holds one reference and a binding takes it over instead of retaining a second
/// time.
///
/// Externs do not come through here: they have their own boundary rules in
/// [`crate::ffi::gen_ffi_call`].
fn gen_borrowed_call(
    ctx: &mut FnCtx<'_>,
    symbol: &str,
    args: &[HirExpr],
    ty: &TypeKind,
) -> GenValue {
    let lowered: Vec<GenValue> = args.iter().map(|a| gen_expr(ctx, a)).collect();
    let value = gen_lowered_call(ctx, symbol, &lowered, ty);
    for argument in &lowered {
        ctx.release_owned(&argument.value);
    }
    if matches!(ty, TypeKind::Str) {
        ctx.mark_owned(&value.value, RefKind::Str);
    }
    if matches!(ty, TypeKind::Func(_, _)) {
        // A returned closure: the caller owns the reference the callee had, so
        // the value is owned here and the caller's binding takes it over.
        ctx.mark_owned(&value.value, RefKind::Env);
    }
    value
}

/// Lower `expr?`: test the Result tag, propagate on `Err`, unwrap on `Ok`.
///
/// The `Err` path returns the whole Result value out of the enclosing
/// function, so the function's own return type must be the same Result shape.
/// The type checker does not yet enforce that, so a mismatch lowers to
/// `unreachable` plus a comment instead of a malformed `ret`.
///
/// The `Err` path does **not** run pending `defer` bodies (Task 12 limitation,
/// recorded in the ledger): expressions have no access to the statement
/// scope's defer list.
fn gen_try(ctx: &mut FnCtx<'_>, expr: &HirExpr, result_ty: &TypeKind) -> GenValue {
    let value = gen_expr(ctx, expr);
    let subject_repr = repr_type(&value.ty);
    let ret_repr = repr_type(&ctx.ret_type);
    let ok_label = ctx.alloc_label_base() + "_try_ok";
    let err_label = ctx.alloc_label_base() + "_try_err";
    let cont_label = ctx.alloc_label_base() + "_try_cont";

    let tag = ctx.tmp();
    ctx.push(format!(
        "{tag} = extractvalue {subject_repr} {}, 0",
        value.value
    ));
    let is_ok = ctx.tmp();
    ctx.push(format!("{is_ok} = icmp eq i8 {tag}, 0"));
    ctx.push(format!(
        "br i1 {is_ok}, label %{ok_label}, label %{err_label}"
    ));

    ctx.push(format!("{err_label}:"));
    if ret_repr == subject_repr && ret_repr != "void" {
        ctx.push(format!("ret {ret_repr} {}", value.value));
    } else {
        ctx.push(
            "; TODO: `?` in a function whose return type is not the same Result shape".to_string(),
        );
        ctx.push("unreachable".to_string());
    }

    ctx.push(format!("{ok_label}:"));
    let payload = ctx.tmp();
    ctx.push(format!(
        "{payload} = extractvalue {subject_repr} {}, 1",
        value.value
    ));
    ctx.push(format!("br label %{cont_label}"));
    ctx.push(format!("{cont_label}:"));

    GenValue {
        value: payload,
        ty: result_ty.clone(),
    }
}

/// Lower a struct literal: define the named type once, then `insertvalue`
/// each field in **literal order**.
///
/// Field indices are literal order, not declaration order: the declaration
/// table that would tell them apart is threaded through in Task 12, where the
/// whole program's struct definitions are available.
fn gen_struct_lit(
    ctx: &mut FnCtx<'_>,
    path: &[String],
    fields: &[(String, HirExpr, Span)],
    ty: &TypeKind,
) -> GenValue {
    let struct_name = format!("%struct.{}", path.last().cloned().unwrap_or_default());
    let lowered: Vec<GenValue> = fields.iter().map(|(_, e, _)| gen_expr(ctx, e)).collect();
    let field_tys: Vec<String> = lowered.iter().map(|v| repr_type(&v.ty)).collect();
    ctx.module.define_named_type(&struct_name, field_tys);
    let repr = repr_type(ty);
    let mut acc = "undef".to_string();
    for (index, value) in lowered.iter().enumerate() {
        // A `string` field is a reference the struct owns: the value is
        // either taken over (a temporary the runtime built) or retained (a
        // literal, a local, a call's result) -- never both, so the reference
        // is counted once.
        if matches!(value.ty, TypeKind::Str) {
            if ctx.is_owned(&value.value) {
                ctx.forget_owned(&value.value);
            } else if !value.value.is_empty() {
                let retained = ctx.tmp();
                ctx.push_str_retain_into(&retained, &value.value);
                // The aggregate below is built from the retained value, so the
                // reference it holds is the retained one.
                let replacement = GenValue {
                    value: retained,
                    ty: value.ty.clone(),
                };
                let current = acc.clone();
                let next = ctx.tmp();
                ctx.push(format!(
                    "{next} = insertvalue {repr} {current}, {} {}, {index}",
                    repr_type(&replacement.ty),
                    replacement.value
                ));
                acc = next;
                continue;
            }
        }
        let next = ctx.tmp();
        ctx.push(format!(
            "{next} = insertvalue {repr} {acc}, {} {}, {index}",
            repr_type(&value.ty),
            value.value
        ));
        acc = next;
    }
    GenValue {
        value: acc,
        ty: ty.clone(),
    }
}

/// Lower a cast. The sema whitelist (Task 8) permits only primitive
/// conversions: `i64` <-> `f64`, and `bool` <-> `i64`.
/// The bit width of an integer-like kind, or `None` for anything else.
///
/// `bool` counts as a one-bit unsigned integer: casting it to `i64` zero-extends,
/// which is the same instruction `zext` gives for any unsigned source.
fn integer_width(kind: &TypeKind) -> Option<u32> {
    match kind {
        TypeKind::Bool => Some(1),
        TypeKind::I64 | TypeKind::CSize => Some(64),
        TypeKind::CInt(width) | TypeKind::CUInt(width) => Some(*width),
        _ => None,
    }
}

/// Whether a kind's machine value is signed, which decides `sext` vs `zext`,
/// `sitofp` vs `uitofp`, and `fptosi` vs `fptoui`.
fn is_signed_integer(kind: &TypeKind) -> bool {
    matches!(kind, TypeKind::I64 | TypeKind::CInt(_))
}

/// The bit width of a floating-point kind, or `None` for anything else.
fn float_width(kind: &TypeKind) -> Option<u32> {
    match kind {
        TypeKind::F64 => Some(64),
        TypeKind::CFloat(width) => Some(*width),
        _ => None,
    }
}

fn gen_cast(ctx: &mut FnCtx<'_>, expr: &HirExpr, target: &TypeKind) -> GenValue {
    let value = gen_expr(ctx, expr);
    let from = repr_type(&value.ty);
    let to = repr_type(target);
    // Two kinds can share a representation (`i64` and `usize` both lower to
    // `i64`); the conversion is then a type-level one and no instruction is
    // needed.
    if from == to {
        return GenValue {
            value: value.value,
            ty: target.clone(),
        };
    }
    let source_int = integer_width(&value.ty);
    let source_float = float_width(&value.ty);
    let target_int = integer_width(target);
    let target_float = float_width(target);
    let op = match (source_int, source_float, target_int, target_float) {
        (Some(from_bits), _, Some(to_bits), _) if from_bits > to_bits => "trunc",
        (Some(_), _, Some(_), _) => {
            if is_signed_integer(&value.ty) {
                "sext"
            } else {
                "zext"
            }
        }
        (Some(_), _, _, Some(_)) => {
            if is_signed_integer(&value.ty) {
                "sitofp"
            } else {
                "uitofp"
            }
        }
        (_, Some(_), Some(_), _) => {
            if is_signed_integer(target) {
                "fptosi"
            } else {
                "fptoui"
            }
        }
        (_, Some(from_bits), _, Some(to_bits)) if from_bits < to_bits => "fpext",
        (_, Some(_), _, Some(_)) => "fptrunc",
        _ => {
            // The checker only admits numeric-to-numeric casts, so this is
            // unreachable for a checked program. A comment and the original
            // value keep a malformed HIR from becoming an invalid module.
            ctx.push(format!("; unsupported cast {from} to {to}"));
            return value;
        }
    };
    let tmp = ctx.tmp();
    ctx.push(format!("{tmp} = {op} {from} {} to {to}", value.value));
    GenValue {
        value: tmp,
        ty: target.clone(),
    }
}
