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

## Revision at phase start (2026-10-29)

The plan was written before 1a shipped. Eight findings from the pre-flight scan
against the code as it now stands, each with the ruling that resolves it.

1. **`var` does not exist.** `crates/yashy-parser/src/stmts.rs` rejects it:
   `` `var` was removed: use `let` ``. `E2004` was defined as "a `var` is never
   assigned after its declaration", so it named a construct the language
   deleted. The nearest live rule -- "a `let` is never reassigned" -- would fire
   on nearly every binding in the shipped corpus (`let text = read_file(path);`
   is not reassigned), which would make the zero-false-positive gate meaningless
   and train users to ignore warnings. The honest replacement is a dead-store
   rule, and that needs the same backward liveness the phase 4 optimizer will
   build.
   **Ruling: `E2004` is dropped from 1b and deferred to phase 4**, where the
   pass manager already owns dataflow and a second, disagreeing liveness
   analysis is not created. `E2004` stays reserved so no other meaning can take
   it. 1b ships **four** warnings, not five. Cost if wrong: one deferred lint,
   recoverable in phase 4 at no migration cost because the code was never
   emitted.

2. **The `pub` suppression has nothing to suppress.** None of the four
   remaining warnings is item-level: `E2000` is a binding, `E2001` an import,
   `E2002` a statement, `E2003` a parameter. A parameter a `pub` function never
   reads is unused for exactly the same reason a private one is, and the fix is
   the same `_name`. The plan's claim that the rule "lives in `yashy-sema`'s
   visibility table" is also false: sema keeps `ItemOrigin { public, file }` for
   name resolution, and the HIR carries no visibility at all.
   **Ruling: drop the `pub` rule and do not add visibility to the HIR.** Cost if
   wrong: a library author sees `E2003` on a deliberately-unused parameter and
   writes `_name`, which is what the warning asks for anyway.

3. **`HirExpr::fault` is private.** `crates/yashy-hir/src/hir.rs` declares
   `fault: Option<FaultSite>` with no `pub` and no accessor, so Task 3's pattern
   `HirExpr::Binary { fault: Some(_), .. }` cannot be written outside the crate.
   **Ruling: Task 3 adds `HirExpr::fault(&self) -> Option<&FaultSite>`** rather
   than making the field public, so the one way to read it stays one way.

4. **`verify_source` is a test helper, not library API.** It lives in
   `crates/yashy-verify/tests/contracts.rs` (and, separately, inside
   `pass/leak.rs`'s unit tests). Tasks 1-3's tests call it.
   **Ruling: the new passes' integration tests go in `tests/contracts.rs`'s
   sibling files**, each with the same small helper copied (the file already
   does this for `leak.rs`), and no `verify_source` is added to the library.

5. **The driver never passes a `ContractMode`.** All three call sites in
   `main.rs` use `&VerifyOptions::default()`. Task 4's "one constant" is
   therefore one `const OPTIONS: VerifyOptions` replacing three
   `VerifyOptions::default()` expressions.

6. **Task 5's tests belong in `tests/test_runner.rs`**, which exists and already
   owns the JSON test report; `tests/verify.rs` owns the corpus.

7. **`VerifyReport` grows twice**: `obligations` in Task 2 and `analyses` in
   Task 8. Both are additive, and `Diagnostic::warning` does not exist yet --
   Task 9 adds it.

8. **The checker enforces exhaustiveness first, so the verifier's re-check
   cannot be reached from a source program.** `check_expr.rs` calls
   `missing_cases` for enum, `Option` and `Result` subjects and reports
   `E0600`; a non-exhaustive switch never produces HIR, and `verify` never
   runs. The plan's Step 1 test (`verify_source` on a bad switch) would pass
   before the pass exists, because the helper returns the *checker's*
   diagnostics when checking fails.
   **Ruling: the pass ships anyway and is tested against checked HIR whose
   arms were removed afterwards.** Exhaustiveness is a soundness property --
   a switch with no arm for a value has nothing to run -- and the verifier is
   where soundness rules are owned; the re-check is the guard against a
   checker regression, and it becomes reachable when phase 8 adds generic
   enums, whose variant lists the checker must compute. The test mutates the
   HIR precisely because that is the shape a checker regression would have.
   The pass's doc comment says all of this, so a future reader does not
   delete it as dead code. Cost if wrong: one extra walk of the HIR.

Also settled here, so the tasks do not each re-decide it: `E2002` fires after an
unconditional `return`, `break`, `continue`, `panic` or `os.exit` in the same
block. There is no `!` type in the language, so the plan's "returns `!`"
suppression is replaced by that list.

## Rulings recorded during the phase

These came from reading the code and the shipped programs while doing the work,
not from the pre-flight scan. They are numbered after the eight above.

9. **Bounds is a proof, not an error. `E1200` is not emitted.** The plan's Task 2
   rejected a provably out-of-range index (`let xs = [1, 2, 3]; let x = xs[5];`
   → `E1200`). The language does not agree with that rule:
   - `docs/tutorial/09-arrays.md` promises "an index may be out of bounds, and
     the type says so: `None` is the answer, and the compiler makes you match
     it", and calls a negative index "also a miss, not an error";
   - `crates/yashy-driver/tests/programs/fixed_arrays.ya` ships
     `print(show(grid[3]))` on `[int; 3]`, with the comment "indexing is checked,
     in range and out", and expects `-1` from `show`'s `None` arm;
   - `tests/verify.rs`'s corpus gate requires `verify --json` to report **zero
     codes**, warnings included, on every shipped program.

   The rule would therefore reject a documented, deliberate language property
   and a program the repository ships. It is also not a soundness rule: an
   out-of-range index never traps, so there is nothing to be unsound about.
   Worse, `.get(i)`, `.first()` and `.last()` all lower to the same
   `HirExpr::Index` (sema `check_expr.rs`), so an `Index` rule would also reject
   `empty.first()` — another documented `None`.

   **Ruling: the pass produces `ProofObligation`s and no diagnostic.** A proved
   index is `established: true`; an index the domain cannot place inside the
   length is `established: false`, which means "keep the runtime check". That is
   exactly what the design spec asks for ("bounds and overflow elision in phase
   3 depend on proofs") and what phase 3's `ProofFacts { proven: BTreeSet<Span> }`
   consumes. `E1200` stays reserved and unemitted, like `E2004`; Task 10 does not
   create its page. **A view is not an obligation either**: `a[0..99]` is defined
   to clamp and `a[2..1]` is the empty view (`docs/tutorial/09-arrays.md`,
   `examples`-level `fixed_arrays.ya`), so there is no check to elide. A pointer
   index is skipped: a pointer carries no length. Cost if wrong: an
   always-`None` index is not diagnosed; the proof record is still there, so a
   later slice can add a lint without redoing the analysis.

10. **The bounds rule shares the contract pass's walk.** The plan called for
    `pass::bounds::check` as its own walk. That would be a second abstract
    interpreter: it would need the same `Env`, the same branch refinements, the
    same loop fixpoint and the same `length_of`, and the two would disagree the
    day one learned a fact. **Ruling: `pass/bounds.rs` holds the rule
    (`check(length, interval) -> Bounds`) and `pass/contracts.rs` calls it from
    its existing `eval`.** One walk, one set of facts. `contracts::check` gains
    an `obligations` sink; `verify` fills `VerifyReport::obligations`.

11. **A provable overflow is a compile error, so the runtime-fault tests hide
    the operand.** Integer `+`, `-`, `*` and unary `-` are checked and panic at
    run time, so an operation whose whole result range is outside `i64` is a
    certain fault and `E1201` rejects it. That is unlike bounds (ruling 9): a
    miss is a defined `None`, an overflow is not a value. Four e2e tests and
    `tests/programs/overflow.ya` overflowed on purpose and now fail to build.
    The rule is not wrong -- those programs can never reach the fault -- so the
    **tests** change: each value goes through `fn same(n: int) -> int { return n; }`,
    which the verifier does not see through, and the runtime check still runs.
    The corpus gate stays at zero codes on every shipped program. Cost if wrong:
    a program that provably overflows no longer builds, so a user who wanted the
    runtime panic has to say what they mean with `wrapping_*`, `checked_*` or
    `saturating_*`; that is the trade the rule makes.

---

### Task 1: Move exhaustiveness into `yashy-hir` and re-check it

**Files:**
- Move: `crates/yashy-sema/src/exhaustive.rs` → `crates/yashy-hir/src/exhaustive.rs` (content unchanged)
- Modify: `crates/yashy-hir/src/lib.rs`, `crates/yashy-sema/src/lib.rs`
- Create: `crates/yashy-verify/src/pass/exhaustive.rs`

**Interfaces:**
- Produces: `yashy_hir::exhaustive::{missing_cases, is_exhaustive}` (same signatures as today).
- Produces: `verify` reports `E0600` for a non-exhaustive `HirStmt::Switch` / `HirExpr::SwitchExpr`.

- [x] **Step 1: Write the failing test**

The test cannot be a source program: the checker rejects a non-exhaustive switch
before the verifier runs, so `verify_source` on the plan's original fixture would
return the *checker's* `E0600` and pass without the pass existing. Ruling 8 in
the revision section explains why the pass ships anyway. `tests/exhaustive.rs`
therefore checks an exhaustive program, removes an arm from the checked HIR, and
asks the verifier:

```rust
#[test]
fn removing_an_arm_is_reported_by_the_recheck() {
    let mut hir = checked(THREE_COLORS);
    drop_arm(&mut hir, "Blue");
    assert_eq!(codes(&verify(&hir, &VerifyOptions::default())), vec!["E0600"]);
}
```

Four tests: an exhaustive switch is silent, a removed arm is `E0600`, a
`default` covers the rest, and a guarded arm does not cover its variant.

- [x] **Step 2: Run it and watch it fail** — `cargo test -p yashy-verify --test exhaustive` → the two detection tests FAIL, the two silence tests pass.

- [x] **Step 3: Move the module and re-export**

`git mv crates/yashy-sema/src/exhaustive.rs crates/yashy-hir/src/exhaustive.rs`;
`pub mod exhaustive;` in `yashy-hir/src/lib.rs`; in `yashy-sema/src/lib.rs`,
`pub use yashy_hir::{exhaustive, hir, ty};`. Sema's
`pub use exhaustive::{is_exhaustive, missing_cases};` keeps working. The module's
`use yashy_parser::ast::{Pattern, VariantDef};` became `pub use`, so the verifier
can build the shapes `missing_cases` takes without a direct `yashy-parser`
dependency.

- [x] **Step 4: Re-check in the verifier**

`pass/exhaustive.rs` walks every function (lambdas included) and every
`HirStmt::Switch` / `HirExpr::SwitchExpr`. For an enum, `Option` or `Result`
subject it asks `missing_cases`; for anything else it reports `E0600` because no
arm list can cover it. Guarded arms do not count, exactly as in the checker.
`to_ast_pattern` is total rather than `Option`: every `HirPattern` has an AST
counterpart after checking, so a `None` branch would be unreachable.

- [x] **Step 5: Run the tests** — `cargo test -p yashy-verify -p yashy-sema -p yashy-hir` → **400 passed, 0 failed**; `cargo check --workspace --all-targets` → clean.

- [x] **Step 6: Commit** — `refactor(verify): re-check switch exhaustiveness over the HIR`

---

### Task 2: Bounds proof for indexing

**Files:**
- Create: `crates/yashy-verify/src/pass/bounds.rs`
- Create: `crates/yashy-verify/tests/bounds.rs`
- Modify: `crates/yashy-verify/src/lib.rs` (`ProofObligation`, `ObligationKind`, `VerifyReport::obligations`)
- Modify: `crates/yashy-verify/src/pass/mod.rs`, `crates/yashy-verify/src/pass/contracts.rs` (the walk records the obligation), `crates/yashy-verify/src/pass/leak.rs` and `crates/yashy-verify/tests/contracts.rs` (the two other `VerifyReport` constructions)

**Interfaces:**
- Consumes: `domain.rs` from 1a; `HirExpr::Index`, `SliceLit`, `FixedArrayLit`; `expr_ty` for a fixed array's length.
- Produces: one `ProofObligation` per array index, `established` true exactly when the domain proves `0 <= i < len` on every path that reaches it. **No diagnostic**: see ruling 9.

```rust
pub enum ObligationKind { Bounds }

pub struct ProofObligation {
    pub kind: ObligationKind,
    pub span: Span,          // the index expression, so phase 3 can find the check
    pub established: bool,
}
```

- [x] **Step 1: Write the failing tests** — `tests/bounds.rs`, ten whole programs:

```rust
#[test]
fn an_index_into_a_known_length_array_is_proved() {
    let source = "fn main() { let xs = [1, 2, 3]; xs[1]; }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty());
    assert!(bounds(&report)[0].established);
}

#[test]
fn a_constant_index_past_the_end_is_unproved_not_rejected() {
    let source = "fn main() { let xs = [1, 2, 3]; xs[5]; }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty());
    assert!(!bounds(&report)[0].established);
}
```

The rest cover a negative index, a `for`-bounded index (proved), a branch that bounds the index (proved), the plan's `while` loop (accepted, unproved -- the widening drops the range), a `[]T` parameter (unproved), a view (no obligation), a pointer index (no obligation), and an index after `return` (no obligation).

- [x] **Step 2: Run and watch them fail** — `cargo test -p yashy-verify --test bounds` → compile error: `ObligationKind`, `ProofObligation` and `VerifyReport::obligations` do not exist.

- [x] **Step 3: Implement**

`VerifyReport` gains `pub obligations: Vec<ProofObligation>`. `pass/bounds.rs` holds the rule: given a length and the index's [`Interval`], answer `InRange`, `OutOfRange` or `Unknown`; empty intervals are vacuously in range. The contract pass's walk calls it from `eval`'s `HirExpr::Index` arm, because that walk already carries the [`Env`]: it resolves the length (a fixed array's `N` from the type, otherwise `length_of`, which knows a literal's element count, a binding's tracked length and a constant view) and the index's interval. An unknown length is `established: false`. The rule is skipped while the loop fixpoint is quiet, so the obligation is recorded once, by the reporting walk.

A pointer index is skipped (no length to compare against). A view is skipped: clamping is its definition, so there is no check to prove (ruling 9).

- [x] **Step 4: Run the tests** — `cargo test -p yashy-verify` → **67 passed, 0 failed** (35 lib unit tests, 10 bounds, 18 contracts, 4 exhaustive); `cargo check --workspace --all-targets` → clean.

- [x] **Step 5: Commit** — `git commit -m "feat(verify): prove array indexing in bounds, without rejecting a miss"`

---

### Task 3: Provable overflow (E1201)

**Files:**
- Create: `crates/yashy-verify/src/pass/overflow.rs`
- Create: `crates/yashy-verify/tests/overflow.rs`
- Modify: `crates/yashy-hir/src/hir.rs` (`HirExpr::fault`)
- Modify: `crates/yashy-verify/src/lib.rs` (`ObligationKind::Overflow`), `crates/yashy-verify/src/pass/mod.rs`, `crates/yashy-verify/src/pass/contracts.rs` (the walk calls the rule)
- Modify: `codes.rs` (`OVERFLOW = "E1201"`), `docs/errors/E1201.md`, `docs/errors.md`
- Modify: `crates/yashy-driver/tests/programs/overflow.ya`, `crates/yashy-driver/tests/e2e.rs` (the runtime-fault tests keep a value the verifier cannot see)

**Interfaces:**
- Consumes: the fault mark the checker already puts on exactly the operations that can fault, read through a new `HirExpr::fault(&self) -> Option<&FaultSite>` accessor added in this task; the field stays private, so there is one way to read it.
- Produces: `E1201` only when the result range is entirely outside `i64`; an `Overflow` obligation for every checked operation, established exactly when the whole result range fits.

- [x] **Step 1: Write the failing tests** — `tests/overflow.rs`, thirteen whole programs: adding one and adding two to `i64::MAX`, subtracting past `i64::MIN`, multiplying, negating `i64::MIN` (all `E1201`); `1 + 2` and a `for`-bounded `i + 1` (silent, established); two parameters and a sum that straddles the boundary (silent, unestablished); a string concatenation, a float product and a comparison (no obligation, because they carry no fault mark); arithmetic after `return` (not recorded).

```rust
#[test]
fn adding_one_to_the_largest_int_is_rejected() {
    let source = "fn main() { let x: int = 9223372036854775807; let y = x + 1; }";
    assert_eq!(codes(&verify_source(source)), vec!["E1201"]);
}

#[test]
fn arithmetic_on_parameters_is_recorded_not_rejected() {
    let report = verify_source("fn f(a: int, b: int) { a + b; } fn main() { }");
    assert!(report.diagnostics.is_empty());
    assert!(report.obligations.iter().any(|o| o.kind == ObligationKind::Overflow && !o.established));
}
```

The plan's `fn f(a: int, b: int) -> int { a + b }` did not parse: an expression statement needs a `;`, and a `-> int` body with no `return` is now a "missing return" error. The test uses a `void` function, which is what it meant.

- [x] **Step 2: Run and watch them fail** — compile error: `ObligationKind` has no `Overflow`.

- [x] **Step 3: Implement** — `pass/overflow.rs` computes the mathematical result range in `i128` *before* clamping (`Interval` clamps and answers `TOP` when a result leaves `i64`, which would lose exactly the fact the rule needs). `Overflow::{InRange, OutOfRange, Unknown}`; empty operands are vacuously in range; division and remainder are not modelled, because the domain does not model them and their faults are a different rule. `contracts.rs` calls it from `eval`'s `Binary` arm when `expr.fault().is_some()`, and from the `Unary` arm for `-x`. An `OutOfRange` verdict is an error with `E1201` and an `Overflow` obligation (`established: false`); `Unknown` is the obligation alone; `InRange` is `established: true`. The rule is skipped while the loop fixpoint is quiet, so it records once.

- [x] **Step 4: Run the tests** — `cargo test -p yashy-verify` → **88 passed, 0 failed** (43 lib unit tests, 10 bounds, 18 contracts, 4 exhaustive, 13 overflow). `cargo check --workspace --all-targets` clean; `cargo test --workspace` **1315 passed**.

- [x] **Step 5: Commit** — `git commit -m "feat(verify): reject provable integer overflow (E1201)"`

The four runtime-fault tests in `tests/e2e.rs` and `tests/programs/overflow.ya` overflowed on purpose, and E1201 now rejects them at compile time. That is the rule working, not a false positive: the program can never reach the fault. The tests are about the *runtime* check, so each value now goes through `fn same(n: int) -> int { return n; }`, which hides the range from the verifier. The runtime path is still reachable for any value the verifier does not know, which is exactly what those tests exist to cover. The corpus gate stays at zero codes on every shipped program.

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

- [ ] **Step 3: Flip the mode** — one constant in `main.rs`, `const OPTIONS: VerifyOptions = VerifyOptions { contracts: ContractMode::Enforce };`, replacing the three `&VerifyOptions::default()` expressions at the `verify`, `build` and `run` call sites. The pass is unchanged; this is the whole point of keeping mode out of the pass.

- [ ] **Step 4: Run the suite** — `cargo test -p yashy-driver --test verify --test e2e --test examples` → PASS. Any failure is a false positive to fix in the domain, not a reason to go back to `Warn`.

- [ ] **Step 5: Commit** — `git commit -m "feat(verify): enforce contracts in build, run and verify"`

---

### Task 5: `yashy test` verifies

**Files:**
- Modify: `crates/yashy-driver/src/main.rs` (`test`, `test_json` compile path)
- Modify: `crates/yashy-driver/tests/test_runner.rs` (the file that already owns the JSON test report; `tests/verify.rs` owns the corpus)

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
- Modify: `crates/yashy-common/src/codes.rs` (`UNUSED_BINDING = "E2000"`, `UNUSED_IMPORT = "E2001"`, `UNREACHABLE_CODE = "E2002"`, `UNUSED_PARAMETER = "E2003"`), create the four pages, modify `docs/errors.md`. `E2004` stays reserved but unemitted; see ruling 1.
- Modify: `crates/yashy-driver/src/main.rs` (`-W`, `--deny warnings`, `--allow warnings`, `YASHY_DENY_WARNINGS`)
- Modify: `crates/yashy-driver/src/pipeline.rs` (the warning pass runs after `check`, before `verify`, and its diagnostics join the same bag)
- Modify: `docs/cli.md` (the three flags, the environment variable, and the exit-code rule)
- Create: `crates/yashy-driver/tests/warnings.rs`
- Create: `crates/yashy-driver/tests/verify_corpus/` is not used — warnings are a positive corpus, so the programs live in `tests/programs/warnings/`

**Interfaces:**
- Produces: four coded warnings, each with a page and a `--deny` test:

  | Code | Fires when | Suppressed when |
  |---|---|---|
  | `E2000` | a `const`/`let` binding is never read | the name is `_` or starts with `_` |
  | `E2001` | an `import` is never used | the module has a side effect the checker records (phase 2's effect table) |
  | `E2002` | a statement cannot be reached | it follows an unconditional `return`, `break`, `continue`, `panic` or `os.exit` |
  | `E2003` | a parameter is never read | the name is `_` or starts with `_` |

- Produces: `yashy build`/`run`/`test`/`check` print warnings to stderr and exit **0**; `--deny warnings` exits **1** and every warning becomes an error in the JSON envelope (`"severity": "error"`, same `code`).
- Produces: `--allow warnings` suppresses them entirely (for a generated file), and `-W none` is a spelling of it. The two are one code path.
- Produces: the JSON envelope's `summary` gains `warnings: usize`, so an agent can gate on it without parsing text.

- [ ] **Step 1: Write the failing tests** — `const x = 1;` with no read is `E2000`; `let _ = f();` and `fn f(_unused: int)` are silent; an unused `import` is `E2001`; code after `return` is `E2002`; code after `os.exit(0)` is silent; `yashy build` with a warning exits 0; `--deny warnings` exits 1; `--allow warnings` prints nothing; the JSON envelope has `severity: "error"` under `--deny` and `summary.warnings == 1`.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — one pass over the typed HIR that collects reads, writes and imports into sets and reports the difference; the suppression rules are predicates on the same sets, not a second walk. `--deny` is applied once, in the driver, by mapping severity after all diagnostics are collected — never inside the pass, so the pass cannot disagree with the flag. The pass is registered in the phase 4 pass manager like every other, so `--no-opt=warnings` disables it for bisecting.
- [ ] **Step 4: Run** `cargo test -p yashy-driver --test warnings --test diagnostics_json` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: four coded warnings, and --deny warnings (E2000-E2003)"`

---

### Task 10: Docs and the gate

**Files:** `docs/errors/{E2000..E2003}.md`; `docs/errors.md`; `docs/tutorial/` (a "Proving bounds and overflow" section and a "Warnings" section); `docs/cli.md`; `AGENTS.md` (the `verify` line gains `--inventory`; the diagnostics section gains the warning rule). `E1201.md` already ships with Task 3. `E1200` has no page: it is reserved but never emitted (ruling 9).

- [ ] **Step 1: Write the pages** in the standard cause/example/fix shape, using the corpus programs as the examples. While here, settle the proof trace in human output: `yashy-common/src/source.rs` prints the label, the notes and the `docs:` line but **not** `diagnostic.proof`, so `E1100.md`'s example shows `proof:`/`missing:`/`remedy:` lines the renderer does not print. Either render the trace (the DX fix, and then `E1100.md` is accurate) or correct the page to match the renderer. `E1201.md` currently shows only what the renderer prints and says the trace is in `--json`.

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
| Warnings fire | `cargo test -p yashy-driver --test warnings` | all four codes fire on their program and stay silent on their suppression |
| Warnings are warnings | `yashy build` on a program with one unused binding | exits 0, prints one `warning:`, and `--deny warnings` exits 1 |
| No panic | `cargo test -p yashy-driver --test never_panics` | passes |
| Workspace | `cargo test --workspace` | green on Windows and Linux |
