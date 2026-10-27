# M9–M10 Implementation Plan — Escape Analysis and Self-Hosting

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove allocations the programmer was forced to write but did not need, then rewrite the compiler in Otter and have it compile itself.

**Architecture:** M9 is an optimiser that deletes explicit `alloc` calls whose result provably never escapes the function, after correctness is already guaranteed by M3's linear ownership — so a missed promotion is only ever a missed speedup, never a bug. M10 rewrites the compiler in Otter in ascending order of self-sufficiency, comparing outputs against the Rust original at each stage.

**Tech Stack:** Rust 1.99 for the oracle implementation, Otter for the self-hosted one, LLVM IR text for both.

**Spec:** §5.2 (escape analysis is an optimisation, not a safety mechanism), §5.3, and the stated goal of self-hosting. **Master plan:** `2026-10-16-otter-master-plan.md`.

## Prerequisites

- M0–M8 complete. The stdlib, the tutorial, and the test suite all exist and are green on both platforms.
- The test suite is broad enough to be a useful oracle for M10. If it is not, widen it before starting M10 — that is the whole safety net.

## Scope and honest expectations

M9 is research-grade and is allowed to be incomplete: a program that never escapes but is not promoted is simply still allocating. M10 depends on M9 only for **correctness of the harness**, not for speed.

The single most valuable thing this plan can do is **not cut the differential test harness**. Every M10 stage compares the Rust compiler and the Otter compiler on the same inputs, byte for byte on diagnostics and structurally on IR.

---

### Task 1: The differential test harness

**Files:**
- Create: `crates/otter-driver/tests/differential.rs`
- Create: `crates/otter-driver/tests/programs/corpus/*.ot` (a corpus of 40+ programs, including all earlier end-to-end programs)
- Create: `crates/otter-driver/tests/programs/should_fail/*.ot` (30+ programs that must be rejected)

**Interfaces:**
- Consumes: the full compiler, both front-ends and back-ends
- Produces: `pub fn compile_both(src: &str) -> (Result<String, Vec<Diagnostic>>, Result<String, Vec<Diagnostic>>)` comparing the Rust compiler against a second binary at `$OTTER_ORACLE`; env `OTTER_DIFFERENTIAL=1` enables it

- [ ] **Step 1: Write the failing test** — with no `OTTER_ORACLE` set, the suite asserts it skips loudly rather than passing silently.

- [ ] **Step 2: Run to verify it fails** → FAIL.

- [ ] **Step 3: Build the corpus.** Reuse every end-to-end program written so far, plus programs that exercise each diagnostic in the test suite.

- [ ] **Step 4: Run to verify it passes** with `OTTER_ORACLE` pointing at the current compiler binary (a self-comparison baseline, which must be trivially green).

- [ ] **Step 5: Commit**

```bash
git add crates/otter-driver/tests/
git commit -m "test: differential harness and regression corpus for the bootstrap"
```

---

### Task 2: Escape analysis — the analysis

**Files:**
- Create: `crates/otter-codegen/src/escape.rs`
- Create: `crates/otter-codegen/tests/escape.rs`
- Create: `crates/otter-codegen/tests/golden/escape_{promote,retain,unknown}.ot`

**Interfaces:**
- Consumes: `HirFn` from M0–M2/M3
- Produces:
  - `pub enum Escape { NoEscape, Escapes, Unknown }` — `Unknown` is the conservative default and means "leave the allocation alone"
  - `pub fn analyse_function(f: &HirFn) -> HashMap<String, Escape>` keyed by binding name
  - Rules: a binding whose address is never taken and is never passed to a function that escapes it is `NoEscape`; anything reachable from a returned value, a stored aggregate, a `move` to another binding, an `extern` call, or a `Thread.spawn` argument is `Escapes`; anything the analysis cannot follow is `Unknown`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_buffer_only_written_and_summed_within_its_function_does_not_escape() {}

#[test]
fn a_buffer_returned_from_the_function_escapes() {}

#[test]
fn a_buffer_passed_to_an_unknown_function_is_unknown_not_no_escape() {
    // The conservative default is the whole safety argument. This is the most
    // important single test in M9.
}

#[test]
fn a_buffer_handed_to_an_extern_call_escapes() {}

#[test]
fn a_buffer_captured_by_a_thread_spawn_escapes() {}

#[test]
fn a_buffer_stored_into_a_returned_struct_escapes() {}

#[test]
fn a_buffer_moved_onto_another_binding_keeps_the_originals_escape_answer() {}
```

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement**

A whole-function intraprocedural pass over the HIR. Default every binding to `Unknown` and clear it only on positive proof. **Any construct the visitor does not explicitly understand leaves its operand `Unknown`** — a missed case is a missed optimisation.

- [ ] **Step 4: Run to verify they pass** → PASS. Add and read the golden files.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-codegen/
git commit -m "feat(codegen): intraprocedural escape analysis, conservative by default"
```

---

### Task 3: Stack promotion

**Files:**
- Modify: `crates/otter-codegen/src/memory.rs` (`gen_alloc` consults the analysis)
- Create: `crates/otter-driver/tests/programs/{stackvec,hotloop}.ot`
- Modify: `crates/otter-driver/tests/e2e.rs`

**Interfaces:**
- Consumes: Task 2's `analyse_function`
- Produces: `gen_alloc` emits `alloca` + alignment padding instead of `@otter_alloc` when the binding is `NoEscape`; `Module::new` gains `pub fn finish_module(&self) -> String`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_no_escape_allocation_emits_an_alloca_and_no_otter_alloc_call() {
    // This is spec §5.2 in code: the explicit alloc disappears.
}

#[test]
fn an_escaping_allocation_still_calls_otter_alloc() {}

#[test]
fn an_unknown_allocation_still_calls_otter_alloc() {
    // Conservative. Getting this wrong is a miscompile, not a slowdown.
}

#[test]
fn dealloc_of_a_promoted_allocation_is_dropped() {
    // Calling free on an alloca is a hard error, so promotion must remove the
    // matching dealloc too — or the program is broken in a new way.
}

#[test]
fn a_large_stack_allocation_is_capped_and_left_on_the_heap() {
    // Promoting a 1 MiB buffer to the stack overflows it. Cap at 4 KiB.
}
```

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement**

Promotion is gated on both `NoEscape` *and* a constant, small size. `dealloc` on a promoted value becomes a no-op **only** when the same binding was promoted — the compiler must track that per binding, not per expression.

- [ ] **Step 4: Run to verify they pass** → PASS, and re-run **every** M3 leak test unchanged. A promotion bug shows up as a leak or a stack overflow there.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-codegen/ crates/otter-driver/
git commit -m "feat(codegen): stack-promote provably non-escaping allocations"
```

---

### Task 4: Benchmarks

**Files:**
- Create: `crates/otter-driver/benches/{alloc_hot,string_build}.ot`
- Create: `crates/otter-driver/tests/bench.rs`

**Interfaces:**
- Consumes: Task 3
- Produces: a benchmark runner reporting allocation counts via a counting allocator in `otter-runtime` (`OTTER_ALLOC_COUNT` env var) and wall time

- [ ] **Step 1: Write the failing test** asserting the counting allocator reports the pre-promotion count before Task 3 and a lower count after — as a stored baseline, not a threshold.

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement the counting allocator** in the runtime, active only when the env var is set, so normal runs pay nothing.

- [ ] **Step 4: Run to verify they pass**, and record the before/after numbers in the commit message.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-driver/
git commit -m "test: allocation benchmarks and counting allocator"
```

---

### Task 5: The compiler in Otter — `otter-common`

**Files:**
- Create: `otter-src/common/{span,source,diagnostic}.ot`
- Create: `crates/otter-driver/tests/bootstrap.rs`

**Interfaces:**
- Consumes: Task 1's differential harness
- Produces: `OTTER_ORACLE` build integration — `otter build otter-src/` compiles the Otter sources alongside the Rust ones, so both implementations are always testable together

- [ ] **Step 1: Write the failing test** that the Otter `Span` type and the Rust `Span` behave identically on the same inputs.

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Port `span.ot` and `source.ot`.** Start with these because they need only `i64`, structs, and pointers — the least the language offers.

- [ ] **Step 4: Run to verify they pass** → PASS. Then extend the differential corpus with every `SourceMap` test case.

- [ ] **Step 5: Commit**

```bash
git add otter-src/ crates/otter-driver/tests/bootstrap.rs
git commit -m "feat(bootstrap): Span and SourceMap in Otter"
```

---

### Task 6: The lexer in Otter

**Files:**
- Create: `otter-src/lexer/{token,lexer,error}.ot`
- Modify: `crates/otter-driver/tests/bootstrap.rs`

**Interfaces:**
- Consumes: Task 5
- Produces: an Otter `tokenize` whose token stream is compared element by element against the Rust lexer's for every golden and fuzz input

- [ ] **Step 1: Write the failing test** comparing token streams on all 40+ corpus programs and every lexer golden file.

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Port the lexer.** This is the first real test of slices, strings, and manual memory at scale.

- [ ] **Step 4: Run to verify they pass** → PASS. Run it against **every file in the fuzz corpus** — this is where an off-by-one in span arithmetic shows up.

- [ ] **Step 5: Commit**

```bash
git add otter-src/lexer/
git commit -m "feat(bootstrap): lexer in Otter, token-for-token identical to Rust"
```

---

### Task 7: The parser in Otter

**Files:**
- Create: `otter-src/parser/{ast,items,stmts,exprs,error}.ot`
- Modify: `crates/otter-driver/tests/bootstrap.rs`

**Interfaces:**
- Consumes: Task 6
- Produces: an Otter `parse_program` whose AST `Debug` output is compared to the Rust parser's for the whole corpus

- [ ] **Step 1: Write the failing test** comparing AST dumps for every corpus program and every diagnostic case.

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Port the parser** including the unclosed-delimiter stack and the nesting limit.

- [ ] **Step 4: Run to verify they pass** → PASS, including every input from the `never_panics` table.

- [ ] **Step 5: Commit**

```bash
git add otter-src/parser/
git commit -m "feat(bootstrap): parser in Otter, AST-identical to Rust"
```

---

### Task 8: Sema in Otter, part one — types and scopes

**Files:**
- Create: `otter-src/sema/{ty,scope,error}.ot`
- Modify: `crates/otter-driver/tests/bootstrap.rs`

**Interfaces:**
- Consumes: Task 7
- Produces: an Otter type checker producing identical diagnostics to the Rust one, message text and span included

- [ ] **Step 1: Write the failing test** comparing diagnostic output byte for byte on all 30+ `should_fail` programs.

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Port types and scopes.** Diagnostic *message text* must match exactly, because the differential harness compares it — this is the strictest constraint in the whole plan and it is worth it.

- [ ] **Step 4: Run to verify they pass** → PASS.

- [ ] **Step 5: Commit**

```bash
git add otter-src/sema/
git commit -m "feat(bootstrap): types and scopes in Otter, diagnostics byte-identical"
```

---

### Task 9: The self-hosted compiler runs the test suite

**Files:**
- Create: `otter-src/main.ot`
- Modify: `crates/otter-driver/tests/bootstrap.rs`
- Create: `.github/workflows/bootstrap.yml`

**Interfaces:**
- Consumes: Tasks 5–8
- Produces: `otter run otter-src/main.ot <file.ot>` — a compiler written in Otter that lexes, parses, and type-checks. It does **not** yet emit LLVM; codegen in Otter is explicitly out of this plan.

- [ ] **Step 1: Write the failing test** running the Otter compiler on the entire corpus and requiring identical output to the Rust compiler.

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Complete the front-end.**

- [ ] **Step 4: Run to verify they pass** → PASS on all 70+ corpus programs.

- [ ] **Step 5: CI gate** — a workflow that runs the bootstrap test on every push, so the self-hosted compiler can never silently drift.

- [ ] **Step 6: Commit**

```bash
git add otter-src/main.ot .github/workflows/bootstrap.yml
git commit -m "feat(bootstrap): the self-hosted Otter compiler runs the full test suite"
```

---

### Task 10: Close out

**Files:**
- Modify: `docs/superpowers/specs/2026-10-16-otter-language-design.md`
- Modify: `docs/superpowers/plans/2026-10-16-otter-master-plan.md`

**Interfaces:**
- Consumes: everything
- Produces: the ledger updated; the spec's "Goals" section stating plainly what was achieved and what was not

- [ ] **Step 1: Reconcile the ledger** with the code.

- [ ] **Step 2: Write the honest closing statement.** Specifically: escape analysis's coverage rate, whether the self-hosted compiler emits code, and which milestones slipped against the original estimate. A language that claims completion it does not have is worse than one that is candid.

- [ ] **Step 3: Commit**

```bash
git add docs/
git commit -m "docs: close the ledger and record the honest M0-M10 outcome"
```

---

## Definition of Done for M9–M10

- Escape analysis never miscompiles: every M3–M6 test still passes unchanged, including every leak test.
- A no-escape allocation emits `alloca` and no `otter_alloc`; an escaping or unknown one still calls `otter_alloc`.
- The benchmark reports a lower allocation count than the M3 baseline with byte-identical program output.
- The differential harness compares the Rust and Otter compilers on 70+ programs, including 30+ that must be rejected, and every diagnostic matches byte for byte.
- The self-hosted Otter compiler passes the entire corpus, and CI fails if it ever drifts.

## Explicitly out of scope, and why

- **Self-hosted codegen.** Writing an LLVM backend in Otter is a project in itself and belongs in its own plan after M10 is stable.
- **A general escape analysis.** Cross-function, interprocedural, and field-sensitive escape analysis is research; M9 is intraprocedural by design.
- **Optimising away `defer`.** Defers stay observable even when analysis could prove they are dead, because making them invisible changes semantics in ways users will not expect.
- **Data-race detection.** Spec §7 defers it indefinitely.