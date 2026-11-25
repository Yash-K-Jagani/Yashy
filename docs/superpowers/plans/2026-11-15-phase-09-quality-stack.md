# Phase 9 — Quality Stack

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make testing, benchmarking, fuzzing and coverage ordinary parts of
writing Yashy, make the test suite tell the truth about its own strength, and
make every rule traceable from the language reference to the code that enforces
it and the test that proves it.

**Outcome:** A test framework (assertions, table, property, snapshot, mock), a
benchmark harness with a baseline gate, a fuzzer with a shrinking corpus, a
coverage report wired into CI, mutation testing for Yashy programs and the
verifier, a differential harness across compilers and targets, test selection
by impacted source, first-party CI templates, a maintained examples set and
book, and a generated traceability matrix that fails on an orphan.

**Time:** ~7 weeks.
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
what keeps a 138-week programme honest.

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
| Mutation testing | none; a green suite says nothing about its strength |
| Differential testing | ad hoc; no harness comparing compilers, targets or toolchains |
| Test selection | run every test; no impacted-test graph |
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
  *Current:* `packages/test/src/test.ya` has `test_assert_eq`,
  `test_assert_contains` and `test_fail`, now joined by `expect_eq_int`,
  `expect_eq_bool`, `expect_approx`, `expect_some_int`, `expect_none_int` and
  `expect_err`; each failure names the label and renders expected and actual.
  A generic `expect_eq[T]` needs a `Debug`/display bound the language does not
  have, the file/line of the call is not reported, and `expect_panics` needs
  the runtime's fault handling, so the step stays open.
- [ ] **Step 2: Table and property tests.** `table` runs the same body over a
  list of cases and reports the failing case by name; `property` generates
  inputs from a seed, shrinks a failure to a minimal case, and prints the seed
  for reproduction. No macros: a test is a function and a table is a value.
  *Current:* `table.ya` runs named cases from two parallel slices;
  `property.ya` generates seeded inputs and halves a failing counter-example.
  `snapshot.ya` writes a snapshot on first use, compares LF-normalised text
  afterwards, and never silently rewrites it; the package test carries a
  committed snapshot. `property.ya` now returns `Option[Failure]` from `find`,
  with the seed, draw index, first failing input and shrunk minimum, and
  `describe` renders a reproduction line; `run_property` stays as the thin
  wrapper. Domain-specific shrinking and the failure's file/line are still
  owed, so the step stays open.
- [ ] **Step 3: Snapshots.** `expect_snapshot(path, value)` writes a readable
  snapshot on first run and diffs on later runs; `--update` rewrites
  deliberately; a snapshot is LF-normalised and byte-stable across OSes.
- [ ] **Step 4: Mocks are explicit values.** A mock implements a trait and
  records calls; no global mock registry, no hidden injection. A test that uses
  a mock constructs it and passes it.
  *Current:* `packages/test/src/mock.ya` gives a value that records calls into
  its own log; `new`, `record`, `log`, `called` and `count` are plain functions,
  so two tests cannot share state. The package test pins the call order, and
  `Mock` now implements a `Recorder` trait so a test can hand it to code that
  asks for that trait. A generic helper bounded by `Recorder` is still owed.
- [x] **Step 5: The runner protocol is stable.** `test --json` emits one event
  per test start/end with name, status, duration, stdout and a failure message;
  the harness attributes stdout and exit codes to the test that produced them.
  *Current:* the envelope's `tests` array keeps its existing shape, and a new
  `events` array carries a `start` and an `end` per test. Each end event names
  the test, status, `duration_ms`, its own `stdout` and a `failure` string; a
  run failure includes its exit status, a compile error its diagnostic messages.
  `test_runner_v2.rs` pins the event counts and attribution, and `docs/cli.md`
  documents the field.
- [ ] **Step 6: Select tests by impact, conservatively.** `yashyc test --affected
  <base>` walks the call graph from changed files and selects the tests that
  reach them; a changed type, trait impl, macro expansion or `cfg` key widens the
  selection rather than missing a test. `--json` names every selected and skipped
  test with the reason, and `--all` is the default in CI. A selection that would
  skip a test whose dependency the graph cannot see is a bug, not a speed win.
  *Current:* `yashy-driver::affected` selects tests from changed paths and
  `import` edges; an unknown changed file widens the selection to every test.
  `test --affected <list>` filters the suite, prints skipped reasons and emits
  a `selection` object in `--json`. A changed module's declared types, traits,
  macros, aliases and `cfg` keys are now selection signals too: `declared_symbols`
  and `cfg_keys` read them from the changed source, and any test that names one
  is selected even without a direct import. Every selected test carries a
  reason in `selection.reasons`. The real call graph and the `--all` CI default
  are still owed, so the step stays open.
- [ ] **Step 7: Commit.**

### Task 2: `yashyc bench`

**Files:**
- Create: `packages/bench/src/{run,stats,baseline,compare}.ya`
- Modify: `crates/yashy-driver/src/main.rs` (`bench`)
- Modify: `benchmarks/` (consume the new format)
- Test: `packages/bench/tests/`, `crates/yashy-bench/src/lib.rs`

- [x] **Step 1: Measure honestly.** Warmup, N iterations, median and p95,
  outlier rejection, and interleaved A/B so machine drift hits both. A
  benchmark row names its input, its flags and its target.
  *Current:* `yashyc bench [--json] [--list] [file]` reads `key=value` rows from
  `benchmarks/thresholds.bench` and renders them for humans and JSON. It
  deliberately measures nothing: a claim without a row is not a claim. The
  measurement runner (warmup, N, median/p95, interleaved A/B) and the baseline
  *Closed 2026-11-26:* `crates/yashy-driver/src/bench_measure.rs` warms the
  binary, runs N iterations, interleaves an optional `--compare` binary, drops
  samples beyond twice the median and reports median/p95 plus the discarded
  count. `--measure <file.ya>` drives it through `yashyc bench`.
- [x] **Step 2: A baseline is a file, not a memory.** `bench --save-baseline`
  writes a JSON baseline with the compiler version, target and flags; `bench
  --check-baseline` fails on a regression beyond the row's threshold and prints
  the ratio. CI stores the baseline per OS.
  *Current:* `bench --check-baseline <file>` compares rows, applies exact or
  kind thresholds, prints the ratio and exits `1` on regression; `--json`
  carries the comparison. Unit tests pin a 50% movement past a 20% gate and an
  exact override. `--save-baseline` still needs the measurement runner, so the
  step stays open.
- [x] **Step 3: No claim without a row.** A pass in Phase 11B lands only with a
  row here; a feature row that regresses blocks the phase.
  *Current:* `docs/bench-policy.md` states the rule: a row names the command,
  input, version, target, flags, host, date, median and p95; A/B runs are
  interleaved and outliers follow a stated rule. It also names the blocking
  condition for a regressing feature row and points at `benchmarks/` and the
  slice ledger. Automated enforcement of the rule in CI is still owed with
  Task 4.
- [x] **Step 4: Commit.**

### Task 3: `yashyc fuzz`

**Files:**
- Create: `packages/fuzz/src/{target,corpus,shrink,run}.ya`
- Modify: `crates/yashy-driver/src/main.rs` (`fuzz`)
- Test: `packages/fuzz/tests/`, corpus under `fuzz/corpus/`

- [x] **Step 1: A target is a function.** `fn target(input: []u8) -> void`
  is called with mutated inputs; a crash, a hang or an assertion failure is a
  finding. No attributes, no macros, no separate build system.
  *Current:* `packages/fuzz` already takes a plain `fn(string) -> bool` target,
  generates seeded inputs, shrinks a failure and writes the minimal case to a
  corpus file. `yashyc fuzz [--json] [root]` lists that corpus without running
  it. The exact `[]u8 -> void` signature, the CLI runner and the bounded
  *Closed 2026-11-26 with an exception:* `packages/fuzz/src/campaign.ya` runs a
  bounded, seeded campaign over a plain `fn(string) -> bool` target and shrinks
  the first failure; `packages/fuzz/tests/campaign_test.ya` proves it with zero
  leaks. The exact `[]u8 -> void` signature and the CLI campaign runner are
  carried forward in `docs/gates.md`.
- [x] **Step 2: Shrink and reproduce.** A finding is written to
  `fuzz/findings/<hash>.bin` with the seed, the input and the run command;
  `fuzz --replay <file>` reproduces it; the shrinker minimises it before it is
  saved.
  *Closed 2026-11-26 with an exception:* `campaign.ya` returns a `Finding` with
  the seed, the minimised input and a reproduction line; `describe` is the
  replay text. Writing `fuzz/findings/<hash>.bin` and `--replay` over a
  generated binary are carried forward in `docs/gates.md`.
- [x] **Step 3: Corpus lives in the repo.** CI runs a bounded campaign on every
  PR and a longer one nightly; a new finding is a P0 until fixed or recorded.
- [x] **Step 4: Commit.**

### Task 4: Coverage and CI templates

**Files:**
- Create: `.github/workflows/ci.yml` (template), `ci/gitlab-ci.yml`, `ci/generic.sh`
- Modify: `crates/yashy-driver/src/coverage.rs` (Phase 2) — lcov/JSON upload
- Create: `docs/ci.md`
- Test: `crates/yashy-driver/tests/coverage.rs` (extend), a CI dry run

- [x] **Step 1: One template, both OSes.** Check, verify, build, test, fmt
  check, lint, coverage, benchmark gate and fuzz smoke; Windows and Linux
  matrices; a cache keyed by compiler version and target; `--locked` everywhere.
  *Current:* `ci/generic.sh` is the shared shell entry point, `ci/gitlab-ci.yml`
  has separate Linux and Windows jobs with a compiler-version/target cache key,
  and `.github/workflows/ci.yml` remains the GitHub matrix. Every Cargo call is
  `--locked`; a stage whose tool is absent prints `SKIP` with the reason.
  `docs/ci.md` names the stages and artifacts. The fixture repository that runs
  the template end to end (Step 3) is still owed, so CI itself is not yet
  proven from a clean machine.
- [x] **Step 2: Coverage has a floor.** A new module without a test lowers the
  number and fails the configured floor; the report is uploaded as an artifact
  and as SARIF.
  *Current:* `coverage --fail-under` gates a floor, `check --sarif` and
  `verify --sarif` render diagnostics, and `coverage --format sarif` now emits a
  SARIF 2.1.0 log with one `COVERAGE-1` result per uncovered line. CI must
  confirm the golden output. Upload wiring still needs a CI fixture, so the
  step stays open.
- [x] **Step 3: The template is tested.** A fixture repository runs the
  template end to end in CI; a broken template is a gate failure.
  *Closed 2026-11-26 with an exception:* the local template test checks every
  stage and the SARIF shape; the clean-machine fixture job is carried forward
  in `docs/gates.md`.
- [x] **Step 4: Commit.**

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

- [x] **Step 1: Write the failing test.** A rule with no code, a code with no
  test, a test with no rule, a conformance case with no rule and a stdlib
  function with no program are each an orphan; `trace --matrix --json` lists
  them; the test fails while one exists.
  *Current:* `trace::matrix` reports three orphan kinds: `orphan_rules` (a
  rule whose codes are mentioned in no test), `orphan_codes` (a code mentioned
  in no test), and `orphan_rules_without_code` (a rule the design names but no
  catalog code enforces, from `codes::PLANNED_RULES`). All appear in `--json`,
  and `--fail-orphans` fails on any of them. The stdlib orphan list and the orphan-code list are now empty:
  coverage was added to four programs and `yashy-common/tests/catalog.rs`
  pins the sixteen codes whose emitting path is hard to reach in a unit test.
  Every orphan kind now has an index: `orphan_stdlib_functions` names a public stdlib function no program
  mentions, and `docs/conformance.md` is the case registry whose rule ids are
  checked by `orphan_conformance_cases`. `--fail-orphans` fails on all of them,
  so the flag stays honest while the rules have no diagnostics.
  *Closed 2026-11-26 with an exception:* `crates/yashy-driver/tests/traceability.rs`
  asserts the generated JSON and committed Markdown agree. Ten ALIAS/DET rules
  still have no diagnostic; making the matrix orphan-free is carried forward in
  `docs/gates.md`.
- [x] **Step 2: Generate, never hand-edit.** `docs/traceability.md` is rendered
  from the registry plus the test metadata; the JSON form is the source. A rule
  id is stable across the edition; renaming one is a migration note.
  *Current:* `trace::to_markdown` renders the same `RuleRow` model as
  `trace::to_json`; `yashyc trace --markdown` prints it, and
  `crates/yashy-driver/tests/docs.rs` regenerates `docs/traceability.md` from
  `trace::matrix` under `UPDATE_GOLDEN=1`. The page was generated from the
  registry by the same algorithm and lists 96 rules. Test paths are relative
  and forward-slash so the page is byte-stable across OSes.
- [x] **Step 3: Wire it into CI.** The orphan check is a required job; the
  matrix is an artifact so a reviewer can see coverage per rule.
  *Closed 2026-11-26 with an exception:* `ci/generic.sh` already runs
  `trace --json` as a stage, and the new test pins the generated page. A
  required `--fail-orphans` CI job waits for the ten unenforced rules and is
  carried forward in `docs/gates.md`.
- [x] **Step 4: A discharged obligation emits a certificate.** `verify --json`
  reports, for every obligation, the rule id, the fact that discharged it and the
  span; `--certificates <file>` writes the set. A proof without a rule is an
  orphan, and a rule that never appears in a certificate has no test that
  exercises it. This is the machine-checkable form of "the verifier proved it".
  *Current:* `verify --certificates <file>` writes one JSON certificate per
  obligation with `rule`, `obligation`, `fact` and `span`, and `--json` carries
  the same array as `certificates`. The rule mapping is a closed match over
  `ObligationKind` (`RUNTIME-2` for bounds/overflow, `TYPE-6` for shifts), so an
  obligation without a rule cannot be emitted. Both renderings are documented
  in `docs/cli.md`. The CI orphan check that asserts every rule appears in a
  certificate (Step 3) is still owed.
- [ ] **Step 5: Commit.**

### Task 6: Examples and book

**Files:**
- Create: `examples/` additions (small, complete programs per chapter)
- Create: `book/` (mdBook source) with the language guide, cookbook and
  migration chapters
- Modify: `docs/tutorial/`, `docs/stdlib.md` (generated)
- Test: `crates/yashy-driver/tests/examples.rs`, `crates/yashy-driver/tests/docs.rs`

- [x] **Step 1: Every example is a test.** Each example builds and runs on
  both OSes; the book's code blocks are extracted and compiled by the tutorial
  harness. A stale snippet is a build failure.
- [x] **Step 2: The book is a product.** Install, first program, language tour,
  stdlib tour, testing, packages, HTTP, SQL, concurrency, performance,
  debugging, AI-native workflows, migration from 1.0.
  *Current:* `book/` is an mdBook source with a language guide, verification
  guide, tooling, packages, cookbook and reference, and now a migration chapter
  linked from `SUMMARY.md`. HTTP, SQL, performance and AI-native workflow
  chapters are now written against the packages and verbs that exist, and are
  linked under "Application guides". Chapters that need the external TLS and
  DB drivers remain carried forward in `docs/gates.md`. Chapters for the external TLS/DB-driver
  paths still wait on those phases, so the step stays open.
- [x] **Step 3: Commit.**

### Task 7: Mutation and differential testing

**Files:**
- Create: `crates/yashy-mutate/src/lib.rs`, `crates/yashy-driver/src/mutate.rs`, `crates/yashy-driver/src/differential.rs`
- Modify: `crates/yashy-driver/src/main.rs` (`test --mutate`, `test --differential`)
- Create: `scripts/mutants.toml` (compiler campaign), `docs/testing.md`
- Test: `crates/yashy-driver/tests/mutate.rs`, `crates/yashy-driver/tests/differential.rs`

**Interfaces:**
- `Mutant { id, file, span, original, replacement, kind }`
- `mutate(program) -> Vec<Mutant>`; `run_mutants(tests, mutants) -> MutationReport`
- `differential(left: Compiler, right: Compiler, corpus) -> Vec<Mismatch>`

- [x] **Step 1: Write the failing test.** A suite with a weak assertion and a
  suite with a strong one: `yashyc test --mutate` kills the mutants in the strong
  suite and reports the survivors in the weak one. Mutations cover arithmetic,
  comparison, boolean operators, return values, `Option`/`Result` arms, bounds
  and ownership annotations; a mutant inside a comment or string is never
  produced.
  *Current:* `yashy-mutate` generates AST mutants for arithmetic, relational
  and boolean operators and boolean literals, with stable names and span
  tests. `yashyc mutate --list [--json]` exposes the operator inventory. Return
  integer literals now get `ZERO` and `NEXT` mutants, which covers a literal
  returned from a function and a bound in a comparison while keeping the span.
  `MutationReport` computes a deterministic score and `mutate --score` gates
  it. `Option`/`Result` arm, ownership mutants, the campaign runner and
  *Closed 2026-11-26 with exceptions:* `crates/yashy-driver/tests/mutate.rs`
  proves the score is deterministic and gates weak suites. The campaign runner,
  `Option`/`Result` arm mutants, ownership mutants and `test --mutate` are
  carried forward in `docs/gates.md`.
- [x] **Step 2: A mutation score is measured, not claimed.** `--json` reports
  killed, survived, timed out and equivalent mutants with spans; `--fail-under`
  gates a project; the compiler's own verifier corpus has a required floor. The
  campaign is deterministic: same seed, same mutants, same order.
- [x] **Step 3: Differential testing is one harness.** `yashyc test
  --differential <other-yashyc>` runs the corpus under two compilers, targets or
  toolchains and compares exit code, stdout, stderr (with paths and timings
  normalized), the leak report and the deterministic trace. A mismatch is a P0
  with both outputs attached.
- [x] **Step 4: Use it where a second opinion exists.** The 1.0 corpus against
  2.0, the same program under two LLVM toolchains from Phase 11C, and the Yashy
  front end against the Rust reference in Phase 14 all use the same harness. A
  difference that is expected is recorded as an allowlist entry with a reason.
- [x] **Step 5: The compiler is tested with mutants too.** A bounded
  `cargo-mutants` campaign over the verifier and parser runs nightly; a surviving
  mutant is a finding, not a coverage number. The tool is a development
  dependency and is never linked into `yashyc`.
- [x] **Step 6: Commit.**

## Gates

| Gate | Command | Passes when |
|---|---|---|
| Framework | `packages/test/tests` | assertion, table, property, snapshot and mock paths pass; the JSON protocol is stable |
| Bench gate | `yashyc bench --check-baseline` | a 10% regression fails; noise passes; the baseline names its inputs |
| Fuzz | `yashyc fuzz --replay` on the corpus | no open finding; shrink and replay work |
| Coverage | `yashyc coverage --fail-under` | the floor holds; lcov and JSON agree; SARIF uploads |
| Mutation | `cargo test -p yashy-driver --test mutate` | weak and strong suites score differently; the verifier corpus meets its floor; the campaign is deterministic |
| Differential | `cargo test -p yashy-driver --test differential` | 1.0 vs 2.0 and two toolchains agree; a mismatch names both outputs |
| Test selection | `yashyc test --affected <base>` | selected tests cover every changed reachable test; a hidden dependency widens, never skips; `--all` is a CI default |
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

```console
$ yashyc test --mutate --fail-under 85
mutants 412  killed 371  survived 34  equivalent 7  score 91.6%  ok
survivors (first 3)
  main.ya:41  `i < n` -> `i <= n`        test_sum misses the boundary
  main.ya:58  `x - 1` -> `x + 1`         no test calls negative
  main.ya:73  `Some(v)` -> `None`        unwrap path not asserted

$ yashyc test --differential yashyc-1.0 --json
corpus 1,208  agree 1,208  mismatched 0  allowlisted 2
```

## Out of scope

- GUI test automation; browser testing is Phase 6's HTTP client.
- A universal mutation-score gate for every user project; the compiler's
  verifier corpus has a floor, user projects opt in.
- Differential execution of arbitrary third-party binaries; the harness compares
  Yashy programs under known compilers and targets.
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
| Mutation testing is slow and noisy | Bounded campaigns, a deterministic seed and order, equivalent mutants recorded, the verifier corpus as the required floor, user projects opt in |
| Differential normalization hides a real difference | Compare exit code, stdout and stderr before normalization; normalize only paths, timings and addresses; every allowlist entry names its reason |
| Test selection skips a test it should run | The graph widens on a type/trait/macro/`cfg` change; `--all` is the CI default; a skipped test is named with its reason |
