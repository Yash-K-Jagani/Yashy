//! The pass registry: the one order the backend applies its optimization
//! decisions in, and the bench row each pass is held to.
//!
//! The backend is a text emitter (the master plan forbids linking LLVM), so a
//! pass here is not a rewrite over an IR data structure. It is a lowering
//! decision that a later emission step consults: the verifier facts the module
//! may rely on, where a non-escaping allocation lives, whether identical
//! literals share a global. Putting them in one registry is what lets
//! `--opt-list` name them, `--no-opt=<name>` disable one for bisecting, and
//! `yashy-bench --per-pass` hold each to its own row.
//!
//! # Disabling
//!
//! A pass reads its own switch from [`CodegenOptions`] and reports whether it
//! applied. Disabling is therefore per pass, not per compiler: the order and
//! the set of passes do not change, only which ones contribute.
//!
//! # Ownership
//!
//! [`owner`] names the pass responsible for the IR of each `HirExpr` variant.
//! The match is exhaustive on purpose: adding a variant makes `pass.rs` fail to
//! compile until its owner is decided, which is stronger than a test that
//! merely notices the gap later.

use yashy_common::ProofFacts;
use yashy_sema::hir::HirExpr;

use crate::module::Module;
use crate::CodegenOptions;

/// What one pass did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PassStats {
    /// Whether the pass changed the module it was handed.
    pub applied: bool,
}

/// The state a pass runs against: the module being built, the options that say
/// which passes are enabled, and the verifier facts they may read.
pub struct PassCtx<'a> {
    pub module: &'a mut Module,
    pub options: CodegenOptions,
    pub facts: &'a ProofFacts,
}

impl<'a> PassCtx<'a> {
    pub fn new(module: &'a mut Module, options: CodegenOptions, facts: &'a ProofFacts) -> Self {
        PassCtx {
            module,
            options,
            facts,
        }
    }
}

/// One registered pass: a name, the decision it makes, and the bench row it
/// must not regress.
#[derive(Clone, Copy)]
pub struct Pass {
    pub name: &'static str,
    pub run: fn(&mut PassCtx<'_>) -> PassStats,
    pub row: &'static str,
}

/// The one order. New optimization decisions are appended here with the row
/// they are held to; a pass that is not in this list is not a pass.
pub fn pipeline() -> &'static [Pass] {
    &[
        Pass {
            name: "facts",
            run: apply_facts,
            row: "fib_run_ms",
        },
        Pass {
            name: "escape",
            run: apply_escape,
            row: "maps_run_ms",
        },
        Pass {
            name: "intern",
            run: apply_intern,
            row: "hello_binary_bytes",
        },
    ]
}

/// The registered pass called `name`, in pipeline order.
pub fn find(name: &str) -> Option<&'static Pass> {
    pipeline().iter().find(|pass| pass.name == name)
}

/// The valid `--no-opt` names, in order, for a usage message.
pub fn names() -> String {
    pipeline()
        .iter()
        .map(|pass| pass.name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Let the module rely on the verifier's facts.
///
/// Off means the module is handed an empty fact set: every check the proof
/// would delete is kept, and no `noalias`/`nonnull`/`readonly`/`align`
/// attribute is emitted. The program still means the same thing; it is merely
/// proved less.
fn apply_facts(ctx: &mut PassCtx<'_>) -> PassStats {
    if ctx.options.facts {
        ctx.module.set_facts(ctx.facts.clone());
    } else {
        ctx.module.set_facts(ProofFacts::empty());
    }
    PassStats {
        applied: ctx.options.facts,
    }
}

/// Let non-escaping allocations become frame slots or block-arena slots.
fn apply_escape(ctx: &mut PassCtx<'_>) -> PassStats {
    ctx.module.set_escape(ctx.options.escape);
    PassStats {
        applied: ctx.options.escape,
    }
}

/// Let identical string literals share one global.
fn apply_intern(ctx: &mut PassCtx<'_>) -> PassStats {
    ctx.module.set_intern(ctx.options.intern);
    PassStats {
        applied: ctx.options.intern,
    }
}

/// The pass that owns a construct's IR.
///
/// Most variants are ordinary lowering and name `lower`, which is not an
/// optimization and cannot be disabled. A variant whose IR an optimization
/// changes names that pass, so a reader can tell which switch controls the IR
/// they are looking at.
pub fn owner(expr: &HirExpr) -> &'static str {
    match expr {
        // A literal's global is shared by the interning pass.
        HirExpr::Literal(..) => "intern",
        // A proven index loses its bounds check under the facts pass.
        HirExpr::Index { .. } => "facts",
        // A moved or addressed binding is what escape analysis follows.
        HirExpr::Move(..) | HirExpr::AddrOf { .. } => "escape",
        HirExpr::Ident(..)
        | HirExpr::Binary { .. }
        | HirExpr::Unary { .. }
        | HirExpr::Call { .. }
        | HirExpr::StructLit { .. }
        | HirExpr::SliceLit { .. }
        | HirExpr::FieldAccess { .. }
        | HirExpr::FixedArrayLit { .. }
        | HirExpr::View { .. }
        | HirExpr::TupleLit { .. }
        | HirExpr::Cast { .. }
        | HirExpr::Try { .. }
        | HirExpr::Await { .. }
        | HirExpr::Spawn { .. }
        | HirExpr::SwitchExpr { .. }
        | HirExpr::Lambda { .. }
        | HirExpr::Capture { .. }
        | HirExpr::CallCapture { .. }
        | HirExpr::HigherOrder { .. } => "lower",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use yashy_common::Span;
    use yashy_parser::ast::Literal;
    use yashy_sema::ty::{Type, TypeKind};

    fn literal() -> HirExpr {
        HirExpr::Literal(
            Literal::I64(1),
            Span::new(0, 1),
            Type {
                kind: TypeKind::I64,
                span: Span::new(0, 1),
            },
        )
    }

    #[test]
    fn the_pipeline_names_every_pass_once() {
        let mut seen = Vec::new();
        for pass in pipeline() {
            assert!(!seen.contains(&pass.name), "duplicate pass `{}`", pass.name);
            seen.push(pass.name);
        }
        assert_eq!(seen, ["facts", "escape", "intern"]);
    }

    #[test]
    fn every_registered_pass_has_a_bench_row() {
        for pass in pipeline() {
            assert!(!pass.row.is_empty(), "`{}` has no row", pass.name);
            assert!(
                pass.row.ends_with("_ms") || pass.row.ends_with("_bytes"),
                "`{}` names `{}`, which is not a gate row",
                pass.name,
                pass.row
            );
        }
    }

    #[test]
    fn a_literal_is_owned_by_interning() {
        assert_eq!(owner(&literal()), "intern");
    }

    #[test]
    fn an_owner_is_the_pass_name_or_the_mandatory_lowering() {
        let owner = owner(&literal());
        assert!(
            owner == "lower" || find(owner).is_some(),
            "`{owner}` is not a registered pass"
        );
    }
}
