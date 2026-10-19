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
//! from the checker (see [`otter_sema::FaultSite`]): the backend is handed IR
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
//! # No input makes this module panic
//!
//! No `unwrap`, no `expect`, no `panic!`, no unchecked indexing. Every
//! identifier lookup, variant tag and payload index is an `Option`, and a
//! missing one lowers to a comment plus a zero rather than aborting the
//! compile.

use std::collections::HashMap;

use otter_common::Span;
use otter_parser::ast::Literal;
use otter_sema::{BinOp, FaultSite, HirExpr, HirFn, TypeKind, UnOp};

use crate::ir::repr_type;
use crate::module::Module;

/// A lowered value: its LLVM representation (an SSA name or a literal like
/// `42`) and the Otter type it came from.
#[derive(Clone, Debug)]
pub struct GenValue {
    /// An LLVM SSA name (`%t3`) or a literal (`42`, `1.0`, `true`, `null`).
    /// The empty string means "no value" — a `()`-valued expression.
    pub value: String,
    /// The Otter type of the value.
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
    /// The terminator for a `return` with no value, and for falling off the end
    /// of a body. `ret void` everywhere except the program's entry point, which
    /// has to yield an `i32` — see [`crate::fn_gen::gen_main`].
    pub unit_terminator: String,
    /// True for a function emitted as the process entry point, where `i32` is
    /// the return type the C runtime expects and an `i64` result is truncated
    /// to match it.
    pub entry_point: bool,
}

impl<'a> FnCtx<'a> {
    /// A context for one function, borrowing the shared module.
    pub fn new(module: &'a mut Module, ret_type: TypeKind) -> Self {
        Self {
            module,
            ret_type,
            next_slot: 0,
            next_label: 0,
            hoisted_allocas: Vec::new(),
            entry_line: None,
            locals: HashMap::new(),
            loops: Vec::new(),
            unit_terminator: "ret void".to_string(),
            entry_point: false,
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

/// Lower `expr` to a value, pushing its instructions into the module.
pub fn gen_expr(ctx: &mut FnCtx<'_>, expr: &HirExpr) -> GenValue {
    match expr {
        HirExpr::Literal(lit, _, ty) => gen_literal(ctx, lit, &ty.kind),
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
            callee, args, ty, ..
        } => gen_call(ctx, callee, args, &ty.kind),
        HirExpr::StructLit {
            path, fields, ty, ..
        } => gen_struct_lit(ctx, path, fields, &ty.kind),
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
        _ => "undef".to_string(),
    };
    GenValue {
        value,
        ty: ty.clone(),
    }
}

/// Lower a literal. Strings are interned as globals and decay to their data
/// pointer (see [`repr_type`]).
fn gen_literal(ctx: &mut FnCtx<'_>, lit: &Literal, ty: &TypeKind) -> GenValue {
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

/// Lower a call: the three runtime builtins, the `Option`/`Result`
/// constructors, and user functions.
fn gen_call(ctx: &mut FnCtx<'_>, callee: &[String], args: &[HirExpr], ty: &TypeKind) -> GenValue {
    let name = callee.last().cloned().unwrap_or_default();
    match name.as_str() {
        // `alloc` and `dealloc` are calls to the runtime's heap (ledger L1),
        // not C functions with matching names: `alloc(n)` counts elements and
        // the runtime counts bytes, so the shape changes here.
        "alloc" => crate::memory::gen_alloc(ctx, args, ty),
        "dealloc" => crate::memory::gen_dealloc(ctx, args),
        // `panic` never returns, so everything after it in the enclosing block
        // is dead. The call and its `unreachable` go into a block of their own
        // rather than into the current one: emitting `unreachable` here would
        // terminate the block and leave the caller appending instructions after
        // a terminator, which is not well-formed IR -- and the caller can be
        // anywhere, including the argument list of another call. So this block
        // branches to the panic block, and a fresh block is opened for whatever
        // follows. That continuation is unreachable -- the process ends inside
        // `otter_panic` -- but it is where the rest of the enclosing expression
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
            crate::faults::close_fault(ctx, &fault, |_| call);
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
            let call = format!("call void @otter_exit({code})");
            let fault = crate::faults::fault_label(ctx);
            let after_label = format!("{}_after_exit", ctx.alloc_label_base());
            ctx.push(format!("br label %{fault}"));
            crate::faults::close_fault(ctx, &fault, |_| call);
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
        // `len` is the byte length of a string today. M6 adds the slice case.
        "len" => crate::strings::gen_str_len(ctx, args),
        // `str_from_bool` is the one conversion whose runtime signature is wider
        // than the language's type. C has no one-bit type, so
        // `otter_str_from_bool` takes an `i64` and the header argues the case;
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
                "{converted} = call i8* @otter_str_from_bool(i64 {as_i64})"
            ));
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
                "call void @otter_print_{suffix}({args})",
                suffix = name.trim_start_matches("print_"),
                args = arg_list.join(", ")
            ));
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
            // Builtins whose runtime symbol is the `otter_`-prefixed name and
            // whose arguments and result need no reshaping: only the name
            // differs, so the common path handles them.
            match name.as_str() {
                "read_line" | "parse_i64" | "parse_f64" | "str_from_i64" | "str_from_f64" => {
                    gen_plain_call(ctx, &format!("otter_{name}"), args, ty)
                }
                _ => gen_plain_call(ctx, &name, args, ty),
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
    // `lowered` is [condition, message, file, line, col], and `otter_panic`
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
    crate::faults::close_fault(ctx, &fault, report);
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
                "{same} = call i1 @otter_str_eq(i8* {}, i8* {})",
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
fn concat(ctx: &mut FnCtx<'_>, left: &str, right: &str) -> String {
    let joined = ctx.tmp();
    ctx.push(format!(
        "{joined} = call i8* @otter_str_concat(i8* {left}, i8* {right})"
    ));
    joined
}

/// A value as the string a message prints: the runtime's conversion for the
/// three scalar types, and a `string` itself, which needs none.
fn value_as_string(ctx: &mut FnCtx<'_>, value: &GenValue) -> String {
    match &value.ty {
        TypeKind::I64 => {
            let text = ctx.tmp();
            ctx.push(format!(
                "{text} = call i8* @otter_str_from_i64(i64 {})",
                value.value
            ));
            text
        }
        TypeKind::F64 => {
            let text = ctx.tmp();
            ctx.push(format!(
                "{text} = call i8* @otter_str_from_f64(double {})",
                value.value
            ));
            text
        }
        // C has no one-bit type, so `otter_str_from_bool` takes an `i64` and the
        // `i1` is zero-extended to meet it -- the same widening `str_from_bool`
        // does in [`gen_call`].
        TypeKind::Bool => {
            let widened = ctx.tmp();
            ctx.push(format!("{widened} = zext i1 {} to i64", value.value));
            let text = ctx.tmp();
            ctx.push(format!(
                "{text} = call i8* @otter_str_from_bool(i64 {widened})"
            ));
            text
        }
        // `string` is already a pointer to bytes, and anything else the checker
        // lets through reads the same way.
        _ => value.value.clone(),
    }
}

/// `call void @otter_panic(<message>, <file>, <line>, <col>)`, the message being
/// the register that holds one: `panic`'s own lowering, and `assert`'s, where
/// the checker wrote the text.
fn panic_call(lowered: &[GenValue], message_at: usize) -> String {
    let args: Vec<String> = lowered
        .iter()
        .skip(message_at)
        .map(operand_of)
        .collect();
    format!("call void @otter_panic({})", args.join(", "))
}

/// The same call with the message given as a name rather than an argument: the
/// assembled message of a comparison assertion, followed by the position.
fn panic_call_with(message: &str, lowered: &[GenValue], first_position: usize) -> String {
    let mut args = vec![format!("i8* {message}")];
    args.extend(lowered.iter().skip(first_position).map(operand_of));
    format!("call void @otter_panic({})", args.join(", "))
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
fn gen_cast(ctx: &mut FnCtx<'_>, expr: &HirExpr, target: &TypeKind) -> GenValue {
    let value = gen_expr(ctx, expr);
    let from = repr_type(&value.ty);
    let to = repr_type(target);
    if from == to {
        return GenValue {
            value: value.value,
            ty: target.clone(),
        };
    }
    let op = match (from.as_str(), to.as_str()) {
        ("i64", "double") => "sitofp",
        ("double", "i64") => "fptosi",
        ("i1", "i64") => "zext",
        ("i1", "double") => "uitofp",
        ("i64", "i1") => "trunc",
        _ => {
            ctx.push(format!(
                "; TODO(Task 12): cast {from} to {to} (unsupported at M0-M2)"
            ));
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
