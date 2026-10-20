//! The LLVM IR backend: typed HIR in, LLVM IR text out.
//!
//! The backend is a text emitter, not an AST builder: the master plan
//! forbids linking LLVM, so every stage here writes IR source lines and the
//! driver hands them to `clang`/`llc` as a subprocess.
//!
//! This module holds the pieces every later emission task shares:
//! [`module::Module`] (the accumulating module: body lines, interned string
//! globals, SSA temporaries), [`ir::llvm_type_string`] (the checked-type to
//! LLVM-type mapping), and `main` discovery (a program with no `fn main`
//! must fail as a diagnostic here, not as a linker error 400 lines later).

pub mod arena;
pub mod expr_gen;
pub mod faults;
pub mod ffi;
pub mod fn_gen;
pub mod ir;
pub mod memory;
pub mod module;
pub mod patterns;
pub mod slices;
pub mod stmt_gen;
pub mod strings;

pub use expr_gen::{gen_expr, gen_expr_to_slot, FnCtx, GenValue};
pub use ffi::gen_ffi_call;
pub use fn_gen::{fn_return_repr, gen_extern_decl, gen_fn, gen_main};
pub use ir::{llvm_type_string, repr_type, variant_payload_index, variant_tag};
pub use module::host_target_triple;
pub use module::Module;
pub use patterns::{bind_pattern, gen_match, pattern_variant_name};
pub use stmt_gen::{gen_block, gen_stmt, run_all_scopes_defers, run_defers, Flow};

use otter_common::{Diagnostic, FileId, Severity, Span};
use otter_sema::{HirFn, HirItem, HirProgram, TypeKind};

/// The choices a caller may make about code generation itself, as opposed to
/// about how the result is compiled.
///
/// The default is deliberately the semantics-preserving one: the IR does not
/// depend on which machine the program is built for, and every floating-point
/// operation rounds exactly as the source spells it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CodegenOptions {
    /// Allow floating-point contraction.
    ///
    /// This is small in the IR — the `contract` fast-math flag on a floating
    /// multiply and an add or subtract — and the backend acts on it only when
    /// the target has a fused multiply-add, so on the portable baseline it does
    /// nothing. What it buys when the target has one is large: on the `float`
    /// benchmark it is worth 1.7x, which is the whole difference between
    /// Otter and `clang -march=native` on that row.
    ///
    /// It is off by default because it changes what a program prints. A fused
    /// multiply-add rounds once where `a * b + c` rounds twice, so the result is
    /// *more* accurate but not the same number — and through a long dependency
    /// chain, such as the benchmark's hundred million iterations, the
    /// difference grows from one ulp into a visible decimal. A program that
    /// cares about its exact output must not be given this without being told,
    /// which is why it is a flag and not a default.
    pub fp_contract: bool,
}

/// Emit a whole checked program as one LLVM IR module.
///
/// `entry` is the path the module reports as its `source_filename`, so an IR
/// dump names the file the user compiled rather than a placeholder.
///
/// `options` are the code-generation choices; [`CodegenOptions::default`] is the
/// semantics-preserving set, and a caller that has no reason to change them
/// should pass it.
///
/// Every function item is emitted; struct and enum *definitions* contribute
/// only the `%struct.Name` lines a literal forces, and a program with no `fn
/// main` still produces a module — the driver calls [`check_main_exists`]
/// first and reports the missing entry point as a diagnostic, which is the
/// only place with a file to point at.
pub fn emit_program(hir: &HirProgram, entry: &str, options: CodegenOptions) -> String {
    let mut module = Module::new();
    module.set_source_filename(entry);
    module.set_fp_contract(options.fp_contract);
    // Declarations first: every function body may call a C function declared
    // later in the file, and a `declare` before the first `define` is the shape
    // LLVM expects. The table is also what tells the call lowering that a name
    // is extern rather than a user function.
    for item in &hir.items {
        if let HirItem::Extern(block) = item {
            for decl in &block.decls {
                gen_extern_decl(&mut module, decl);
                module.add_extern_decl(decl.clone());
            }
        }
    }
    for item in &hir.items {
        match item {
            HirItem::Fn(f) => {
                // The entry point is the one function whose signature the
                // operating system dictates, so it is emitted through its own
                // path.
                if f.name == "main" {
                    gen_main(&mut module, f);
                } else {
                    gen_fn(&mut module, f);
                }
            }
            // Handled in the pre-pass above: a C declaration contributes a
            // `declare` and nothing else. The backend must not invent a body
            // for a function it cannot see.
            HirItem::Extern(_) => {}
            HirItem::Struct(_) | HirItem::Enum(_) | HirItem::Import(_) => {}
        }
    }
    module.finish()
}

/// The `fn main` item, when the program has one.
///
/// Sema (Task 8) already rejected duplicate `main`s, so the first match is
/// the only match.
pub fn find_main(items: &[HirItem]) -> Option<&HirFn> {
    items.iter().find_map(|item| match item {
        HirItem::Fn(f) if f.name == "main" => Some(f),
        _ => None,
    })
}

/// Check that the program has a `fn main` whose return type is an entry
/// point (`()` or `i64`) and that takes no parameters.
///
/// The diagnostic's span is the synthetic `Span { start: 0, end: 0 }`: a
/// missing `main` points at nothing in particular, and a file-level caret
/// at offset zero reads as "the start of the file", which is exactly what a
/// missing entry point means. The `FileId` is index zero because only the
/// driver knows which file is being compiled; presence, not attribution, is
/// this function's job.
///
/// Parameters are rejected for a different reason: the C runtime starts the
/// process by calling `main` with its own argument convention, and the entry
/// point this backend emits takes none. A `main` with parameters would produce
/// a symbol the loader cannot call rather than a program.
pub fn check_main_exists(items: &[HirItem]) -> Result<(), Diagnostic> {
    let file = FileId::from_index(0);
    match find_main(items) {
        None => Err(Diagnostic {
            severity: Severity::Error,
            message: "no `fn main` found: every Otter program must define one".to_string(),
            file,
            span: Span::new(0, 0),
            label: None,
        }),
        Some(main) => {
            if let Some(first) = main.params.first() {
                return Err(Diagnostic {
                    severity: Severity::Error,
                    message: format!(
                        "`main` takes no parameters, but it declares `{}`",
                        first.name
                    ),
                    file,
                    span: first.span,
                    label: None,
                });
            }
            let ok = matches!(
                main.ret.kind,
                TypeKind::Unit | TypeKind::I64 | TypeKind::Never
            );
            if ok {
                Ok(())
            } else {
                Err(Diagnostic {
                    severity: Severity::Error,
                    message: format!("`main` must return nothing or `i64`, found `{}`", main.ret),
                    file,
                    span: main.span,
                    label: None,
                })
            }
        }
    }
}
