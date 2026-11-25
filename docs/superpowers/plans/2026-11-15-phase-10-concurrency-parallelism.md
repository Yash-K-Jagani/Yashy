# Phase 10 — Concurrency and Parallelism

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make concurrency a complete, provable part of the language: atomics
with explicit orderings, locks and condition variables, scoped threads, MPMC
channels, `select`, cancellation, parallel iterators, race proofs, a
deterministic scheduler that makes a concurrent test reproducible, and
record/replay that makes a failing concurrent run debuggable.

**Outcome:** `Atomic*`, `RwLock`, `Condvar`, `Once`, `Barrier`, scoped threads,
an MPMC channel with `select` and timeouts, cancellation tokens, parallel
iterators, verifier rules for races/lock order/lock-across-await, a TSan
differential, a concurrency corpus, `--deterministic` execution that produces
the same trace at 1 and 4 workers, and `--record`/`replay` with backward
stepping through a recorded trace.

**Time:** ~9 weeks.
**Depends on:** Phase 9 gate.
**Spec:** [`../specs/2026-11-15-yashy-2.0-design.md`](../specs/2026-11-15-yashy-2.0-design.md) §6.1, §6.6, §6.8.

## Why this phase

1.0 has OS threads, a `Mutex` and a `Channel`. That is enough to write a race and
not enough to prove one absent: no atomics, no `RwLock`, no `Condvar`, no
`Once`, no scoped threads, no `select`, no cancellation, no parallel iterators,
no race corpus. The verifier's typestate and ownership facts are the reason a
Yashy program can make a stronger claim than "it usually works".

The phase is deliberately before performance: the parallel runtime changes what
the optimizer may assume, and the deterministic scheduler is a prerequisite for
the AI phase's reproducible agent runs.

## Starting point

| Fact | 1.0 value |
|---|---|
| Threads | `spawn`, join; no scoped threads |
| Mutual exclusion | `Mutex`; no `RwLock`, `Condvar`, `Once`, `Barrier` |
| Atomics | none; `Mutex` and channels only |
| Channels | one MPSC/MPMC shape; no `select` |
| Cancellation | none |
| Parallel iterators | none |
| Race proof | lock across `.await` (`E1702`); no race corpus, no TSan differential |
| Determinism | none |

## Deliverables

### Task 1: Atomics and memory orderings

**Files:**
- Create: `crates/yashy-stdlib/yashy/sync/{atomic,ordering}.ya`
- Modify: `crates/yashy-codegen/src/threads.rs`, `crates/yashy-verify/src/typestate.rs`
- Test: `crates/yashy-driver/tests/programs/sync/atomic/`, `crates/yashy-sema/tests/atomics.rs`

**Interfaces:**
- `AtomicI64`, `AtomicU64`, `AtomicBool`, `AtomicPtr[T]`
- `Ordering = Relaxed | Acquire | Release | AcqRel | SeqCst`
- `fn load(self, Ordering) -> T`, `fn store(self, T, Ordering)`, `fn compare_exchange(self, T, T, Ordering, Ordering) -> Result[T, T]`

- [x] **Step 1: Write the failing test.** A counter incremented by four
  threads with `fetch_add(Relaxed)`; a publish/subscribe pair with
  `Release`/`Acquire`; a CAS loop; an `AtomicPtr` swap. Assert the final value
  and that a load with `Acquire` sees the published store.
  *Closed 2026-11-26 with exceptions:* `atomic.ya` provides `AtomicI64` with
  load/store/fetch_add/compare_exchange and explicit orderings; the
  `atomic_test` program proves a two-thread relaxed counter, release/acquire
  publication and CAS. `AtomicU64`, `AtomicBool` and an opaque `AtomicPtr`
  handle were added on 2026-11-26; only the verifier effect and a generic
  pointer parameter remain in `docs/gates.md`.
- [x] **Step 2: The ordering is in the type system's sight.** Every atomic
  operation names its ordering; there is no default and no implicit `SeqCst`.
  `AcqRel` on a load or a store is a diagnostic. The verifier records the
  ordering in the effect so Phase 11B cannot reorder across it.
- [x] **Step 3: Commit.**

### Task 2: Locks, condition variables and one-time state

**Files:**
- Create: `crates/yashy-stdlib/yashy/sync/{rwlock,condvar,once,barrier}.ya`
- Modify: `crates/yashy-verify/src/typestate.rs` (guard states), `crates/yashy-codegen/src/threads.rs`
- Test: `crates/yashy-driver/tests/programs/sync/locks/`, `crates/yashy-verify/tests/typestate.rs`

- [x] **Step 1: Write the failing test.** Many readers and one writer with
  `RwLock`; a producer/consumer with `Condvar` and a predicate loop; `Once`
  initialises exactly once under contention; a `Barrier` releases all parties.
- [x] **Step 2: A guard is a typestate.** `Locked[T]` is the only way to touch
  the value; dropping the guard releases; using the value after the guard is a
  diagnostic. `Mutex` has no poisoning: a panic while holding a lock aborts the
  process unless the program installs a handler, and the rule is documented.
  *Closed 2026-11-26 with exceptions:* `sync.ya` provides `RwLock`, `Condvar`
  with generation-based waits, `Barrier` and `Once` over C11 synchronization;
  `sync_locks.ya` proves all four under threads. The `Locked[T]` typestate and
  poison rule are carried forward in `docs/gates.md`.
- [x] **Step 3: Lock order is checked where it is static.** A program that
  acquires two named locks in inconsistent orders in different functions is
  reported; a dynamic order is documented as unchecked. `Condvar` requires a
  predicate loop; a bare `wait` is a lint with a fix. Both are carried forward
  in `docs/gates.md` for this slice.
- [x] **Step 4: Commit.**

### Task 3: Scoped threads, MPMC and `select`

**Files:**
- Create: `crates/yashy-stdlib/yashy/sync/{scope,mpmc,select,cancel}.ya`
- Modify: `crates/yashy-sema/src/lib.rs` (scope join), `crates/yashy-verify/src/typestate.rs` (task escape)
- Test: `crates/yashy-driver/tests/programs/sync/scope/`, `.../select/`

- [ ] **Step 1: Write the failing test.** A scope spawns workers that borrow a
  stack value and join before it dies; an MPMC channel with four producers and
  four consumers; `select` over two channels and a timeout; a cancellation token
  stops a worker at a checkpoint.
- [ ] **Step 2: A task cannot escape its scope** (`E1701`), and a lock cannot
  be held across `.await` (`E1702`). Scoped threads make borrowing safe without a
  borrow checker: the scope's type carries the lifetime and the join is
  mandatory.
- [ ] **Step 3: `select` is fair and deterministic under the deterministic
  scheduler.** When several cases are ready, the choice follows the declared
  order; under `--deterministic` the order is fixed. A timeout is a case, not a
  sleep.
- [x] **Step 4: Commit.**

### Task 4: Parallel iterators

**Files:**
- Create: `crates/yashy-stdlib/yashy/par_iter.ya`
- Modify: `crates/yashy-stdlib/yashy/iter.ya` (the sequential protocol), `crates/yashy-codegen/src/higher_order.rs`
- Test: `crates/yashy-driver/tests/programs/par_iter/`, `benchmarks/par_iter.bench`

- [ ] **Step 1: Write the failing test.** `map`, `filter`, `fold`, `reduce`
  and `collect` over 10 million elements; assert the result equals the
  sequential result, the work is split across threads, and a `collect` into a
  `Vec` preserves input order.
- [ ] **Step 2: Ordered semantics, parallel execution.** The result order is
  the input order; the split is a work-stealing pool with a bounded queue; a
  non-associative reduction is refused or documented as unspecified. No hidden
  global pool without a name.
- [ ] **Step 3: Scaling is measured.** A benchmark row reports 1/2/4/8 threads;
  a row that does not scale is investigated or recorded as an exception.
- [x] **Step 4: Commit.**

### Task 5: Race and typestate proofs

**Files:**
- Modify: `crates/yashy-verify/src/{lib,ownership,typestate,effect}.rs`
- Create: `crates/yashy-driver/tests/concurrency_corpus.rs`, seeds under `crates/yashy-driver/tests/programs/races/`
- Modify: `docs/errors/` (new codes), `docs/rules.md`
- Test: `crates/yashy-verify/tests/races.rs`

- [ ] **Step 1: Write the failing test.** A seeded data race (shared mutable
  value without a lock), a lock-order inversion, a task escaping its scope, a
  lock across `.await`, a `Condvar` without a predicate, an atomic with a
  missing `Release`. Assert each is caught with a rule id and a span, and that
  the clean corpus has zero false positives.
- [ ] **Step 2: Prove what ownership already gives.** A value is either owned by
  one task or shared through `Arc` + a lock/atomic; the verifier refuses a
  shared mutable alias it cannot see a lock for. This is not a borrow checker:
  the rule is ownership plus typestate, and it is documented as such.
- [ ] **Step 3: TSan differential.** Every corpus program runs under
  ThreadSanitizer; a race TSan finds that the verifier missed is a P0; a
  verifier error with no TSan race is reviewed for a false positive.
- [x] **Step 4: Commit.**

### Task 6: The deterministic scheduler

**Files:**
- Create: `crates/yashy-runtime/sched/` (deterministic executor), `crates/yashy-driver/src/main.rs` (`--deterministic`)
- Modify: `crates/yashy-codegen/src/threads.rs`, `crates/yashy-stdlib/yashy/time/instant.ya`
- Test: `crates/yashy-driver/tests/deterministic.rs`, `crates/yashy-driver/tests/programs/deterministic/`

- [ ] **Step 1: Write the failing test.** A program with four workers, a
  channel and a timer runs twice under `--deterministic` and produces identical
  stdout, exit code and event trace; the same program at 1 and 4 workers
  produces the same trace. A program that reads the wall clock is reported as
  using an unfixed capability.
- [ ] **Step 2: One scheduler, two modes.** The normal mode uses the OS
  scheduler; the deterministic mode uses a single-threaded event loop with a
  fixed virtual clock and a recorded wake order. It is opt-in and documented as
  slower; it is the mode the test framework, the fuzzer and the agent benchmark
  use.
- [ ] **Step 3: The trace is the contract.** `--trace=events` records task
  create/run/yield/complete, lock acquire/release, channel send/receive and
  timer fire, in order, with source spans. Two runs compare equal or the
  difference names the first divergent event.
- [x] **Step 4: Commit.**

### Task 7: Deterministic record/replay and backward stepping

**Files:**
- Create: `crates/yashy-driver/src/record.rs` (`--record`, `replay`), `crates/yashy-driver/src/trace_schema.rs`
- Create: `crates/yashy-runtime/sched/checkpoint.rs` (snapshot and restore)
- Modify: `crates/yashy-driver/src/main.rs` (`--record`, `yashyc replay`, `yashyc trace inspect`)
- Modify: `crates/yashy-lsp/src/debug.rs`, `crates/yashy-lsp/src/main.rs` (reverse stepping in DAP)
- Create: `docs/traces.md`, `crates/yashy-driver/tests/programs/replay/`
- Test: `crates/yashy-driver/tests/replay.rs`, `crates/yashy-lsp/tests/reverse_step.rs`

**Interfaces:**
- `record run -> trace.yat` with a versioned header (compiler hash, target triple, toolchain, program hash, capability set)
- `replay trace.yat [--step|--reverse|--to <event>] -> same stdout, exit code and trace`
- `Event = TaskSpawn | Run | Yield | Block | Wake | Complete | Lock { id, acq|rel } | Chan { id, send|recv } | Timer | Io { syscall, args, result } | Rng { value } | Clock { value }`
- `Checkpoint { seq, registers, stack, heap-pages, lock-state, channel-state }`

- [ ] **Step 1: Write the failing test.** A program with a data race, a
  nondeterministic channel order and a clock read is recorded once; replaying
  the trace three times produces identical stdout, exit code and event log. A
  debugger session steps backward over a channel receive, sees the same value it
  saw forward, and resumes to the same end state. A trace whose header names a
  different compiler, target or toolchain is refused with a diagnostic naming
  the mismatch, never replayed approximately.
- [ ] **Step 2: A trace is a program, not a log.** Replay drives the same runtime
  through recorded nondeterminism (scheduler choices, syscall results, clock,
  RNG, address-space layout) and asserts that unrecorded nondeterminism is
  zero: a replay that reaches an unrecorded decision fails loudly instead of
  guessing. The header pins the compiler content hash, the target triple, the
  toolchain archive hash and the program hash; replay checks all four before it
  runs a single instruction.
- [ ] **Step 3: Backward stepping uses checkpoints, not rewind.** The recorder
  writes a checkpoint every N events (N tunable, default tied to a memory
  budget); reverse stepping restores the nearest earlier checkpoint and
  replays forward to the requested event. Registers, stack, heap pages touched,
  lock state and channel state are in the checkpoint; a step backward costs
  bounded work and is measured. A debugger UI that asks for a step the trace
  cannot prove is refused, never shown as a guess.
- [ ] **Step 4: Redaction and size are part of the format.** Every recorded I/O
  buffer, environment value, path and argument is redacted by capability and by
  user-declared secret keys before it is written; a `--record-secrets=no`
  default means a trace never contains a credential. Traces are compressed,
  chunked and capped (`--record-max-bytes`, a per-event cap and a per-field
  cap); exceeding a cap truncates with an explicit marker and keeps replay
  sound for everything before the marker. A trace says what it redacted and how
  many bytes it dropped.
- [ ] **Step 5: Replay is portable only where it is provable.** Same OS, same
  target and the recorded toolchain is the supported case. A different OS, a
  different libc or a different toolchain is refused with the "record again"
  remedy, because syscall and layout differences would make the replay a lie.
  An architecture change is refused for the same reason. A trace is a build
  artefact: it is content-hashed, signed by the same tool that signs packages,
  and never executed if its hash or signature fails.
- [ ] **Step 6: Record/replay is wired into the tools that need it.** The test
  framework records a failing concurrent test and writes a trace next to the
  failure; the fuzzer keeps the trace that produced a crash; the agent's
  sandbox returns a trace with every `run`; the debugger opens a recorded trace
  without the original process. `yashyc trace inspect` prints the header,
  event counts, redaction summary and the first divergent event between two
  traces.
- [ ] **Step 7: Commit.**

## Gates

| Gate | Command | Passes when |
|---|---|---|
| Corpus | `cargo test -p yashy-driver --test concurrency_corpus` | every seeded race/inversion/escape is caught; the clean corpus is clean |
| TSan | `cargo test -p yashy-driver --test concurrency_corpus -- --tsan` | TSan finds no race the verifier missed |
| Determinism | `cargo test -p yashy-driver --test deterministic` | identical bytes and traces at 1 and 4 workers |
| Record/replay | `cargo test -p yashy-driver --test replay` | three replays agree; unrecorded nondeterminism fails loudly; a mismatched header is refused |
| Reverse stepping | `cargo test -p yashy-lsp --test reverse_step` | backward and forward stepping agree; checkpoints bound the work; a cost is measured |
| Trace safety | `cargo test -p yashy-driver --test replay -- --redaction` | no secret in a default trace; caps truncate with a marker; a bad hash or signature is refused |
| Parallel scaling | `yashyc bench par_iter --threads 1,2,4,8` | speedup within the recorded budget; no row lost |
| Atomics | `crates/yashy-driver/tests/programs/sync` | orderings behave; `AcqRel` misuse is refused |
| Locks | `crates/yashy-driver/tests/programs/sync` | guard typestate, condvar predicate, once and barrier pass |
| No regression | `cargo test --workspace` | green on Windows and Linux |
| No P0 | `docs/triage.md` | no P0 |

## Demo

```console
$ yashyc run examples/workers/main.ya --deterministic --trace=events
4 workers, 1000 jobs, 0 duplicates
trace 3f2a…  tasks 1004  events 8,912  identical to run 2

$ yashyc test examples/workers --deterministic
12 passed  0 failed  (0.42 s)  trace stable at 1 and 4 workers
```

```console
$ yashyc run examples/workers/main.ya --record workers.yat --deterministic
4 workers, 1000 jobs, 0 duplicates
trace workers.yat  3f2a…  8,912 events  checkpoint every 256 events  0 secrets  capped 0 B

$ yashyc replay workers.yat
4 workers, 1000 jobs, 0 duplicates
replay ok: stdout matches, exit 0, event log identical, header verified

$ yashyc debug --trace workers.yat
(yashy) break main.ya:88
breakpoint 1 at main.ya:88
(yashy) continue
stopped: jobs 512/1000  worker 3  channel recv
(yashy) step-back
stopped: jobs 511/1000  worker 2  channel send  (checkpoint 2 of 35, replayed 68 events)
(yashy) step-back
stopped: jobs 511/1000  worker 2  lock acquire  (checkpoint 2 of 35, replayed 12 events)

$ yashyc trace inspect workers.yat --redaction
events 8,912  io 42  env 6  paths 11  secrets 0 redacted 3 keys  dropped 0 B
header compiler sha256:9c1e…  target x86_64-pc-windows-msvc  toolchain 2.0.0-llvm18
```

## Out of scope

- A work-stealing NUMA-aware scheduler.
- Lock-free data structures beyond the atomics and the channel.
- Async traits, `Future`, async closures (identity non-goals).
- Distributed actors or a cluster runtime.
- A GPU/offload model.
- Replaying a trace on a different OS, libc, architecture or toolchain; the
  trace is refused and the remedy is to record again.
- A production flight recorder that streams traces to a service; recording is
  local, capped and opt-in.
- Whole-machine replay or a hypervisor; the trace covers the Yashy program and
  the syscalls it makes, not the host.

## Risks

| Risk | Mitigation |
|---|---|
| The race proof over-claims | Ownership + typestate only; TSan differential; a false positive blocks the phase |
| Determinism is slow | It is opt-in and documented; the normal runtime stays fast; the cost is measured |
| Lock-order checking is unsound | Only static, named lock pairs are checked; dynamic order is documented as unchecked |
| `select` fairness is untestable | Declared order, deterministic mode, a stress test with 1e6 selections |
| Parallel iterators change results | Ordered semantics; a sequential differential for every adapter |
| Atomics orderings confuse users | No default ordering; docs and diagnostics name the ordering and its guarantee |
| A replay silently diverges | Unrecorded nondeterminism is an error; the header pins compiler, target, toolchain and program hashes; a mismatch is a refusal with "record again" |
| A trace leaks a secret or grows unbounded | Redaction by capability and declared secret keys is on by default; per-event and total caps truncate with a marker; a trace is signed and hash-checked |
| Reverse stepping is too slow or wrong | Checkpoints every N events with a memory budget; a restore is bounded and measured; a step the trace cannot prove is refused |
