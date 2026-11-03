# Phase 4 — Optimization: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the compiler fast enough to feel instant and the programs fast enough to stop apologising for — by spending what phases 1–3 proved. The verifier already knows which pointers are unique, which values are not optional and which functions are pure; this phase turns those facts into LLVM attributes, promotes what never escapes, and measures every pass so no later phase can quietly buy one row by selling another.

**Architecture:** Two halves that share one mechanism. The **dev loop** becomes a memoised query pipeline (`parse` → `resolve` → `check` → `verify` → `emit`, keyed by content hash) run in parallel over independent units, which is both the speed win and the principled key for phase 3's cache. The **program** gets five things in dependency order: verifier facts as IR attributes (`noalias`, `nonnull`, `readonly`, `align`, `dereferenceable`); the two passes LLVM cannot do because they need ownership information (escape-to-stack/arena promotion, literal interning); inlined runtime fast paths; ThinLTO + PGO at release; and a pass manager with a per-pass regression gate. Inlining, GVN and unrolling stay LLVM's job, with a test that asserts we are not reimplementing them.

**Tech Stack:** a new `crates/yashy-query` (std-only, no dependencies), `crates/yashy-codegen/src/{ir.rs,module.rs,fn_gen.rs,memory.rs,arena.rs}` (attribute and fast-path emission), `crates/yashy-verify` (`ProofFacts`, already produced in phase 3), `crates/yashy-runtime` (`alwaysinline` C entry points), `crates/yashy-driver/src/{main.rs,pipeline.rs}` (`--opt-report`, `--no-opt`, `--pgo`), and `crates/yashy-bench`.

**Spec:** `docs/superpowers/specs/2026-10-27-yashy-master-design.md` §5, §8 row 4. Index: `docs/superpowers/plans/2026-10-27-yashy-master-plan.md`. Predecessor: `2026-10-27-phase-03b-native-foundation.md`.

## Global Constraints

See the master index. The ones that bite here:

- **No new runtime dependencies.** The query engine is hand-written; there is no salsa, no `rayon`, no `crossbeam`. Parallelism is `std::thread::scope` and `available_parallelism()`.
- **A pass may not change semantics.** The differential corpus (ASan/UBSan, plus the existing `e2e` programs) is the proof, and a pass that only passes because a test was relaxed is a bug.
- **One rule, one implementation.** The query engine owns incremental invalidation; the cache owns storage; a pass owns one transformation. No second copy of any of them.
- **No unproven speed claim.** Every number in this plan comes from `key=value` bench artifacts, medians of 5, on Windows and Linux.
- **Both operating systems or it is not done** — including PGO, where the profile file format differs.
- **Format touched files only.**

## Written cap (this phase)

- **No `inline` keyword, no attributes in the language.** LLVM decides what to inline and `--opt-report` says what it decided. Syntax that promises a backend outcome is a lie the moment the backend changes.
- **No struct-layout optimization.** Field reordering and niche packing are a separate design with an ABI story; the `object`/`struct` layout stays declaration order.
- **No custom register allocator, no instruction selector, no scheduler.** LLVM's are better than anything this plan would produce in ten weeks.
- **No `-march=native` by default.** `--native` stays opt-in; a binary that only runs on the machine that built it is not a release artifact.
- **No profile collection on the user's machine without `--pgo`.** A build that writes `.profraw` files by surprise is a privacy and a disk-space bug.

## Review Focus

1. **A stale query result is a wrong binary, so the key must be complete.** Every query key covers the function's HIR hash, the compiler version, the target triple, the relevant flags, and — for cross-function queries — the hashes of what it read. Task 1's gate is not "it is fast", it is "editing a function re-analyses it and its dependents, and nothing else".
2. **Escape promotion is gated on the verifier, not on a heuristic.** An object is promoted to the stack only when the ownership facts say no reference to it leaves the function, and to an arena only when no reference leaves the scope. A shared value that gets promoted is a use-after-free that only shows up under load, which is why the ASan/UBSan differential corpus is a gate and not a nice-to-have.
3. **`noalias` is a promise with undefined behaviour behind it.** It may be emitted only where ownership proves the pointer is the unique live reference for the call's duration. A wrong `noalias` miscompiles silently; the attribute emitter must be a table lookup from `ProofFacts`, never a default.
4. **The per-pass gate compares passes against each other, not against nothing.** Each pass records its own before/after rows; a pass that improves `fib` by 8% and regresses `wordcount` by 5% fails the gate even though the mean improved. The table is the artefact.
5. **`--opt-report` explains, it does not scold.** Every unelided bounds check, heap allocation and non-inlined call names *why* it survived and, where one exists, the contract that would remove it. A report that says "not inlined" without a reason is noise, and noise is not read.

---

### Task 1: `yashy-query` — a memoised pipeline

**Files:**
- Create: `crates/yashy-query/{Cargo.toml,src/lib.rs,src/query.rs,src/db.rs}`
- Modify: `Cargo.toml` (member), `crates/yashy-driver/Cargo.toml`
- Modify: `crates/yashy-driver/src/pipeline.rs` (every stage goes through the database)
- Modify: `crates/yashy-driver/src/main.rs` (`--timings` rows `query_hits`, `query_misses`)
- Create: `crates/yashy-query/tests/incremental.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct Db { /* memo tables, one per query kind */ }
  impl Db {
      pub fn parse(&mut self, file: FileId, text: &str) -> QueryResult<Vec<Item>>;
      pub fn check(&mut self, file: FileId) -> QueryResult<CheckedFile>;
      pub fn verify_fn(&mut self, f: FnId) -> QueryResult<FnFacts>;
      pub fn emit_fn(&mut self, f: FnId) -> QueryResult<String>;
  }
  pub struct QueryStats { pub hits: u64, pub misses: u64, pub recomputed: Vec<String> }
  ```
- Produces: invalidation by input hash, not by timestamp. A query re-runs when any input it *read* changed; the read set is recorded on the first execution (the classic red-green algorithm, hand-written).
- Produces: `yashy check --timings` prints `query_hits=N query_misses=M recomputed=fn:a,fn:b`.

- [x] **Step 1: Write the failing tests** — a second `check` on unchanged sources has `misses == 0`; editing one function's body re-runs `check` for that file and `verify_fn`/`emit_fn` for that function only; editing a struct field re-runs every function that mentions the type; `recomputed` is sorted and deterministic.
- [x] **Step 2: Run and watch them fail** — `cargo test -p yashy-query` → FAIL.
- [x] **Step 3: Implement** — a `HashMap<QueryKey, Entry>` per query kind, where `QueryKey` is the content hash (FNV-1a, the same function phase 3's cache uses) of the query's arguments. Each execution records the keys it read; a changed key invalidates every entry that read it, transitively. No `unsafe`, no interior mutability tricks beyond `&mut self`, because a single-threaded database is the thing that makes Task 2's parallel front end a scoped-thread problem rather than a lock problem.
- [x] **Step 4: Run** `cargo test -p yashy-query -p yashy-driver --test e2e` → PASS, and the driver's output is byte-identical to before the change.
- [x] **Step 5: Commit** — `git commit -m "feat(query): a memoised pipeline with hash-based invalidation"`

---

### Task 2: Parallel front end

**Files:**
- Modify: `crates/yashy-driver/src/modules.rs` (split discovery from parsing)
- Modify: `crates/yashy-driver/src/pipeline.rs` (parse files on a bounded thread pool)
- Modify: `crates/yashy-driver/src/main.rs` (`--jobs` and the `--timings` row `frontend_threads`)
- Create: `crates/yashy-driver/tests/parallel_determinism.rs`

**Interfaces:**
- Produces: `parse` of independent files runs on `available_parallelism()` threads, capped by `--jobs`; the loader's depth-first walk still decides which file is discovered first.
- Produces: diagnostics are collected per file and merged in source order, so `--json` and human output are byte-identical to the serial run.
- Produces: `--timings` reports `frontend_threads=N` (1 when `--jobs 1`).

- [x] **Step 1: Write the failing tests** — a 10 k-line synthetic project reports `frontend_threads > 1`; two runs with `--jobs 1` and `--jobs 0` produce byte-identical `--json`; a file with three errors reports them in span order regardless of completion order.
- [x] **Step 2: Watch them fail.**
- [x] **Step 3: Implement** — `std::thread::scope`, a bounded pool of workers pulling files from a shared index, results collected into a `Vec` indexed by module position and merged at the end. Determinism is a property of the merge, not of the scheduler: no output is written from a worker thread.
- [x] **Step 4: Run** `cargo test -p yashy-driver --test parallel_determinism --test diagnostics_json` → PASS.
- [x] **Step 5: Commit** - `git commit -m "perf(driver): parallel parse and check with deterministic output"`

---

### Task 3: Verifier facts become IR attributes

**Files:**
- Modify: `crates/yashy-codegen/src/{module.rs,fn_gen.rs,expr_gen.rs}` (parameter and return attributes)
- Modify: `crates/yashy-codegen/src/lib.rs` (`CodegenOptions.facts: Option<&ProofFacts>`)
- Modify: `crates/yashy-driver/src/pipeline.rs` (pass phase 3's `ProofFacts` into codegen)
- Create: `crates/yashy-driver/tests/programs/attributes.ya`, `crates/yashy-codegen/tests/attributes.rs`

**Interfaces:**
- Produces: `define void @f(i8* noalias nonnull align 8 dereferenceable(24) %p)` where, and only where, the facts say so:
  | Fact | Attribute | Source |
  |---|---|---|
  | unique live reference for the call | `noalias` | ownership pass |
  | type is not `Option[...]` | `nonnull` | type checker |
  | effect is `pure` on the parameter | `readonly` | effects pass |
  | allocator alignment | `align N` | `memory.rs`/`arena.rs` constants |
  | slice length | `dereferenceable(N)` | slice type + length |
- Produces: a `FnCtx` method `attrs_for(&self, param: &HirParam) -> Vec<&'static str>` — one table, consulted everywhere, so no call site invents its own attribute.

- [x] **Step 1: Write the failing tests** — a `fn f(p: *mut i64)` that never copies `p` gets `noalias`; a function that stores `p` into a global does not; a `pure fn` parameter gets `readonly`; a non-optional pointer gets `nonnull`; an `Option[*mut i64]` parameter does not; the emitted IR parses with `llvm-as` (or `clang -x ir -c`) and the module still runs.
- [x] **Step 2: Watch them fail.**
- [x] **Step 3: Implement** — the facts are a map `FnId → ParamFacts` produced in phase 3; codegen reads it and emits attributes in the signature. If `facts` is `None` (a caller that has no verifier), no attributes are emitted — the conservative path is the default, so a missing fact can never become a wrong promise.
- [x] **Step 4: Run** `cargo test -p yashy-codegen --test attributes` and the ASan/UBSan differential corpus from phase 3 → PASS.
- [x] **Step 5: Commit** — `git commit -m "perf(codegen): emit noalias, nonnull, readonly and alignment from proofs"`

---

## Task 4: Escape analysis and promotion

Status: landed.

### What landed

- `crates/yashy-codegen/src/escape.rs`: `STACK_LIMIT` (4096 bytes), `AllocSite`
  (`of`/`start`/`end`), `ArenaId` (`of_block`), `Placement { Heap, Stack, Arena }`,
  `classify(f, facts) -> HashMap<AllocSite, Placement>`, `promotion_counts(hir,
  facts) -> (stack, arena)`, and a whole-function scope-aware walker that handles
  allocs, reads/writes, moves, calls, aggregates, lambdas/captures, control
  flow and defers. Escapes are conservative: any binding handed to a call,
  return, aggregate or capture escapes; reads/writes/dealloc through the
  pointer do not.
- `crates/yashy-codegen/src/lib.rs`: `CodegenOptions.escape` (default `false`,
  preserving library callers), wired through `emit_program`; `pub use
  escape::{promotion_counts, AllocSite, ArenaId, Placement}`.
- `crates/yashy-codegen/src/module.rs`: `Module::escape` flag, `set_escape`,
  `escape_enabled`, `facts()` accessor.
- `crates/yashy-codegen/src/fn_gen.rs`: `gen_body` classifies `f` when escape
  is enabled and populates `FnCtx::placements`.
- `crates/yashy-codegen/src/expr_gen.rs`: `FnCtx` gained `placements`,
  `promoted_slots`, `arena_handles`, `arena_scopes`; `gen_call` gained a `Span`
  parameter; helpers for placement lookup, arena handle creation and release.
- `crates/yashy-codegen/src/memory.rs`: `gen_alloc` takes a span and lowers a
  `Stack` site to `alloca [N x i64]` (one per site, hoisted to entry), an
  `Arena` site to `yashy_arena_alloc`; `gen_dealloc` skips `yashy_free` for
  promoted slots.
- `crates/yashy-codegen/src/stmt_gen.rs`: `gen_block` pushes/pops an arena
  scope and releases its arenas at block exit (after defers); `Const`/`Var`
  record promotion on binding slots via `note_promotion`; `Assign` suppresses
  the old-value free when the slot was promoted and re-notes promotion on the
  new value; `return`/`break`/`continue` release open arenas; `capture_linear_locals`
  skips nothing new (`__arena_*` names do not exist).
- `crates/yashy-driver/src/pipeline.rs`: `BuildOptions.escape` defaults `true`
  (manual `Default`); `compile_hir_timed` pushes `TimingRow::Promotion { stack,
  arena }` when escape is enabled; `pipeline` imports `promotion_counts`.
- `crates/yashy-driver/src/main.rs`: `--no-opt=escape` parsing (usage error for
  any other pass), `escape` threaded into `BuildOptions` and into the
  `--emit-ir` `CodegenOptions`, `--timings` rows `promoted_stack`/`promoted_arena`
  printed to stderr and to JSON as `promotion` (`promoted_stack`,
  `promoted_arena`), `accumulate` merges them, `report_timings` prints them.
- `crates/yashy-query/src/db.rs`: both `CodegenOptions` sites pass `escape:
  true` so the memoised pipeline keeps promotion on.
- `crates/yashy-codegen/tests/escape.rs`: 7 tests — stack promotion (no
  allocator call, no free), returned pointer stays on the heap, stored in a
  returned struct stays on the heap, loop-body dynamic allocation uses a block
  arena freed at scope exit, 512×i64 stack vs 513×i64 arena (4 KiB cap), callee
  argument stays on the heap, `--no-opt=escape` keeps heap+free.
- `crates/yashy-driver/tests/escape_flags.rs`: 3 tests — default build promotes
  and drops the free, `--no-opt=escape` keeps alloc/free, `--timings` reports
  `promoted_stack=1 promoted_arena=0`, unknown `--no-opt=` pass is a usage
  error.
- `crates/yashy-driver/tests/verify_corpus/escape_shared.ya`: a pointer shared
  with a callee and returned to its owner is heap-placed and freed exactly once.
- `crates/yashy-driver/tests/verify.rs`: added the `escape_shared` corpus entry
  (expects 0 errors).

### Design notes

- Stack promotion requires a constant element count `≤ STACK_LIMIT` (4096
  bytes) and a pointer that never escapes the function; one entry-block
  `alloca` per site, hoisted once regardless of loop iterations.
- Arena placement requires a non-constant or over-limit allocation whose
  binding was declared in the same block as the allocation; the block gets one
  arena handle, freed once at block exit after its defers run. An assignment to
  an outer binding stays on the heap.
- Promotion tracking is keyed by binding *slot* (`HashSet<String>` in
  `FnCtx`), not by name, so shadowing cannot make a later `free` skip the wrong
  allocation; `dealloc`/`yashy_free` is suppressed for promoted slots.
- `promotion_counts` re-walks functions and lambdas for timing rows, since
  `emit_program` returns only IR.
- No verifier "shared" fact exists yet, so `classify` cannot cross-check
  ownership facts; documented in `escape.rs`.

### Acceptance

- `cargo check --workspace --all-targets` → clean (pre-existing warnings only).
- `cargo test -p yashy-codegen --test escape` → 7 passed.
- `cargo test -p yashy-codegen --lib escape` → 7 passed.
- `cargo test -p yashy-codegen --test attributes` → still passes.
- `cargo test -p yashy-codegen --lib` → all suites pass.
- `cargo test -p yashy-driver --test escape_flags` → 3 passed.
- `cargo test -p yashy-driver --test verify` → all seeded corpus tests pass,
  including `escape_shared`.
- `yashy build ... --emit-ir` on a promoted program shows one
  `alloca [8 x i64]` and no `@yashy_free`; `--timings` prints
  `promoted_stack=1 promoted_arena=0`.

### Commit

- `ad6d842` `perf(codegen): promote non-escaping allocations to stack and arena`

---

### Task 5: Interning and fast paths

**Files:**
- Modify: `crates/yashy-codegen/src/{strings.rs,ir.rs,slices.rs}` (literal pool, tuple and array fast paths)
- Modify: `crates/yashy-runtime/src/runtime.c` (`yashy_str_lit`, integer printing without a temporary allocation)
- Modify: `crates/yashy-codegen/src/module.rs` (the literal pool as a module-level constant table)
- Create: `crates/yashy-driver/tests/programs/interning.ya`

**Interfaces:**
- Produces: every string literal in a module is one `@.str.N` constant plus one interned runtime string, created once at first use; `"a"` written twice is one object.
- Produces: `yashy_print_i64` writes digits directly to the output buffer (no `sprintf`, no allocation); `yashy_print_str` takes the length it already has.
- Produces: tuple and small-array construction uses `insertvalue` chains rather than a heap allocation where the value does not escape (Task 4's placement, applied to aggregates).

- [ ] **Step 1: Write the failing tests** — `interning.ya` asserts `"x" == "x"` and that two identical literals compare equal by pointer (the interned representation); a loop printing 1 000 000 integers allocates zero times (asserted with a counting allocator); `hello`'s binary size and allocation count are recorded before and after.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — the literal pool is a `BTreeMap<String, String>` (name by content, so output is deterministic) filled before bodies are emitted; the runtime's printing path is rewritten to write into the caller's buffer, which is the same buffer `println` already uses.
- [ ] **Step 4: Run** `cargo test -p yashy-driver --test e2e interning` and `cargo test -p yashy-runtime` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "perf: intern literals and print integers without allocating"`

---

### Task 6: Inlined runtime calls

**Files:**
- Modify: `crates/yashy-runtime/include/yashy_runtime.h` (`static inline` fast paths, `always_inline`)
- Modify: `crates/yashy-runtime/src/runtime.c` (the slow paths stay out of line)
- Modify: `crates/yashy-codegen/src/{slices.rs,maps.rs}` (call the fast path where the check is already proven)
- Modify: `crates/yashy-bench/src/lib.rs` (rows `wordcount_run_ms`, `maps_run_ms`)

**Interfaces:**
- Produces: `yashy_slice_get_unchecked`, `yashy_map_get_fast`, `yashy_alloc_small` as `static inline` in the header, with the checked/allocating versions kept in `runtime.c` and called on the slow path.
- Produces: the codegen calls the unchecked form **only** when `ProofFacts` says the index is in bounds — the same fact that already deletes the check in phase 3, reused rather than re-derived.
- Produces: a comment in the header naming which proof licenses each unchecked entry point, so a reader cannot mistake it for a general API.

- [ ] **Step 1: Write the failing tests** — a slice index proven in bounds emits no `yashy_slice_get` call in the IR (grep the IR in the test); an unproven index still calls the checked version; `wordcount` and `maps` rows improve; the runtime's own tests still pass with the inline paths.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — move the hot half of each function into the header as `static inline`, keep the cold half in the `.c`, and mark the fast path `__attribute__((always_inline))` (guarded for MSVC with `__forceinline`). The C shim is compiled into every program already, so this costs nothing at link time.
- [ ] **Step 4: Run** `cargo test -p yashy-runtime` and the bench rows on both OSes → PASS.
- [ ] **Step 5: Commit** — `git commit -m "perf(runtime): inline the hot paths, keep the checks that are unproven"`

---

### Task 7: ThinLTO and PGO

**Files:**
- Modify: `crates/yashy-driver/src/pipeline.rs` (release link flags)
- Create: `benchmarks/pgo/` (a training corpus: `hello`, `fib`, `wordcount`, `maps`, `json`, `strings`)
- Create: `scripts/pgo.sh`, `scripts/pgo.ps1`
- Modify: `crates/yashy-bench/src/lib.rs` (rows `release_run_ms`)
- Modify: `docs/cli.md` (`--pgo`, `--no-lto`)

**Interfaces:**
- Produces: `--release` links with `-flto=thin`; `--pgo` builds with `-fprofile-generate`, runs the training corpus, then rebuilds with `-fprofile-use=<file>`; the profile file lives under `.yashy/pgo/` and is never written outside it.
- Produces: `--no-lto` for bisecting a suspected LTO-only miscompile; the flag exists so the bug can be attributed rather than worked around.
- Produces: the training corpus is checked in, so a PGO build is reproducible from a clone.

- [ ] **Step 1: Write the failing tests** — a release build reports `lto=thin` in `--timings`; a `--pgo` build on the corpus produces a profile file and a second build uses it; the resulting binary is ≤ 10% larger than the ThinLTO build and ≥ 5% faster on at least two feature rows; both OSes produce and consume their own profile format.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — the driver already shells out to `clang`; this is flags plus a two-pass orchestration in one function. Profile data is per-OS by construction (`%p.profraw` on Windows, `default.profraw` elsewhere), so the script does not normalise it — it names it per platform and the cache key includes it.
- [ ] **Step 4: Run** the bench on both OSes, medians of 5 → PASS.
- [ ] **Step 5: Commit** — `git commit -m "perf(build): ThinLTO by default, PGO from a checked-in corpus"`

---

### Task 8: The pass manager and the per-pass gate

**Files:**
- Create: `crates/yashy-codegen/src/pass.rs`
- Modify: `crates/yashy-codegen/src/lib.rs` (`emit_program` runs the pipeline)
- Modify: `crates/yashy-driver/src/main.rs` (`--no-opt=<pass>`, `--opt-list`)
- Modify: `crates/yashy-bench/src/lib.rs` (`--per-pass` mode: run the bench with each pass disabled and print a delta table)
- Create: `crates/yashy-codegen/tests/pass_order.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct Pass { pub name: &'static str, pub run: fn(&mut PassCtx) -> PassStats, pub row: &'static str }
  pub fn pipeline() -> &'static [Pass];   // the one order
  ```
- Produces: `--opt-list` prints the passes in order with the bench row each is held to; `--no-opt=escape` (Task 4) is one instance of the general form.
- Produces: `yashy-bench --per-pass` prints `pass | row | baseline | current | delta | verdict` and fails if any pass regresses its own row by more than 2%.

- [ ] **Step 1: Write the failing tests** — the pipeline is deterministic (two runs produce equal IR); disabling a pass changes only the IR that pass produces; an unknown `--no-opt` name is a usage error listing the valid names; `--per-pass` output has one row per pass per bench row.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — the pass manager is a `Vec<Pass>` and a loop; the cost model is the bench table, not a heuristic inside the compiler. Later phases register their lowering here: phase 5's `arena` block, phase 8's monomorphised trait bodies, phase 10's async state-machine lowering. A construct that emits IR without registering a pass is caught by the test that every `HirExpr` variant has an owner.
- [ ] **Step 4: Run** `cargo test -p yashy-codegen --test pass_order` and `yashy-bench --per-pass` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(codegen): a pass manager with a per-pass regression gate"`

---

### Task 9: `yashy build --opt-report`

**Files:**
- Create: `crates/yashy-driver/src/opt_report.rs`
- Modify: `crates/yashy-codegen/src/{slices.rs,memory.rs,module.rs}` (record a reason at every decision point)
- Modify: `crates/yashy-driver/src/main.rs` (`build --opt-report[=json]`), `docs/cli.md`
- Create: `crates/yashy-driver/tests/opt_report.rs`

**Interfaces:**
- Produces: human form, grouped by function:
  ```text
  src/main.ya:12  fn wordcount
    bounds check kept      xs[i]        no proof that i < xs.len()
                                       add `requires i < xs.len()` or iterate with `for x in xs`
    heap allocation        line 18      escapes into the returned Map
    call not inlined       line 21      yashy_map_get (runtime, cold path)
  ```
- Produces: `--opt-report=json` with `{"schema_version":1,"tool":"yashy","functions":[{"name","file","span","items":[{"kind","span","reason","fix"}]}]}` — same schema discipline as every other command, so an agent can act on it.
- Produces: every entry has a `kind` (`bounds`, `overflow`, `alloc`, `not-inlined`, `promoted`, `not-promoted`), a `reason`, and a `fix` that is `null` when no fix exists.

- [ ] **Step 1: Write the failing tests** — a proven-in-bounds index produces no `bounds` entry; an unproven one produces exactly one, with the `requires` fix named; a promoted allocation reports `promoted` with its placement; `--opt-report=json` is stable across runs; the report is empty for a program with nothing to say (and says so in one line, not nothing).
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — the decision points already exist (phase 3 deletes checks; Task 4 classifies placements); this task adds a `Vec<OptNote>` to the emission context and renders it. No analysis is duplicated to produce a report — a report that runs its own analysis can disagree with the compiler, which is worse than no report.
- [ ] **Step 4: Run** `cargo test -p yashy-driver --test opt_report` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(driver): --opt-report explains every check and allocation that survived"`

---

### Task 10: Docs and gate

**Files:** `docs/optimization.md` (new: what Yashy proves, what LLVM decides, the pass list, how to read `--opt-report`), `docs/cli.md` (`--opt-report`, `--no-opt`, `--opt-list`, `--pgo`, `--no-lto`), `AGENTS.md` (layout: `yashy-query`; the rule that optimization is a phase, not a habit), `llms.txt`, `benchmarks/README.md`.

- [ ] **Step 1: Write the doc** with a worked example: a slow function, its `--opt-report`, the contract added, and the report afterwards. The tutorial harness compiles every block.
- [ ] **Step 2: Run the gate table on both OSes, medians of 5.**
- [ ] **Step 3: Commit and record** — `git commit -m "docs: the optimization program, and close phase 4"`; update the index status and the ledger.

---

## Gate summary

| Gate | Command | Passes when |
|---|---|---|
| Incremental analysis | `cargo test -p yashy-query --test incremental` + `--timings` | unchanged project: `query_misses == 0`; one-function edit re-analyses that function and its dependents only |
| Parallel and identical | `cargo test -p yashy-driver --test parallel_determinism` | `frontend_threads > 1`; `--json` byte-identical to `--jobs 1` |
| Attributes only where proven | `cargo test -p yashy-codegen --test attributes` | every attribute traced to a fact; no facts ⇒ no attributes |
| Escape promotion sound | `cargo test -p yashy-codegen --test escape` + ASan/UBSan corpus | no promoted value is reachable after its scope; corpus clean |
| Allocation and size | `yashy-bench` rows | `wordcount` allocations −40%; `hello` binary within the phase 3 budget |
| Runtime | `yashy-bench` on both OSes | `fib` ≤ 1.1× C; feature rows ≤ 1.8×; release ≥ 5% faster with PGO |
| Per-pass gate | `yashy-bench --per-pass` | no pass regresses its own row by more than 2% |
| Explained | `cargo test -p yashy-driver --test opt_report` | every surviving check and allocation has a reason |
| No regression | `cargo test --workspace` | green on Windows and Linux |
