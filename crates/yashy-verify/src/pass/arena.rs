//! The arena pass: an `arena { }` block releases everything it owns.
//!
//! The language promises that an explicit arena block frees every allocation
//! written inside it, on every exit -- fall-through, `return`, `break`,
//! `continue`, and panic. That promise is sound only if no value owned by the
//! block is used after the block ends, so the verifier refuses two uses:
//!
//! * `E1003`: the value crosses the boundary. It is returned from the
//!   function, stored in a binding declared outside the block, stored in a
//!   struct field or slice element, or handed to `thread_spawn`, which keeps
//!   it alive on another thread.
//! * `E1004`: the value is freed individually with `.free()`. The block frees
//!   it at exit, so an individual free would be a double free.
//!
//! The findings come from the same escape analysis code generation uses to
//! place the allocation ([`yashy_hir::escape::arena_findings`]). The pass never
//! re-derives which allocations belong to a block: two rules that disagree
//! about an arena are worse than one, because a build that skipped the
//! verifier would then emit code the verifier had accepted.
//!
//! Ordinary call arguments are **not** crossings. A callee that receives a
//! pointer may read and write it while the block is live; ownership does not
//! move. The escape analysis already treats a call argument as an escape for
//! promotion, which keeps the arena from relocating a value the analysis could
//! not follow.

use yashy_common::{codes, Diagnostic, FileId, ProofTrace, Severity};
use yashy_hir::escape::{arena_findings, ArenaFindingKind};
use yashy_hir::hir::{HirFn, HirItem, HirProgram};

/// Run the arena pass over every function in `program`.
pub fn check(program: &HirProgram, out: &mut Vec<Diagnostic>) {
    for item in &program.items {
        if let HirItem::Fn(function) = item {
            check_fn(function, out);
        }
    }
}

fn check_fn(function: &HirFn, out: &mut Vec<Diagnostic>) {
    let facts = yashy_common::ProofFacts::empty();
    let file = FileId::from_index(0);
    for finding in arena_findings(function, &facts) {
        let (code, message, missing_fact, remedy) = match finding.kind {
            ArenaFindingKind::Escape => (
                codes::ARENA_ESCAPE,
                format!(
                    "arena value `{}` is used after its `arena {{ }}` block ends",
                    finding.name
                ),
                format!(
                    "`{}` stays inside the `arena {{ }}` block that frees it",
                    finding.name
                ),
                format!(
                    "use `{}` inside the block, or allocate it outside the block",
                    finding.name
                ),
            ),
            ArenaFindingKind::Free => (
                codes::ARENA_FREE,
                format!(
                    "arena value `{}` is freed individually; the `arena {{ }}` block frees it",
                    finding.name
                ),
                format!(
                    "`{}` is not released before the `arena {{ }}` block ends",
                    finding.name
                ),
                format!(
                    "drop the `.free()` on `{}` and let the block release it",
                    finding.name
                ),
            ),
        };
        out.push(
            Diagnostic::new(Severity::Error, file, finding.span, message)
                .with_label("the `arena { }` block that owns this value ends here")
                .with_note("the block frees every allocation written inside it, on every exit path")
                .with_code(code)
                .with_proof(ProofTrace {
                    obligation: "arena",
                    missing_fact,
                    remedy,
                }),
        );
    }
}
