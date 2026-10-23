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

use yashy_sema::{HirExternDecl, HirFn, TypeKind};

use crate::ir::repr_type;
use crate::module::Module;
use crate::stmt_gen::{gen_block, run_defers, Flow};
use crate::FnCtx;

/// The LLVM type a function returns: `void` for the no-value type, else the type's
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

/// Emit a lambda as a private function `@<name>(i8* %env, args...)`.
///
/// The environment parameter is the convention every function value carries:
/// a call through a value passes the environment as its first argument, so
/// the function behind the value has to accept one. A lambda that captures
/// nothing ignores it.
///
/// The function is written into a capture buffer rather than into the module
/// body, because a lambda is lowered while the function that contains it is
/// still open, and a `define` block cannot be written inside a function. The
/// caller closes the capture and queues the block.
pub fn gen_lambda(module: &mut Module, f: &HirFn, name: &str) {
    let ret_repr = fn_return_repr(&f.ret.kind);
    let mut params = vec!["i8* %env".to_string()];
    for (index, param) in f.params.iter().enumerate() {
        params.push(format!("{} %arg{index}", repr_type(&param.ty.kind)));
    }
    module.push(format!("define {ret_repr} @{}({}) {{", name, params.join(", ")));

    gen_body(module, f, false, Some("%env"));
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
/// `define i32 @main()` — whatever the Yashy signature says:
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
    // A function that *receives* a function value keeps the calls it makes
    // through that value indirect.
    //
    // Inlining such a function into its caller would let the optimizer see
    // the value it was handed and fold the indirect call back into a direct
    // one. Yashy does not do that: a call through a function value is a call
    // through a function value, and the program is entitled to see it that
    // way — the same reason WebAssembly keeps `call_indirect` indirect. The
    // attribute is the only way to say so in LLVM IR.
    //
    // A function value held in a *local* binding is a different case and is
    // left alone: the optimizer can see the binding's value, and folding that
    // call is an ordinary optimization of an ordinary program.
    let attributes = if f
        .params
        .iter()
        .any(|param| matches!(param.ty.kind, TypeKind::Func(_, _)))
    {
        " noinline"
    } else {
        ""
    };
    module.push(format!(
        "define {ret_repr} @{}({}){attributes} {{",
        f.name,
        params.join(", ")
    ));

    gen_body(module, f, entry_point, None);
}

/// The body of a function: its parameter spills, its statements, and its
/// terminators.
///
/// The signature is the caller's business -- a named function and a lambda
/// have different ones -- but everything after the opening brace is the same
/// code, and keeping it in one place is what keeps the two from drifting.
///
/// `env` is the environment pointer a closure reads its captures from, when
/// the function is one. It is the SSA name the parameter was bound to.
fn gen_body(module: &mut Module, f: &HirFn, entry_point: bool, env: Option<&str>) {
    {
        let mut ctx = FnCtx::new(module, f.ret.kind.clone());
        ctx.entry_point = entry_point;
        if entry_point {
            ctx.unit_terminator = "ret i32 0".to_string();
        }
        ctx.captures = f.captures.clone();
        ctx.capture_offsets = crate::env::capture_offsets(&f.captures);
        ctx.env = env.map(|name| name.to_string());
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
