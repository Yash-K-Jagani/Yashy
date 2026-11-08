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
pub mod async_lower;
pub mod env;
pub mod escape;
pub mod expr_gen;
pub mod faults;
pub mod ffi;
pub mod file;
pub mod fixed;
pub mod fn_gen;
pub mod higher_order;
pub mod ir;
pub mod maps;
pub mod memory;
pub mod module;
pub mod pass;
pub mod patterns;
pub mod slices;
pub mod stmt_gen;
pub mod strings;
pub mod threads;

pub use escape::{promotion_counts, AllocSite, ArenaId, Placement};
pub use expr_gen::{gen_expr, gen_expr_to_slot, FnCtx, GenValue};
pub use ffi::gen_ffi_call;
pub use fn_gen::{fn_return_repr, gen_extern_decl, gen_fn, gen_main};
pub use ir::{llvm_type_string, repr_type, variant_payload_index, variant_tag};
pub use module::host_target_triple;
pub use module::{FnOptReport, Module, OptKind, OptNote, OptReport, VariantRepr};
pub use pass::{pipeline, Pass, PassCtx, PassStats};
pub use patterns::{bind_pattern, gen_match, pattern_variant_name};
pub use stmt_gen::{gen_block, gen_stmt, run_all_scopes_defers, run_defers, Flow};

use yashy_common::{Diagnostic, FileId, Severity, Span};
use yashy_sema::{HirFn, HirItem, HirProgram, TypeKind};

/// The choices a caller may make about code generation itself, as opposed to
/// about how the result is compiled.
///
/// The default is deliberately the semantics-preserving one: the IR does not
/// depend on which machine the program is built for, and every floating-point
/// operation rounds exactly as the source spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodegenOptions {
    /// Allow floating-point contraction.
    ///
    /// This is small in the IR — the `contract` fast-math flag on a floating
    /// multiply and an add or subtract — and the backend acts on it only when
    /// the target has a fused multiply-add, so on the portable baseline it does
    /// nothing. What it buys when the target has one is large: on the `float`
    /// benchmark it is worth 1.7x, which is the whole difference between
    /// Yashy and `clang -march=native` on that row.
    ///
    /// It is off by default because it changes what a program prints. A fused
    /// multiply-add rounds once where `a * b + c` rounds twice, so the result is
    /// *more* accurate but not the same number — and through a long dependency
    /// chain, such as the benchmark's hundred million iterations, the
    /// difference grows from one ulp into a visible decimal. A program that
    /// cares about its exact output must not be given this without being told,
    /// which is why it is a flag and not a default.
    pub fp_contract: bool,
    /// Promote allocations that provably do not escape their function.
    ///
    /// This changes no program's meaning: a promoted allocation is one whose
    /// pointer can only be read or written through before the frame (or block)
    /// it lives in goes away. It changes only where the bytes live, so a build
    /// can turn it off to bisect a suspected miscompile without changing the
    /// language. Off by default in the library so a caller that asks for the
    /// plain backend gets exactly the IR it always did; the driver enables it.
    pub escape: bool,
    /// Rely on the verifier's facts when emitting.
    ///
    /// On by default, because a fact is a proof the verifier already accepted:
    /// turning it off cannot make the program more correct, only slower and
    /// noisier. `--no-opt=facts` exists for bisecting an attribute-driven
    /// miscompile.
    pub facts: bool,
    /// Let identical string literals share one global.
    ///
    /// On by default and semantics-preserving: interning changes which global a
    /// literal points at, never the bytes it reads. `--no-opt=intern` emits one
    /// global per literal for bisecting.
    pub intern: bool,
    /// Generate poll functions for `async fn`s.
    ///
    /// On by default. `--no-opt=async` disables suspension: every `await`
    /// drives its child inline, so the flag bisects a miscompile without
    /// changing what a program prints.
    pub async_: bool,
}

/// The shape the async pass exposes: a function poll's frame layout and the
/// poll function's name.
pub use async_lower::{lower_async, FrameLayout};

impl Default for CodegenOptions {
    /// The semantics-preserving set: no contraction, no promotion, but the
    /// facts the caller supplied are honoured and identical literals share one
    /// global, because both have been the backend's behaviour since they
    /// landed and neither changes what a program prints.
    fn default() -> Self {
        CodegenOptions {
            fp_contract: false,
            escape: false,
            facts: true,
            intern: true,
            async_: true,
        }
    }
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
pub fn emit_program(
    hir: &HirProgram,
    entry: &str,
    options: CodegenOptions,
    facts: &yashy_common::ProofFacts,
) -> String {
    emit(hir, entry, options, facts, false).0
}

/// Emit a whole program and the reasons behind every decision that survived.
///
/// The IR is identical to [`emit_program`]'s for the same inputs; the second
/// value is the note log the driver renders as `--opt-report`. Collecting is
/// opt-in because the notes are a report about a build, not part of one.
pub fn emit_program_with_notes(
    hir: &HirProgram,
    entry: &str,
    options: CodegenOptions,
    facts: &yashy_common::ProofFacts,
) -> (String, module::OptReport) {
    emit(hir, entry, options, facts, true)
}

fn emit(
    hir: &HirProgram,
    entry: &str,
    options: CodegenOptions,
    facts: &yashy_common::ProofFacts,
    record: bool,
) -> (String, module::OptReport) {
    let mut module = Module::new();
    module.set_source_filename(entry);
    module.set_fp_contract(options.fp_contract);
    module.set_report(record);
    // The registered passes, in their one order. A pass reads its switch from
    // `options`, so disabling one changes only what that pass contributes.
    {
        let mut ctx = pass::PassCtx::new(&mut module, options, facts);
        for pass in pass::pipeline() {
            (pass.run)(&mut ctx);
        }
    }
    // One spawn is enough: the checker recorded that the program can share a
    // string with a thread, so every retain/release in the module is atomic.
    // This has to be set before the first `define` is emitted, because the
    // choice is read when each function's `FnCtx` is built.
    module.set_atomic_strings(hir.uses_threads);
    // Declarations first: every function body may call a C function declared
    // later in the file, and a `declare` before the first `define` is the shape
    // LLVM expects. The table is also what tells the call lowering that a name
    // is extern rather than a user function.
    for item in &hir.items {
        if let HirItem::Extern(block) = item {
            for decl in &block.decls {
                // Two declarations may resolve to one symbol (they describe the
                // same C function); the `declare` is emitted once, keyed by the
                // symbol, and both Yashy names call it.
                if module.mark_extern_symbol(decl.symbol()) {
                    gen_extern_decl(&mut module, decl);
                }
                module.add_extern_decl(decl.clone());
                module.add_fn_signature(
                    &decl.name,
                    decl.params.iter().map(|p| p.ty.kind.clone()).collect(),
                    decl.ret.kind.clone(),
                );
            }
        }
    }
    // Function names are values too. Record every signature before any body is
    // lowered: a thunk may be requested by the first function emitted, while
    // the function it wraps is emitted later. `main` is skipped because its
    // emitted signature is the operating system's `i32`, not its Yashy type.
    for item in &hir.items {
        if let HirItem::Fn(f) = item {
            if f.name != "main" {
                module.add_fn_signature(
                    &f.name,
                    f.params.iter().map(|p| p.ty.kind.clone()).collect(),
                    f.ret.kind.clone(),
                );
            }
        }
    }
    // Named type definitions are emitted once, before any body. A struct or
    // enum used only as another type's payload may never be the subject of a
    // literal, so waiting for a literal to define it would leave an unknown
    // type in the module. These come from the *declarations*, not from the
    // values, so a field's layout cannot depend on which literal was lowered
    // first.
    for item in &hir.items {
        match item {
            HirItem::Struct(s) => {
                let fields = s.fields.iter().map(|f| repr_type(&f.ty.kind)).collect();
                module.define_named_type(&format!("%struct.{}", s.name), fields);
                // The field *types*, not just their representation: a `string`
                // field is a reference the struct owns, and its representation
                // is the same as a raw pointer's.
                module.add_struct_fields(
                    &s.name,
                    s.fields.iter().map(|f| f.ty.kind.clone()).collect(),
                );
                module.add_struct_field_names(
                    &s.name,
                    s.fields.iter().map(|f| f.name.clone()).collect(),
                );
            }
            HirItem::Enum(e) => {
                let variants = e
                    .variants
                    .iter()
                    .map(|v| VariantRepr {
                        name: v.name.clone(),
                        fields: v
                            .fields
                            .iter()
                            .map(|f| (f.name.clone(), f.ty.kind.clone()))
                            .collect(),
                    })
                    .collect();
                module.define_enum(&e.name, variants, e.repr.is_some());
            }
            HirItem::Fn(_) | HirItem::Import(_) | HirItem::Extern(_) => {}
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
    // Synthetic entries are complete `define` blocks that lowering queued
    // while the checked functions were still open; they can only be appended
    // once every one of those has closed.
    for function in module.take_synthesized_functions() {
        for line in function {
            module.push(line);
        }
    }
    let ir = module.finish();
    (ir, module.take_opt_report())
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
/// point (`void` or `int`) and that takes no parameters.
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
            message: "no `fn main` found: every Yashy program must define one".to_string(),
            file,
            span: Span::new(0, 0),
            label: None,
            notes: Vec::new(),
            code: None,
            proof: None,
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
                    notes: Vec::new(),
                    code: None,
                    proof: None,
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
                    message: format!("`main` must return `void` or `int`, found `{}`", main.ret),
                    file,
                    span: main.span,
                    label: None,
                    notes: Vec::new(),
                    code: None,
                    proof: None,
                })
            }
        }
    }
}
