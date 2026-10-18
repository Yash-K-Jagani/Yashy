//! Function emission: a checked [`HirFn`] to one `define` block.
//!
//! Every parameter is spilled into an alloca in the entry block, so the body
//! reads and writes it through memory rather than reasoning about SSA phi
//! nodes — the whole M0–M2 backend is deliberately SSA-naive and leans on
//! `mem2reg` in the optimizer instead.
//!
//! The function always ends with exactly one terminator: a body that returns
//! on every path has already emitted its `ret`, and a body that falls through
//! emits `ret void` or `ret <zero>` for its return type.

use otter_sema::{HirFn, TypeKind};

use crate::ir::repr_type;
use crate::module::Module;
use crate::stmt_gen::{gen_block, run_defers, Flow};
use crate::FnCtx;

/// The LLVM type a function returns: `void` for `()`, else the type's
/// representation.
pub fn fn_return_repr(kind: &TypeKind) -> String {
    match kind {
        TypeKind::Unit | TypeKind::Never => "void".to_string(),
        other => repr_type(other),
    }
}

/// Emit `f` — its signature, its parameter spills, its body — into `module`.
pub fn gen_fn(module: &mut Module, f: &HirFn) {
    let ret_repr = fn_return_repr(&f.ret.kind);
    let params: Vec<String> = f
        .params
        .iter()
        .enumerate()
        .map(|(index, param)| format!("{} %arg{index}", repr_type(&param.ty.kind)))
        .collect();
    module.push(format!(
        "define {ret_repr} @{}({}) {{",
        f.name,
        params.join(", ")
    ));

    {
        let mut ctx = FnCtx::new(module, f.ret.kind.clone());
        ctx.push("entry:");
        let slots = ctx.alloc_params(f);
        for (index, (slot, param)) in slots.iter().zip(f.params.iter()).enumerate() {
            let repr = repr_type(&param.ty.kind);
            ctx.push(format!("store {repr} %arg{index}, ptr {slot}"));
        }

        let mut defers = Vec::new();
        let flow = gen_block(&mut ctx, &f.body, &mut defers);
        if flow == Flow::Fallthrough {
            run_defers(&mut ctx, &mut defers);
        }
        if flow != Flow::Return {
            match f.ret.kind {
                TypeKind::Unit | TypeKind::Never => ctx.push("ret void"),
                _ => ctx.push(format!("ret {} 0", repr_type(&f.ret.kind))),
            }
        }
        ctx.push("}");
    }
}
