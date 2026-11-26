# Phase 11A — Representation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Decide what memory *is* before teaching the optimizer to exploit it:
layout, aliasing promises, stack lifetime, heap calls, small values and startup
— each measured, each inspectable, each proven before it is spent.

**Outcome:** A layout report that matches the IR and a field reordering pass for
non-`repr(C)` types; an alias analysis whose every `noalias`/`nonnull`/
`dereferenceable` attribute is justified by a verifier fact; escape analysis
that promotes non-escaping constant allocations to the stack and an explicit
`stack_alloc`; a direct-heap accounting surface (`--alloc-report`), small-string
and map/array layout work, and a startup path measured on both OSes.

**Time:** ~6 weeks.
**Depends on:** Phase 10 gate.
**Spec:** [`../specs/2026-11-15-yashy-2.0-design.md`](../specs/2026-11-15-yashy-2.0-design.md) §6.6, §6.8; [`../../architecture.md`](../../architecture.md) §5, §11.

## Why this slice

Phase 11B can only optimise what this slice makes true. `noalias` is sound only
because owning pointers are linear; a field reorder is sound only because a
non-`repr(C)` type has no source-level layout promise; stack promotion is sound
only because the verifier proves the pointer does not escape. Building the
optimizer first would teach it to assume a model the language never promised —
the classic miscompile that no benchmark catches until a user does.

This is also where "as fast as possible" becomes a set of numbers: allocation
sites, stack slots, bytes per value, and cold-start milliseconds.

## Starting point

| Fact | 1.0 value |
|---|---|
| Layout | `repr(C)` only; the compiler knows sizes; no user report |
| Aliasing | owning pointers are linear; no `noalias` policy; no audit |
| Stack | values live on the stack; `alloc` always calls the allocator; no promotion |
| Heap | `alloc` calls C `malloc`; no allocator interface, no per-site report |
| Strings | refcount + length + pointer; no small-string form |
| Map | open addressing, measured 3.8× on `map_array` |
| Startup | runtime init before `main`; no startup row |
| Reports | `--opt-report` exists; no layout/stack/alloc report |

## Deliverables

### Task 1: Layout, niche and the report

**Files:**
- Modify: `crates/yashy-hir/src/layout.rs`, `crates/yashy-codegen/src/ir.rs`
- Create: `crates/yashy-driver/src/layout_report.rs`
- Modify: `crates/yashy-driver/src/main.rs` (`inspect --layout`)
- Test: `crates/yashy-driver/tests/layout.rs`, `crates/yashy-hir/tests/layout.rs`

- [x] **Step 1: Write the failing test.** For every corpus type, `inspect
  --layout --json` reports size, alignment, field offsets, padding and the niche
  (if any); the same numbers are read from the generated IR; a mismatch fails.
  *Closed 2026-11-26:* `inspect --layout <type> --json` and
  `inspect --layout --all --json` both render one report shape from the same
  `yashy-hir` walk a `sizeof` query uses: `size`, `align`, per-field `offset`
  and `size`, `padding`, `saved` and `niche`. `crates/yashy-driver/tests/layout.rs`
  independently re-derives each declared struct and enum from the `= type { }`
  aggregate the backend emits and fails on any difference; `find` now resolves
  enums as well as structs, so `--all` reports every declared type.
- [ ] **Step 2: Reorder only what is free.** A non-`repr(C)` struct's fields may
  be reordered to reduce padding; the report shows the chosen order and the
  bytes saved. A `repr(C)`, `repr(transparent)` or `repr(packed)` type is never
  reordered. An enum niche is used only when it is ABI-safe.
  *Still owed 2026-11-26:* the report exposes `saved: 0` and `niche: null`;
  no type is reordered or exploits a niche yet. Reordering changes the
  field-index contract shared by the checker's constant folds and the
  backend's `insertvalue`/`extractvalue`, so it needs its own slice.
- [x] **Step 3: `size_of`/`align_of`/`offset_of` read the same table.** The
  const evaluator and the report cannot disagree; a disagreement is a compiler
  bug with a test.
  *Closed 2026-11-26:* `crates/yashy-driver/tests/layout.rs` compares
  checker-folded `sizeof`/`alignof`/`offsetof` constants with the HIR layout
  walk and with `inspect --layout`'s `find`, pins the C padding rule as an
  independent anchor, and checks that the literals reach the emitted IR.
  `every_declared_type_matches_the_ir_aggregate` adds the generated-IR half.
- [ ] **Step 4: Commit.**

### Task 2: Alias analysis and the attribute audit

**Files:**
- Create: `crates/yashy-codegen/src/alias.rs`
- Modify: `crates/yashy-codegen/src/ffi.rs`, `fn_gen.rs`, `ir.rs`
- Modify: `crates/yashy-driver/src/opt_report.rs`
- Create: `crates/yashy-driver/tests/alias_audit.rs`
- Test: `crates/yashy-driver/tests/alias_corpus.rs` (Phase 4 corpus)

**Interfaces:**
- `AliasFacts { unique: bool, non_null: bool, dereferenceable: Option<u64>, provenance: Provenance }`
- `attributes_for(param: &HirParam, facts: &AliasFacts) -> Vec<IrAttribute>`
- `opt_report` records `(attribute, fact, span)` for every emitted attribute.

- [x] **Step 1: Write the failing test.** An owning pointer parameter gets
  `noalias`/`nonnull`; an FFI `*const T` gets neither; a `noalias` `unsafe`
  parameter gets it because the caller promised; an unknown provenance gets
  nothing. `--opt-report=json` names the fact and the span for each attribute.
  *Closed 2026-11-26 with exceptions:* `crates/yashy-codegen/tests/attributes.rs`
  pins the IR for a read-only pointer (`noalias nonnull readonly align 8`), a
  written one (`noalias nonnull align 8`), an escaping one (`nonnull align 8`)
  and an `Option[*mut int]` (no attribute). There is no separate `unsafe
  noalias` parameter spelling; a safe owning pointer earns the same word from
  the escape fact. FFI declarations are `declare`, not `define`, and carry no
  parameter attributes; `crates/yashy-driver/tests/alias_corpus.rs` covers the
  unsafe trust assumptions.
- [x] **Step 2: Only proved facts.** The analysis consumes ownership, escape and
  bounds facts; it never invents one. A pointer from an `unsafe` block with
  unknown provenance is opaque until a checked cast or a bounds fact proves it.
  *Closed 2026-11-26:* `fn_gen::parameters` reads only `Module::param_attrs`,
  and `without_facts_no_attribute_is_emitted` proves that disabling the facts
  removes every attribute. A pointer whose provenance is not proved has no
  table entry, so it is opaque by omission rather than by a negative claim.
- [ ] **Step 3: Audit in CI.** For every corpus program, every emitted attribute
  has an entry in the report; an attribute without a fact fails the audit. The
  whole-program reference path produces the same binary as the per-module path.
  *Partly closed 2026-11-26:* `crates/yashy-driver/tests/alias_audit.rs` builds
  each corpus source twice and fails when an attribute from `--emit-ir` has no
  matching fact, or when an `attribute` note has no span. The whole-program
  reference path is not wired into this harness, so binary equality remains
  owed and the step stays open.
- [ ] **Step 4: Commit.**

### Task 3: Stack promotion and `stack_alloc`

**Files:**
- Create: `crates/yashy-codegen/src/stack.rs`
- Modify: `crates/yashy-codegen/src/escape.rs` (Phase 1/4 facts), `memory.rs`
- Modify: `crates/yashy-verify/src/ownership.rs` (escape refusal)
- Create: `crates/yashy-driver/src/stack_report.rs`
- Test: `crates/yashy-driver/tests/stack.rs`, `crates/yashy-driver/tests/programs/stack/`

- [ ] **Step 1: Write the failing test.** `let p = alloc(16); p[0] = 1;` with no
  escape runs with zero heap calls and `--stack-report` names the site; a
  pointer returned, stored in a field or sent to another task is not promoted;
  `stack_alloc(64)` works and an escaping `stack_alloc` is refused with a span.
- [ ] **Step 2: Promotion is a proof, not a heuristic.** Constant size, no
  escape, no address observation, no `unsafe` escape. The report says why a site
  was promoted or why it was not. Stack depth is accounted per function; a
  recursion that can exceed the guard is a diagnostic or a documented runtime
  trap with a deterministic message.
- [ ] **Step 3: Commit.**

### Task 4: Direct heap accounting

**Files:**
- Create: `crates/yashy-runtime/alloc/` (size classes, counters, hooks)
- Modify: `crates/yashy-stdlib/yashy/alloc.ya`, `crates/yashy-codegen/src/memory.rs`
- Create: `crates/yashy-driver/src/alloc_report.rs`
- Test: `crates/yashy-driver/tests/alloc_report.rs`, `crates/yashy-driver/tests/leaks.rs`

- [ ] **Step 1: Write the failing test.** `--alloc-report` lists every site with
  count, bytes, size class, whether it was stack-promoted, and the call path;
  `alloc_uninit` appears with an `unsafe` marker; a leak appears at exit with the
  allocation span.
- [ ] **Step 2: No hidden allocation.** The runtime allocates only through the
  interface; a runtime call that allocates is a bug. `alloc` zeroes; the
  zeroing is removed only when the optimizer proves the program writes before it
  reads, and the report says which happened.
- [ ] **Step 3: Alignment and size are explicit.** Every allocation carries size
  and alignment; a mismatch on free is a trap with a stable message; an
  over-aligned type uses the aligned path.
- [ ] **Step 4: Commit.**

### Task 5: Small strings, map and array layout

**Files:**
- Modify: `crates/yashy-codegen/src/strings.rs`, `maps.rs`, `fixed.rs`
- Modify: `crates/yashy-stdlib/yashy/string.ya`, `collections/{vec,map}.ya`
- Test: `crates/yashy-driver/tests/programs/strings/`, `.../collections/`
- Bench: `benchmarks/feature_rows.bench`

- [ ] **Step 1: Write the failing test.** A string ≤ 22 bytes does not
  allocate; a longer one does; a `Vec` grows by a documented factor; a `Map` with
  1M integer keys keeps its load factor and iteration order; `map_array`,
  `map_scan` and `debug_format` rows are re-measured.
- [ ] **Step 2: The layout is the optimisation.** Small-string inline storage,
  a map with one probe sequence and a documented load factor, and array growth
  that does not copy twice. No benchmark-specific special case; the shape is the
  win.
- [ ] **Step 3: Commit.**

### Task 6: Startup and the representation gate

**Files:**
- Create: `crates/yashy-runtime/startup.c`
- Modify: `crates/yashy-driver/src/pipeline.rs` (link order), `main.rs`
- Create: `crates/yashy-driver/tests/startup.rs`
- Modify: `docs/architecture.md` §5, §11; `docs/performance-improvements.md`

- [ ] **Step 1: Write the failing test.** `hello` runs in under 2 ms of runtime
  startup on both OSes; a static-init-heavy program reports its init cost; the
  binary size is recorded; a startup regression beyond 10% fails.
- [ ] **Step 2: Initialise only what is used.** No eager runtime table, no
  global allocator warmup, no locale or timezone scan at startup. A program that
  uses a feature pays for it; one that does not, does not.
- [ ] **Step 3: Commit.**

## Gates

| Gate | Command | Passes when |
|---|---|---|
| Layout | `cargo test -p yashy-driver --test layout` | report matches IR; reorder saves bytes; `repr(C)` unchanged |
| Alias audit | `cargo test -p yashy-driver --test alias_audit` | every attribute has a fact and a span; the corpus binary matches the reference |
| Stack | `cargo test -p yashy-driver --test stack` | promoted sites have zero heap calls; escaping `stack_alloc` is refused |
| Heap | `cargo test -p yashy-driver --test alloc_report` | every site is reported; no hidden allocation; leak spans at exit |
| Feature rows | `yashyc bench feature_rows --check-baseline` | `map_array` < 1.3×, allocation-heavy rows < 1.3×, no row worse than Phase 9 |
| Startup | `cargo test -p yashy-driver --test startup` | `hello` < 2 ms; regression < 10% |
| Leaks | `cargo test -p yashy-driver --test leaks` | zero |
| No regression | `cargo test --workspace` | green on Windows and Linux |
| No P0 | `docs/triage.md` | no P0 |

## Demo

```console
$ yashyc inspect --layout Order --json
{ "type":"Order", "size":24, "align":8, "saved":8,
  "fields":[{"name":"id","offset":0},{"name":"total","offset":8},{"name":"items","offset":16}] }

$ yashyc build main.ya --alloc-report
alloc sites: 3
  main.ya:12  alloc(16)    0 heap calls  stack-promoted  (no escape)
  main.ya:31  Vec.push     1,024 calls   16,384 B     class 32
  main.ya:44  alloc_uninit 1 call        64 B         unsafe

$ yashyc run examples/hello/main.ya --startup
hello, world
startup 0.41 ms  (init 0.08 ms, args 0.02 ms)
```

## Out of scope

- A new register allocator or instruction selector (11B).
- A custom global allocator replacing the system one by default; the interface
  allows one, the default stays the platform allocator.
- Compressed pointers or pointer tagging beyond the documented niche.
- A garbage collector, a borrow checker, or named lifetimes.
- PGO/LTO/BOLT (11B).

## Risks

| Risk | Mitigation |
|---|---|
| Field reordering breaks FFI | `repr(C)`/`transparent`/`packed` never reorder; the layout report is the gate |
| `noalias` miscompiles | Every attribute names a proved fact; the audit fails an unjustified one; the corpus binary equals the reference |
| Stack promotion changes semantics | Requires constant size + no escape + no address observation; the verifier refuses an escaping `stack_alloc` |
| Small-string optimisation changes ownership | The string's release path is one function; a leak/double-free test covers inline and heap forms |
| Startup hides work in a constructor | No eager init; a startup row per feature; a regression fails |
| The report becomes a second source of truth | The report is rendered from the same layout/alias/escape facts codegen uses |
