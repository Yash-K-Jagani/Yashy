# Phase 1b — Verifier Core, Enforcing: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the usable verifier from 1a into an enforcing one: bounds and provable overflow become compile errors, contracts become errors, `yashy test` verifies, and the corpus is strong enough that a zero-false-positive gate means something.

**Architecture:** Three new proof passes over the HIR (bounds, overflow, exhaustiveness re-check) share the interval domain built in 1a. Exhaustiveness moves into `yashy-hir` so the verifier can call it without depending on `sema`. Contract mode flips from `Warn` to `Enforce` at the driver's call sites, not in the pass.

**Tech Stack:** `yashy-verify` (no external deps), the 1a interval domain, `yashy-hir::exhaustive`, the existing corpus harness in `crates/yashy-driver/tests/verify.rs`.

**Spec:** `docs/superpowers/specs/2026-10-27-yashy-master-design.md` §4, §8 row 1b. Index: `docs/superpowers/plans/2026-10-27-yashy-master-plan.md`. Predecessor: `2026-10-27-phase-01a-verifier-core.md`.

## Global Constraints

See the master index for the full list; every task includes it. The ones that bite here:

- No new runtime dependencies; `yashy-verify` stays dependency-free.
- One rule, one implementation: exhaustiveness has one algorithm, and it moves rather than being copied.
- A malformed program produces a diagnostic, never a panic.
- Both operating systems or it is not done.
- No unproven claim: "proven in bounds" is a statement about the domain, not about all inputs.
- **A warning is a diagnostic with a code, a page, and a `--deny` test.** `Severity::Warning` already exists in `yashy-common`; this phase is where something finally produces one. A lint without a code is not allowed, because `yashy explain` cannot explain it.
- Format touched files only.

## Review Focus

1. **A rejected bounds check must be a real out-of-bounds, not a domain limitation.** If the domain cannot decide, the index is *not* rejected — the runtime check stays and the obligation is recorded for phase 3. Rejecting undecidable indices would reject nearly every loop.
2. **Overflow is rejected only when it is provable**, e.g. a literal at `i64::MAX` plus one. Unproven arithmetic is not an error.
3. **A `switch` that is exhaustive only because of a `default` stays accepted**, and a new variant added later must break every non-defaulted switch.
4. **`yashy test` verifying must not change test semantics**: a failing test still exits 1 for the test's reason, not the verifier's.
5. **The inventory is complete or it is a lie**: every `extern` declaration and every `unsafe_call: true` site appears, including ones inside stdlib modules.
6. **A warning must never fail a build by default.** `yashy build` exits 0 with warnings; only `--deny warnings` (or `YASHY_DENY_WARNINGS=1`) turns them into errors. A compiler that fails on an unused binding by default is a compiler people turn off.
7. **A warning must be silent on code that is legitimately unused.** A `pub` item is reachable from another module and is never "unused"; a parameter named `_` is deliberately ignored; a `var` that is only mutated through a `self: *mut` receiver is not "never reassigned". Each suppression is a test, not a comment.

---

### Task 1: Move exhaustiveness into `yashy-hir` and re-check it

**Files:**
- Move: `crates/yashy-sema/src/exhaustive.rs` → `crates/yashy-hir/src/exhaustive.rs` (content unchanged)
- Modify: `crates/yashy-hir/src/lib.rs`, `crates/yashy-sema/src/lib.rs`
- Create: `crates/yashy-verify/src/pass/exhaustive.rs`

**Interfaces:**
- Produces: `yashy_hir::exhaustive::{missing_cases, is_exhaustive}` (same signatures as today).
- Produces: `verify` reports `E0600` for a non-exhaustive `HirStmt::Switch` / `HirExpr::SwitchExpr`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn a_switch_missing_a_variant_is_rejected_by_the_verifier() {
    let source = "enum Color { Red, Green, Blue } fn main() { let c = Color::Red; switch c { Color::Red => {}, Color::Green => {} } }";
    assert_eq!(codes(&verify_source(source)), vec!["E0600"]);
}
```

- [ ] **Step 2: Run it and watch it fail** — `cargo test -p yashy-verify exhaustive` → FAIL.

- [ ] **Step 3: Move the module and re-export**

`git mv crates/yashy-sema/src/exhaustive.rs crates/yashy-hir/src/exhaustive.rs`; add `pub mod exhaustive;` to `yashy-hir/src/lib.rs`; in `yashy-sema/src/lib.rs` replace `pub mod exhaustive;` with `pub use yashy_hir::exhaustive;`. Sema's `pub use exhaustive::{is_exhaustive, missing_cases};` keeps working.

- [ ] **Step 4: Re-check in the verifier**

For each switch, collect the arm patterns and `is_default`, map `HirPattern` to the AST `Pattern` shape the function takes (a small `fn to_ast_pattern(&HirPattern) -> Option<Pattern>` in `pass/exhaustive.rs`, `None` for forms the checker has not resolved), and report `E0600` with the missing variant names in the label. A `None` conversion means "not checkable here" and is skipped, not reported.

- [ ] **Step 5: Run the tests** — `cargo test -p yashy-verify && cargo test -p yashy-sema` → PASS.

- [ ] **Step 6: Commit** — `git commit -m "refactor(verify): re-check switch exhaustiveness over the HIR"`

---

### Task 2: Bounds proof for indexing (E1200)

**Files:**
- Create: `crates/yashy-verify/src/pass/bounds.rs`
- Modify: `crates/yashy-verify/src/pass/mod.rs`, `crates/yashy-common/src/codes.rs` (`BOUNDS = "E1200"`)
- Create: `docs/errors/E1200.md`, modify `docs/errors.md`

**Interfaces:**
- Consumes: `domain.rs` from 1a; `HirExpr::Index`, `View`, `SliceLit`, `FixedArrayLit`.
- Produces: `E1200` when the index range is provably outside `[0, len)`; `ProofObligation` recorded when undecidable.

```rust
pub struct ProofObligation {
    pub kind: ObligationKind,   // Bounds | Overflow | Contract | Invariant
    pub span: Span,
    pub established: bool,
}
```

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_constant_index_past_the_end_is_rejected() {
    let source = "fn main() { let xs = [1, 2, 3]; let x = xs[5]; }";
    assert_eq!(codes(&verify_source(source)), vec!["E1200"]);
}

#[test]
fn a_loop_index_bounded_by_the_length_is_accepted() {
    let source = "fn main() { let xs = [1, 2, 3]; let i = 0; while i < xs.len() { let x = xs[i]; i = i + 1; } }";
    assert!(verify_source(source).diagnostics.is_empty());
}

#[test]
fn an_undecidable_index_is_recorded_not_rejected() {
    let source = "fn f(i: int, xs: []int) { let x = xs[i]; } fn main() { }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty());
    assert!(report.obligations.iter().any(|o| o.kind == ObligationKind::Bounds && !o.established));
}
```

- [ ] **Step 2: Run and watch them fail** — `cargo test -p yashy-verify bounds` → FAIL.

- [ ] **Step 3: Implement**

`VerifyReport` gains `pub obligations: Vec<ProofObligation>`. For `HirExpr::Index { base, index }` and `HirExpr::View { base, start, end }`, take the length from the base's type (`FixedArray(_, n)` is exact; `Slice`/`View` length comes from a `.len()` call the domain tracked; otherwise unknown) and the index's interval. Reject only when `hi < 0 || lo >= len`. Otherwise record an obligation with `established: false` so phase 3 can decide about the runtime check.

- [ ] **Step 4: Run the tests** — PASS.

- [ ] **Step 5: Commit** — `git commit -m "feat(verify): prove or reject array indexing (E1200)"`

---

### Task 3: Provable overflow (E1201)

**Files:**
- Create: `crates/yashy-verify/src/pass/overflow.rs`
- Modify: `codes.rs` (`OVERFLOW = "E1201"`), `docs/errors/E1201.md`, `docs/errors.md`

**Interfaces:**
- Consumes: `HirExpr::Binary { fault: Some(_), .. }` — the checker already marks exactly the operations that can fault.
- Produces: `E1201` only when the result range is entirely outside `i64`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn adding_one_to_the_largest_int_is_rejected() {
    let source = "fn main() { let x: int = 9223372036854775807; let y = x + 1; }";
    assert_eq!(codes(&verify_source(source)), vec!["E1201"]);
}

#[test]
fn adding_two_small_ints_is_not() {
    assert!(verify_source("fn main() { let x = 1 + 2; }").diagnostics.is_empty());
}

#[test]
fn arithmetic_on_parameters_is_recorded_not_rejected() {
    let report = verify_source("fn f(a: int, b: int) -> int { a + b } fn main() { }");
    assert!(report.diagnostics.is_empty());
    assert!(report.obligations.iter().any(|o| o.kind == ObligationKind::Overflow && !o.established));
}
```

- [ ] **Step 2: Run and watch them fail** → FAIL.

- [ ] **Step 3: Implement** — the same domain arithmetic as the contract pass; `i64::MAX + 1` is `[MIN, MIN]`-wrapped, so the pass must compute the mathematical result range *before* wrapping and compare against the bounds. Undecidable → obligation, never an error.

- [ ] **Step 4: Run the tests** → PASS.

- [ ] **Step 5: Commit** — `git commit -m "feat(verify): reject provable integer overflow (E1201)"`

---

### Task 4: Contracts enforce

**Files:**
- Modify: `crates/yashy-driver/src/main.rs` (`verify`, `build`, `run` pass `ContractMode::Enforce`)
- Modify: `crates/yashy-driver/tests/verify.rs`

**Interfaces:**
- Consumes: `ContractMode::Enforce` from 1a.
- Produces: an unproved contract exits `1` on `verify`, `build` and `run`.

- [ ] **Step 1: Write the failing test** — a program whose `requires` fails at a call site exits 1 from `yashy build`, and the message carries `E1100` and the clause text.

- [ ] **Step 2: Watch it fail** — currently the diagnostic is a warning, so exit is 0.

- [ ] **Step 3: Flip the mode** — one constant in `main.rs`, `const CONTRACTS: ContractMode = ContractMode::Enforce;`, used by every call site. The pass is unchanged; this is the whole point of keeping mode out of the pass.

- [ ] **Step 4: Run the suite** — `cargo test -p yashy-driver --test verify --test e2e --test examples` → PASS. Any failure is a false positive to fix in the domain, not a reason to go back to `Warn`.

- [ ] **Step 5: Commit** — `git commit -m "feat(verify): enforce contracts in build, run and verify"`

---

### Task 5: `yashy test` verifies

**Files:**
- Modify: `crates/yashy-driver/src/main.rs` (`test`, `test_json` compile path)
- Modify: `crates/yashy-driver/tests/verify.rs`

**Interfaces:**
- Consumes: `check_and_verify` from 1a.
- Produces: a test program that fails verification is reported as a **compile failure of that test**, not as a failing test.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_test_program_that_fails_verification_is_a_compile_error() {
    // a temp project whose tests/leak.ya leaks; `yashy test --json`
    // assert tests[0].status == "compile_error", summary.compile_errors == 1
}

#[test]
fn a_failing_test_still_reports_as_failed_not_compile_error() {
    // a temp project whose tests/fail.ya asserts false; assert status "failed"
}
```

- [ ] **Step 2: Watch them fail.**

- [ ] **Step 3: Implement** — the test runner's compile step calls `check_and_verify`; a verifier diagnostic marks the test `compile_error` with the same diagnostic shape as a check error. The exit code stays `1`.

- [ ] **Step 4: Run the tests** → PASS.

- [ ] **Step 5: Commit** — `git commit -m "feat(test): verify every test program before running it"`

---

### Task 6: The unsafe and FFI inventory

**Files:**
- Create: `crates/yashy-verify/src/pass/inventory.rs`
- Modify: `crates/yashy-driver/src/main.rs` (`verify --inventory`)
- Modify: `docs/cli.md`

**Interfaces:**
- Produces: `pub struct Assumption { pub kind: AssumptionKind, pub name: String, pub file: FileId, pub span: Span }`, `AssumptionKind::{ExternDecl, UnsafeCall}`.
- Produces: `yashy verify --inventory` (human table) and `--inventory --json` (`"assumptions": [{kind, name, file, span}]`).

- [ ] **Step 1: Write the failing test** — a program with one `extern` block and one `unsafe` call produces exactly two assumptions, in source order.

- [ ] **Step 2: Watch it fail.**

- [ ] **Step 3: Implement** — walk `HirItem::Extern` and every `HirExpr::Call { unsafe_call: true }`. The list is the evidence that "proven" never silently means "trusted"; it is not a diagnostic and never changes the exit code.

- [ ] **Step 4: Run the test** → PASS.

- [ ] **Step 5: Commit** — `git commit -m "feat(verify): inventory every unsafe and FFI assumption"`

---

### Task 7: The realistic corpus and the zero-false-positive gate

**Files:**
- Create: `crates/yashy-driver/tests/verify_corpus/` (one file per seeded bug, as named in 1a Task 10, plus `bounds_oob.ya`, `overflow_max.ya`, `exhaustive_missing.ya`, `contract_unsat.ya`)
- Modify: `crates/yashy-driver/tests/verify.rs` (the expected-code table)
- Modify: `crates/yashy-driver/tests/programs/file_cli.ya` (extend: arguments, error path, no leak on the failure path)

**Interfaces:**
- Produces: a gate, not a feature.

- [ ] **Step 1: Add the four seeded bugs** and extend the table so each file's expected code set is asserted exactly, in order.

- [ ] **Step 2: Run the corpus** — `cargo test -p yashy-driver --test verify` → every row passes.

- [ ] **Step 3: Run the zero-false-positive gate**

```bash
for f in examples/*/main.ya crates/yashy-driver/tests/programs/*.ya; do
  yashy verify "$f" || echo "FALSE POSITIVE: $f"
done
```

Expected: no `FALSE POSITIVE` line, on Windows and Linux. Any line is a verifier bug with the program attached; fix the rule, never the program.

- [ ] **Step 4: Commit** — `git commit -m "test: close the phase 1b corpus and zero-false-positive gate"`

---

### Task 8: Per-function verification memo

**Files:**
- Modify: `crates/yashy-verify/src/lib.rs`

**Interfaces:**
- Produces: `verify` computes each function once per program, keyed by `(name, body span)`, so a function reached from many call sites is not re-analysed.

- [ ] **Step 1: Write the failing test** — a program with a function called from 50 sites reports one diagnostic, not 50, and the pass counts one analysis (expose `VerifyReport::analyses: usize` for the test).

- [ ] **Step 2: Implement** — a `BTreeMap<(String, u32), ()>` of analysed functions; obligations are recorded once per function.

- [ ] **Step 3: Run and commit** — `git commit -m "perf(verify): analyse each function once"`

---

### Task 9: The warning surface

**Files:**
- Create: `crates/yashy-verify/src/pass/warnings.rs`
- Modify: `crates/yashy-common/src/diagnostic.rs` (`Diagnostic::warning(code, message, span)`, a `docs_url` for warnings, and `Severity` ordering so `--deny` can upgrade in one place)
- Modify: `crates/yashy-common/src/codes.rs` (`UNUSED_BINDING = "E2000"`, `UNUSED_IMPORT = "E2001"`, `UNREACHABLE_CODE = "E2002"`, `UNUSED_PARAMETER = "E2003"`, `NEVER_REASSIGNED = "E2004"`), create the five pages, modify `docs/errors.md`
- Modify: `crates/yashy-driver/src/main.rs` (`-W`, `--deny warnings`, `--allow warnings`, `YASHY_DENY_WARNINGS`)
- Modify: `crates/yashy-driver/src/pipeline.rs` (the warning pass runs after `check`, before `verify`, and its diagnostics join the same bag)
- Modify: `docs/cli.md` (the three flags, the environment variable, and the exit-code rule)
- Create: `crates/yashy-driver/tests/warnings.rs`
- Create: `crates/yashy-driver/tests/verify_corpus/` is not used — warnings are a positive corpus, so the programs live in `tests/programs/warnings/`

**Interfaces:**
- Produces: five coded warnings, each with a page and a `--deny` test:

  | Code | Fires when | Suppressed when |
  |---|---|---|
  | `E2000` | a `const`/`var`/`let` binding is never read | the name is `_` or starts with `_` |
  | `E2001` | an `import` is never used | the module has a side effect the checker records (phase 2's effect table) |
  | `E2002` | a statement cannot be reached | the function returns `!` before it (a deliberate `exit`) |
  | `E2003` | a parameter is never read | the name is `_` or starts with `_` |
  | `E2004` | a `var` is never assigned after its declaration | the value's address is taken (`*mut`) |

- Produces: `yashy build`/`run`/`test`/`check` print warnings to stderr and exit **0**; `--deny warnings` exits **1** and every warning becomes an error in the JSON envelope (`"severity": "error"`, same `code`).
- Produces: `--allow warnings` suppresses them entirely (for a generated file), and `-W none` is a spelling of it. The two are one code path.
- Produces: a `pub` item is never `E2000`/`E2003`, because it is reachable from another module; the reachability rule is the one phase 6 will use, so it lives in `yashy-sema`'s visibility table from the start.
- Produces: the JSON envelope's `summary` gains `warnings: usize`, so an agent can gate on it without parsing text.

- [ ] **Step 1: Write the failing tests** — `const x = 1;` with no read is `E2000`; `let _ = f();` and `fn f(_unused: int)` are silent; a `pub fn` with an unread local is still `E2000` for the *local* but never for the `pub` item itself; an unused `import` is `E2001`; code after `return` is `E2002`; `var x = 1;` never reassigned is `E2004`; `var x = 1; bump(&mut x);` is silent; `yashy build` with a warning exits 0; `--deny warnings` exits 1; `--allow warnings` prints nothing; the JSON envelope has `severity: "error"` under `--deny` and `summary.warnings == 1`.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — one pass over the typed HIR that collects reads, writes and imports into sets and reports the difference; the suppression rules are predicates on the same sets, not a second walk. `--deny` is applied once, in the driver, by mapping severity after all diagnostics are collected — never inside the pass, so the pass cannot disagree with the flag. The pass is registered in the phase 4 pass manager like every other, so `--no-opt=warnings` disables it for bisecting.
- [ ] **Step 4: Run** `cargo test -p yashy-driver --test warnings --test diagnostics_json` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: five coded warnings, and --deny warnings (E2000-E2004)"`

---

### Task 10: Docs and the gate

**Files:** `docs/errors/E1200.md`, `E1201.md`, `docs/errors/{E2000..E2004}.md`; `docs/errors.md`; `docs/tutorial/` (a "Proving bounds and overflow" section and a "Warnings" section); `docs/cli.md`; `AGENTS.md` (the `verify` line gains `--inventory`; the diagnostics section gains the warning rule).

- [ ] **Step 1: Write the pages** in the standard cause/example/fix shape, using the corpus programs as the examples.

- [ ] **Step 2: Run the gate** — the table below, on both operating systems.

- [ ] **Step 3: Commit and record** — `git commit -m "docs: document the enforcing verifier and close phase 1b"`; record the hash in the index and the ledger.

---

## Gate summary

| Gate | Command | Passes when |
|---|---|---|
| Corpus catches every seeded bug | `cargo test -p yashy-driver --test verify` | every expected code set matches exactly |
| Zero false positives | the loop in Task 7 | no `FALSE POSITIVE` line, both OSes |
| Contracts enforce | `cargo test -p yashy-driver --test verify` | unproved contract exits 1 |
| Test integration | `cargo test -p yashy-driver --test test_runner` | compile_error vs failed are distinguished |
| Inventory | `yashy verify --inventory --json` on a program with FFI | every `extern` and `unsafe` site listed |
| Warnings fire | `cargo test -p yashy-driver --test warnings` | all five codes fire on their program and stay silent on their suppression |
| Warnings are warnings | `yashy build` on a program with one unused binding | exits 0, prints one `warning:`, and `--deny warnings` exits 1 |
| No panic | `cargo test -p yashy-driver --test never_panics` | passes |
| Workspace | `cargo test --workspace` | green on Windows and Linux |
