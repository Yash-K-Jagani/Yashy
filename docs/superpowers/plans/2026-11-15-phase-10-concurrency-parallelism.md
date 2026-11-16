# Phase 10 — Concurrency and Parallelism

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make concurrency a complete, provable part of the language: atomics
with explicit orderings, locks and condition variables, scoped threads, MPMC
channels, `select`, cancellation, parallel iterators, race proofs, and a
deterministic scheduler that makes a concurrent test reproducible.

**Outcome:** `Atomic*`, `RwLock`, `Condvar`, `Once`, `Barrier`, scoped threads,
an MPMC channel with `select` and timeouts, cancellation tokens, parallel
iterators, verifier rules for races/lock order/lock-across-await, a TSan
differential, a concurrency corpus, and `--deterministic` execution that
produces the same trace at 1 and 4 workers.

**Time:** ~8 weeks.
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

- [ ] **Step 1: Write the failing test.** A counter incremented by four
  threads with `fetch_add(Relaxed)`; a publish/subscribe pair with
  `Release`/`Acquire`; a CAS loop; an `AtomicPtr` swap. Assert the final value
  and that a load with `Acquire` sees the published store.
- [ ] **Step 2: The ordering is in the type system's sight.** Every atomic
  operation names its ordering; there is no default and no implicit `SeqCst`.
  `AcqRel` on a load or a store is a diagnostic. The verifier records the
  ordering in the effect so Phase 11B cannot reorder across it.
- [ ] **Step 3: Commit.**

### Task 2: Locks, condition variables and one-time state

**Files:**
- Create: `crates/yashy-stdlib/yashy/sync/{rwlock,condvar,once,barrier}.ya`
- Modify: `crates/yashy-verify/src/typestate.rs` (guard states), `crates/yashy-codegen/src/threads.rs`
- Test: `crates/yashy-driver/tests/programs/sync/locks/`, `crates/yashy-verify/tests/typestate.rs`

- [ ] **Step 1: Write the failing test.** Many readers and one writer with
  `RwLock`; a producer/consumer with `Condvar` and a predicate loop; `Once`
  initialises exactly once under contention; a `Barrier` releases all parties.
- [ ] **Step 2: A guard is a typestate.** `Locked[T]` is the only way to touch
  the value; dropping the guard releases; using the value after the guard is a
  diagnostic. `Mutex` has no poisoning: a panic while holding a lock aborts the
  process unless the program installs a handler, and the rule is documented.
- [ ] **Step 3: Lock order is checked where it is static.** A program that
  acquires two named locks in inconsistent orders in different functions is
  reported; a dynamic order is documented as unchecked. `Condvar` requires a
  predicate loop; a bare `wait` is a lint with a fix.
- [ ] **Step 4: Commit.**

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
- [ ] **Step 4: Commit.**

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
- [ ] **Step 4: Commit.**

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
- [ ] **Step 4: Commit.**

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
- [ ] **Step 4: Commit.**

## Gates

| Gate | Command | Passes when |
|---|---|---|
| Corpus | `cargo test -p yashy-driver --test concurrency_corpus` | every seeded race/inversion/escape is caught; the clean corpus is clean |
| TSan | `cargo test -p yashy-driver --test concurrency_corpus -- --tsan` | TSan finds no race the verifier missed |
| Determinism | `cargo test -p yashy-driver --test deterministic` | identical bytes and traces at 1 and 4 workers |
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

## Out of scope

- A work-stealing NUMA-aware scheduler.
- Lock-free data structures beyond the atomics and the channel.
- Async traits, `Future`, async closures (identity non-goals).
- Distributed actors or a cluster runtime.
- A GPU/offload model.

## Risks

| Risk | Mitigation |
|---|---|
| The race proof over-claims | Ownership + typestate only; TSan differential; a false positive blocks the phase |
| Determinism is slow | It is opt-in and documented; the normal runtime stays fast; the cost is measured |
| Lock-order checking is unsound | Only static, named lock pairs are checked; dynamic order is documented as unchecked |
| `select` fairness is untestable | Declared order, deterministic mode, a stress test with 1e6 selections |
| Parallel iterators change results | Ordered semantics; a sequential differential for every adapter |
| Atomics orderings confuse users | No default ordering; docs and diagnostics name the ordering and its guarantee |
