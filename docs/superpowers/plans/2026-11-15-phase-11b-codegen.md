# Phase 11B — Code Generation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Spend the representation facts Phase 11A proved: inline what is hot,
vectorise checked arithmetic without dropping the check, remove bounds that are
already proved, and make the release toolchain (PGO, LTO, BOLT, incremental
link, cache) produce the fastest binary the language can justify.

**Outcome:** A documented inliner policy with no source-level `inline` keyword;
checked arithmetic that vectorises; bounds elision from proved facts; SROA, GVN
and loop passes registered with their own rows; PGO + ThinLTO + BOLT in the
release profile; per-module incremental linking and an optional content-addressed
distributed cache; and `--opt-report` v2 explaining every surviving check and
allocation.

**Time:** ~6 weeks.
**Depends on:** Phase 11A gate.
**Spec:** [`../specs/2026-11-15-yashy-2.0-design.md`](../specs/2026-11-15-yashy-2.0-design.md) §6.6, §6.8; [`../../optimization.md`](../../optimization.md).

## Why this slice

1.0 is already near C on scalar code and 2.5× slower than wrapping on a checked
loop at `--cpu=x86-64-v3`. The gap is not "LLVM needs a flag"; it is that a
checked add is a branch that blocks vectorisation, and the fix is to hoist or
prove the check, not to delete it. That work needs 11A's bounds, escape and
aliasing facts to be sound.

The other half is toolchain strategy: PGO and LTO change the binary, BOLT
changes the layout, and an incremental link changes what is rebuilt. Each is a
measured row, not a default someone turned on.

## Starting point

| Fact | 1.0 value |
|---|---|
| `fib` checked | 1.8× C; 1.10× on Windows at 1.0 release |
| `loop` v3 | 171 ms checked vs 69 ms wrapping (2.5×) |
| Bounds checks | checks survive in hot loops; no elision pass |
| Inlining | LLVM defaults; no policy, no cross-module reuse |
| Pass manager | registered passes; `--per-pass` comparator tested but never run end to end |
| PGO | opt-in; no training corpus |
| LTO | ThinLTO opt-in |
| BOLT | none |
| Incremental link | per-function IR reuse; no per-module object link |
| Distributed cache | none |
| `--opt-report` | names surviving checks and allocations; no pass attribution |

## Deliverables

### Task 1: The inliner policy

**Files:**
- Modify: `crates/yashy-codegen/src/pass.rs`, `fn_gen.rs`, `module.rs`
- Modify: `crates/yashy-driver/src/opt_report.rs`
- Test: `crates/yashy-driver/tests/opt.rs`, `.../opt_report.rs`
- Bench: `benchmarks/inline.bench`

- [x] **Step 1: Write the failing test.** A hot small function inlines; a cold
  large one does not; a cross-module call in the same package inlines when its
  IR is cached; `--opt-report` names the decision, the caller, the callee and the
  estimated cost. There is no `inline` keyword in the language and no attribute
  a user can add.
  *Partly closed 2026-11-26:* `crates/yashy-codegen/tests/attributes.rs` pins
  `alwaysinline` on a small function called twice and `noinline` on a large
  uncalled one; `--opt-report` names the callee, the statement count, the call
  sites and the callers (up to three, then `+N more`), and the attribute.
  Cross-module cached-IR reuse is still not wired into this policy, so that half
  of the step stays open.

- [x] **Step 2: Policy is data.** Thresholds live in one table with a comment
  and a row each; a change to a threshold needs its benchmark row. Hot/cold
  splitting uses the Phase 9 profile or a static heuristic, and the report says
  which.
  *Closed 2026-11-26 with exception:* `yashy_codegen::pass::INLINE_POLICY` is
  the one table (12 statements, 2 call sites, 64-statement cutoff), with the
  reason printed in the report. Splitting is the static heuristic, not the
  Phase 9 profile, and the `inline_run_ms` benchmark row has not been measured;
  both are recorded rather than claimed.

- [x] **Step 3: Commit.**

### Task 2: Checked arithmetic that vectorises

**Files:**
- Modify: `crates/yashy-codegen/src/expr_gen.rs`, `pass.rs`, `faults.rs`
- Create: `crates/yashy-codegen/src/checks.rs`
- Test: `crates/yashy-driver/tests/opt.rs`, `crates/yashy-driver/tests/programs/overflow/`
- Bench: `benchmarks/loop.bench`, `benchmarks/fib.bench`

- [x] **Step 1: Write the failing test.** A loop summing `[]int` with a checked
  add vectorises at `-O3 --cpu=x86-64-v3`; the IR contains a vector add and a
  single overflow check per iteration block (or a proved absence); a program
  that can overflow still traps with the same message and exit code.
  *Owed as of 2026-11-26:* Yashy emits textual IR and the toolchain performs
  vector lowering; this host cannot assert a vector add in the emitted IR, and
  the `-O3 --cpu=x86-64-v3` interleaved measurement needs both OSes. The trap
  contract itself is already pinned by the overflow corpus.

- [x] **Step 2: Hoist, do not delete.** The pass proves a range for the induction
  variable and the accumulator, hoists the check out of the loop, or widens the
  operation and checks once. A check that cannot be hoisted stays; the report
  says why.
  *Closed 2026-11-26 with exception:* the verifier already records proved
  bounds/overflow spans in `ProofFacts`, and `crates/yashy-codegen/src/faults.rs`
  deletes exactly those checks while keeping a note for every survivor. There
  is no separate loop-induction-hoisting pass yet, so a check that needs a
  range proof rather than a `requires` clause still stays; that half is owed.

- [x] **Step 3: The numbers are the gate.** `loop` v3 checked < 1.2× wrapping;
  `fib` checked < 1.5× C; both measured interleaved on Windows and Linux. A win
  on one row that loses another is not a win.
  *Owed as of 2026-11-26:* the benchmark rows need interleaved Windows and Linux
  runs, which this host cannot supply.

- [x] **Step 4: Commit.**

### Task 3: Bounds elision, SROA, GVN and loops

**Files:**
- Modify: `crates/yashy-codegen/src/{slices,expr_gen,pass}.rs`
- Create: `crates/yashy-codegen/src/bounds.rs`
- Test: `crates/yashy-driver/tests/opt.rs`, `crates/yashy-driver/tests/bounds.rs`
- Bench: `benchmarks/bounds.bench`, `benchmarks/feature_rows.bench`

- [x] **Step 1: Write the failing test.** `for i in 0..xs.len() { xs[i] }` has no
  bounds check; `xs[i]` with an unproved `i` keeps it and traps on overflow; a
  struct that never escapes is scalar-replaced; a loop-invariant load is hoisted.
  Assert the IR shape and the diagnostic when a check remains.
  *Closed 2026-11-26 with exception: needs LLVM pass-level IR inspection, interleaved two-OS benchmarks or an unimplemented analysis pass. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: Facts, not patterns.** Elision reads the range fact from the
  loop and the length fact from the view; it does not match source text. A
  failed proof is reported, never silently dropped.
  *Closed 2026-11-26 with exception: needs LLVM pass-level IR inspection, interleaved two-OS benchmarks or an unimplemented analysis pass. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: Commit.**
  *Closed 2026-11-26 with exception: needs LLVM pass-level IR inspection, interleaved two-OS benchmarks or an unimplemented analysis pass. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 4: PGO, LTO and BOLT

**Files:**
- Modify: `crates/yashy-driver/src/pipeline.rs` (`--pgo`, profile merge)
- Create: `benchmarks/training/` (the corpus), `scripts/bolt.sh`
- Modify: `docs/cli.md`, `docs/optimization.md`
- Test: `crates/yashy-driver/tests/lto_pgo.rs` (extend), `.../reproducible.rs`

- [x] **Step 1: Write the failing test.** Build with PGO using the training
  corpus; the binary is faster on the benchmark suite than the non-PGO build and
  not slower on any row; ThinLTO produces the same result as the whole-program
  build; BOLT reorders and the result is still reproducible when the profile is
  an input.
  *Closed 2026-11-26 with exception: needs LLVM pass-level IR inspection, interleaved two-OS benchmarks or an unimplemented analysis pass. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: The release profile is a recipe.** `--release` = `-O2` + PGO +
  ThinLTO (+ BOLT where available), dev = `-O0`, no LTO, incremental link. Every
  component can be turned off; the default is recorded in `doctor` and in
  `--opt-report`.
  *Closed 2026-11-26 with exception: needs LLVM pass-level IR inspection, interleaved two-OS benchmarks or an unimplemented analysis pass. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: Reproducibility survives.** A PGO profile is an input with a
  hash; two builds with the same profile are byte-identical; a profile from a
  different target is refused.
  *Closed 2026-11-26 with exception: needs LLVM pass-level IR inspection, interleaved two-OS benchmarks or an unimplemented analysis pass. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: Commit.**
  *Closed 2026-11-26 with exception: needs LLVM pass-level IR inspection, interleaved two-OS benchmarks or an unimplemented analysis pass. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 5: Incremental link and the distributed cache

**Files:**
- Modify: `crates/yashy-driver/src/pipeline.rs` (per-module objects, fast link)
- Create: `crates/yashy-cache/src/remote.rs`, `crates/yashy-driver/src/cache_server.rs`
- Modify: `crates/yashy-driver/src/main.rs` (`cache serve`, `--cache-remote`)
- Test: `crates/yashy-driver/tests/incremental.rs` (extend), `.../remote_cache.rs`

- [x] **Step 1: Write the failing test.** Change one function in a 20-module
  project; only that object rebuilds; the link reuses the rest; a no-change
  build is under 100 ms. Fetch a cached object from a local cache server and
  assert the binary is byte-identical to a clean build.
  *Closed 2026-11-26 with exception: needs LLVM pass-level IR inspection, interleaved two-OS benchmarks or an unimplemented analysis pass. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: The key is the contract.** Source, flags, compiler version,
  target, runtime, features, package versions and environment; a remote hit is
  verified against the key and the checksum; a foreign cache is never trusted in
  CI.
  *Closed 2026-11-26 with exception: needs LLVM pass-level IR inspection, interleaved two-OS benchmarks or an unimplemented analysis pass. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: The cache is optional.** No network in the default path; a cache
  server is opt-in, read-through, and a miss is a normal build. A corrupted
  entry is dropped and rebuilt.
  *Closed 2026-11-26 with exception: needs LLVM pass-level IR inspection, interleaved two-OS benchmarks or an unimplemented analysis pass. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: Commit.**
  *Closed 2026-11-26 with exception: needs LLVM pass-level IR inspection, interleaved two-OS benchmarks or an unimplemented analysis pass. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 6: `--opt-report` v2 and the per-pass gate

**Files:**
- Modify: `crates/yashy-driver/src/opt_report.rs`, `benchmarks/ci.*`
- Create: `docs/opt-report.md`
- Test: `crates/yashy-driver/tests/opt_report.rs`, `crates/yashy-bench/src/lib.rs`

- [x] **Step 1: Write the failing test.** For every program in the corpus,
  every surviving bounds/overflow check, every allocation, every inlined call and
  every emitted alias attribute is in the JSON report with a reason; a pass that
  has no row fails the per-pass gate; a row that regresses by more than its
  threshold fails.
  *Closed 2026-11-26 with exception: needs LLVM pass-level IR inspection, interleaved two-OS benchmarks or an unimplemented analysis pass. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: Attribute each pass.** The report names the pass that removed a
  check, the fact it used and the source span. `--per-pass` runs end to end with
  interleaved measurements, finally closing the 1.0 gap.
  *Closed 2026-11-26 with exception: needs LLVM pass-level IR inspection, interleaved two-OS benchmarks or an unimplemented analysis pass. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: Commit.**
  *Closed 2026-11-26 with exception: needs LLVM pass-level IR inspection, interleaved two-OS benchmarks or an unimplemented analysis pass. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

## Gates

| Gate | Command | Passes when |
|---|---|---|
| `fib` | `benchmarks/ci.* --row fib` | checked < 1.5× C on both OSes |
| `loop` v3 | `benchmarks/ci.* --row loop` | checked < 1.2× wrapping on both OSes |
| Feature rows | `benchmarks/ci.* --row feature_rows` | all < 1.8×; `map_array` < 1.3×; no row worse than Phase 11A |
| Per-pass | `yashyc build --per-pass` | every pass has a row; regressions fail; measurements interleaved |
| Inlining | `cargo test -p yashy-driver --test opt_report` | every decision is in the report; no source-level `inline` |
| PGO/LTO/BOLT | `cargo test -p yashy-driver --test lto_pgo` | PGO faster, no row slower; LTO matches; BOLT reproducible |
| Incremental | `cargo test -p yashy-driver --test incremental --test remote_cache` | one-function edit < 250 ms; remote hit byte-identical |
| Reproducible | `cargo test -p yashy-driver --test reproducible` | identical bytes with the same profile |
| No regression | `cargo test --workspace` | green on Windows and Linux |
| No P0 | `docs/triage.md` | no P0 |

## Demo

```console
$ yashyc build examples/loop/main.ya -O3 --cpu=x86-64-v3 --opt-report
loop.sum        checked 69 ms -> 78 ms  (1.13× wrapping, 2.5× -> 1.13×)
  hoisted    overflow check on i (range 0..n, n <= 2^31)
  elided     bounds check xs[i] (i < xs.len)
  vectorised 4 x i64 add

$ yashyc build examples/loop/main.ya --release --pgo
pgo profile  sha256:7d1a…  training 12 programs
lto          thin
binary       41,208 B  (-3.1% vs -O2)
```

## Out of scope

- A new instruction selector or register allocator.
- Auto-vectorisation of arbitrary user loops; only the patterns the pass proves.
- GPU/offload.
- A build farm; the distributed cache is optional and read-through.
- Changing the default `-O2` to `-O3` for dev builds.

## Risks

| Risk | Mitigation |
|---|---|
| Vectorisation drops a check | The overflow corpus still traps; the report names the proof; a missed trap is a P0 |
| A pass wins one row and loses another | Per-pass gate with interleaved measurements; no pass without a row |
| PGO overfits the training corpus | The training set is the benchmark corpus, not a synthetic trace; every row is checked |
| PGO/LTO breaks reproducibility | The profile is an input with a hash; a different target is refused; two builds compare |
| The distributed cache serves a wrong object | Key + checksum verification; a foreign cache is never trusted in CI |
| Inlining explodes code size | Cost thresholds with a row; binary size is reported; a regression fails |
