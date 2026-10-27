# Phase 2 — Effects and Typestate: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the verifier prove what a function does to the world (`pure` / `io` / `unsafe`) and what state a resource is in (`allocated`/`freed`, `locked`/`unlocked`, `open`/`closed`), so hidden IO, double-close and unproven thread sharing become compile errors.

**Architecture:** Effects are inferred bottom-up over the call graph to a fixpoint, then checked against an optional declaration; the declaration is a promise the inference must not exceed. Typestate is a per-resource lattice threaded through the same control-flow walk the leak pass already uses, keyed by binding. Race freedom is a rule over `thread_spawn` captures that consumes both: a shared value must be behind a `Mutex`/`Channel` typestate, not a bare linear pointer.

**Tech Stack:** `yashy-verify` (no external deps), `HirFn`/`HirItem::Extern`, the existing mutex and channel builtins in `crates/yashy-runtime`, and `HirProgram::uses_threads`.

**Spec:** `docs/superpowers/specs/2026-10-27-yashy-master-design.md` §4.1 (effects, typestate), §8 row 2. Index: `docs/superpowers/plans/2026-10-27-yashy-master-plan.md`.

## Global Constraints

See the master index. The ones that bite here:

- The whitelist is closed: contracts, effects, typestate. No fourth construct without a written exception.
- No new runtime dependencies.
- One rule, one implementation.
- Both operating systems or it is not done.
- Format touched files only.

## Review Focus

1. **`alloc` and `free` are pure.** They touch memory, not the world. If they were `io`, no function could ever be pure and the effect system would be decoration.
2. **An unannotated function is inferred, not rejected.** Effects are opt-in; a program with no annotations must compile exactly as it does today.
3. **A `Mutex` locked on one branch and unlocked on both is a state error**, and the merge must report it at the merge, not at the lock.
4. **A string captured by a spawned thread is allowed** (refcounts are atomic when `uses_threads` is set); a bare `*mut T` capture is not.
5. **Effect inference terminates.** The call graph may be recursive; the fixpoint must be bounded by the lattice height, not by an iteration count.

---

### Task 1: Effect syntax and the HIR field

**Files:**
- Modify: `crates/yashy-lexer/src/token.rs` (`pure`, `io`, `unsafe` as modifiers; `unsafe` already exists for `extern` — confirm and reuse)
- Modify: `crates/yashy-parser/src/{ast.rs,items.rs}`
- Modify: `crates/yashy-hir/src/hir.rs` (`HirFn.effect: Effect`, `HirItem::Extern` carries `unsafe` implicitly)
- Modify: `crates/yashy-fmt/src/lib.rs`
- Create: `crates/yashy-verify/src/effect.rs`

**Interfaces:**
- Produces:
  ```rust
  #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
  pub struct Effect { pub io: bool, pub unsafe_: bool, pub async_: bool }
  impl Effect { pub const PURE: Effect; pub fn join(self, other: Effect) -> Effect; pub fn is_pure(self) -> bool; }
  pub enum DeclaredEffect { Inferred, Pure, Io, Unsafe, Async }
  ```
- Produces: `HirFn.declared: DeclaredEffect`.

- [ ] **Step 1: Write the failing tests** — `pure fn f() -> int { 1 }` parses; `io fn g() { println("x"); }` parses; `fn h()` has `DeclaredEffect::Inferred`; the formatter round-trips all four spellings.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — a modifier before `fn`, exactly where `pub` already sits (`Item::Public` wraps an item; effects need the same treatment: `Item::Effect { effect, item, span }`). Reuse the existing `unsafe` keyword rather than adding a second one.
- [ ] **Step 4: Run** `cargo test -p yashy-parser -p yashy-sema -p yashy-fmt` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: parse pure, io and unsafe function modifiers"`

---

### Task 2: Effect inference to a fixpoint

**Files:**
- Create: `crates/yashy-verify/src/pass/effects.rs`
- Modify: `crates/yashy-verify/src/lib.rs`

**Interfaces:**
- Consumes: `HirFn.declared`, `HirExpr::Call { unsafe_call, .. }`, `HirItem::Extern`.
- Produces: `pub fn infer_effects(hir: &HirProgram) -> BTreeMap<String, Effect>`; `verify` reports `E1300` when a declaration is exceeded.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_pure_function_that_prints_is_rejected() {
    let source = "pure fn f() { println(\"x\"); } fn main() { }";
    assert_eq!(codes(&verify_source(source)), vec!["E1300"]);
}

#[test]
fn an_unannotated_caller_of_an_io_function_is_inferred_io() {
    let source = "io fn g() { println(\"x\"); } fn h() { g(); } fn main() { h(); }";
    assert!(verify_source(source).diagnostics.is_empty());
}

#[test]
fn a_pure_function_calling_an_io_function_is_rejected() {
    let source = "io fn g() { println(\"x\"); } pure fn h() { g(); } fn main() { }";
    assert_eq!(codes(&verify_source(source)), vec!["E1300"]);
}

#[test]
fn mutual_recursion_terminates_and_propagates() {
    let source = "fn a(n: int) { if n > 0 { b(n - 1); } } fn b(n: int) { println(\"x\"); a(n); } fn main() { }";
    let report = verify_source(source);
    assert!(report.diagnostics.is_empty());
    assert_eq!(report.effects["a"].io, true);
}
```

- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — worklist over the call graph; a function's effect starts at `PURE` and joins each callee's effect plus its own body's direct effects (`println`/`print`/`read_line`/`read_file`/`write_file`/`getcwd`/`process_*`/`rand`/`time` are `io`; `alloc`/`free`/`len`/arithmetic/string building are `pure`; `extern` calls are `unsafe`). Terminate on lattice height (three flags ⇒ at most three updates per function).
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(verify): infer function effects to a fixpoint (E1300)"`

---

### Task 3: Declarations are promises

**Files:**
- Modify: `crates/yashy-verify/src/pass/effects.rs`
- Modify: `crates/yashy-common/src/codes.rs` (`EFFECT = "E1300"`, `UNSAFE_CALL = "E1301"`)
- Create: `docs/errors/E1300.md`, `docs/errors/E1301.md`, modify `docs/errors.md`

**Interfaces:**
- Produces: `E1300` — "declared `pure`, but the body performs `io`"; the label names the call that leaks the effect.
- Produces: `E1301` — an `unsafe` call outside an `unsafe fn` (or inside a `pure` one, which is the same violation stated twice).

- [ ] **Step 1: Write the failing tests** — `pure fn f() { unsafe_c_fn(); }` gives `E1301` and not `E1300` (one diagnostic per mistake); an `unsafe fn` may call the extern; `io fn` may not call `unsafe` without declaring it.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — declaration check: `declared` must be ≥ inferred. `DeclaredEffect::Pure` requires `inferred.is_pure()`; `Io` requires `!inferred.unsafe_`; `Unsafe` allows everything. A declaration that is *stricter* than the body is the error; a declaration that is *looser* is allowed and documented as "the function may do this, it currently does not".
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(verify): check declared effects against inferred ones"`

---

### Task 4: Typestate lattices

**Files:**
- Create: `crates/yashy-verify/src/typestate.rs`, `crates/yashy-verify/src/pass/typestate.rs`
- Modify: `codes.rs` (`DOUBLE_CLOSE = "E1400"`, `USE_AFTER_CLOSE = "E1401"`)
- Create: `docs/errors/E1400.md`, `E1401.md`

**Interfaces:**
- Produces:
  ```rust
  pub enum ResourceState { Allocated, Freed, Unlocked, Locked, Open, Closed }
  pub struct StateLattice { /* per binding, per path */ }
  ```
- Produces: `E1400` on a second release of the same resource; `E1401` on a use of a resource in a terminal state.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn closing_a_channel_twice_is_rejected() {
    let source = "fn main() { let c = chan_new(4); chan_close(c); chan_close(c); }";
    assert_eq!(codes(&verify_source(source)), vec!["E1400"]);
}

#[test]
fn using_a_channel_after_close_is_rejected() {
    let source = "fn main() { let c = chan_new(4); chan_close(c); chan_send(c, 1); }";
    assert!(codes(&verify_source(source)).contains(&"E1401"));
}

#[test]
fn unlocking_a_mutex_that_is_not_locked_is_rejected() {
    let source = "fn main() { let m = mutex_new(); mutex_unlock(m); }";
    assert_eq!(codes(&verify_source(source)), vec!["E1400"]);
}

#[test]
fn lock_then_unlock_is_accepted() {
    assert!(verify_source("fn main() { let m = mutex_new(); mutex_lock(m); mutex_unlock(m); }").diagnostics.is_empty());
}
```

- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — the lattice is driven by the builtin call names, not by types alone: `mutex_new` → `Unlocked`, `mutex_lock` requires `Unlocked` and produces `Locked`, `mutex_unlock` requires `Locked` and produces `Unlocked`, `mutex_free` requires `Unlocked`; `chan_new` → `Open`, `chan_close` requires `Open` → `Closed`, `chan_send`/`chan_recv` require `Open`; `alloc` → `Allocated`, free-family → `Freed`. Merge = intersection for "state is X on every path"; a disagreement at a merge is `E1400` with the two states named.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(verify): track resource typestate across the CFG (E1400, E1401)"`

---

### Task 5: Race freedom for spawned threads

**Files:**
- Modify: `crates/yashy-verify/src/pass/typestate.rs`
- Modify: `codes.rs` (`RACE = "E1402"`), `docs/errors/E1402.md`

**Interfaces:**
- Consumes: `HirProgram::uses_threads`, `HirExpr::Lambda` captures, `HirExpr::Call { callee: ["thread_spawn", ..] }`.
- Produces: `E1402` when a capture that reaches a spawned thread is a bare linear value.

- [ ] **Step 1: Write the failing tests** — spawning a closure that captures `*mut i64` is `E1402`; capturing a `Mutex` is accepted; capturing a `string` is accepted (atomic refcounts when `uses_threads`).
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — a capture is shareable if its type is `Copy`, or `Mutex`, or `Channel`, or `Str` (when `uses_threads`). Everything else is `E1402` with the capture named and the remedy "wrap it in a `Mutex`".
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(verify): reject unproven sharing across threads (E1402)"`

---

### Task 6: Contracts may only call pure functions

**Files:**
- Modify: `crates/yashy-verify/src/pass/contracts.rs`

**Interfaces:**
- Consumes: `infer_effects` from Task 2.
- Produces: `E1300` when a `requires`/`ensures`/`invariant` expression calls a non-pure function.

- [ ] **Step 1: Write the failing test** — `fn f(n: int) -> int requires g(n) > 0 { n }` with `io fn g` gives `E1300` at the contract span.
- [ ] **Step 2: Watch it fail.**
- [ ] **Step 3: Implement** — this is the check deliberately deferred from phase 1a; the pass already walks contract expressions, so it now also asks the effect map about each callee.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(verify): require contracts to be pure"`

---

### Task 7: Corpus, docs, gate

**Files:** `crates/yashy-driver/tests/verify_corpus/{effect_hidden_io.ya,effect_unsafe.ya,typestate_double_close.ya,typestate_after_close.ya,race_bare_pointer.ya}`; `docs/tutorial/` section "Effects and resource states"; `AGENTS.md` (identity section mentions effects and typestate).

- [ ] **Step 1: Add the five seeded bugs** and extend the expected-code table.
- [ ] **Step 2: Run the zero-false-positive loop** from phase 1b Task 7 on examples and programs.
- [ ] **Step 3: Run the gate table below on both OSes.**
- [ ] **Step 4: Commit and record** — `git commit -m "test: close phase 2 with the effect and typestate corpus"`.

---

## Gate summary

| Gate | Command | Passes when |
|---|---|---|
| Effect inference is sound | `cargo test -p yashy-verify effects` | all five effect tests pass; recursion terminates |
| Every `unsafe` is declared | `yashy verify --inventory` on the corpus | no `E1301`-free unsafe call site |
| Typestate | `cargo test -p yashy-verify typestate` | double-close, use-after-close, bad unlock rejected |
| Races | `cargo test -p yashy-verify race` | bare-pointer capture rejected; mutex/string accepted |
| No regression | `cargo test --workspace` | green on Windows and Linux |
| No false positives | the corpus loop | none on examples, programs, `file_cli.ya` |
