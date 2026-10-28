# Phase 1a — Verifier Core, Usable: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a verifier that owns the language's ownership rules, catches leaks and `defer` mistakes, runs contracts in warning mode, and reports every result as both human output and schema-v1 JSON — with per-phase timings, so the rest of the plan is measurable.

**Architecture:** A new `yashy-verify` crate holds the ownership rule set and a whole-function pass over the typed HIR. `yashy-sema` delegates its move/free decisions to that rule set instead of owning a copy. The HIR and its `Type` move to a new `yashy-hir` crate so `sema` can depend on `verify` without a dependency cycle. Contracts are parsed into the HIR and discharged by an internal interval domain; in this slice an unproved contract is a **warning**, not an error.

**Tech Stack:** Rust (workspace crate, no external dependencies), `yashy-common` diagnostics, the existing `clap`-free argument parsing in `crates/yashy-driver/src/main.rs`, and the existing schema-v1 JSON writer in `crates/yashy-common/src/json.rs`.

**Spec:** `docs/superpowers/specs/2026-10-27-yashy-master-design.md` §4 (verification), §6.1 (compiler observability), §8 row 1a. Master index: `docs/superpowers/plans/2026-10-27-yashy-master-plan.md`.

## Global Constraints

Copied from the master plan; every task includes them.

- No GC, borrow checker, exceptions, implicit coercion, operator overloading, class inheritance, or procedural macros.
- Ownership is explicit: `move` transfers, `defer` releases at scope exit, `.free()` releases.
- Tests are ordinary `.ya` programs with `main`; exit 0 is a pass.
- `yashy-verify` takes **no external crate dependency**.
- A malformed program produces a diagnostic, never a Rust panic (`never_panics` suite).
- One rule, one implementation: `yashy-verify` decides ownership; `sema` calls it.
- Both operating systems or it is not done.
- Do not claim macOS support.
- Format touched files only; `cargo fmt --all --check` has pre-existing drift.
- No unproven claim: no safety claim without a proof or a test; no speed claim without a number from both operating systems.

## Review Focus

Detail in the master index; the five failure modes this slice's tests must pin:

1. **Ownership handed out is not a leak** — returned, stored in a struct field, pushed into a slice, or inserted into a map.
2. **The merge point uses the right join for each question** — union for "may have been moved" (a read after a freeing branch is rejected), intersection for "released on every path" (a leak on the branch that skipped the release is reported).
3. **A contract naming an unknown identifier reports the identifier**, not "cannot prove".
4. **`--no-verify` always leaves evidence** — a warning on stderr and the `.unverified` sidecar; `YASHY_REQUIRE_VERIFY=1` refuses it.
5. **`--timings` never changes program output** — stderr and JSON only.

---

### Task 1: Extract the typed IR into `yashy-hir`

**Files:**
- Create: `crates/yashy-hir/Cargo.toml`
- Create: `crates/yashy-hir/src/lib.rs`
- Create: `crates/yashy-hir/src/hir.rs` (moved from `crates/yashy-sema/src/hir.rs`, content unchanged)
- Create: `crates/yashy-hir/src/ty.rs` (moved from `crates/yashy-sema/src/ty.rs`, content unchanged)
- Modify: `Cargo.toml` (add the workspace member)
- Modify: `crates/yashy-sema/Cargo.toml` (depend on `yashy-hir`)
- Modify: `crates/yashy-sema/src/lib.rs` (delete `pub mod hir;` and `pub mod ty;`, add re-exports)
- Delete: `crates/yashy-sema/src/hir.rs`, `crates/yashy-sema/src/ty.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `yashy_hir::hir` (all HIR types), `yashy_hir::ty::{Type, TypeKind, Mutability}`. `yashy_sema` re-exports both as `yashy_sema::hir` and `yashy_sema::ty`, so every existing import keeps working.

- [x] **Step 1: Create the crate and move the two files**

`crates/yashy-hir/Cargo.toml` declares `name = "yashy-hir"`, `edition = "2021"`, and dependencies `yashy-common` and `yashy-parser` (path deps, matching the style of `crates/yashy-sema/Cargo.toml`). No other dependency.

`crates/yashy-hir/src/lib.rs`:

```rust
//! The typed IR and its type representation.
//!
//! These lived inside `yashy-sema` until phase 1a. They are their own crate
//! because `yashy-verify` must read the HIR while `yashy-sema` calls into the
//! verifier's ownership rules, and two crates cannot depend on each other.

pub mod hir;
pub mod ty;
```

Move `crates/yashy-sema/src/hir.rs` and `crates/yashy-sema/src/ty.rs` with `git mv`, byte for byte. Their `use crate::ty::…` and `use yashy_common::…` lines already resolve inside the new crate.

- [x] **Step 2: Make `yashy-sema` re-export instead of own**

In `crates/yashy-sema/src/lib.rs`, replace `pub mod hir;` and `pub mod ty;` with:

```rust
pub use yashy_hir::{hir, ty};
```

Leave `pub use hir::*;` and `pub use ty::{Mutability, Type, TypeKind};` untouched.

- [x] **Step 3: Build the workspace and confirm nothing changed**

Run: `cargo check --workspace --all-targets`
Expected: PASS with no new warnings. `yashy-codegen` and `yashy-driver` need no edits because their `yashy_sema::hir::…` paths now resolve through the re-export.

- [x] **Step 4: Prove the move is a move**

Run: `cargo test --workspace`
Expected: every test passes, and the count equals the count before this task. Record both counts in the commit message.

Run: `git status --short crates/yashy-sema/src/hir.rs crates/yashy-sema/src/ty.rs`
Expected: both show as deleted (the content is added under `crates/yashy-hir/src/`).

- [x] **Step 5: Commit**

```bash
git add -A
git commit -m "refactor: move the typed IR into yashy-hir so the verifier can read it"
```

---

### Task 2: Move the ownership rules into `yashy-verify`; `sema` delegates

**Files:**
- Create: `crates/yashy-verify/Cargo.toml`
- Create: `crates/yashy-verify/src/lib.rs`
- Create: `crates/yashy-verify/src/ownership.rs` (from `crates/yashy-sema/src/ownership.rs`, plus the two rule functions)
- Modify: `Cargo.toml`, `crates/yashy-sema/Cargo.toml` (depend on `yashy-verify`)
- Modify: `crates/yashy-sema/src/lib.rs` (re-export from `yashy_verify`)
- Modify: `crates/yashy-sema/src/scope.rs:146-181` (`move_out`, `check_live` call the verifier's rules)
- Delete: `crates/yashy-sema/src/ownership.rs`

**Interfaces:**
- Consumes: `yashy_hir::ty::TypeKind`.
- Produces:
  - `yashy_verify::ownership::{Ownership, Linearity, linearity_of, AlreadyMoved, move_out, check_live}`
  - `pub enum Ownership { Live, Moved }` (default `Live`)
  - `pub enum Linearity { Linear, Copy }`
  - `pub fn linearity_of(ty: &TypeKind) -> Linearity`
  - `pub struct AlreadyMoved { pub name: String }`
  - `pub fn move_out(state: &mut Ownership, name: &str) -> Result<(), AlreadyMoved>`
  - `pub fn check_live(state: Ownership, name: &str) -> Result<(), AlreadyMoved>`

- [x] **Step 1: Write the failing test**

In `crates/yashy-verify/src/ownership.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_move_is_rejected_and_names_the_binding() {
        let mut state = Ownership::Live;
        assert_eq!(move_out(&mut state, "buf"), Ok(()));
        assert_eq!(
            move_out(&mut state, "buf"),
            Err(AlreadyMoved { name: "buf".to_string() })
        );
    }

    #[test]
    fn a_rebound_binding_is_live_again() {
        let mut state = Ownership::Moved;
        state = Ownership::Live;
        assert_eq!(check_live(state, "buf"), Ok(()));
    }

    #[test]
    fn a_moved_binding_cannot_be_read() {
        assert_eq!(
            check_live(Ownership::Moved, "buf"),
            Err(AlreadyMoved { name: "buf".to_string() })
        );
    }

    #[test]
    fn pointers_slices_maps_mutexes_and_channels_are_linear() {
        assert_eq!(linearity_of(&TypeKind::Ptr(Box::new(TypeKind::I64))), Linearity::Linear);
        assert_eq!(linearity_of(&TypeKind::Slice(Box::new(TypeKind::I64))), Linearity::Linear);
        assert_eq!(linearity_of(&TypeKind::Mutex), Linearity::Linear);
        assert_eq!(linearity_of(&TypeKind::I64), Linearity::Copy);
    }
}
```

- [x] **Step 2: Run it and watch it fail**

Run: `cargo test -p yashy-verify`
Expected: FAIL — the crate does not exist yet.

- [x] **Step 3: Create the crate and move the rules**

`crates/yashy-verify/Cargo.toml`: `name = "yashy-verify"`, dependencies `yashy-common` and `yashy-hir` only.

`crates/yashy-verify/src/lib.rs`:

```rust
//! The verifier: the single owner of Yashy's static rules.
pub mod ownership;

pub use ownership::{check_live, linearity_of, move_out, AlreadyMoved, Linearity, Ownership};
```

`ownership.rs` carries the doc comment, `Linearity`, `Ownership` and `linearity_of` from `crates/yashy-sema/src/ownership.rs` verbatim, plus the two rule functions:

```rust
/// Spend a binding's value. A second spend of the same binding is the error
/// that stops a double free.
pub fn move_out(state: &mut Ownership, name: &str) -> Result<(), AlreadyMoved> {
    match *state {
        Ownership::Moved => Err(AlreadyMoved { name: name.to_string() }),
        Ownership::Live => {
            *state = Ownership::Moved;
            Ok(())
        }
    }
}

/// Reject a read of a binding whose value has been spent.
pub fn check_live(state: Ownership, name: &str) -> Result<(), AlreadyMoved> {
    match state {
        Ownership::Moved => Err(AlreadyMoved { name: name.to_string() }),
        Ownership::Live => Ok(()),
    }
}
```

- [x] **Step 4: Make `sema` call the rules**

`crates/yashy-sema/src/scope.rs`: `move_out` and `check_live` keep their signatures and their `TypeError` construction (message and `codes::MOVED_VALUE` stay byte-identical), but the decision comes from `yashy_verify::ownership`:

```rust
pub fn move_out(&mut self, name: &str, span: Span) -> Result<(), TypeError> {
    // resolve the binding, then:
    //   yashy_verify::move_out(&mut binding.ownership, name)
    //     .map_err(|e| TypeError::new(span, format!("value `{}` has already been moved", e.name))
    //         .with_code(codes::MOVED_VALUE))
    // an unbound name keeps the existing UNKNOWN_NAME diagnostic with its suggestion.
}
```

`crates/yashy-sema/src/lib.rs` replaces its `pub mod ownership;` with `pub use yashy_verify::ownership::{linearity_of, Linearity, Ownership};` so `yashy_sema::Ownership` and `crate::ownership::` paths keep working.

- [x] **Step 5: Run the whole suite**

Run: `cargo test -p yashy-verify && cargo test --workspace`
Expected: PASS. The `E0500` message text is unchanged, so `crates/yashy-driver/tests/e2e.rs` and `never_panics.rs` pass without edits.

- [x] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor: make yashy-verify the single owner of the ownership rules"
```

---

### Task 3: `verify` entry point and the leak pass

**Files:**
- Create: `crates/yashy-verify/src/lib.rs` (add `verify`, `VerifyOptions`, `VerifyReport`)
- Create: `crates/yashy-verify/src/pass/mod.rs`, `crates/yashy-verify/src/pass/leak.rs`
- Modify: `crates/yashy-common/src/codes.rs` (add `LEAK = "E1000"`)
- Create: `docs/errors/E1000.md`
- Modify: `docs/errors.md` (add the row)

**Interfaces:**
- Consumes: `yashy_hir::hir::{HirProgram, HirItem, HirFn, HirStmt, HirExpr}`.
- Produces:
  - `pub struct VerifyOptions { pub contracts: ContractMode }`
  - `pub enum ContractMode { Warn, Enforce }`
  - `pub struct VerifyReport { pub diagnostics: Vec<Diagnostic> }`
  - `pub fn verify(hir: &HirProgram, options: &VerifyOptions) -> VerifyReport`
- Dev-only dependency: `yashy-sema`, so the unit tests can parse and check a source string. This is a dev-dependency cycle (`sema` depends on `verify`), which Cargo permits because dev-dependencies are not part of the library build. The library still depends on `yashy-hir` and `yashy-common` only.

- [x] **Step 1: Write the failing test**

`crates/yashy-verify/src/pass/leak.rs`, using a helper that parses and checks a `.ya` source string through `yashy_sema` (add `yashy-sema` as a **dev-dependency** only, so the library keeps its no-cycle rule):

```rust
#[test]
fn an_unfreed_pointer_is_a_leak() {
    let report = verify_source("fn main() { let buf = alloc(16); }");
    assert_eq!(codes(&report), vec!["E1000"]);
    assert!(report.diagnostics[0].message.contains("`buf`"));
}

#[test]
fn freeing_the_pointer_is_not_a_leak() {
    assert!(verify_source("fn main() { let buf = alloc(16); buf.free(); }").diagnostics.is_empty());
}

#[test]
fn returning_the_pointer_is_not_a_leak() {
    let source = "fn make() -> *mut int { let buf = alloc(16); return buf; } fn main() { let p = make(); p.free(); }";
    assert!(verify_source(source).diagnostics.is_empty());
}

#[test]
fn storing_the_pointer_in_a_struct_field_is_not_a_leak() {
    let source = "struct Holder { p: *mut int } fn main() { let buf = alloc(16); let h = Holder { p: buf }; }";
    assert!(verify_source(source).diagnostics.is_empty());
}

#[test]
fn freeing_on_one_branch_and_reading_after_the_merge_is_rejected() {
    let source = "fn main() { let buf = alloc(16); let c = 1 > 0; if c { buf.free(); } else { } buf.free(); }";
    assert_eq!(codes(&verify_source(source)), vec!["E0500"]);
}

#[test]
fn freeing_on_both_branches_is_not_a_leak() {
    let source = "fn main() { let buf = alloc(16); let c = 1 > 0; if c { buf.free(); } else { buf.free(); } }";
    assert!(verify_source(source).diagnostics.is_empty());
}

#[test]
fn freeing_on_only_one_branch_is_a_leak_on_the_other_path() {
    let source = "fn main() { let buf = alloc(16); let c = 1 > 0; if c { buf.free(); } else { } }";
    assert_eq!(codes(&verify_source(source)), vec!["E1000"]);
}
```

Two corrections to the tests as first drafted, made during execution and
recorded here because the plan is the record:

- The language spells the pointer type `*mut int`, not `*mut i64`; `i64` is a
  parse-level error that tells the user to write `int`.
- `h.p.free()` does not type-check: the checker requires a method receiver to be
  a local binding, and reports "a field access is not a binding". The struct
  test therefore checks the hand-off itself (`Holder { p: buf }` releases
  `buf`), which is what the rule is about.

- [x] **Step 2: Run it and watch it fail**

Run: `cargo test -p yashy-verify leak`
Expected: FAIL — `verify_source` and the pass do not exist.

- [x] **Step 3: Add the code and the catalog page**

`codes.rs`: `pub const LEAK: &str = "E1000";` with title `"leaked linear value"`, summary `"A linear value is never released."`, page `include_str!("../../../docs/errors/E1000.md")`, appended to `CODES`.

`docs/errors/E1000.md` follows the existing page shape (`# E1000 — leaked linear value`, `## Cause`, `## Example`, `## Fix`) and its example is the first test above.

`verify` walks each `HirFn` body once and returns diagnostics; the leak pass is:

1. Collect every `HirStmt::Var`/`Const`/`Destructure` binding whose type is `Linear`.
2. Mark a binding **released** when it is: the receiver of a free-family call (`.free()`, `free_slice`, `map_free`, `chan_free`, `mutex_free`); the operand of `HirExpr::Move`; the value of `HirStmt::Return`; a field value of `HirExpr::StructLit`; an element of `HirExpr::SliceLit`/`TupleLit`/`FixedArrayLit`; an argument to `HirExpr::Call`; or the target of `HirStmt::Assign` with `release_previous: true`.
3. Walk each function as a small control-flow graph, not as a flat list. At an `if`/`switch`/loop merge, the released set is the **intersection** of the branches: a binding counts as released only if every path released it. This is the opposite join from the "has been moved" set that `sema` keeps (a union, because any path that moved the value makes a later read unsafe), and both are required.
4. At every function exit (`HirStmt::Return` and the end of the body), report E1000 at the binding's span for each binding that is still live.

Rule 2 is deliberately generous: anything that could hand the value to someone else counts as released, because a false "leak" on correct code is worse than a missed leak that the runtime leak report (`YASHY_LEAK_REPORT=1`) already catches.

`verify_source(source: &str) -> VerifyReport` parses and checks the string with `yashy-sema`, then runs `verify`. **When the check fails, the report is the check diagnostics and nothing else** (dated decision, 2026-10-28): the verifier reasons over the typed HIR, and a program that did not type-check has no HIR to reason over. That is why the test above expects `E0500` alone: `sema`'s union join rejects the second `free` before the verifier ever runs.

- [x] **Step 4: Run the tests**

Run: `cargo test -p yashy-verify`
Expected: PASS, all four.

- [x] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(verify): catch a linear value that is never released (E1000)"
```

---

### Task 4: `defer` discipline (E1001) — withdrawn from 1a, 2026-10-28

> **Dated exception.** This task cannot be built as written, and the finding is
> recorded here rather than worked around.
>
> 1. `defer` takes a **block**, not an expression. `defer buf.free();` does not
>    parse; the spelling is `defer { buf.free(); }`. Every test in this task
>    used the expression form.
> 2. `yashy-sema` checks a `defer` body **eagerly**, at the point the `defer` is
>    written (`Checker::check_stmt`, `Stmt::Defer` -> `check_block_with_ret`).
>    Registering `defer { buf.free(); }` therefore marks `buf` moved from that
>    statement onwards. Every double release this task describes is already
>    `E0500` before the verifier runs: the second `defer`'s body reads a spent
>    binding, and the check fails first.
>
> So `E1001` as specified has **no reachable input** in 1a. Implementing it
> would ship a rule, a catalog page and a public code that never fire — dead
> code with a promise attached, which is worse than an honest gap.
>
> What the eager model does leave open is a different bug, found while
> checking this: a `defer` written inside a **loop** that releases a binding
> declared **outside** the loop. The body is checked once, but it runs at every
> iteration's block exit, so the value is freed once per iteration. That is a
> real, reachable `E1001`, and it belongs with the `defer` model rather than
> with this slice, because the fix may be to make `defer` release at scope exit
> instead of moving the binding eagerly — a language change that the design
> spec's memory section (§8.1) owns.
>
> **Decision:** Task 4 is withdrawn from 1a. `E1001` stays reserved for the
> `defer` rule and is not assigned to anything else. The loop case is written
> down as the first test for phase 1b's `defer` work, and the tutorial's
> description of the eager behaviour (`docs/tutorial/07-memory.md`, and the
> comment at the top of `tests/programs/buffer.ya`) is the evidence that the
> behaviour is known and load-bearing today.

**Files:** none in 1a.

**Acceptance:** `docs/errors.md` does not list `E1001`; `crates/yashy-common/src/codes.rs`
does not define it. The next slice that touches `defer` reopens this task with
the loop test above.

---

### Task 5: `yashy verify` with human and JSON output

**Files:**
- Modify: `crates/yashy-driver/src/main.rs` (dispatch, `fn verify`, JSON)
- Modify: `crates/yashy-driver/src/pipeline.rs` (a `verify` entry that runs check then verify)
- Create: `crates/yashy-driver/tests/verify.rs`
- Modify: `docs/cli.md` (a `## yashy verify` section and the schema additions)

**Interfaces:**
- Consumes: `yashy_verify::{verify, VerifyOptions, ContractMode}`, `VerifyReport`.
- Produces:
  - `yashy verify <file.ya> [--json]` — exit `0` when no errors, `1` when a verifier diagnostic is an error, `2` on usage.
  - JSON envelope: the same keys as `check` plus `"tool": "verify"`, and each diagnostic may carry a `"proof"` object:
    ```json
    "proof": { "obligation": "leak", "missing_fact": "`buf` is not released on every path out of its scope", "remedy": "release it with `buf.free()`, `defer { buf.free(); }`, or by returning it" }
    ```

- [x] **Step 1: Write the failing test**

`crates/yashy-driver/tests/verify.rs`:

```rust
#[test]
fn verify_reports_a_leak_in_human_output() {
    // write a temp .ya with an unfreed alloc, run `yashy verify`, assert exit 1
    // and that stdout/stderr contains "error[E1000]" and the binding name.
}

#[test]
fn verify_json_carries_the_proof_record() {
    // run `yashy verify --json`, parse with the existing test JSON helper,
    // assert diagnostics[0].code == "E1000",
    //        diagnostics[0].proof.obligation == "leak",
    //        diagnostics[0].proof.remedy is a non-empty string,
    //        summary.errors == 1.
}

#[test]
fn verify_accepts_every_program_in_the_examples_and_programs_directories() {
    // for each examples/*/main.ya and crates/yashy-driver/tests/programs/*.ya,
    // run `yashy verify`; assert exit 0.
}
```

- [x] **Step 2: Run and watch it fail**

Run: `cargo test -p yashy-driver --test verify`
Expected: FAIL — `verify` is not a command.

- [x] **Step 3: Implement the command**

`main.rs`: add `"verify" => verify(rest),` beside `"check"`, and a `fn verify(args: &[String]) -> ExitCode` that mirrors `check` (same argument loop, same `read_source_with`, same exit codes) but calls:

```rust
pub fn check_and_verify(
    sources: &mut SourceMap,
    entry_path: &Path,
    entry_source: String,
    options: &VerifyOptions,
) -> Result<(HirProgram, VerifyReport), Vec<Diagnostic>>
```

`check_and_verify` calls `load_and_check`, then `yashy_verify::verify(&hir, options)`. Diagnostics are concatenated: check diagnostics first, then verifier diagnostics.

Add `fn diagnostic_json` support for the optional `"proof"` key by introducing `pub struct ProofTrace { pub obligation: &'static str, pub missing_fact: String, pub remedy: String }` in `yashy-common/src/diagnostic.rs` and an `Option<ProofTrace>` field on `Diagnostic`, set through `with_proof`. `check` output is unchanged because the field is `None` there.

- [x] **Step 4: Make `build` and `run` verify by default**

`build` (`main.rs:193`) and `run` switch from `load_and_check` to `check_and_verify`, so the spec's "an unverified binary is not a build" is true from this slice. A verifier error exits `1` with the same rendering as a check error.

`yashy test` stays on `load_and_check` in 1a and moves to `check_and_verify` in 1b, once the corpus is clean — recorded here so the gap is a decision, not an oversight.

This is the step that will surface false positives: `crates/yashy-driver/tests/e2e.rs` and `examples` now run every program through the verifier. Fix the rule, never the test.

- [x] **Step 5: Run the tests**

Run: `cargo test -p yashy-driver --test verify --test diagnostics_json --test e2e --test examples`
Expected: PASS. `diagnostics_json` passing unchanged is the evidence that the envelope is additive; `e2e` and `examples` passing is the evidence that `build`/`run` did not gain a false positive.

- [x] **Step 6: Document the command**

`docs/cli.md`: a `## yashy verify <file.ya> [--json]` section, a note in `## yashy build` that it verifies by default, and in `## JSON diagnostics (schema v1)` a paragraph for the optional `proof` object with the field names above.

- [x] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(driver): add yashy verify and run the verifier on every build"
```

**Note found in Step 5 (dated, 2026-10-28): `arena_alloc` is a borrow.** The
corpus test failed on `tests/programs/arena.ya`: the pass tracked `first` and
`second` as owning values, because `arena_alloc` returns `*mut int` and nothing
in the type system distinguishes a block the arena owns from a block the caller
owns. The checker already had the fact -- `arena_alloc` is its one exception to
"a linear parameter consumes its argument" -- so the fact moved into
`yashy_hir::hir::is_arena_alloc` and both crates read it. The pass now treats
`arena_alloc`'s first argument as a borrow and does not track its result; a
leaked *arena* is still `E1000`, which a test pins. Because the design has no
borrow checker and no lifetimes (§1), a view type is out of scope, so this
predicate is the permanent shape rather than a stopgap.

**Note found in Step 5 (dated, 2026-10-28): a viewed array is a warning.** The
tutorial harness then failed on `docs/tutorial/09-arrays.md:249`, the view
example. A `[]T` that has been viewed cannot be released -- `xs.free()` is
refused while the view is alive, and the tutorial says so in prose -- so the
program is not wrong; the language has no way to release the value. Reporting an
error would refuse a documented idiom, and `build` now verifies by default, so
the idiom would stop building. Silence would be a verifier that claims to catch
leaks and does not. The pass therefore reports it as `Severity::Warning` with
the same `E1000` code and a proof trace that says why; `verify` exits `0`, and
the tutorial program builds and runs. This is the first real warning, ahead of
1b's `-W` surface, and it is a deviation worth naming: 1b owns the warning
policy, and this one exists because a documented program needs it.

---

### Task 6: Contract syntax (`requires`, `ensures`, `invariant`)

**Files:**
- Modify: `crates/yashy-lexer/src/token.rs` (three keywords; the table lives here, not in `lib.rs`)
- Modify: `crates/yashy-parser/src/ast.rs` (`Contract` on `Item::Fn`, `Stmt::Invariant`)
- Modify: `crates/yashy-parser/src/items.rs`, `crates/yashy-parser/src/stmts.rs` (parse the clauses)
- Modify: `crates/yashy-hir/src/hir.rs` (`HirFn { requires, ensures }`, `HirContract`, `HirStmt::LoopInvariant`)
- Modify: `crates/yashy-sema/src/check_expr.rs` (check contract expressions as `bool`, bind `result`)
- Modify: `crates/yashy-fmt/src/lib.rs` (print the clauses)
- Create: `crates/yashy-parser/tests/contracts.rs`, `crates/yashy-sema/tests/contracts.rs` (the parser and checker tests; the crate's tests are ordinary files under `tests/`, not in-file modules)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub struct HirContract { pub expr: HirExpr, pub span: Span }`
  - `HirFn.requires: Vec<HirContract>`, `HirFn.ensures: Vec<HirContract>`
  - `HirStmt::ForRange`/`ForIn`/`ForC` gain `invariants: Vec<HirContract>` — **or**, if that is too invasive, a `HirStmt::LoopInvariant(HirContract)` statement that the checker accepts only as the first statement of a loop body. Choose the statement form; it needs no change to three existing variants and the verifier reads it in order.

Syntax, final spelling:

```yashy
fn clamp(n: int) -> int
    requires n >= 0
    ensures result >= 0
{
    while n > 10
        invariant n >= 0
    {
        n = n - 1;
    }
    return n;
}
```

- [x] **Step 3: Write the failing parser tests**

`crates/yashy-parser/tests/contracts.rs` (eleven tests): the clauses reach the
tree, keep the span of their expression, repeat, survive `pub`, and a `const fn`
refuses one. `crates/yashy-sema/tests/contracts.rs` (eleven tests): the clauses
reach the HIR, `result` exists only in `ensures`, a non-`bool` clause is
`E0100`, a method contract is refused, and a generic contract is checked per
instantiation.

```rust
#[test]
fn a_function_can_declare_a_precondition_and_a_postcondition() {
    let program = parse("fn f(n: int) -> int requires n > 0 ensures result > 0 { return n; }");
    // assert one Item::Fn whose contracts are [requires n > 0, ensures result > 0]
}

#[test]
fn a_loop_can_declare_an_invariant() {
    let program = parse("fn f(n: int) { while n > 0 invariant n >= 0 { n = n - 1; } }");
    // assert the loop body's first statement is LoopInvariant(n >= 0)
}

#[test]
fn a_contract_without_a_bool_expression_is_a_type_error() {
    // `requires 1` must produce E0100 pointing at the contract expression
}
```

- [x] **Step 4: Run and watch them fail**

Run: `cargo test -p yashy-parser contracts && cargo test -p yashy-sema contracts`
Expected: FAIL — the keywords do not exist.

- [x] **Step 5: Lex and parse**

Lexer: `requires`, `ensures`, `invariant` become keywords (`KEYWORDS` 28 → 31); `result` stays an ordinary identifier, bound only inside `ensures`.

Parser: clauses appear after the return type and before the body, in either order and repeated as written; a loop's `invariant` appears after the loop header and before `{`, and every loop form (`for` range, `for ... in`, C-style `for`, `while`) takes one. An empty clause list is the common case and does not change any existing parse. The HIR carries `HirContract { expr, span }`; a loop invariant is `HirStmt::LoopInvariant(HirContract)`, accepted only among the first statements of a loop body — the statement form avoids changing three existing loop variants and the verifier reads it in order.

Three decisions taken while building this:

- **`const fn` refuses a clause at parse time.** It runs at compile time and has no call site to check one against, so a clause written there is an error rather than a clause silently dropped.
- **A method refuses a clause in the checker.** Discharging it at every call through a receiver is not part of this slice; refusing is the honest answer, because a contract the compiler ignores is worse than one it rejects.
- **The clauses are ordinary expressions with no purity rule yet.** Nothing is marked `pure` in this slice, so the check that a clause cannot call an impure function waits for effects in phase 2.

- [x] **Step 6: Check and format**

Sema: every contract expression is checked as `bool` (`E0100` on mismatch). `requires` is checked with the parameters in scope and no `result`; `ensures` is checked in a scope of its own where `result` is declared with the function's return type, so neither the body nor a precondition can name it. A generic function's clauses are checked once per substitution, like its body.

Formatter: contracts print one per line, indented four spaces, and the body brace starts its own line at the signature's indent. A `{` that looks like a struct literal (`{ ident :`) belongs to the clause's expression, not to the body, so the clause list ends at the first brace that is not one.

- [x] **Step 7: Run the tests**

Run: `cargo test -p yashy-parser -p yashy-sema -p yashy-fmt`
Expected: PASS, including the existing formatter round-trip tests.

Result: 12 fmt tests, 11 parser contract tests, 11 sema contract tests; full workspace 1229 tests, exit 0.

- [x] **Step 8: Commit**

```bash
git add -A
git commit -m "feat: parse requires, ensures and loop invariants into the HIR"
```

Committed as `12bc350`.

---

### Task 7: Discharge contracts in warn mode (E1100)

**Files:**
- Create: `crates/yashy-verify/src/domain.rs` (interval + congruence)
- Create: `crates/yashy-verify/src/pass/contracts.rs`
- Modify: `crates/yashy-verify/src/lib.rs`
- Modify: `crates/yashy-common/src/codes.rs` (`CONTRACT = "E1100"`)
- Create: `docs/errors/E1100.md`, modify `docs/errors.md`

**Interfaces:**
- Consumes: `HirContract`, `HirFn.requires/ensures`, `HirStmt::LoopInvariant`.
- Produces: `E1100` diagnostics whose severity is `Warning` under `ContractMode::Warn` and `Error` under `ContractMode::Enforce`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_provable_precondition_is_silent() {
    let source = "fn f(n: int) -> int requires n > 0 ensures result > 0 { return n; } fn main() { let x = f(1); }";
    assert!(verify_source(source).diagnostics.is_empty());
}

#[test]
fn an_unprovable_precondition_warns_and_names_the_fact() {
    let source = "fn f(n: int) -> int requires n > 0 { return n; } fn main() { let x = f(-1); }";
    let report = verify_source(source);
    assert_eq!(codes(&report), vec!["E1100"]);
    assert_eq!(report.diagnostics[0].severity, Severity::Warning);
    assert!(report.diagnostics[0].message.contains("n > 0"));
}

#[test]
fn a_contract_naming_an_unknown_identifier_is_a_type_error_not_a_proof_failure() {
    let source = "fn f(n: int) -> int requires m > 0 { return n; } fn main() { }";
    assert_eq!(codes(&verify_source(source)), vec!["E0001"]);
}
```

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test -p yashy-verify contracts`
Expected: FAIL — no contract pass.

- [ ] **Step 3: Implement the domain and the pass**

`domain.rs`: a value is an interval `[lo, hi]` over `i64` plus a congruence `(modulus, residue)`; `join` widens to the union hull, `widen` after two iterations drops to `[-inf, inf]`. Operations: `+`, `-`, `*` by a constant, comparisons, and `len()` for a slice whose length is a literal. No other operation is modelled.

`contracts.rs`, for each function:

1. Seed the environment with each parameter's declared type range (`int` = `[i64::MIN, i64::MAX]`).
2. Add the `requires` facts to the environment (they are assumptions inside the body).
3. Walk the body, updating the environment; a `LoopInvariant` is assumed at the loop head and **must** be re-established at the back edge — if it is not, E1100 with `obligation: "invariant"`.
4. At each `return`, check every `ensures` with `result` bound to the returned expression's range. If a fact cannot be established, E1100 with `obligation: "ensures"`, `missing_fact` naming the clause text, and `remedy: "add a `requires` that implies this, or simplify the clause"`.
5. At a call site, check the callee's `requires` against the argument ranges. Failure is E1100 with `obligation: "requires"`.

Anything the domain cannot decide is a failure, never an assumption. `ContractMode::Warn` sets `Severity::Warning`; the driver's `verify` uses `Warn` in this slice, and `Enforce` is wired but unused until 1b.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p yashy-verify && cargo test -p yashy-driver --test verify`
Expected: PASS. The examples corpus still verifies clean because a warning does not fail the run.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(verify): discharge contracts in an interval domain, warn in 1a (E1100)"
```

---

### Task 8: `--timings`

**Files:**
- Modify: `crates/yashy-driver/src/pipeline.rs` (`PhaseTiming`, `load_and_check_timed`)
- Modify: `crates/yashy-driver/src/main.rs` (flag on `check`, `build`, `run`, `test`, `verify`; stderr and JSON output)
- Modify: `crates/yashy-driver/tests/verify.rs` (a timing test)

**Interfaces:**
- Consumes: `std::time::Instant`.
- Produces:
  - `pub struct PhaseTiming { pub phase: &'static str, pub ms: u64 }`
  - `pub fn load_and_check_timed(sources, entry_path, entry_source, release, timings: &mut Vec<PhaseTiming>) -> Result<HirProgram, Vec<Diagnostic>>`
  - Human form on stderr: `timing: check 41 ms`
  - JSON form: `"timings": [{"phase": "check", "ms": 41}]`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn timings_go_to_stderr_and_the_json_report_and_never_to_stdout() {
    // `yashy run hello.ya --timings` stdout equals `yashy run hello.ya` stdout.
    // stderr contains a line starting with "timing: ".
    // `yashy check hello.ya --json --timings` has a non-empty "timings" array
    //   whose phases include "load", "check" and "verify".
}
```

- [ ] **Step 2: Run and watch it fail**

Run: `cargo test -p yashy-driver --test verify timings`
Expected: FAIL — unknown option `--timings`.

- [ ] **Step 3: Instrument the phases**

Phases measured: `load` (module loading), `check` (`check_modules_with`), `verify`, `emit` (`emit_program`), `link` (`clang`). `load_and_check` becomes a wrapper that calls `load_and_check_timed` with a scratch vector, so no call site changes.

`--timings` is accepted by every command that compiles; it prints after the work finishes, to stderr, one line per phase in order, and adds the array to the JSON envelope. It never writes to stdout.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p yashy-driver --test verify --test diagnostics_json`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(driver): add --timings so phase budgets are measurable"
```

---

### Task 9: Narrow `--no-verify` and stamp provenance

**Files:**
- Modify: `crates/yashy-driver/src/main.rs` (`build`, `run`: accept the flag, refuse it with contracts, refuse it under `YASHY_REQUIRE_VERIFY`)
- Modify: `crates/yashy-driver/src/pipeline.rs` (sidecar write/remove)
- Create: `crates/yashy-driver/tests/unverified.rs`

**Interfaces:**
- Consumes: `HirProgram` (to detect contracts).
- Produces: sidecar file `<output>.unverified` containing
  `{"schema_version":1,"verified":false,"reason":"--no-verify","yashy_version":"0.1.0"}`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn no_verify_writes_a_sidecar_and_warns() {
    // build hello.ya with --no-verify; assert stderr contains "warning:" and
    // that hello.unverified exists and parses to verified: false.
}

#[test]
fn a_verified_build_removes_a_stale_sidecar() {
    // build with --no-verify, then build without it; assert the sidecar is gone.
}

#[test]
fn no_verify_is_refused_when_the_program_has_a_contract() {
    // a program with `requires n > 0`; assert exit 2 and a message naming
    // the function and the flag.
}

#[test]
fn require_verify_makes_no_verify_a_usage_error() {
    // YASHY_REQUIRE_VERIFY=1 with --no-verify; assert exit 2 and the env var
    // named in the message.
}
```

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test -p yashy-driver --test unverified`
Expected: FAIL — the flag does not exist.

- [ ] **Step 3: Implement**

Refusals are `EXIT_USAGE` (2) with a one-line message naming the reason; they are not diagnostics, because nothing is wrong with the program. The sidecar is written after a successful link and removed after a verified build; a failed link removes neither.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p yashy-driver --test unverified`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(driver): make --no-verify narrow, loud and traceable"
```

---

### Task 10: Realistic corpus, gate, and documentation

**Files:**
- Create: `crates/yashy-driver/tests/programs/file_cli.ya` (reads a file, counts lines and words, writes a report, frees everything)
- Create: `crates/yashy-driver/tests/verify_corpus/` with one `.ya` per seeded bug and a table in `crates/yashy-driver/tests/verify.rs` naming the expected code
- Modify: `AGENTS.md` (add `yashy verify`, `--timings` to the command list; note the phase plan)
- Modify: `llms.txt`, `README.md` (the verify command)
- Modify: `docs/tutorial/` (one chapter section: "Proving your program", using a leak and a contract)

**Interfaces:**
- Consumes: everything above.
- Produces: the phase gate.

- [ ] **Step 1: Add the realistic program**

`file_cli.ya` must be a correct program: it allocates, writes, frees, and exits 0. It is compiled and run by the existing `e2e` harness, and verified by `verify` with no diagnostics. This is the anti-false-positive test that matters most.

- [ ] **Step 2: Add the seeded-bug corpus**

One file per bug class, named for it: `leak_unfreed.ya`, `leak_branch.ya`, `leak_loop.ya`, `defer_twice.ya`, `defer_after_free.ya`, `contract_requires.ya`, `contract_ensures.ya`, `contract_invariant.ya`. The table in `verify.rs` asserts the exact code set for each, in order.

- [ ] **Step 3: Run the gate**

```bash
cargo test --workspace
cargo test -p yashy-driver --test verify --test unverified --test e2e --test never_panics
cargo test -p yashy-driver --test examples --test tutorial
```

Expected on Windows and Linux: all pass. Then:

```bash
yashy verify examples/wordcount/main.ya
yashy verify examples/json-report/main.ya
yashy verify crates/yashy-driver/tests/programs/file_cli.ya
```

Expected: exit 0, no output on stdout.

- [ ] **Step 4: Measure the budget**

Run `yashy check --timings` and `yashy verify --timings` on `benchmarks/large.ya` five times on each OS; record the median `verify` share of `check + verify` in this plan. Gate: verify ≤ 30% of check wall time.

- [ ] **Step 5: Update the docs**

`AGENTS.md` commands block gains `yashy verify`, `cargo test -p yashy-driver --test verify`, and a line that the active phase plan is the source of truth. `docs/tutorial/` gains a section that uses a real diagnostic from the corpus. `llms.txt` and `README.md` gain the command.

- [ ] **Step 6: Commit and record**

```bash
git add -A
git commit -m "test: seed the verifier corpus and close the phase 1a gate"
```

Record the commit hash in the master index table and in `.superpowers/sdd/2026-10-27-phase-01a/progress.md`, and set the phase's Status to `complete`.

---

## Gate summary

Phase 1a is complete when, on Windows and Linux:

| Gate | Command | Passes when |
|---|---|---|
| Ownership rules have one owner | `cargo test -p yashy-verify` | rule tests pass; `crates/yashy-sema/src/ownership.rs` no longer exists; `cargo tree -p yashy-sema` lists `yashy-verify` |
| Build verifies by default | `cargo test -p yashy-driver --test e2e --test examples` | every program builds and runs with no verifier diagnostic |
| Leak and defer corpus | `cargo test -p yashy-driver --test verify` | every seeded bug produces its expected code |
| No false positives on real code | `cargo test -p yashy-driver --test verify` + `--test examples` | zero verifier diagnostics on examples, programs and `file_cli.ya` |
| Contracts warn, never block | `cargo test -p yashy-verify contracts` | unprovable contract is a `Warning`; unknown identifier is `E0001` |
| JSON is additive | `cargo test -p yashy-driver --test diagnostics_json` | unchanged tests pass |
| Timing budget | `yashy check --timings`, `yashy verify --timings` | verify ≤ 30% of check, medians of 5 |
| Escape hatch is traceable | `cargo test -p yashy-driver --test unverified` | all four tests pass |
| Nothing panics | `cargo test -p yashy-driver --test never_panics` | all pass |

A gate that cannot be met is recorded as a dated exception in this file; the phase cannot be marked complete while an exception is open.
