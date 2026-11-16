# Phase 9 — Quality Stack

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make testing, benchmarking, fuzzing and coverage ordinary parts of
writing Yashy, and make every rule traceable from the language reference to the
code that enforces it and the test that proves it.

**Outcome:** A test framework (assertions, table, property, snapshot, mock), a
benchmark harness with a baseline gate, a fuzzer with a shrinking corpus, a
coverage report wired into CI, first-party CI templates, a maintained examples
set and book, and a generated traceability matrix that fails on an orphan.

**Time:** ~6 weeks.
**Depends on:** Phase 8 gate.
**Spec:** [`../specs/2026-11-15-yashy-2.0-design.md`](../specs/2026-11-15-yashy-2.0-design.md) §6.8, §6.9.

## Why this phase

A language is only as good as the loop a person uses to prove their program
works. 1.0's tests are ordinary programs with a `main`, which is the right
foundation and a thin tool: no assertions with values, no table, no property, no
snapshot, no mock, no benchmark gate, no fuzzer, no coverage.

The traceability matrix is the other half. Every rule in this plan names an id;
every diagnostic code names a rule; every conformance case and stdlib function
names the rule it covers. Generating the matrix — instead of writing it — is
what keeps a 119-week programme honest.

## Starting point

| Fact | 1.0 value |
|---|---|
| Test runner | one process per test; Phase 1 replaces it |
| Assertions | `assert` and `print`; no value rendering |
| Table/property/snapshot | none |
| Mocks | none |
| Benchmark | `yashy-bench` rows and comparators; no `yashyc bench` |
| Fuzz | none |
| Coverage | none until Phase 2 |
| CI | workflows exist; no first-party template |
| Traceability | rules, codes and tests are separate lists |

## Deliverables

### Task 1: The test framework

**Files:**
- Create: `packages/test/src/{assert,table,property,snapshot,mock,report}.ya`
- Modify: `crates/yashy-driver/src/test_harness.rs` (Phase 1) — the JSON event stream
- Test: `packages/test/tests/`, `crates/yashy-driver/tests/test_runner_v2.rs` (extend)

- [ ] **Step 1: Assertions that show values.** `expect_eq(a, b)` renders
  both values with `Debug`; `expect_err`, `expect_some`, `expect_none`,
  `expect_contains`, `expect_approx` and `expect_panics`. A failure names the
  file, line, expected and actual.
- [ ] **Step 2: Table and property tests.** `table` runs the same body over a
  list of cases and reports the failing case by name; `property` generates
  inputs from a seed, shrinks a failure to a minimal case, and prints the seed
  for reproduction. No macros: a test is a function and a table is a value.
- [ ] **Step 3: Snapshots.** `expect_snapshot(path, value)` writes a readable
  snapshot on first run and diffs on later runs; `--update` rewrites
  deliberately; a snapshot is LF-normalised and byte-stable across OSes.
- [ ] **Step 4: Mocks are explicit values.** A mock implements a trait and
  records calls; no global mock registry, no hidden injection. A test that uses
  a mock constructs it and passes it.
- [ ] **Step 5: The runner protocol is stable.** `test --json` emits one event
  per test start/end with name, status, duration, stdout and a failure message;
  the harness attributes stdout and exit codes to the test that produced them.
- [ ] **Step 6: Commit.**

### Task 2: `yashyc bench`

**Files:**
- Create: `packages/bench/src/{run,stats,baseline,compare}.ya`
- Modify: `crates/yashy-driver/src/main.rs` (`bench`)
- Modify: `benchmarks/` (consume the new format)
- Test: `packages/bench/tests/`, `crates/yashy-bench/src/lib.rs`

- [ ] **Step 1: Measure honestly.** Warmup, N iterations, median and p95,
  outlier rejection, and interleaved A/B so machine drift hits both. A
  benchmark row names its input, its flags and its target.
- [ ] **Step 2: A baseline is a file, not a memory.** `bench --save-baseline`
  writes a JSON baseline with the compiler version, target and flags; `bench
  --check-baseline` fails on a regression beyond the row's threshold and prints
  the ratio. CI stores the baseline per OS.
- [ ] **Step 3: No claim without a row.** A pass in Phase 11B lands only with a
  row here; a feature row that regresses blocks the phase.
- [ ] **Step 4: Commit.**

### Task 3: `yashyc fuzz`

**Files:**
- Create: `packages/fuzz/src/{target,corpus,shrink,run}.ya`
- Modify: `crates/yashy-driver/src/main.rs` (`fuzz`)
- Test: `packages/fuzz/tests/`, corpus under `fuzz/corpus/`

- [ ] **Step 1: A target is a function.** `fn target(input: []u8) -> void`
  is called with mutated inputs; a crash, a hang or an assertion failure is a
  finding. No attributes, no macros, no separate build system.
- [ ] **Step 2: Shrink and reproduce.** A finding is written to
  `fuzz/findings/<hash>.bin` with the seed, the input and the run command;
  `fuzz --replay <file>` reproduces it; the shrinker minimises it before it is
  saved.
- [ ] **Step 3: Corpus lives in the repo.** CI runs a bounded campaign on every
  PR and a longer one nightly; a new finding is a P0 until fixed or recorded.
- [ ] **Step 4: Commit.**

### Task 4: Coverage and CI templates

**Files:**
- Create: `.github/workflows/ci.yml` (template), `ci/gitlab-ci.yml`, `ci/generic.sh`
- Modify: `crates/yashy-driver/src/coverage.rs` (Phase 2) — lcov/JSON upload
- Create: `docs/ci.md`
- Test: `crates/yashy-driver/tests/coverage.rs` (extend), a CI dry run

- [ ] **Step 1: One template, both OSes.** Check, verify, build, test, fmt
  check, lint, coverage, benchmark gate and fuzz smoke; Windows and Linux
  matrices; a cache keyed by compiler version and target; `--locked` everywhere.
- [ ] **Step 2: Coverage has a floor.** A new module without a test lowers the
  number and fails the configured floor; the report is uploaded as an artifact
  and as SARIF.
- [ ] **Step 3: The template is tested.** A fixture repository runs the
  template end to end in CI; a broken template is a gate failure.
- [ ] **Step 4: Commit.**

### Task 5: The traceability matrix

**Files:**
- Create: `crates/yashy-driver/src/trace_matrix.rs`
- Modify: `crates/yashy-common/src/codes.rs` (rule id per code)
- Modify: `crates/yashy-driver/src/main.rs` (`trace --matrix`)
- Create: `docs/traceability.md` (generated)
- Test: `crates/yashy-driver/tests/traceability.rs`, `crates/yashy-driver/tests/docs.rs`

**Interfaces:**
- `RuleId(&'static str)`; `Rule { id, title, section, codes: Vec<Code>, tests: Vec<TestId>, conformance: Vec<CaseId> }`
- `matrix() -> TraceMatrix`; `orphans() -> Vec<Orphan>`

- [ ] **Step 1: Write the failing test.** A rule with no code, a code with no
  test, a test with no rule, a conformance case with no rule and a stdlib
  function with no program are each an orphan; `trace --matrix --json` lists
  them; the test fails while one exists.
- [ ] **Step 2: Generate, never hand-edit.** `docs/traceability.md` is rendered
  from the registry plus the test metadata; the JSON form is the source. A rule
  id is stable across the edition; renaming one is a migration note.
- [ ] **Step 3: Wire it into CI.** The orphan check is a required job; the
  matrix is an artifact so a reviewer can see coverage per rule.
- [ ] **Step 4: A discharged obligation emits a certificate.** `verify --json`
  reports, for every obligation, the rule id, the fact that discharged it and the
  span; `--certificates <file>` writes the set. A proof without a rule is an
  orphan, and a rule that never appears in a certificate has no test that
  exercises it. This is the machine-checkable form of "the verifier proved it".
- [ ] **Step 5: Commit.**

### Task 6: Examples and book

**Files:**
- Create: `examples/` additions (small, complete programs per chapter)
- Create: `book/` (mdBook source) with the language guide, cookbook and
  migration chapters
- Modify: `docs/tutorial/`, `docs/stdlib.md` (generated)
- Test: `crates/yashy-driver/tests/examples.rs`, `crates/yashy-driver/tests/docs.rs`

- [ ] **Step 1: Every example is a test.** Each example builds and runs on
  both OSes; the book's code blocks are extracted and compiled by the tutorial
  harness. A stale snippet is a build failure.
- [ ] **Step 2: The book is a product.** Install, first program, language tour,
  stdlib tour, testing, packages, HTTP, SQL, concurrency, performance,
  debugging, AI-native workflows, migration from 1.0.
- [ ] **Step 3: Commit.**

## Gates

| Gate | Command | Passes when |
|---|---|---|
| Framework | `packages/test/tests` | assertion, table, property, snapshot and mock paths pass; the JSON protocol is stable |
| Bench gate | `yashyc bench --check-baseline` | a 10% regression fails; noise passes; the baseline names its inputs |
| Fuzz | `yashyc fuzz --replay` on the corpus | no open finding; shrink and replay work |
| Coverage | `yashyc coverage --fail-under` | the floor holds; lcov and JSON agree; SARIF uploads |
| CI | `ci` dry run | template passes on Windows and Linux; cache hit measured |
| Traceability | `cargo test -p yashy-driver --test traceability` | zero orphans; every discharged obligation has a certificate and a rule |
| Examples/book | `cargo test -p yashy-driver --test examples --test docs` | every example and code block runs |
| No regression | `cargo test --workspace` | green on Windows and Linux |
| No P0 | `docs/triage.md` | no P0 |

## Demo

```console
$ yashyc test --json | yashyc test report
142 passed  0 failed  3 skipped  (1.84 s)

$ yashyc bench --check-baseline
map_insert  1.03× baseline  ok
json_parse  0.87× baseline  ok  (faster)
sort_u64    1.12× baseline  FAIL (threshold 1.10)

$ yashyc trace --matrix --json | yashyc trace orphans
0 orphans: 214 rules, 96 codes, 1,208 tests, 412 conformance cases
```

## Out of scope

- GUI test automation; browser testing is Phase 6's HTTP client.
- Mutation testing as a required gate (a later experiment).
- A hosted coverage or benchmark service.
- Property generation for user-defined arbitrary instances beyond the built-in
  generators; the RFC can add more.

## Risks

| Risk | Mitigation |
|---|---|
| The test framework becomes a macro language | Tests are functions and values; no attributes, no macros |
| Property tests are flaky | A seed is printed and replayable; shrinking is deterministic |
| Snapshots hide a change | Diffs are shown; `--update` is explicit; LF-normalised |
| Benchmarks game the machine | Interleaved A/B, baseline per OS, thresholds per row |
| Coverage becomes the goal | A floor, not a target; review reads tests, not the percentage |
| The matrix is gamed by empty tests | A test must fail when the rule is removed; the orphan check also requires a triggering test |
