# Phase 1 — Instant Feedback

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the edit → check → run → test loop fast enough that a person and an agent stop thinking about it.

**Outcome:** A one-harness test runner, an optional build daemon, `yashyc watch`, an incremental cache that links only what changed, a parallel front end and codegen path, an honest compile-time regression row, and parser error recovery for the LSP.

**Time:** ~8 weeks.
**Depends on:** 1.0 complete (`v1.0.0`, `c2abc55`).
**Spec:** [`../specs/2026-11-15-yashy-2.0-design.md`](../specs/2026-11-15-yashy-2.0-design.md) §2.1, §6.4, §6.6, §7.

## Why this phase

1.0's own gate says 100 tests must run in under 2 seconds. The measured number
is **1235 seconds** for 101 tests: about 12 seconds each, serially, with ~2.5
seconds attributed to linking and the rest to per-process host cost. That is not
a test runner; it is a test queue. Every later phase in this plan — libraries,
concurrency, performance, AI — is developed on this loop, so fixing it first is
the cheapest work in the programme.

The same phase closes the two process gaps 1.0 carried: compile-time regression
detection is gated at 100% because 20% is inside the runner's noise, and
`--per-pass` has never run end to end because sequential measurements move by
~25%. Both are measurement problems, and both are cheaper to fix before the
compiler grows more phases to measure.

## Starting point

| Fact | 1.0 value |
|---|---|
| 101 tests | 1235 s; serial; ~2.5 s link per test |
| Full build | 259 ms; one `clang` invocation |
| Front end | 22 ms |
| Incremental analysis | < 200 ms (phase 4 gate) |
| LSP | diagnostics, hover, definition, completion, semantic tokens; no error recovery |
| Daemon | none; every command is a fresh process |
| Watch | none |
| Compile-time regression | gated at 100% because 20% is noise |
| T2 | doc block attachment is CRLF-sensitive |

## Deliverables

### Task 1: Test harness v2

**Files:**
- Create: `crates/yashy-driver/src/test_harness.rs`
- Modify: `crates/yashy-driver/src/main.rs` (the `test` command)
- Modify: `crates/yashy-codegen/src/fn_gen.rs` (unique test symbol names)
- Test: `crates/yashy-driver/tests/test_runner_v2.rs`

**Interfaces:**
- `TestBuild { harness: PathBuf, tests: Vec<TestEntry> }`
- `TestEntry { name: String, source: PathBuf, symbol: String }`
- `run_harness(harness: &Path, entries: &[TestEntry], jobs: usize) -> TestReport`

- [x] **Step 1: Write the failing test.** Build a project with three tests:
  one passes, one exits 3, one prints and passes. Assert the report names all
  three, the failing one is `Failed { exit: 3 }`, and stdout is attributed to
  the test that wrote it.
- [x] **Step 2: Run it and watch it fail** with "one binary per test" or a
  missing `TestReport`.
- [x] **Step 3: Compile every test into one harness.** For a test build, rename
  each test's `main` to `__yashy_test_<n>` at the HIR boundary, generate a
  harness `main` that prints `TEST <name>` before calling each symbol, and link
  once. A non-zero exit or a crash reruns the tests one at a time in fresh
  processes to attribute the failure; the normal path stays one process.
- [x] **Step 4: Preserve the 1.0 semantics.** A test is a Yashy program with
  `main`; exit 0 is a pass. Keep `--filter`, `--json`, `--timings`, `--list`,
  `--sandbox`, `--target`, `--jobs`. `compile_error` and `failed` stay
  distinct.
- [x] **Step 5: Commit.** `git commit -m "feat(test): run a project's tests in one harness"`.

### Task 2: The daemon

**Files:**
- Create: `crates/yashy-driver/src/daemon.rs`
- Create: `crates/yashy-driver/src/daemon_protocol.rs`
- Modify: `crates/yashy-driver/src/main.rs` (`serve`)
- Test: `crates/yashy-driver/tests/daemon.rs`

**Interfaces:**
- `Daemon::start(root: &Path) -> DaemonHandle`
- `Request::{Check, Verify, Build, Test, Diagnostics, Shutdown}`
- `Response::{Diagnostics(Vec<Diagnostic>), Built(PathBuf), Error(String)}`
- JSON-RPC over a local socket; the same protocol the LSP can reuse.

- [x] **Step 1: Write the failing test.** Start a daemon on a fixture, send
  `Check`, assert diagnostics; edit the file, send `Check` again, assert the
  new diagnostics and `cache_hits > 0`.
- [x] **Step 2: Implement the daemon as a cache, not a second compiler.** It
  owns a `SourceMap`, the query `Db`, the content-addressed cache and the
  toolchain. Every request routes to the same driver functions as the one-shot
  path. `shutdown` closes cleanly; a daemon crash never loses source.
- [x] **Step 3: Make it optional.** Every command works one-shot. `serve` is
  opt-in; `watch` and the LSP may use it. A stale daemon is detected by
  protocol version and compiler version and is replaced.
- [x] **Step 4: Commit.**

### Task 3: `yashyc watch`

**Files:**
- Create: `crates/yashy-driver/src/watch.rs`
- Modify: `crates/yashy-driver/src/main.rs` (`watch`)
- Test: `crates/yashy-driver/tests/watch.rs`

- [ ] **Step 1: Write the failing test.** Watch a fixture, touch a file, assert
  a diagnostics event arrives in under 100 ms on the test host; then fix the
  file and assert a success event.
- [ ] **Step 2: Implement the loop.** Watch the project root (`yashy.toml`) or
  the named file; debounce 50 ms; send `Check`; print human diagnostics or
  `--json` events; `--run` builds and runs on success, `--test` runs the test
  suite. A failed build keeps the last good binary running (server mode) and
  reports the failure.
- [ ] **Step 3: Commit.**

### Task 4: Incremental cache v2

**Files:**
- Modify: `crates/yashy-cache/src/lib.rs` (key v2)
- Modify: `crates/yashy-driver/src/pipeline.rs` (per-module object cache)
- Modify: `crates/yashy-query/src/db.rs` (module-level queries)
- Test: `crates/yashy-driver/tests/incremental.rs`

- [ ] **Step 1: Write the failing test.** Build a 10-module fixture; change one
  function; assert only that module's object is rebuilt and the link reuses the
  rest. Build with no change; assert `cache_hits == modules` and no `clang`
  invocation for IR.
- [ ] **Step 2: Extend the key.** Source bytes, flags, compiler version, target,
  runtime hash, package versions, features and the relevant environment. A
  `yashyc cache verify` recomputes and reports a mismatch.
- [ ] **Step 3: Cache per-module objects.** Emit one object per module where
  the language allows it; link with the cached runtime object and the fast
  linker if present (`lld`, `mold`, `ld64.lld`, fallback to the default).
  A no-change build returns the cached binary in < 50 ms.
- [ ] **Step 4: Never serve a stale binary.** A stale hit is a P0. The test
  mutates every key input in turn and asserts a miss.
- [ ] **Step 5: Commit.**

### Task 5: Parallel front end and codegen

**Files:**
- Modify: `crates/yashy-query/src/db.rs` (shard per file)
- Modify: `crates/yashy-driver/src/pipeline.rs` (`--jobs`)
- Test: `crates/yashy-driver/tests/parallel_determinism.rs` (extend)

- [ ] **Step 1: Write the failing test.** Check a 20-module fixture with
  `--jobs 1` and `--jobs 4`; assert byte-identical JSON and `frontend_threads
  > 1` in the latter.
- [ ] **Step 2: Shard the query database per file.** Parse and check in
  parallel; codegen per module in parallel; merge in source order so diagnostics
  and IR are deterministic.
- [ ] **Step 3: Commit.**

### Task 6: An honest compile-time regression row

**Files:**
- Modify: `benchmarks/ci.sh`, `benchmarks/ci.ps1`
- Modify: `benchmarks/thresholds.bench`
- Test: `crates/yashy-bench/src/lib.rs` (comparator tests)

- [ ] **Step 1: Write the failing test.** Synthetic measurement files: a 30%
  regression fails, a 15% regression passes, a noisy pair with the same median
  passes.
- [ ] **Step 2: Normalize.** Measure a same-run reference (clang on the same
  program) and report the ratio; interleave A/B measurements so machine drift
  hits both. Gate compile rows at 20% on the ratio, not the raw millisecond.
- [ ] **Step 3: Commit.**

### Task 7: Parser error recovery

**Files:**
- Modify: `crates/yashy-parser/src/lib.rs`, `crates/yashy-parser/src/items.rs`,
  `crates/yashy-parser/src/exprs.rs`, `crates/yashy-parser/src/stmts.rs`
- Modify: `crates/yashy-lsp/src/server.rs`
- Test: `crates/yashy-parser/tests/recovery.rs`, `crates/yashy-lsp/tests/recovery.rs`

- [ ] **Step 1: Write the failing test.** Half-typed inputs: `fn`, `fn main(`,
  an unclosed string, a missing `}`. Assert at least one diagnostic, a stable
  span, and that the LSP still answers `hover`/`completion`.
- [ ] **Step 2: Recover at item and statement boundaries.** Skip to the next
  `;`, `}`, or item keyword; never loop without consuming a token; cap
  diagnostics per file. A recovered parse is never used for `build`; it is for
  the LSP only.
- [ ] **Step 3: Commit.**

### Task 8: T2, while the parser is open

**Files:**
- Modify: `crates/yashy-lexer/src/lexer.rs` (`skip_line_comment`)
- Test: `crates/yashy-parser/tests/doc_comments.rs`

- [ ] **Step 1: Write the failing test.** The same source with LF and CRLF:
  one doc block, one `yashyc doc` byte stream.
- [ ] **Step 2: Stop comment scanning at `\r\n` as well as `\n`.** Run the
  existing doc tests; update `docs/triage.md` to close T2.
- [ ] **Step 3: Commit.**

## Gates

| Gate | Command | Passes when |
|---|---|---|
| 100 tests | `cargo test -p yashy-driver --test test_runner_v2` | 100 tests in the fixture finish in < 2 s on Windows and Linux; the 1.0 corpus produces the same results |
| Incremental | `cargo test -p yashy-driver --test incremental` | no-change build < 50 ms; one-function edit check < 100 ms; one-function edit build < 250 ms |
| Edit-to-diagnostic | `yashyc watch` on a 10k-line fixture | < 50 ms from write to diagnostics on the test host |
| Parallel and identical | `cargo test -p yashy-driver --test parallel_determinism` | `--jobs 4` byte-identical to `--jobs 1`; `frontend_threads > 1` |
| Compile-time row | `benchmarks/ci.*` | a 30% regression fails; a 15% regression or a noisy pair passes |
| LSP recovery | `cargo test -p yashy-parser --test recovery -p yashy-lsp --test recovery` | every incomplete input gets a diagnostic and an answer |
| T2 closed | `cargo test -p yashy-parser --test doc_comments` | LF and CRLF produce one doc block and one byte stream |
| No regression | `cargo test --workspace` | green on Windows and Linux |
| Crash corpus | `cargo test -p yashy-driver --test never_panics --test sandbox_escape` | green |
| No P0 | `docs/triage.md` | no P0; T3 and T2 closed |

## Demo

```console
$ yashyc watch examples/hello
watching examples/hello
check main.ya .......... ok  (18 ms)
build ................... ok  (91 ms)
run ..................... hello, world
```

```console
$ yashyc test examples/hello --timings
100 tests ................ ok  (1.42 s)
  harness ................ 0.31 s
  run .................... 1.11 s
```

## Out of scope

- Full LSP feature set (Phase 2).
- Incremental linking across packages (Phase 3/11).
- Distributed cache (Phase 11B).
- Hot reload of a running server; Phase 1 does watch + auto-run, not state
  migration.
- A sampling profiler (Phase 2).

## Risks

| Risk | Mitigation |
|---|---|
| One harness hides a failing test | Failure path reruns per test in a fresh process; exit codes and stdout attribution are tested |
| Daemon becomes a required service | One-shot remains the contract; daemon is optional and version-checked |
| Per-module objects change semantics | The whole-program path remains the reference; the two produce byte-identical output on the corpus |
| Cache key misses an input | Every key input has a mutation test; `cache verify` recomputes; CI never trusts a foreign cache |
| Error recovery accepts invalid code | Recovered parses are LSP-only; `build` still requires a clean parse |
