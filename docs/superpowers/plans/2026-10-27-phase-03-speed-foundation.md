# Phase 3 — Speed Foundation: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the edit→test loop fast enough that it stops taxing every later feature: a content-addressed cache shared across projects, per-function IR reuse, `-O0` for the dev loop, parallel tests, a fast linker, and verifier-driven removal of bounds and overflow checks.

**Architecture:** A new `yashy-cache` crate owns the global store and its invalidation key. Codegen splits into per-function emission so a function's IR can be reused when its HIR hash and options are unchanged. The verifier's `ProofObligation` list becomes `ProofFacts`, which codegen consults to delete a runtime check it can prove unnecessary. The phase closes with a two-week async spike that ships a design document, not code.

**Tech Stack:** `std::fs`, `std::hash`, the existing `fnv1a` and `runtime_cache_key` in `crates/yashy-driver/src/pipeline.rs` (a runtime-object cache already exists and is the model to follow), `lld`/`mold` when present, and `crates/yashy-bench`.

**Spec:** `docs/superpowers/specs/2026-10-27-yashy-master-design.md` §5, §8 row 3. Index: `docs/superpowers/plans/2026-10-27-yashy-master-plan.md`. Predecessor: `2026-10-27-phase-02-effects-typestate.md`.

## Global Constraints

See the master index. The ones that bite here:

- No new runtime dependencies; the cache is `std`-only.
- No speed claim without a number from Windows **and** Linux, medians of 5.
- The verifier never runs less than it does today; speed may not buy itself by skipping proofs.
- Cache staleness must be impossible to hide: `yashy cache verify` recomputes a key.
- Both operating systems or it is not done.

## Review Focus

1. **A stale cache produces a wrong binary.** The key must cover every module's text, every flag, the compiler version, and the target triple. If any of those is missing, the cache is a correctness bug, not a speed feature.
2. **`-O0` for `run` must not change observable behaviour** except `debug_assert` semantics, which the language already ties to release mode.
3. **Parallel tests must not interleave stdout into a false pass.** Each test's output is captured separately.
4. **Eliding a bounds check must be driven by a proof, never by a heuristic.** If `ProofFacts` has no entry for a span, the check stays.
5. **A missing linker falls back, never fails.** `mold` absent means `lld`, absent means the system linker.

---

### Task 1: The `yashy-cache` crate and the global store

**Files:**
- Create: `crates/yashy-cache/{Cargo.toml,src/lib.rs,src/key.rs,src/store.rs}`
- Modify: `Cargo.toml` (member), `crates/yashy-driver/Cargo.toml`
- Modify: `crates/yashy-driver/src/main.rs` (`cache clean|verify|stats`)
- Modify: `docs/cli.md`

**Interfaces:**
- Produces:
  ```rust
  pub struct CacheKey([u8; 32]);
  pub fn program_key(sources: &[(String, String)], flags: &str, version: &str, triple: &str) -> CacheKey;
  pub struct Store { root: PathBuf }
  impl Store {
      pub fn open() -> std::io::Result<Store>;      // global, per-user
      pub fn get(&self, key: &CacheKey) -> Option<PathBuf>;
      pub fn put(&self, key: &CacheKey, bytes: &[u8]) -> std::io::Result<PathBuf>;
      pub fn clean(&self) -> std::io::Result<u64>;
      pub fn verify(&self, key: &CacheKey, sources: &[(String, String)], flags: &str) -> bool;
      pub fn stats(&self) -> Stats;                  // entries, bytes, oldest
  }
  ```
- Store root: `%LOCALAPPDATA%\yashy\cache` on Windows, `$XDG_CACHE_HOME/yashy` (fallback `~/.cache/yashy`) elsewhere. Per-project lockfile at `.yashy/cache.lock` records the keys this project produced.

- [x] **Step 1: Write the failing tests** — `program_key` changes when any module's text changes, when a flag changes, when the version changes, and when the triple changes; `put` then `get` round-trips; `clean` removes everything and reports the bytes freed; a corrupted entry makes `get` return `None` rather than bytes.
- [x] **Step 2: Watch them fail.**
- [x] **Step 3: Implement** — FNV-1a over a canonical serialisation (`path \0 len \0 bytes \0` per module, then flags, version, triple), matching the existing `runtime_cache_key` style. Entries are stored as `<key>/ir.ll` and `<key>/meta.json`; `get` validates the meta's own hash before returning.
- [x] **Step 4: Run** `cargo test -p yashy-cache` → PASS.
- [x] **Step 5: Commit** — `git commit -m "feat(cache): add a global content-addressed store with a verifiable key"`

---

### Task 2: Per-function IR reuse

**Files:**
- Modify: `crates/yashy-codegen/src/lib.rs` (`emit_program` assembles from parts), `src/fn_gen.rs`
- Modify: `crates/yashy-driver/src/pipeline.rs` (`compile_hir` uses the cache)

**Interfaces:**
- Produces: `pub fn emit_function(f: &HirFn, options: CodegenOptions) -> String` and `pub fn emit_prelude(items: &[HirItem], options: CodegenOptions) -> String`.
- Produces: `emit_program` keeps its signature and becomes the assembler; the cache stores `emit_function` output per function key.

- [x] **Step 1: Write the failing tests** — emitting a program twice produces byte-identical IR; changing one function's body changes only that function's IR string and leaves the others equal.
- [x] **Step 2: Watch them fail.**
- [x] **Step 3: Implement** — a function key hashes the function's HIR `Debug` form (stable within a compiler version, which the key includes), `CodegenOptions`, and the triple. Lambdas are named by their enclosing function and index so their IR is reusable too.
- [x] **Step 4: Measure** — `yashy check --timings` and `yashy build --timings` on `benchmarks/large.ya`; record emit time before and after. Gate: unchanged-function emit is not on the critical path.
- [x] **Step 5: Commit** — `git commit -m "perf(codegen): reuse per-function IR from the cache"`

---

### Task 3: Dev-loop optimization defaults

**Files:**
- Modify: `crates/yashy-driver/src/pipeline.rs` (`resolve_opt`)
- Modify: `crates/yashy-driver/tests/run.rs`

**Interfaces:**
- Produces: `run` and `test` default to `-O0`; `build` to `-O2`; `--release` to `-O3`; an explicit `-O` always wins.

- [x] **Step 1: Write the failing test** — `yashy run` passes `-O0` to `clang` (assert via `--emit-ir` plus a dry-run flag or by timing), and `yashy build` passes `-O2`; `-O3` explicit wins over both.
- [x] **Step 2: Watch it fail.**
- [x] **Step 3: Implement** — `resolve_opt` currently takes `(release, explicit)`; it needs the command's default: `resolve_opt(command: CommandKind, release: bool, explicit: Option<OptLevel>)`.
- [x] **Step 4: Run** `cargo test -p yashy-driver --test run --test opt` → PASS.
- [x] **Step 5: Commit** — `git commit -m "perf(driver): -O0 for the dev loop, -O2 for build, -O3 for release"`

---

### Task 4: Parallel test workers

**Files:**
- Modify: `crates/yashy-driver/src/main.rs` (`test`, `test_json`)
- Modify: `crates/yashy-driver/tests/test_runner.rs`

**Interfaces:**
- Produces: `yashy test --jobs N` (default: `std::thread::available_parallelism()`), output captured per test and printed in discovery order.

- [x] **Step 1: Write the failing tests** — 100 trivial tests run in under 2 s on a machine with ≥ 4 cores; a test that prints and fails still reports its captured output; `--jobs 1` reproduces today's order exactly.
- [x] **Step 2: Watch them fail.**
- [x] **Step 3: Implement** — a work queue over `std::thread::scope`; each worker compiles and runs one test; results are collected into a `Vec<Option<Outcome>>` by index so ordering is deterministic.
- [x] **Step 4: Run** → PASS; record the 100-test time in this plan.
- [x] **Step 5: Commit** — `git commit -m "perf(test): run tests in parallel with captured output"`

---

### Task 5: Fast-linker detection

**Files:**
- Modify: `crates/yashy-driver/src/toolchain.rs`
- Modify: `crates/yashy-driver/src/pipeline.rs` (link invocation)

**Interfaces:**
- Produces: `pub enum Linker { Mold, Lld, System }` and `Toolchain::linker: Linker`, chosen once per run by probing `-fuse-ld=`.

- [x] **Step 1: Write the failing test** — with a fake `clang` that rejects `-fuse-ld=mold` and accepts `-fuse-ld=lld`, detection returns `Lld`; with both rejected, `System`.
- [x] **Step 2: Watch it fail.**
- [x] **Step 3: Implement** — probe by compiling a one-line program with each flag into a temp dir; cache the answer in the store keyed by `clang --version` output.
- [x] **Step 4: Run** → PASS; record link time before/after on Linux.
- [x] **Step 5: Commit** — `git commit -m "perf(driver): detect mold and lld for linking"`

---

### Task 6: Compiler rebuild speed

**Files:**
- Create: `.cargo/config.toml` (workspace-local, linker selection)
- Modify: `Cargo.toml` (`[profile.dev]` settings)
- Modify: `.github/workflows/ci.yml` (cache the Rust build)

**Interfaces:**
- Produces: no public API; the gate is wall-clock.

- [x] **Step 1: Measure the baseline** — touch `crates/yashy-sema/src/check_expr.rs` and time `cargo build -p yashy-driver` five times; record the median.
- [x] **Step 2: Apply** — `lld`/`mold` for the workspace link, `debug = 1` for dependencies in `[profile.dev]`, `split-debuginfo = "unpacked"` on Windows, and `sccache` in CI when the runner provides it.
- [x] **Step 3: Measure again** — gate: rebuild after touching `sema` < 60 s, both OSes.
- [x] **Step 4: Commit** — `git commit -m "perf(build): cut the compiler rebuild loop"`

---

### Task 7: Verifier-driven bounds and overflow elision

**Files:**
- Modify: `crates/yashy-verify/src/lib.rs` (`VerifyReport::facts()` → `ProofFacts`)
- Modify: `crates/yashy-codegen/src/lib.rs` (`emit_program` takes `&ProofFacts`), `src/slices.rs`, `src/fixed.rs`, `src/expr_gen.rs`
- Modify: `crates/yashy-driver/src/pipeline.rs` (thread the facts through)

**Interfaces:**
- Produces:
  ```rust
  pub struct ProofFacts { proven: BTreeSet<Span> }   // spans with a discharged obligation
  impl ProofFacts { pub fn is_proven(&self, span: Span) -> bool; }
  ```
- Produces: `emit_program(hir: &HirProgram, entry: &str, options: CodegenOptions, facts: &ProofFacts) -> String`.

- [x] **Step 1: Write the failing tests** — a loop whose index is proven in bounds emits IR with no bounds-check call; the same loop with an unproven index still emits it; runtime output is identical either way.
- [x] **Step 2: Watch them fail.**
- [x] **Step 3: Implement** — `slices.rs` and `fixed.rs` consult `facts.is_proven(index_span)` before emitting the check. `expr_gen.rs` does the same for `HirExpr::Binary`'s overflow call. Default `ProofFacts::empty()` keeps every check, so an unverified build is unchanged.
- [x] **Step 4: Measure** — `benchmarks/fib.ya` and `benchmarks/loop-wrap.ya` runtime before/after; gate: no regression, and the bounds-check count in the IR drops for the proven corpus.
- [x] **Step 5: Commit** — `git commit -m "perf(codegen): delete a runtime check when the verifier proved it"`

---

### Task 8: The bench gate grows a dev-loop row and per-phase budgets

**Files:**
- Modify: `crates/yashy-bench/src/lib.rs` (rows), `crates/yashy-bench/src/main.rs`
- Modify: `benchmarks/README.md`, `benchmarks/baselines/*.bench`
- Modify: `.github/workflows/bench.yml`

**Interfaces:**
- Produces: rows `incremental_compile_ms`, `test_100_ms`, `hello_compile_ms`, `large_compile_ms`, `fib_run_ms`, `hello_binary_bytes`; each with a per-phase budget in the baseline file (`budget_incremental_compile_ms=200`, …).

- [x] **Step 1: Write the failing test** — a baseline missing a budget key fails the gate with a message naming the key.
- [x] **Step 2: Implement** — the report compares each row against its budget and its baseline; PR runs are report-only, scheduled runs are hard.
- [x] **Step 3: Run** — record the new baseline on both OSes in `benchmarks/baselines/`.
- [x] **Step 4: Commit** — `git commit -m "test(bench): gate the dev loop and per-phase budgets"`

---

### Task 9: The async spike (2 weeks, design only)

**Files:**
- Create: `docs/superpowers/design/2026-11-async-spike.md`

**Interfaces:**
- Produces: a design document with four decisions: state-machine lowering shape, executor model (single-threaded `block_on` plus a thread pool for `spawn`), cancellation representation, and how the verifier treats an `await` point (a yield is not an `io` effect, but a suspended borrow is a typestate question).

- [x] **Step 1: Write the document** — no shipping code. It answers, with the current code named: where the state machine is built (codegen `fn_gen.rs`), how a local that lives across an `await` is stored (an explicit frame struct, not the LLVM stack), and which of the eight async features are cut from v1.
- [x] **Step 2: Review it against phase 10's cap** — one executor, no async traits, no generic runtime. A design that needs more is out of scope.
- [x] **Step 3: Commit** — `git commit -m "docs: async spike — lowering, executor and verifier interaction"`

---

### Task 10: Gate

- [x] **Step 1: Run the gate table on both OSes.**
- [x] **Step 2: Record the numbers** in `benchmarks/baselines/{linux,windows}.bench` and in this plan.
- [x] **Step 3: Commit and record** — `git commit -m "test: close phase 3 with the speed gate green"`; update the index and the ledger.

---

## Gate summary

| Gate | Command | Passes when |
|---|---|---|
| Incremental rebuild | `yashy build` twice on a 1000-line program | second run < 200 ms |
| 100 tests | `yashy test` on a 100-test project | < 2 s |
| Hello | `yashy build benchmarks/hello.ya` | < 1 s, binary < 500 KB |
| Runtime | `yashy-bench` | scalar ≤ 1.2× C, feature rows ≤ 2× |
| Compiler rebuild | touch `sema`, `cargo build -p yashy-driver` | < 60 s median |
| Cache correctness | `cargo test -p yashy-cache` + `yashy cache verify` | key changes on every input that matters |
| No check lost without a proof | `cargo test -p yashy-codegen` | unproven spans keep their checks |
| Async spike | the design doc exists and passes its own cap review | four decisions answered |
