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
pub use fn_gen::{
    fn_return_repr, gen_extern_decl, gen_fn, gen_fn_decl, gen_main, gen_main_with_symbol,
};
pub use ir::{llvm_type_string, repr_type, variant_payload_index, variant_tag};
pub use module::data_layout;
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
    /// The triple the module header and data layout are written for.
    ///
    /// The host unless `--target` says otherwise. It is the one value the IR,
    /// the `cfg` filter and the `clang --target` line share; see
    /// [`Module::set_target`].
    pub target: &'static str,
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
            target: module::host_target_triple(),
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
    emit(hir, entry, options, facts, false, None, false, None).0
}

/// Emit one LLVM IR module per source file, in the order `units` gives.
///
/// Each unit carries every declaration and type definition the program has —
/// the same pre-pass a whole-program emission runs — but only the function
/// bodies written in its own file. That is what makes the units separable: a
/// unit's text depends on the whole program's *declarations* and on its own
/// bodies, so editing one function body changes exactly one unit's text and
/// the other units' cached objects stay valid.
///
/// `units` is `(file, source_filename)` in a stable order. A single unit is the
/// whole program and produces the same text [`emit_program`] would, with no
/// symbol suffixes; with more than one, synthesized helpers (`__fnvalue_*`,
/// `__lambda_*`, `yashy_thread_entry_*`) gain a `_u<index>` suffix so two units
/// can define one without colliding at link time. A body whose span names no
/// unit is emitted by the first unit, so no item can be dropped.
pub fn emit_units(
    hir: &HirProgram,
    units: &[(FileId, String)],
    options: CodegenOptions,
    facts: &yashy_common::ProofFacts,
) -> Vec<String> {
    emit_units_inner(hir, units, options, facts, false)
}

/// [`emit_units`] with statement-span markers for the debug pass.
pub fn emit_units_with_debug(
    hir: &HirProgram,
    units: &[(FileId, String)],
    options: CodegenOptions,
    facts: &yashy_common::ProofFacts,
) -> Vec<String> {
    emit_units_inner(hir, units, options, facts, true)
}

fn emit_units_inner(
    hir: &HirProgram,
    units: &[(FileId, String)],
    options: CodegenOptions,
    facts: &yashy_common::ProofFacts,
    debug_locations: bool,
) -> Vec<String> {
    (0..units.len())
        .map(|index| emit_unit_at(hir, units, index, options, facts, debug_locations))
        .collect()
}

/// Emit the IR for one of `units`.
///
/// This is the parallel-safe entry point: it borrows the program read-only and
/// writes nothing shared, so a driver may emit every unit on its own thread.
/// The unit's index is the whole of its identity, which is why the synthesized
/// symbol suffix and the body filter both come from `index` alone and not from
/// any accumulation order.
pub fn emit_unit(
    hir: &HirProgram,
    units: &[(FileId, String)],
    index: usize,
    options: CodegenOptions,
    facts: &yashy_common::ProofFacts,
) -> String {
    emit_unit_at(hir, units, index, options, facts, false)
}

fn emit_unit_at(
    hir: &HirProgram,
    units: &[(FileId, String)],
    index: usize,
    options: CodegenOptions,
    facts: &yashy_common::ProofFacts,
    debug_locations: bool,
) -> String {
    // One slot per source file, indexed by the file's own number. `SourceMap`
    // hands out contiguous ids from zero, so a vector is both total and
    // cheaper than a map here.
    let mut assignment = vec![0usize; source_count(units)];
    for (position, (file, _)) in units.iter().enumerate() {
        let slot = file.index() as usize;
        if let Some(owner) = assignment.get_mut(slot) {
            if *owner == usize::MAX || position < *owner {
                *owner = position;
            }
        }
    }
    let (_, name) = &units[index];
    let filter = UnitFilter {
        assignment: &assignment,
        unit: index,
    };
    let tag = if units.len() > 1 {
        format!("_u{index}")
    } else {
        String::new()
    };
    emit(
        hir,
        name,
        options,
        facts,
        false,
        None,
        debug_locations,
        Some((filter, &tag)),
    )
    .0
}

/// Which unit a function body belongs to.
///
/// A vector rather than a map: `SourceMap` numbers files from zero without
/// gaps, so `assignment[file.index()]` is total, and `FileId` deliberately
/// implements neither `Hash` nor `Ord`.
struct UnitFilter<'a> {
    assignment: &'a [usize],
    unit: usize,
}

impl UnitFilter<'_> {
    /// Whether this item's body is emitted by the unit being written. An item
    /// whose file belongs to no unit goes to unit zero, the entry's, so a
    /// synthetic item is emitted exactly once.
    fn owns(&self, file: FileId) -> bool {
        self.assignment
            .get(file.index() as usize)
            .copied()
            .unwrap_or(0)
            == self.unit
    }
}

/// One past the highest file number the units name.
fn source_count(units: &[(FileId, String)]) -> usize {
    units
        .iter()
        .map(|(file, _)| file.index() as usize + 1)
        .max()
        .unwrap_or(0)
}

/// Emit a whole program with statement-span markers for the debug pass.
///
/// The markers are comment lines in the IR; a normal build emits none of them,
/// so golden IR stays byte-identical. The driver converts them to `!dbg`
/// attachments and LLVM metadata after emission.
pub fn emit_program_with_debug(
    hir: &HirProgram,
    entry: &str,
    options: CodegenOptions,
    facts: &yashy_common::ProofFacts,
) -> String {
    emit(hir, entry, options, facts, false, None, true, None).0
}

/// Emit a whole program whose `fn main` uses `entry_symbol` instead of `@main`.
///
/// The test harness renames each test's entry point to `__yashy_test_<n>` so
/// many tests can compile to separate objects and link into one binary.
/// Entry-point semantics (the `i32(i32, i8**)` signature and runtime
/// initialisation) are unchanged; only the emitted symbol differs.
pub fn emit_program_with_entry(
    hir: &HirProgram,
    entry: &str,
    options: CodegenOptions,
    facts: &yashy_common::ProofFacts,
    entry_symbol: &str,
) -> String {
    emit(
        hir,
        entry,
        options,
        facts,
        false,
        Some(entry_symbol),
        false,
        None,
    )
    .0
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
    emit(hir, entry, options, facts, true, None, false, None)
}

fn emit(
    hir: &HirProgram,
    entry: &str,
    options: CodegenOptions,
    facts: &yashy_common::ProofFacts,
    record: bool,
    entry_symbol: Option<&str>,
    debug_locations: bool,
    unit: Option<(UnitFilter<'_>, &str)>,
) -> (String, module::OptReport) {
    let mut module = Module::new();
    module.set_debug_locations(debug_locations);
    module.set_source_filename(entry);
    if let Some((_, tag)) = &unit {
        module.set_unit_tag(tag);
    }
    module.set_target(options.target);
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
                let order = if s.field_order.len() == s.fields.len() {
                    s.field_order.clone()
                } else {
                    (0..s.fields.len()).collect()
                };
                let fields = order
                    .iter()
                    .map(|&index| repr_type(&s.fields[index].ty.kind))
                    .collect();
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
                module.add_struct_field_order(&s.name, order);
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
                if let Some((filter, _)) = &unit {
                    if !filter.owns(f.file) {
                        // Another unit emits this body. All this one needs is
                        // the signature, so the call can be lowered.
                        gen_fn_decl(&mut module, f);
                        continue;
                    }
                }
                // The entry point is the one function whose signature the
                // operating system dictates, so it is emitted through its own
                // path. The test harness renames each test's `main` to a unique
                // symbol (`__yashy_test_<n>`) while keeping entry-point
                // semantics, so many tests can link into one harness binary.
                // `__yashy_test_<n>` is a renamed entry point emitted by the
                // test harness: several of them can share one module, so each
                // needs the entry signature and the OS symbol is its own name.
                if f.name == "main" || f.name.starts_with("__yashy_test_") {
                    if let Some(symbol) = entry_symbol {
                        gen_main_with_symbol(&mut module, f, symbol);
                    } else {
                        gen_main_with_symbol(&mut module, f, &f.name);
                    }
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
            suggestions: Vec::new(),
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
                    suggestions: Vec::new(),
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
                    suggestions: Vec::new(),
                })
            }
        }
    }
}
