//! Expression lowering: typed HIR expressions to LLVM IR instructions.
//!
//! Every value-producing expression is lowered once, into a fresh SSA
//! temporary or alloca slot — never twice. [`gen_expr`] returns the value;
//! [`gen_expr_to_slot`] additionally spills it into an alloca, which is the
//! single entry point statements, `return` and match arms use.
//!
//! # Interim semantics (ledger L4)
//!
//! `Add Sub Mul` emit bare `add`/`sub`/`mul` — **no `nsw`/`nuw`** — so
//! wraparound is defined rather than UB until M4 makes arithmetic checked.
//! `Div Rem` emit bare `sdiv`/`srem` with no zero guard: division by zero is
//! UB until M4. Both are pinned by `tests/golden/ops.expected.ll`, so M4's
//! replacement is a deliberate diff rather than a silent change.
//!
//! A construct not lowered yet (`?` propagation, `FieldAccess`) becomes a
//! comment plus a zero of its type: visibly incomplete IR, never a silently
//! wrong answer.
//!
//! # No input makes this module panic
//!
//! No `unwrap`, no `expect`, no `panic!`, no unchecked indexing. Every
//! identifier lookup, variant tag and payload index is an `Option`, and a
//! missing one lowers to a comment plus a zero rather than aborting the
//! compile.

use std::collections::{HashMap, HashSet};

use otter_common::Span;
use otter_parser::ast::Literal;
use otter_sema::{BinOp, HirExpr, HirFn, TypeKind, UnOp};

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
    /// Bound names (`locals`, parameters, pattern bindings) to their slot and
    /// declared type.
    pub locals: HashMap<String, (String, TypeKind)>,
    /// Struct types already defined in this module, so a second literal of the
    /// same struct does not re-emit `%struct.Name = type { … }`.
    pub seen_structs: HashSet<String>,
}

impl<'a> FnCtx<'a> {
    /// A context for one function, borrowing the shared module.
    pub fn new(module: &'a mut Module, ret_type: TypeKind) -> Self {
        Self {
            module,
            ret_type,
            next_slot: 0,
            next_label: 0,
            locals: HashMap::new(),
            seen_structs: HashSet::new(),
        }
    }

    /// Allocate the parameters of `f` into fresh slots so an identifier read
    /// is a `load` from a stable address. The slots are not initialised here;
    /// [`crate::gen_fn`] stores the incoming SSA arguments into them.
    pub fn alloc_params(&mut self, f: &HirFn) -> Vec<String> {
        let mut slots = Vec::new();
        for param in &f.params {
            let slot = self.alloc_slot();
            let repr = repr_type(&param.ty.kind);
            self.module.push(format!("{slot} = alloca {repr}"));
            self.locals
                .insert(param.name.clone(), (slot.clone(), param.ty.kind.clone()));
            slots.push(slot);
        }
        slots
    }

    /// A fresh alloca name, `%slot0`, `%slot1`, …
    pub fn alloc_slot(&mut self) -> String {
        let name = format!("%slot{}", self.next_slot);
        self.next_slot += 1;
        name
    }

    /// A fresh label base, `match0`, `match1`, … so nested switches do not
    /// collide.
    pub(crate) fn alloc_label_base(&mut self) -> String {
        let name = format!("match{}", self.next_label);
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
                Some((slot, slot_ty)) => {
                    let tmp = ctx.tmp();
                    let repr = repr_type(&slot_ty);
                    ctx.push(format!("{tmp} = load {repr}, ptr {slot}"));
                    GenValue {
                        value: tmp,
                        ty: slot_ty,
                    }
                }
                // `None` (and any other value-position variant name) has no
                // slot; it is a unit variant of the type the expression
                // already carries.
                None => zero_of(&ty.kind, ctx),
            }
        }
        HirExpr::Binary {
            op, lhs, rhs, span, ..
        } => gen_binary(ctx, *op, lhs, rhs, *span),
        HirExpr::Unary { op, operand, .. } => gen_unary(ctx, *op, operand),
        HirExpr::Move(inner, _, _) => gen_expr(ctx, inner),
        HirExpr::Call {
            callee, args, ty, ..
        } => gen_call(ctx, callee, args, &ty.kind),
        HirExpr::StructLit {
            path, fields, ty, ..
        } => gen_struct_lit(ctx, path, fields, &ty.kind),
        HirExpr::FieldAccess { field, span, ty, .. } => {
            ctx.push(format!(
                "; TODO(Task 12): field access `{field}` needs the declaration table (at {span:?})"
            ));
            zero_of(&ty.kind, ctx)
        }
        HirExpr::Cast { expr, ty, .. } => gen_cast(ctx, expr, &ty.kind),
        HirExpr::Try { span, ty, .. } => {
            ctx.push(format!("; TODO(Task 12): `?` propagation (at {span:?})"));
            zero_of(&ty.kind, ctx)
        }
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
    ctx.push(format!("{slot} = alloca {repr}"));
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
        Literal::Str(s) => {
            let name = ctx.module.intern_string(&s.bytes);
            let tmp = ctx.tmp();
            let array_ty = format!("[{} x i8]", s.bytes.len() + 1);
            ctx.push(format!(
                "{tmp} = getelementptr inbounds {array_ty}, ptr {name}, i64 0, i64 0"
            ));
            GenValue {
                value: tmp,
                ty: ty.clone(),
            }
        }
    }
}

/// Format an `f64` as an LLVM float literal.
///
/// Rust's `Debug` output is nearly right but omits the decimal point for
/// whole numbers and spells the non-finite values as words LLVM cannot parse,
/// so those three cases are handled explicitly.
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
    if text.contains('.') || text.contains('e') || text.contains('E') {
        text
    } else {
        format!("{text}.0")
    }
}

/// Lower a binary operation. Arithmetic is wrapping (no `nsw`/`nuw`),
/// integer division is a bare `sdiv`, comparisons pick `icmp`/`fcmp` with
/// the signed/ordered predicates, and `&&`/`||` are `and`/`or` on `i1`.
fn gen_binary(
    ctx: &mut FnCtx<'_>,
    op: BinOp,
    lhs: &HirExpr,
    rhs: &HirExpr,
    _span: Span,
) -> GenValue {
    let l = gen_expr(ctx, lhs);
    let r = gen_expr(ctx, rhs);
    let is_float = matches!(l.ty, TypeKind::F64);
    let ty = l.ty.clone();
    let (mnemonic, result_ty) = match op {
        BinOp::Add => ("add", ty.clone()),
        BinOp::Sub => ("sub", ty.clone()),
        BinOp::Mul => ("mul", ty.clone()),
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
    ctx.push(format!(
        "{tmp} = {mnemonic} {op_ty} {}, {}",
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
fn gen_unary(ctx: &mut FnCtx<'_>, op: UnOp, operand: &HirExpr) -> GenValue {
    let value = gen_expr(ctx, operand);
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
            let lowered: Vec<GenValue> = args.iter().map(|a| gen_expr(ctx, a)).collect();
            let arg_list: Vec<String> = lowered
                .iter()
                .map(|v| format!("{} {}", repr_type(&v.ty), v.value))
                .collect();
            let ret_repr = repr_type(ty);
            if ret_repr == "void" {
                ctx.push(format!("call void @{name}({})", arg_list.join(", ")));
                return GenValue {
                    value: String::new(),
                    ty: ty.clone(),
                };
            }
            let tmp = ctx.tmp();
            ctx.push(format!(
                "{tmp} = call {ret_repr} @{name}({})",
                arg_list.join(", ")
            ));
            GenValue {
                value: tmp,
                ty: ty.clone(),
            }
        }
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
    if ctx.seen_structs.insert(struct_name.clone()) {
        let field_tys: Vec<String> = lowered.iter().map(|v| repr_type(&v.ty)).collect();
        ctx.push(format!(
            "{struct_name} = type {{ {} }}",
            field_tys.join(", ")
        ));
    }
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
