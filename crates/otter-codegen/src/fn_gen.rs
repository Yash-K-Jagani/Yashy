//! Function emission: a checked [`HirFn`] to one `define` block.
//!
//! Every parameter is spilled into an alloca in the entry block, so the body
//! reads and writes it through memory rather than reasoning about SSA phi
//! nodes — the whole M0–M2 backend is deliberately SSA-naive and leans on
//! `mem2reg` in the optimizer instead.
//!
//! Every *other* alloca the body needs lands in the entry block too. They are
//! diverted into `FnCtx::push_alloca` as the body is lowered and spliced in
//! after the `entry:` label here, so no block other than `entry` ever allocates
//! — see that method for why an alloca inside a loop is a real cost and not
//! just a style question.
//!
//! The function always ends with exactly one terminator: a body that returns
//! on every path has already emitted its `ret`, and a body that falls through
//! emits `ret void` or `ret <zero>` for its return type.

use otter_sema::{HirExternDecl, HirFn, TypeKind};

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
    gen_fn_impl(module, f, false);
}

/// Emit one C function declaration as an LLVM `declare`.
///
/// A `declare` is the whole contract: the symbol name, the return type and the
/// parameter types. Emitting a `define` here would be wrong in a way that links
/// — an empty body would silently replace the C function — so this function has
/// no path that produces a body.
pub fn gen_extern_decl(module: &mut Module, d: &HirExternDecl) {
    let params: Vec<String> = d.params.iter().map(|p| repr_type(&p.ty.kind)).collect();
    let ret = fn_return_repr(&d.ret.kind);
    module.push(format!(
        "declare {ret} @{}({})",
        d.name,
        params.join(", ")
    ));
}

/// Emit `f` as the program's entry point.
///
/// The C runtime starts a process by calling `main` and using its `int` return
/// as the exit status, so the entry point is always emitted as
/// `define i32 @main()` — whatever the Otter signature says:
///
/// * `fn main()` becomes an `i32` function whose `return` and fall-off-the-end
///   both yield 0. Emitting `void main` instead leaves the exit status as
///   whatever the last call happened to leave in the return register, which is
///   a real bug and not a theoretical one: `fn main() { print_str("hello"); }`
///   exits with the number of bytes its last runtime call returned, so adding a
///   `==` on strings to a program silently changed its exit code.
/// * `fn main() -> i64` keeps its `i64` body but truncates the result to `i32`
///   at every `return`, which is the width the operating system reads.
///
/// The checker rejects an entry point that returns anything else, so there is no
/// third case to write. `main`'s own name is left alone, so a recursive `main`
/// still resolves.
pub fn gen_main(module: &mut Module, f: &HirFn) {
    gen_fn_impl(module, f, true);
}

/// The shared body of [`gen_fn`] and [`gen_main`]: the only difference is
/// whether the signature and the terminators use the entry point's `i32`.
fn gen_fn_impl(module: &mut Module, f: &HirFn, entry_point: bool) {
    let ret_repr = if entry_point {
        "i32".to_string()
    } else {
        fn_return_repr(&f.ret.kind)
    };
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
        ctx.entry_point = entry_point;
        if entry_point {
            ctx.unit_terminator = "ret i32 0".to_string();
        }
        ctx.begin_entry();
        let slots = ctx.alloc_params(f);
        for (index, (slot, param)) in slots.iter().zip(f.params.iter()).enumerate() {
            let repr = repr_type(&param.ty.kind);
            ctx.push(format!("store {repr} %arg{index}, ptr {slot}"));
        }

        let flow = gen_block(&mut ctx, &f.body);
        if flow == Flow::Fallthrough {
            run_defers(&mut ctx);
        }
        if flow != Flow::Return {
            if entry_point {
                ctx.push("ret i32 0");
            } else {
                match f.ret.kind {
                    TypeKind::Unit | TypeKind::Never => ctx.push("ret void"),
                    _ => ctx.push(format!("ret {} 0", repr_type(&f.ret.kind))),
                }
            }
        }
        ctx.push("}");
        // Every alloca the body asked for lands here, directly after `entry:`.
        ctx.finish();
    }
}
