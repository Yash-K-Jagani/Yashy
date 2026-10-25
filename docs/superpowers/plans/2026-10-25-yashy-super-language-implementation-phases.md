# Yashy Super Language — Phased Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan phase by phase. Steps use checkbox (`- [ ]`) syntax for tracking. Do not start a phase until the previous phase gate is met or an exception is recorded.

**Goal:** Deliver Yashy as a fast, safe, AI-native general-purpose language across seven execution phases, with a working, testable increment at the end of each phase.

**Architecture:** A Rust/LLVM compiler with a C runtime, one `yashy` driver, a bundled stdlib, and a machine-readable contract (JSON, stable error codes, `AGENTS.md`, `llms.txt`). Phase 1 is the five-slice R1 program. Later phases add a fast dev loop, safety hardening, packages and services, AI-native tooling, 1.0 stability, the remaining language features, and async/self-hosting.

**Tech Stack:** Rust workspace, LLVM/clang, C runtime, GitHub Actions, Windows and Linux (macOS at 1.0).

**Spec:** `docs/superpowers/specs/2026-10-25-yashy-super-language-v2-design.md`

**Master plan:** `docs/superpowers/plans/2026-10-25-yashy-super-language-master.md`

**Detailed R1 plans:**

- Slice 1: `docs/superpowers/plans/2026-10-25-yashy-r1-install-first-cli.md`
- Slice 2: `docs/superpowers/plans/2026-10-25-yashy-r1-runtime-stdlib.md`
- Slice 3: `docs/superpowers/plans/2026-10-25-yashy-r1-diagnostics-benchmarks.md`
- Slice 3b: master plan §7.4; detailed task plan written after slice 3's baseline
- Slice 4: `docs/superpowers/plans/2026-10-25-yashy-r2-package-manager-spike.md`

**Status:** Plan set awaiting owner approval. No code starts until the v2 spec, master plan, the four slice plans, and this phased plan are approved.

**Sequencing:** Balanced. Speed and the AI-native foundation land in R1/R2/R2.5; the remaining language features land in 1.0/1.1; async, embedded/WASM, and self-hosting land in 2.0.

---

## 0. How to read this plan

- A **phase** is a release-sized outcome with a gate. Phases run in order.
- A **step** is a work package, normally one two-week slice. A phase-sized step may split into two-week units.
- A **task-level plan** names exact files, interfaces, tests, and commands. The R1 slices already have task-level plans. Later phases get a task-level plan at phase start, using the `writing-plans` skill, because the codebase will have changed.
- This document is the phase-and-step map. It does not duplicate the detailed R1 task lists; it tells the implementer which plan to execute, in what order, and what proves the phase is done.
- The order is the commitment. Dates are planning horizons for one full-time maintainer; the dates move when reality moves.
- A phase that misses its gate gets a recorded exception with an owner and a date, not a quiet extension.
- Every phase updates the master plan and the release notes before it is declared complete.

**Phase rule:** no phase starts before the previous gate is green or an exception is recorded. **Phase 0 is the only phase that may start now.**

---

## 1. Global constraints

These constraints apply to every phase. They are not restated in every step.

### 1.1 Language identity

Yashy keeps its identity. Do not add:

- A garbage collector.
- A borrow checker.
- Exceptions as the primary error path.
- Implicit coercion.
- Operator overloading.
- Class inheritance.
- Power-user procedural macros.

The model is ownership plus reference counting, `Option[T]` for absence, explicit error values, and a small audited `unsafe` surface.

### 1.2 Compiler and driver

- One `yashy` driver. One obvious way to run, test, build, format, and inspect a project.
- `run` and `test` default to `-O0`.
- `build` defaults to `-O2`.
- `build --release` defaults to `-O3`.
- An explicit `-O` always wins.
- Bundled stdlib discovery works without `YASHY_PATH`.
- Windows arguments use C `char**` code-page encoding; `getcwd()` returns UTF-8. The difference is documented and tested.

### 1.3 Tests

- R1 tests remain ordinary `tests/**/*.ya` programs with `main`.
- No `#[test]` attributes and no new test macros in R1.
- The existing driver suite under `crates/yashy-driver/tests/` remains the integration harness.
- A feature is not done until it has: tests, docs, diagnostics, and a machine-readable form where the feature is observable by an agent.

### 1.4 Packages and supply chain

- Packages have no build scripts.
- A lockfile is mandatory.
- Checksums are designed in the R1 spike and implemented in the R2 MVP.
- The dependency's `src/` is the import root; name collisions are errors.
- Registry signing is a 1.0 requirement.

### 1.5 Capacity and release discipline

- One full-time maintainer.
- Two-week slices; weekly demo; a gate review at the end of each phase.
- Main stays green.
- No silent scope growth.
- Cut from the phase's cut list before extending the phase.
- No performance claim without a benchmark.
- No safety claim without a test or an audit.

---

## 2. Review focus: five failure modes

These are the five input classes most likely to break the product. Each has an owning phase and a required test.

| # | Failure mode | Why it matters | Owning phase | Required evidence |
|---|---|---|---|---|
| 1 | Windows argument encoding vs UTF-8 `getcwd()` | A CLI that mangles non-ASCII arguments is not usable; a UTF-8 `getcwd()` that disagrees with `args()` is a silent correctness bug. | Phase 1, slice 2 | Arguments with spaces, quotes, and non-ASCII characters on Windows and Linux; `getcwd()` success and failure; docs that state the encoding difference. |
| 2 | Missing or broken LLVM/clang | A prebuilt compiler that cannot link is worse than no compiler. | Phase 1, slice 1 | `doctor` matrix for missing clang, wrong version, and broken PATH; `doctor --json` matches the human facts; install guide gives one recovery command. |
| 3 | Lockfile and offline reproducibility | Agents and CI must build the same dependency graph twice, including with no network. | Phase 1, slice 4; Phase 3, slice 9 | Path and git dependency builds; lockfile reproducible from a clean checkout; `--offline` works; collision produces a clear error. |
| 4 | Human and JSON diagnostic divergence | An agent that sees different facts from a human cannot be trusted. | Phase 1, slice 3; Phase 3, slice 7 | Schema tests for `check --json` and `test --json`; the same code, span, and fix in both forms; `explain` returns the catalog entry. |
| 5 | Benchmark noise and regression | A performance gate that is noisy gets ignored; a gate that is ignored stops protecting users. | Phase 1, slice 3 and 3b; Phase 2, slice 5 | Stored baselines for Windows and Linux; `key=value` artifacts; PR report-only; scheduled/manual hard gate; thresholds compile +20%, binary +10%, runtime +20%. |

---

## 3. Phase map

| Phase | Release | Theme | Steps | Primary outcome | Gate | Detailed plan |
|---|---|---|---|---|---|---|
| **0** | Plan set | Approve and freeze | 0.1–0.5 | The spec, master, slice plans, and this plan are approved; no code has started. | Owner approval recorded; plan set committed. | This document |
| **1** | R1 — 0.2.0 | It works | 1.1–1.5 | A stranger installs Yashy on Windows or Linux and ships a CLI in minutes. | Clean install; first CLI; `doctor` + JSON; uninstall; checksum; benchmark gate; no known UB. | Slices 1, 2, 3, 3b, 4 |
| **2** | R1.5 — 0.2.x | It's fast | 2.1–2.2 | The edit/check/test loop stops being the tax. | Incremental < 200 ms; 100 tests < 2 s; sanitizer CI green; fuzz 7 days. | Task plan at phase start |
| **3** | R2 — 0.3.0 | It's usable | 3.1–3.4 | Dependencies, services, and editors become real. | Git-dependency project from clean checkout; HTTP service; LSP conformance; M9; three tools. | Task plan at phase start |
| **4** | R2.5 — 0.4.0 | It's AI-native | 4.1–4.6 | An agent can inspect, edit, test, and fix a project from structured facts. | Context ≤ 8k tokens; MCP conformance; sandbox escape tests; schema diff CI; benchmark v0. | Task plan at phase start |
| **5** | 1.0 | It's stable | 5.1–5.7 | A stranger can bet a project on it. | All spec §7 metrics; no P0; reproducible; signed; 90% agent benchmark. | Task plan at phase start |
| **6** | 1.1 | It's complete | 6.1–6.8 | The language is complete for the five domains. | Feature acceptance tests/docs/diagnostics/JSON; cross-compile CI; `no_std`; Unicode; macros. | Task plan at phase start |
| **7** | 2.0 | It's a super language | 7.1–7.7 | Scale, independence, and ecosystem. | Self-hosted compiler; async conformance; ecosystem showcase; parity; no P0. | Task plan at phase start |

**Release names are fixed for this plan:** R1 = 0.2.0, R1.5 = 0.2.x, R2 = 0.3.0, R2.5 = 0.4.0, 1.0, 1.1, 2.0.

### 3.1 Domain coverage

| Domain | First real proof | Phase |
|---|---|---|
| **CLI/dev tools** | The stdin `wc` first CLI in R1; the flagship CLI tool in R2. | R1, R2 |
| **Automation/data** | The automation/data tool that processes files and JSON in R2. | R2 |
| **Backend** | The HTTP service in R2; the production HTTP guide and hardening at 1.0. | R2, 1.0 |
| **Embedded/edge** | Cross-compilation, the `no_std` subset, and the embedded example in 1.1. | 1.1 |
| **General-purpose/learners** | Traits, iterators, macros, and Unicode in 1.1; the book and playground at 1.0; the learner path in 2.0. | 1.0, 1.1, 2.0 |

---

## 4. Dependency graph

```
Phase 0 (approval)
    |
    v
Phase 1 (R1: slices 1 -> 2 -> 3 -> 3b -> 4)
    |
    v
Phase 2 (R1.5: slice 5 fast loop, slice 6 safety)
    |
    v
Phase 3 (R2: slice 7 LSP/fix/doc, slice 8 HTTP/stdlib, slice 9 packages, slice 10 tools)
    |
    v
Phase 4 (R2.5: index/context/MCP/sandbox/agent benchmark)
    |
    v
Phase 5 (1.0: macOS, registry, reproducible, docs, debugger, MCP v1, sandbox v1, freeze)
    |
    v
Phase 6 (1.1: traits, generics, iterators, macros, Unicode, cross-compile, no_std)
    |
    v
Phase 7 (2.0: async, self-hosting, ecosystem, SIMD, const generics, MCP v2)
```

**Cross-phase dependencies that are easy to miss:**

- Slice 3b depends on slice 3's baseline. Do not optimize before the benchmark exists.
- Slice 4 depends on slice 2 for program arguments and slice 3 for benchmark discipline.
- Slice 5 (fast loop) depends on slice 3's benchmark harness and slice 1's release artifacts.
- Slice 6 (safety) depends on slice 2's runtime work and slice 3b's optimization changes.
- Slice 7 (LSP) depends on slice 3's diagnostics and slice 1's `AGENTS.md`/`llms.txt`.
- Slice 9 (packages) depends on slice 4's design and slice 7's module graph.
- Slice 11 (AI) depends on slices 3, 7, and 9. Do not start it before slice 9's graph is stable.
- Slice 12 (1.0) depends on every previous slice.
- Phase 6 depends on the 1.0 schema freeze; do not change the AI contract silently while adding language features.
- Phase 7 depends on the 1.1 async and WASM decisions.

---

## 5. Phase 0 — Approve and freeze the plan set

**Goal:** turn the plan set into an approved execution contract. No code.

**Estimate:** 1–2 days.

### Step 0.1 — Review the spec

- [ ] Read the v2 spec end to end, especially §§1–7 (vision, identity, metrics), §13 (AI contract), §14 (roadmap), §15 (first execution phase), §20 (cut list), and §21 (decisions).
- [ ] Confirm the five domains and the non-goals are still correct.
- [ ] Confirm the release names and dates are planning horizons, not promises.
- [ ] Resolve or defer every open question in spec §21. Record the decision in the master plan.

**Gate:** the spec has no unresolved question that blocks slice 1.

### Step 0.2 — Review the master plan

- [ ] Read the master plan's execution model, current-state/gaps, workstreams, release roadmap, first execution phase, post-R1 slices, AI contract, speed/safety/DX programs, capacity, risks, cut list, and definition of done.
- [ ] Confirm the master plan matches the spec on every release assignment.
- [ ] Confirm the master plan's §19 slice index points at current plans.

**Gate:** no contradiction between the spec and the master plan.

### Step 0.3 — Review the four detailed slice plans and this plan

- [ ] Read slices 1, 2, 3, and 4.
- [ ] Read slice 3b's section in the master plan §7.4.
- [ ] Read this phased plan.
- [ ] Confirm each R1 slice has a gate command that can run on a clean checkout.
- [ ] Confirm slice 3b is before 0.2.0 and is not optional.
- [ ] Confirm no slice requires `YASHY_PATH`, `#[test]`, or a package build script.

**Gate:** the owner can explain what ships in 0.2.0, what is cut, and what proves each slice.

### Step 0.4 — Record approval

- [ ] Record the approval date and owner in the master plan.
- [ ] Change this plan's status from “awaiting approval” to “approved”.
- [ ] Do not start slice 1 until this is done.

**Gate:** approval is written down.

### Step 0.5 — Commit the plan set

- [ ] Commit the v2 spec, master plan, four slice plans, and this plan as one plan-set commit.
- [ ] Push the plan set.
- [ ] Tag the plan commit if the repository uses tags for planning baselines.

**Gate:** the plan set is on `main` (or the agreed branch) and reviewable by a second person.

**Phase 0 cut order:** nothing. If approval is blocked, stop. Do not start code to “make progress.”

---

## 6. Phase 1 — R1 “It works” (0.2.0)

**Theme:** a stranger can install Yashy, write a CLI, and get a useful error.

**Estimate:** months 1–3, five slices.

**Prerequisites:** Phase 0 approved.

**Phase outcome:** Windows and Linux installers; `doctor` + `--json`; first-CLI guide; `args`/`now`/`getcwd`/`sleep`; error codes and “did you mean”; `check --json`/`test --json`; `explain`; benchmark gate; top-three performance fixes; package-manager spike; `AGENTS.md`/`llms.txt`; safety baseline.

### 6.1 Phase 1 workstream contributions

| Workstream | What Phase 1 contributes |
|---|---|
| Speed | Benchmark gate, Windows/Linux baselines, top-three fixes in slice 3b, `--release` = `-O3`. |
| Safety | Bounds/overflow behavior documented and tested; `unsafe` inventory; leak tests. |
| DX | Error codes, “did you mean,” JSON check/test, `explain`, formatter idempotence, `doctor --json`. |
| UX | Windows/Linux install, checksums, PATH, uninstall, first CLI, quickstart. |
| Features | `args`, `getcwd`, `now`, `sleep`; no new language features. |
| AI-native | `AGENTS.md`, `llms.txt`, JSON schema v1, deterministic diagnostics, package-spike audit/offline. |

### 6.2 Step 1.1 — Slice 1: Install and first CLI

**Outcome:** a fresh Windows or Linux machine installs a prebuilt Yashy, runs `yashy doctor`, and completes a first CLI project. Linux release artifact and `install.sh` exist.

**Detailed plan:** `2026-10-25-yashy-r1-install-first-cli.md`. Execute its task list exactly. The checkboxes below are the phase-level checkpoints.

**Primary areas:** `.github/workflows/release.yml`, `scripts/install.ps1`, new `scripts/install.sh`, new checksum script, `crates/yashy-driver/src/main.rs`, `crates/yashy-driver/src/toolchain.rs`, `crates/yashy-driver/tests/doctor.rs`, `crates/yashy-driver/tests/init.rs`, `docs/`, repository-root `AGENTS.md` and `llms.txt`.

**Steps:**

- [ ] Add the Linux release artifact and `install.sh` with checksum verification, PATH setup, and uninstall.
- [ ] Add Windows installer parity: checksum verification, PATH, uninstall, and a clean removal path.
- [ ] Add `yashy doctor --json`; the JSON must carry the same facts as the human output.
- [ ] Ensure `doctor` gives one recovery command and LLVM/clang install guidance for a missing or broken toolchain.
- [ ] Make `yashy init` write `AGENTS.md` and a README with build/test/run commands.
- [ ] Write the first-CLI guide using stdin `wc`, adapted from `wc.ya`/`wc_words.ya`.
- [ ] Write the quickstart and `llms.txt`; create the repository-root `AGENTS.md`.
- [ ] Add the clean-VM install/uninstall tests, tampered-checksum test, PATH tests, and `doctor` matrix.
- [ ] Add the guide end-to-end test and the `init` file test.
- [ ] Run the targeted tests, then the full driver suite, then the clean-VM gate.

**Acceptance:** clean install on Windows and Linux; first program in under five minutes (full guide under thirty); `doctor` + `--json`; uninstall is clean; checksum works; guide is tested; `AGENTS.md`/`llms.txt` are tested; CI is green.

**Gate:** the slice 1 gate in the detailed plan. **Cut order:** macOS, shell completions, man pages, package-manager install, version manager.

### 6.3 Step 1.2 — Slice 2: Runtime and stdlib gaps

**Outcome:** `args()`, `getcwd()`, `now()`, and `sleep(ms)` work on Windows and Linux, with ownership and leak tests. The guide gains an arguments section. Bundled stdlib discovery works without `YASHY_PATH`.

**Detailed plan:** `2026-10-25-yashy-r1-runtime-stdlib.md`. Execute its task list exactly.

**Primary areas:** `crates/yashy-runtime/runtime.c`, `crates/yashy-runtime/` tests, `crates/yashy-driver/src/modules.rs`, `crates/yashy-driver/src/pipeline.rs`, `crates/yashy-driver/tests/programs/`, `crates/yashy-driver/tests/modules.rs`, `crates/yashy-driver/tests/leaks.rs`, stdlib docs.

**Steps:**

- [ ] Implement `args()` excluding the program path.
- [ ] Implement `getcwd()` returning `""` on failure.
- [ ] Implement `now()` as milliseconds since the Unix epoch.
- [ ] Implement `sleep(ms)`; `ms <= 0` is a no-op.
- [ ] Update the runtime entry signature if needed while keeping `fn main()` parameterless at the Yashy level.
- [ ] Make bundled stdlib discovery work without `YASHY_PATH`.
- [ ] Document the Windows `char**` code-page argument encoding and the UTF-8 `getcwd()` result.
- [ ] Add tests for spaces, quotes, non-ASCII arguments, `getcwd()` failure, `now()` wall clock, `sleep(ms)` duration, recursion, `-> i64`, and zero live allocations.
- [ ] Add the arguments section to the first-CLI guide.

**Acceptance:** tests pass on Windows and Linux; docs exist; no `YASHY_PATH` requirement; leak tests at zero live allocations.

**Gate:** the slice 2 gate in the detailed plan. **Cut order:** timezone/date formatting, HTTP, logging.

### 6.4 Step 1.3 — Slice 3: Diagnostics, benchmarks, and AI contract v0

**Outcome:** errors teach; performance is measured; agents can read results.

**Detailed plan:** `2026-10-25-yashy-r1-diagnostics-benchmarks.md`. Execute its task list exactly.

**Primary areas:** `crates/yashy-common/` (diagnostic codes), `crates/yashy-lexer/`, `crates/yashy-parser/`, `crates/yashy-sema/`, `crates/yashy-driver/src/main.rs`, `crates/yashy-driver/tests/`, `benchmarks/`, `.github/workflows/`, `docs/errors/`, `AGENTS.md`, `llms.txt`.

**Steps:**

- [ ] Add stable error codes (`E....`) and spans.
- [ ] Add “did you mean” for identifiers, fields, methods, imports, and missing `?`.
- [ ] Add `yashy check --json` and `yashy test --json` with schema v1.
- [ ] Add `yashy explain <code>`.
- [ ] Add the diagnostic corpus tests that assert code, span, and fix where available.
- [ ] Add the benchmark gate for compile, binary size, and runtime.
- [ ] Store `key=value` benchmark artifacts with no dependencies.
- [ ] Add Windows and Linux baselines.
- [ ] Write the top-three performance-fixes plan for slice 3b.
- [ ] Extend `AGENTS.md` and `llms.txt` with the AI contract v0.

**Acceptance:** JSON schema v1; 90% of the common error corpus has a code and span; benchmark gate runs report-only on PRs and hard on scheduled/manual runs; `explain` returns the catalog entry; artifacts parse with a shell script.

**Gate:** the slice 3 gate in the detailed plan. **Cut order:** LSP completion/rename, `fix`, `index`, MCP, sandbox.

### 6.5 Step 1.4 — Slice 3b: Top-three performance fixes

**Outcome:** the three worst benchmark rows improve; no row is worse than 2× C without an exception.

**Detailed plan:** master plan §7.4; write the task-level plan after slice 3's baseline exists.

**Primary areas:** `crates/yashy-codegen/`, `crates/yashy-runtime/runtime.c`, `benchmarks/features/array.ya`, `benchmarks/features/debug_format.ya`, `benchmarks/features/` tuple rows, `crates/yashy-driver/tests/opt.rs`.

**Steps:**

- [ ] Record the before numbers on Windows and Linux.
- [ ] Fix array mapping: 3.76× → target ≤ 2×.
- [ ] Fix debug formatting: 3.42× → target ≤ 2×.
- [ ] Fix tuples: 1.79× → target ≤ 1.5×.
- [ ] Add differential tests and run the full suite after each optimization.
- [ ] Record after numbers and a short design note for each fix.
- [ ] Confirm no other row regresses beyond the gate.

**Acceptance:** each targeted row improves; no row > 2× without a recorded exception; benchmark gate green; semantics unchanged.

**Gate:** the slice 3b gate. **Cut order:** M9, PGO/LTO, SIMD (later releases).

### 6.6 Step 1.5 — Slice 4: Package-manager spike

**Outcome:** a decision-complete design plus a throwaway prototype for path and git dependencies. No registry, no stable CLI commitment.

**Detailed plan:** `2026-10-25-yashy-r2-package-manager-spike.md`. Execute its task list exactly.

**Primary areas:** new manifest/lock parser, `crates/yashy-driver/src/modules.rs`, `crates/yashy-driver/src/main.rs`, `crates/yashy-driver/tests/`, `docs/`, package fixtures.

**Steps:**

- [ ] Define `yashy.toml` and `yashy.lock`.
- [ ] Support path and git dependencies.
- [ ] Make the dependency's `src/` the import root; make name collisions errors.
- [ ] Add `yashy pkg add`, `remove`, `tree`, and `audit`; all support `--json`.
- [ ] Forbid build scripts.
- [ ] Pin commits; record the checksum design for the R2 MVP.
- [ ] Add offline/cache mode.
- [ ] Write the registry design document; do not implement it.
- [ ] Add tests for path/git deps, lockfile reproducibility, collisions, offline, audit, and JSON.
- [ ] Build the demo project with one path dependency and one git dependency.

**Acceptance:** a path and a git dependency build and test; lockfile is reproducible; a collision produces a clear error; no build scripts; audit works; `--json` and `--offline` work; docs exist.

**Gate:** the slice 4 gate in the detailed plan. **Cut order:** registry, semver resolution, publishing, signing (1.0).

### 6.7 Phase 1 release gate

- [ ] A clean Windows machine installs from the release artifact and runs the first CLI in under five minutes.
- [ ] A clean Linux machine does the same.
- [ ] The full first-CLI guide completes in under thirty minutes.
- [ ] One external tester completes the guide without maintainer help.
- [ ] `yashy doctor` and `yashy doctor --json` agree on the same facts.
- [ ] `yashy test` works on a fresh `init`.
- [ ] The benchmark gate is green; slice 3b rows meet their targets or have exceptions.
- [ ] `check --json` and `test --json` pass schema v1 tests.
- [ ] No known UB; the `unsafe` inventory is complete.
- [ ] `AGENTS.md` commands run; `llms.txt` links resolve.
- [ ] Package spike: path/git, lockfile, collision, no build scripts, audit, `--json`, `--offline`.
- [ ] Release notes and tag `0.2.0` are published.

**Phase 1 cut order:** macOS, completions, man pages, package-manager install, version manager. Do **not** cut the installer, `run`/`test`/`init`, core stdlib, the guide, or the benchmark gate.

**Phase 1 definition of ready for slice 1:**

- [ ] The spec, master, slice plans, and this plan are approved.
- [ ] The Windows and Linux release jobs are understood.
- [ ] The first-CLI guide uses stdin, not `args()`, so slice 1 does not depend on slice 2.
- [ ] The `doctor` JSON shape is agreed.
- [ ] The installer uninstall path is agreed.
- [ ] No unresolved design work blocks the first task.

---

## 7. Phase 2 — R1.5 “It's fast” (0.2.x)

**Theme:** the dev loop stops being the tax.

**Estimate:** month 4, two slices.

**Prerequisites:** Phase 1 gate green.

**Phase outcome:** `run`/`test` `-O0`; `build` `-O2`; `--release` `-O3`; explicit `-O` wins; fast linker detection; content-addressed cache; `watch`; parallel tests; fuzz harnesses; ASan/UBSan/TSan CI; `audit --unsafe`; reproducible-build prototype.

**Task-level plan:** write at phase start using `writing-plans`. Primary areas: `crates/yashy-driver/src/pipeline.rs`, `crates/yashy-driver/src/main.rs`, `crates/yashy-driver/src/toolchain.rs`, `crates/yashy-codegen/`, `crates/yashy-runtime/`, `.github/workflows/`, `fuzz/`, `crates/yashy-driver/tests/`.

### Step 2.1 — Slice 5: Fast dev loop

- [ ] Make `yashy run` and `yashy test` default to `-O0`.
- [ ] Make `yashy build` default to `-O2`.
- [ ] Make `yashy build --release` default to `-O3`.
- [ ] Make an explicit `-O` win over every default.
- [ ] Add fast-linker detection (`lld`, `mold`) with a documented fallback.
- [ ] Add a content-addressed cache for modules and runtime artifacts.
- [ ] Add `cache clean` and cache visibility.
- [ ] Add `yashy watch`.
- [ ] Add a parallel test runner.
- [ ] Add `yashy build --json` with artifacts and timing.
- [ ] Add tests that prove the defaults, the explicit override, cache invalidation, and no semantic change.
- [ ] Add the synthetic incremental project and the 100-test project.
- [ ] Document the dev optimization decision.

**Acceptance:** incremental rebuild < 200 ms on the synthetic project; 100 tests < 2 s; no semantic change; `build --json` schema tests pass.

**Cut order:** `watch`, parallel tests, fast linker (keep cache and `-O0`).

### Step 2.2 — Slice 6: Safety hardening

- [ ] Document and test the bounds/overflow policy.
- [ ] Add the `unsafe` inventory and `yashy audit --unsafe`.
- [ ] Add fuzz harnesses for the lexer, parser, sema, codegen, and runtime boundaries.
- [ ] Add ASan, UBSan, and TSan CI.
- [ ] Add the reproducible-build prototype for one target.
- [ ] Add leak tests for the new runtime paths.
- [ ] Add the safety gate to CI.

**Acceptance:** zero known UB; sanitizer CI green; fuzz targets green for 7 days; `audit --unsafe` reports the inventory; reproducible prototype matches on one platform.

**Cut order:** TSan, reproducible prototype (keep ASan/UBSan, fuzz, `audit --unsafe`).

### Phase 2 gate

- [ ] Incremental rebuild < 200 ms.
- [ ] 100 tests < 2 s.
- [ ] Sanitizer CI green.
- [ ] Fuzz targets green for 7 days.
- [ ] `audit --unsafe` works.
- [ ] No P0 bugs.
- [ ] Release notes and tag `0.2.x` are published.

**Phase 2 cut order:** `watch`, parallel tests, fast linker, TSan, reproducible prototype. Do **not** cut `-O0` defaults, cache, ASan/UBSan, fuzz, or `audit --unsafe`.

**Phase 2 definition of ready:**

- [ ] The R1 benchmark harness is stable.
- [ ] The R1 release artifacts are reproducible enough to compare.
- [ ] The cache design is agreed.
- [ ] The sanitizer baseline is clean.

---

## 8. Phase 3 — R2 “It's usable” (0.3.0)

**Theme:** dependencies, services, and editors become real.

**Estimate:** months 4–7, four slices.

**Prerequisites:** Phase 2 gate green.

**Phase outcome:** package manager MVP; HTTP/log/cli/net/test/bench/fuzz; LSP completion/references/rename/signature; `fix`/`doc`/stack traces; `upgrade`/`uninstall`; M9; precompiled stdlib; three flagship tools; dependency audit.

**Task-level plan:** write at phase start. Primary areas: `crates/yashy-lsp/`, `crates/yashy-driver/`, `crates/yashy-stdlib/`, `crates/yashy-codegen/`, `crates/yashy-runtime/`, `crates/yashy-fmt/`, `.github/workflows/`, `docs/`, `editors/`, `examples/`.

### Step 3.1 — Slice 7: LSP, fix, doc

- [ ] Add completion for identifiers, fields, methods, imports, and stdlib symbols.
- [ ] Add references and rename.
- [ ] Add signature help.
- [ ] Add `yashy fix` with structured edits and `--json`.
- [ ] Add `yashy doc` with `--json`.
- [ ] Add `yashy upgrade` and `yashy uninstall`; R1's installer-level uninstall remains the fallback.
- [ ] Add readable stack traces.
- [ ] Add code actions for the diagnostics that have structured edits.
- [ ] Add LSP conformance tests in VS Code, Neovim, and Zed (or the agreed editor matrix).
- [ ] Add the fix corpus and doc coverage tests.

**Acceptance:** LSP conformance; fix corpus; doc coverage; panic traces readable; `fix --json` and `doc --json` schema tests pass.

**Cut order:** `fix` extras, `doc` extras, code actions beyond the top diagnostics.

### Step 3.2 — Slice 8: HTTP, logging, and stdlib

- [ ] Add `http` client and server.
- [ ] Add `log`.
- [ ] Add `cli`.
- [ ] Add `net`.
- [ ] Add `test`, `bench`, and `fuzz` helpers.
- [ ] Improve JSON.
- [ ] Add precompiled stdlib.
- [ ] Add M9 escape analysis.
- [ ] Write the service guide.
- [ ] Add a real HTTP service test with graceful shutdown.
- [ ] Add the M9 benchmark.

**Acceptance:** a real HTTP service with tests; logging guide; CLI parser examples; M9 benchmark; precompiled stdlib works without `YASHY_PATH`.

**Cut order:** `bench`/`fuzz` helpers, M9 if the benchmark is inconclusive (record the exception).

### Step 3.3 — Slice 9: Package manager MVP

- [ ] Finalize the manifest and lockfile.
- [ ] Support path and git dependencies in the MVP.
- [ ] Add `add`, `remove`, `update`, `tree`, and `audit`.
- [ ] Add checksums.
- [ ] Add offline mode.
- [ ] Keep build scripts forbidden.
- [ ] Add the local registry prototype.
- [ ] Add a real package with a dependency.
- [ ] Add lockfile reproducibility, audit, and offline tests.

**Acceptance:** a real package with a path/git dependency builds and tests; lockfile reproducible; audit works; offline works; no build scripts.

**Cut order:** `update`, local registry prototype (keep add/remove/tree/audit/offline/checksums).

### Step 3.4 — Slice 10: Three flagship tools

- [ ] Ship a CLI tool.
- [ ] Ship an automation/data tool that processes files and JSON.
- [ ] Ship a small HTTP service.
- [ ] Add tests, docs, and benchmark artifacts for each.
- [ ] Confirm all three build and run on Windows and Linux.

**Acceptance:** all three build/run on Windows and Linux; docs; benchmarks.

**Cut order:** third flagship tool (keep two).

### Phase 3 gate

- [ ] A git-dependency project builds and tests from a clean checkout.
- [ ] The automation script processes files and JSON.
- [ ] The HTTP service serves and shuts down cleanly.
- [ ] LSP conformance passes.
- [ ] M9 benchmark is green.
- [ ] Three flagship tools are shipped (or two with a recorded exception).
- [ ] Release notes and tag `0.3.0` are published.

**Phase 3 cut order:** third flagship tool, `fix` extras, `doc` extras, `bench`/`fuzz` helpers. Do **not** cut the package manager, HTTP, LSP completion, M9, or the first flagship tool.

**Phase 3 definition of ready:**

- [ ] R1.5 is stable.
- [ ] The LSP transport decision is agreed.
- [ ] The HTTP runtime model (blocking vs evented) is agreed.
- [ ] The package checksum design from the spike is accepted.
- [ ] The M9 benchmark definition is agreed.

---

## 9. Phase 4 — R2.5 “It's AI-native” (0.4.0)

**Theme:** the agent contract becomes real.

**Estimate:** months 7–9, one phase-sized step split into six work packages.

**Prerequisites:** Phase 3 gate green; slice 9's graph stable.

**Phase outcome:** `index`/`outline`/`graph`/`search`/`context`; context packs; MCP v0; sandbox v0; agent benchmark v0; schema stabilization and diff CI; AI docs updated.

**Task-level plan:** write at phase start. Primary areas: `crates/yashy-driver/src/main.rs`, `crates/yashy-driver/src/` index/context modules, `crates/yashy-lsp/`, `crates/yashy-sema/`, `.github/workflows/`, `docs/ai/`, `AGENTS.md`, `llms.txt`.

### Step 4.1 — Index, outline, graph, search

- [ ] Write `.yashy/index.json` or stdout.
- [ ] Include packages, modules, symbols, signatures, docs, imports, tests, entry points, and diagnostics.
- [ ] Add `yashy outline`.
- [ ] Add `yashy graph` for modules and dependencies.
- [ ] Add `yashy search`.
- [ ] Add deterministic output tests.
- [ ] Add index schema tests.

**Acceptance:** index/outline/graph/search produce stable JSON; deterministic for the same inputs.

**Cut order:** `search`, `graph` (keep index and outline).

### Step 4.2 — Context packs

- [ ] Add `yashy context <symbol>`.
- [ ] Include definition, signature, docs, callers, callees, tests, and diagnostics.
- [ ] Add `--max-tokens`.
- [ ] Add deterministic context tests.
- [ ] Measure context size on the flagship projects.

**Acceptance:** context pack ≤ 8k tokens on the reference project; deterministic for the same index and query.

**Cut order:** callers/callees if the graph is unstable; keep definition, signature, docs, tests, diagnostics.

### Step 4.3 — MCP v0

- [ ] Expose `check`, `build`, `test`, `fmt`, `explain`, `context`, `search`, `index`, and a sandboxed `run`.
- [ ] Never expose arbitrary shell execution.
- [ ] Add MCP conformance tests.
- [ ] Add the tool schema and error mapping.
- [ ] Document the MCP setup for at least two clients.

**Acceptance:** MCP conformance; no shell tool; sandboxed `run` only.

**Cut order:** MCP v0 (keep index/context). If cut, R2.5 still ships the local AI tools.

### Step 4.4 — Sandbox v0

- [ ] Implement default-deny capabilities for fs, net, env, and process.
- [ ] Add CPU, memory, output, and file-size limits.
- [ ] Add deterministic mode: fixed time, fixed RNG seed, fixed env.
- [ ] Add adversarial escape tests.
- [ ] Add the sandbox config format.
- [ ] Add the sandbox to CI.

**Acceptance:** sandbox escape tests green; default deny; deterministic mode works.

**Cut order:** deterministic mode, resource limits (keep default deny and escape tests).

### Step 4.5 — Agent benchmark v0

- [ ] Define 10 tasks with held-out variants.
- [ ] Measure success, time, tokens, tool calls, human interventions, and sandbox violations.
- [ ] Run in CI with a fixed model and tool budget.
- [ ] Publish results.
- [ ] Add the prompt-injection scenario.

**Acceptance:** benchmark v0 runs; trend is measured; prompt-injection scenario is included.

**Cut order:** benchmark v0 (keep the harness definition). If cut, 1.0's 90% gate is at risk; record the exception.

### Step 4.6 — Schema stabilization and docs

- [ ] Add schema diff CI.
- [ ] Stabilize the JSON shapes for index, context, MCP, and diagnostics.
- [ ] Update `AGENTS.md` and `llms.txt` for the new tools.
- [ ] Write `docs/ai/patterns.md` and `docs/ai/anti-patterns.md`.
- [ ] Add tested examples for every AI command.

**Acceptance:** schema diff CI green; docs build; examples run.

**Cut order:** anti-patterns doc (keep patterns and schema diff CI).

### Phase 4 gate

- [ ] Context pack ≤ 8k tokens.
- [ ] MCP conformance (or a recorded exception).
- [ ] Sandbox escape tests green.
- [ ] Schema diff CI green.
- [ ] Agent benchmark v0 runs (or a recorded exception) and the trend is improving.
- [ ] `AGENTS.md`/`llms.txt` updated.
- [ ] Release notes and tag `0.4.0` are published.

**Phase 4 cut order:** MCP v0, agent benchmark v0, `search`, `graph`. Do **not** cut index, outline, context, sandbox v0, or schema diff CI.

**Phase 4 definition of ready:**

- [ ] Slice 9's package graph is stable.
- [ ] The index schema is agreed.
- [ ] The MCP transport is agreed.
- [ ] The sandbox threat model is written.
- [ ] The benchmark tasks are drafted and held out.

---

## 10. Phase 5 — 1.0 “It's stable” (months 9–12)

**Theme:** a stranger can bet a project on it.

**Estimate:** months 9–12, one phase-sized step split into seven work packages.

**Prerequisites:** Phase 4 gate green.

**Phase outcome:** semver and compatibility policy; public registry with signed packages; macOS; HTTP hardening; book/API/cookbook; debugger/profiler; playground; package audit; reproducible/signed releases; LSP code actions; `lint`/coverage/`task`; MCP v1; sandbox v1; schema freeze; agent benchmark 90%.

**Task-level plan:** write at phase start. Primary areas: release workflows, registry service, `crates/yashy-driver/`, `crates/yashy-lsp/`, `crates/yashy-codegen/`, `docs/`, `website/` or `playground/`, `scripts/`.

### Step 5.1 — macOS

- [ ] Add the macOS installer and CI.
- [ ] Verify install, PATH, uninstall, and `doctor`.
- [ ] Add macOS to the release artifacts.

**Acceptance:** clean macOS install and first CLI; `doctor` green.

**Cut order:** macOS is not cut at 1.0. If blocked, record a P0 exception and slip 1.0.

### Step 5.2 — Registry, signing, reproducible builds

- [ ] Ship the public registry with signed packages.
- [ ] Add package audit.
- [ ] Add signed and reproducible releases.
- [ ] Add the compatibility policy and semver rules.
- [ ] Add the registry abuse/withdrawal process.

**Acceptance:** registry serves signed packages; audit works; reproducible build matches; artifacts signed.

**Cut order:** package audit UI (keep the command), withdrawal process automation (keep the manual process).

### Step 5.3 — Book, API reference, cookbook, error catalog

- [ ] Write the book.
- [ ] Generate the API reference.
- [ ] Write the cookbook.
- [ ] Complete the error catalog.
- [ ] Add the docs build gate and tested examples.

**Acceptance:** docs build; examples run; 100% of coded diagnostics have a catalog entry.

**Cut order:** cookbook extras (keep the book, API reference, and error catalog).

### Step 5.4 — Debugger, profiler, playground

- [ ] Add the debugger.
- [ ] Add the profiler.
- [ ] Add the playground.
- [ ] Add guides for each.

**Acceptance:** debugger steps through a program; profiler reports hotspots; playground runs the examples.

**Cut order:** playground, profiler, coverage, `lint`, `task`.

### Step 5.5 — LSP code actions, lint, coverage, task

- [ ] Add LSP code actions, inlay hints, and workspace symbols.
- [ ] Add `yashy lint`.
- [ ] Add coverage.
- [ ] Add `yashy task`.

**Acceptance:** LSP conformance for code actions; lint/coverage/task work on the flagship projects.

**Cut order:** coverage, `lint`, `task` (keep LSP code actions).

### Step 5.6 — MCP v1, sandbox v1, schema freeze

- [ ] Ship MCP v1.
- [ ] Ship sandbox v1.
- [ ] Freeze the JSON schema.
- [ ] Add schema migration notes.
- [ ] Add the sandbox review.

**Acceptance:** MCP conformance; sandbox escape tests; schema freeze recorded.

**Cut order:** none of these. Do not cut MCP v1, sandbox v1, or schema freeze.

### Step 5.7 — Agent benchmark 90% and performance gates

- [ ] Run the agent benchmark gate at 90%.
- [ ] Sustain the performance gates.
- [ ] Add the no-P0 review.
- [ ] Publish the 1.0 release notes.

**Acceptance:** 90% agent benchmark; all spec §7 metrics; no P0; reproducible; signed.

**Cut order:** none of the 1.0 gate items.

### Phase 5 gate

- [ ] All spec §7 metrics.
- [ ] No P0.
- [ ] Reproducible build.
- [ ] Signed artifacts.
- [ ] 90% agent benchmark.
- [ ] macOS install.
- [ ] Schema freeze.
- [ ] Release notes and tag `1.0.0` are published.

**Phase 5 cut order:** playground, profiler, coverage, `lint`, `task`. Do **not** cut registry, macOS, reproducibility, MCP v1, sandbox v1, schema freeze, or the agent benchmark.

**Phase 5 definition of ready:**

- [ ] R2.5 is stable.
- [ ] The registry threat model is written.
- [ ] The reproducibility target is agreed.
- [ ] The agent benchmark harness is stable.
- [ ] The macOS CI runner is available.

---

## 11. Phase 6 — 1.1 “It's complete” (months 12–24)

**Theme:** the language itself is complete for the five domains.

**Estimate:** months 12–24, one phase-sized step split into eight work packages.

**Prerequisites:** 1.0 gate green; schema frozen.

**Phase outcome:** traits/interfaces; generic constraints; associated types; trait objects; iterators; declarative macros + `yashy expand`; Unicode; `--target`; `no_std`; WASM decision; embedded example; AI context for the new features.

**Task-level plan:** write at phase start. Primary areas: `crates/yashy-lexer/`, `crates/yashy-parser/`, `crates/yashy-sema/`, `crates/yashy-codegen/`, `crates/yashy-stdlib/`, `crates/yashy-driver/`, `docs/`, `examples/`.

### Step 6.1 — Traits and interfaces

- [ ] Design traits/interfaces without class inheritance.
- [ ] Add trait declarations, impls, and bounds.
- [ ] Add diagnostics for missing/conflicting impls.
- [ ] Add tests, docs, and JSON diagnostics.

**Acceptance:** traits work in the flagship projects; diagnostics are coded; docs exist.

**Cut order:** trait objects (keep traits and bounds).

### Step 6.2 — Generic constraints, associated types, trait objects

- [ ] Add generic constraints.
- [ ] Add associated types.
- [ ] Add trait objects if the design is sound.
- [ ] Add diagnostics and tests.

**Acceptance:** each feature has tests/docs/diagnostics/JSON; no identity violation.

**Cut order:** trait objects.

### Step 6.3 — Iterators

- [ ] Add the iterator protocol.
- [ ] Add adapter and consumer methods.
- [ ] Add zero-cost lowering where practical.
- [ ] Add benchmarks and docs.

**Acceptance:** iterator benchmarks are within budget; docs and tests exist.

**Cut order:** advanced adapters (keep the core protocol).

### Step 6.4 — Declarative macros and `yashy expand`

- [ ] Add declarative macros.
- [ ] Add `yashy expand`.
- [ ] Add macro expansion tests and diagnostics.
- [ ] Keep power-user procedural macros out.

**Acceptance:** macro expansion tests; `expand` works; no procedural macros.

**Cut order:** `expand` extras (keep declarative macros and basic expand).

### Step 6.5 — Unicode strings

- [ ] Add Unicode-correct string operations.
- [ ] Add grapheme/word boundary helpers where needed.
- [ ] Add Unicode conformance tests.
- [ ] Document the encoding model.

**Acceptance:** Unicode conformance; docs; tests.

**Cut order:** advanced segmentation (keep correct basic operations).

### Step 6.6 — Cross-compilation and `no_std`

- [ ] Add `--target`.
- [ ] Add cross-compile CI.
- [ ] Add the `no_std` subset.
- [ ] Add the `no_std` example and size profile.
- [ ] Document the toolchain requirements.

**Acceptance:** cross-compile CI; `no_std` example; docs.

**Cut order:** extra targets (keep one embedded target and `no_std`).

### Step 6.7 — WASM decision and embedded example

- [ ] Make the WASM decision.
- [ ] Implement WASM only if viable.
- [ ] Ship the embedded example and guide.
- [ ] Add the AI context for embedded/cross-compile.

**Acceptance:** decision recorded; embedded example works; context updated.

**Cut order:** WASM implementation (keep the decision and embedded example).

### Step 6.8 — AI context for the new features

- [ ] Extend index/outline/context for traits, generics, macros, Unicode, and cross-compile.
- [ ] Update `AGENTS.md`/`llms.txt`.
- [ ] Add tested examples.
- [ ] Keep the schema additive; no silent break.

**Acceptance:** context covers the new features; schema diff CI green.

**Cut order:** context for macros (keep traits/Unicode).

### Phase 6 gate

- [ ] Each feature has acceptance tests, docs, diagnostics, and a JSON form.
- [ ] Cross-compile CI green.
- [ ] `no_std` example works.
- [ ] Unicode conformance.
- [ ] Macro expansion tests.
- [ ] Release notes and tag `1.1.0` are published.

**Phase 6 cut order:** WASM implementation, trait objects, embedded support. Do **not** cut traits, iterators, macros, Unicode, cross-compilation, or `no_std`.

**Phase 6 definition of ready:**

- [ ] 1.0 is stable.
- [ ] The trait design is reviewed against the identity rules.
- [ ] The macro design is declarative-only.
- [ ] The cross-compile target is chosen.
- [ ] The WASM decision criteria are written.

---

## 12. Phase 7 — 2.0 “It's a super language” (months 24–36)

**Theme:** scale, independence, and ecosystem.

**Estimate:** months 24–36, one phase-sized step split into seven work packages.

**Prerequisites:** 1.1 gate green; async and WASM decisions recorded.

**Phase outcome:** async implementation if the decision is yes; self-hosting; ecosystem frameworks; registry at scale; learner path; SIMD; const generics; MCP v2; parity with the G5 roadmap.

**Task-level plan:** write at phase start. Primary areas: `crates/`, `compiler/` or self-hosted tree, `registry/`, `docs/`, `examples/`, `website/`.

### Step 7.1 — Async

- [ ] Implement async only if the 1.1 decision is yes.
- [ ] Add the async runtime model.
- [ ] Add async conformance tests.
- [ ] Add async HTTP/service examples.
- [ ] Document the model and its limits.

**Acceptance:** async conformance; examples work; no identity violation.

**Cut order:** advanced combinators (keep the core model).

### Step 7.2 — Self-hosting

- [ ] Write the self-hosted compiler.
- [ ] Build the stdlib with it.
- [ ] Pass the full suite.
- [ ] Compare performance with the Rust compiler.
- [ ] Keep the bootstrap reproducible.

**Acceptance:** self-hosted compiler builds stdlib and passes the suite; bootstrap documented.

**Cut order:** self-hosting performance parity (keep correctness and bootstrap).

### Step 7.3 — Ecosystem and frameworks

- [ ] Ship web, CLI, and data frameworks outside the stdlib.
- [ ] Add the ecosystem showcase.
- [ ] Add compatibility tests against the core.
- [ ] Document the package boundaries.

**Acceptance:** ecosystem showcase; frameworks build on the stable core.

**Cut order:** ecosystem frameworks (keep async, self-hosting, registry at scale).

### Step 7.4 — Registry at scale

- [ ] Scale the registry.
- [ ] Add federation if required.
- [ ] Add abuse/withdrawal automation.
- [ ] Add mirrors and availability targets.

**Acceptance:** registry at scale; mirrors; documented operations.

**Cut order:** federation (keep the single registry at scale).

### Step 7.5 — Learner path

- [ ] Complete the book, exercises, playground, and classroom guide.
- [ ] Add the learner benchmark.
- [ ] Test the path with real learners.

**Acceptance:** learner path works end to end; feedback recorded.

**Cut order:** classroom guide (keep book, exercises, playground).

### Step 7.6 — SIMD and const generics

- [ ] Add SIMD.
- [ ] Add const generics.
- [ ] Add benchmarks and docs.
- [ ] Keep the safety model.

**Acceptance:** benchmarks; docs; tests; no P0.

**Cut order:** SIMD, const generics (keep async, self-hosting, registry).

### Step 7.7 — MCP v2 and agent-native ecosystem

- [ ] Ship MCP v2.
- [ ] Add agent-native ecosystem tools.
- [ ] Update `AGENTS.md`/`llms.txt`.
- [ ] Keep the agent benchmark gate.

**Acceptance:** MCP v2 conformance; ecosystem tools; benchmark gate sustained.

**Cut order:** ecosystem tools (keep MCP v2 and the benchmark).

### Phase 7 gate

- [ ] Self-hosted compiler builds Yashy.
- [ ] Async conformance (if shipped).
- [ ] Ecosystem showcase.
- [ ] Performance parity.
- [ ] No P0.
- [ ] Release notes and tag `2.0.0` are published.

**Phase 7 cut order:** SIMD, const generics, ecosystem frameworks. Do **not** cut async (if decided yes), self-hosting, or registry at scale.

**Phase 7 definition of ready:**

- [ ] 1.1 is stable.
- [ ] The async decision is recorded.
- [ ] The self-hosting bootstrap plan is written.
- [ ] The registry scale target is agreed.
- [ ] The learner path is drafted.

---

## 13. Cross-phase workstream matrix

| Workstream | Phase 1 (R1) | Phase 2 (R1.5) | Phase 3 (R2) | Phase 4 (R2.5) | Phase 5 (1.0) | Phase 6 (1.1) | Phase 7 (2.0) |
|---|---|---|---|---|---|---|---|
| **Speed** | Benchmark gate; baselines; top-three fixes; `--release` `-O3` | `-O0`/`-O2` defaults; cache; fast linker; `watch`; parallel tests | M9; precompiled stdlib; parallel codegen; fast paths | Context/index performance; sandbox overhead budget | PGO/LTO; bounds/overflow elision; sustained gates | Cross-compile pipeline; `no_std` size | SIMD; self-hosting performance; ecosystem builds |
| **Safety** | Bounds/overflow policy; `unsafe` inventory; leak tests | Fuzz; ASan/UBSan/TSan; `audit --unsafe`; reproducible prototype | Dependency audit; lockfile; no build scripts | Sandbox v0; escape tests | Sandbox v1; reproducible; signed; package audit | `no_std` safety subset; embedded notes | Trusted-core invariants; self-hosted safety |
| **DX** | Error codes; did-you-mean; JSON check/test; `explain`; formatter idempotence; `doctor --json` | `watch`; parallel tests; cache visibility; stack traces | LSP completion/references/rename/signature; `fix`; `doc`; completions; man pages; `upgrade`/`uninstall` | Index/outline/graph/search/context; MCP v0 | LSP code actions/inlay hints; debugger/profiler; coverage; `lint`; `task`; playground | Call/type hierarchy; `expand`; cross-compile diagnostics | MCP v2; self-hosted tooling |
| **UX** | Windows/Linux install; checksums; PATH; uninstall; doctor; first CLI; quickstart | Faster edit loop; clearer cache behavior | Completions; man pages; `upgrade`; `env --json`; templates; troubleshooting | Agent setup docs; MCP clients | macOS; signed installers; version manager; playground; full docs | Embedded/WASM install guides; cross-compile toolchains | Learner path; ecosystem showcase |
| **Features** | `args`/`now`/`getcwd`/`sleep`; no new language features | No new language features | HTTP/log/cli/net/test/bench/fuzz | No new language features | No new language features | Traits; generics; iterators; macros; Unicode; cross-compile; `no_std`; WASM decision | Async; self-hosting; SIMD; const generics |
| **AI-native** | `AGENTS.md`; `llms.txt`; JSON schema v1; deterministic diagnostics | Stable schema; cache-aware `build --json` | `fix --json`; `doc --json`; package audit; stable contract | Index/context; MCP v0; sandbox v0; agent benchmark v0; schema diff CI | MCP v1; sandbox v1; schema freeze; 90% benchmark | Context for traits/Unicode/macros; cross-compile context | MCP v2; agent-native ecosystem; self-hosted toolchain |

---

## 14. Phase gate checklist

Every phase gate answers these questions. A “no” blocks the release unless an exception is recorded.

### 14.1 Product

- [ ] The phase outcome is demonstrable by a person who did not write the code.
- [ ] The must-ship list is complete or cut with a recorded exception.
- [ ] The cut list was respected; no uncut item was silently dropped.

### 14.2 Tests

- [ ] Unit, integration, and end-to-end tests pass.
- [ ] Leak tests pass at zero live allocations for new runtime paths.
- [ ] Sanitizer/fuzz gates pass where applicable.
- [ ] The full driver suite passes.
- [ ] The phase's adversarial tests pass (encoding, sandbox, offline, schema).

### 14.3 Docs

- [ ] Human docs exist for every new command and feature.
- [ ] Examples are tested.
- [ ] `AGENTS.md` and `llms.txt` are updated.
- [ ] Error catalog entries exist for new codes.
- [ ] The quickstart still works.

### 14.4 Machine-readable contract

- [ ] Every observable command has a documented JSON schema.
- [ ] Human and JSON output carry the same facts.
- [ ] `schema_version` is present.
- [ ] Schema diff CI is green (from R2.5).
- [ ] No secrets in output.
- [ ] No interactive prompt in `--json` or non-TTY mode.

### 14.5 Performance

- [ ] Benchmarks run on Windows and Linux.
- [ ] The phase's thresholds pass, or an exception is recorded.
- [ ] No row is worse than 2× C without an exception.
- [ ] The benchmark gate is report-only on PRs and hard on scheduled/manual runs.

### 14.6 Safety

- [ ] No known UB.
- [ ] `unsafe` inventory is current.
- [ ] Sandbox escape tests pass where applicable.
- [ ] No package build scripts.
- [ ] Lockfile and checksums are enforced where applicable.

### 14.7 Release

- [ ] Release notes are written.
- [ ] The version tag is created.
- [ ] The artifacts are signed where applicable.
- [ ] The master plan and this plan are updated.
- [ ] The next phase's definition of ready is satisfied.

---

## 15. Risk register

| Risk | Likelihood | Impact | Mitigation | Owner phase |
|---|---|---|---|---|
| LLVM compile time dominates the dev loop | High | High | `-O0` defaults; content cache; fast linker; parallel tests; `watch` | Phase 2 |
| Cache invalidation is wrong | Medium | High | Content hashes; conservative invalidation; `cache clean`; cache tests | Phase 2 |
| Optimization breaks semantics | Medium | High | Differential tests; full suite; benchmark gate; sanitizers | Phase 1 (3b), Phase 2 |
| Windows argument encoding surprises users | High | Medium | Document code-page vs UTF-8; test spaces/quotes/non-ASCII | Phase 1 (slice 2) |
| A prebuilt install cannot link | Medium | High | `doctor` matrix; one recovery command; LLVM guidance; install tests | Phase 1 (slice 1) |
| Package resolver scope creep | High | High | Path/git only in R1 spike; no registry; cut list | Phase 1 (slice 4), Phase 3 |
| Lockfile is not reproducible | Medium | High | Pinned commits; checksums; clean-checkout test; offline test | Phase 1 (slice 4), Phase 3 |
| Human/JSON diagnostics diverge | Medium | High | Schema tests; same facts; `explain`; schema diff CI | Phase 1 (slice 3), Phase 3 |
| Benchmark noise makes the gate useless | High | Medium | Stored baselines; `key=value` artifacts; report-only PRs; hard scheduled gate | Phase 1 (slice 3), Phase 2 |
| LSP scope exceeds the slice | Medium | Medium | Completion/references/rename/signature only; cut extras | Phase 3 |
| HTTP runtime model is wrong | Medium | High | Decide blocking vs evented before slice 8; prototype; service test | Phase 3 |
| MCP/sandbox security bug | Low | Critical | Default deny; escape tests; external review; no shell tool | Phase 4, Phase 5 |
| Schema churn breaks agents | Medium | High | `schema_version`; additive 0.x; diff CI; freeze at 1.0 | Phase 4, Phase 5 |
| Agent benchmark overfits | Medium | Medium | Held-out variants; public harness; no task-specific hacks | Phase 4, Phase 5 |
| Registry supply-chain attack | Medium | Critical | No build scripts; lockfile; checksums; signing; audit; withdrawal | Phase 5 |
| Reproducibility across platforms | Medium | High | Per-platform first; document differences; signed artifacts | Phase 5 |
| Trait/generic design violates identity | Medium | High | Design review against non-goals; no inheritance; no operator overloading | Phase 6 |
| Macro system becomes procedural | Medium | High | Declarative only; `expand`; no power-user procedural macros | Phase 6 |
| Unicode scope explodes | Medium | Medium | Correct basic operations; advanced segmentation cut | Phase 6 |
| Cross-compile toolchains are unavailable | Medium | High | One embedded target; documented toolchain; CI matrix | Phase 6 |
| Async model is wrong | Medium | Critical | Decide in 1.1; prototype; conformance tests; cut if not viable | Phase 7 |
| Self-hosting slips | High | High | Bootstrap plan; keep Rust compiler until parity; cut performance parity first | Phase 7 |
| Solo capacity | High | High | Two-week slices; cut lists; quarterly re-plan; no silent growth | All |

---

## 16. Cut order across releases

Cut in this order. Never cut a gate item.

1. **R1:** macOS, completions, man pages, package-manager install, version manager.
2. **R1.5:** `watch`, parallel tests, fast linker, TSan, reproducible prototype.
3. **R2:** third flagship tool (keep two), `fix` extras, `doc` extras, `bench`/`fuzz` helpers.
4. **R2.5:** MCP v0, agent benchmark v0, `search`, `graph` (keep index, outline, context, sandbox v0).
5. **1.0:** playground, profiler, coverage, `lint`, `task` (keep registry, macOS, reproducibility, MCP v1, sandbox v1, schema freeze, agent benchmark).
6. **1.1:** WASM implementation, trait objects, embedded support (keep traits, iterators, macros, Unicode, cross-compilation, `no_std`).
7. **2.0:** SIMD, const generics, ecosystem frameworks (keep async if decided yes, self-hosting, registry at scale).

**Never cut:** the installer, `run`/`test`/`init`, core stdlib, the first-CLI guide, the benchmark gate, the JSON schema, the error codes, the lockfile, no build scripts, or the phase's definition of done.

---

## 17. New things locked in during this pass

These were added or made explicit while aligning the v2 plan set. They are already reflected in the spec, master, and slice plans.

- `yashy doctor --json` in R1, with the same facts as the human output.
- Installer-level uninstall in R1; `yashy uninstall` as a CLI command in R2.
- `AGENTS.md` and `llms.txt` created in slice 1 and extended in slice 3.
- Error codes, `check --json`, `test --json`, and `explain` in R1.
- `yashy pkg audit`, `--json`, and `--offline` in the R1 spike and R2 MVP.
- Checksums designed in the R1 spike and implemented in the R2 MVP.
- First-run targets: install + init + run under five minutes; full guide under thirty minutes.
- `now()` and `sleep(ms)` instead of `time()`; `time()` is rejected as a function name.
- Windows arguments use C `char**` code-page encoding; `getcwd()` returns UTF-8.
- `run`/`test` `-O0`, `build` `-O2`, `--release` `-O3`; explicit `-O` wins.
- R1 ships installer-level uninstall; the CLI uninstall is R2.
- Precompiled stdlib and M9 are R2; sandbox v0 is R2.5.
- R1 tests remain ordinary `tests/**/*.ya` programs with `main`.

**No new language features were added beyond the v2 spec.** The R1 cut list protects the schedule. Any future feature request goes into the next phase's cut list, not into R1.

---

## 18. Definition of done for this plan

This plan is done when:

1. The v2 spec is reviewed and approved.
2. The master plan is reviewed and approved.
3. Slice plans 1, 2, 3, and 4 are reviewed and approved.
4. Slice 3b's section is approved; its task-level plan is written after slice 3's baseline.
5. This phased plan is reviewed and approved.
6. Every phase has a gate that can run on a clean checkout.
7. Every phase has a cut order.
8. Every phase has a definition of ready.
9. No code has started.

**The implementation is done when:**

1. Phases 1 → 7 pass their gates in order, or record exceptions with owners and dates.
2. 0.2.0 ships with the R1 must-ship list.
3. Each later release ships its must-ship list or its cut list.
4. Every release meets spec §7 or records an exception.
5. The agent benchmark, benchmark gate, and safety gate are green at 1.0.
6. The self-hosted compiler builds the stdlib and passes the suite at 2.0.
7. The master plan, this plan, and the release notes are current.

---

## 19. Immediate next action

- [ ] Review the v2 spec, master plan, slices 1–4, and this phased plan.
- [ ] Approve them or request changes.
- [ ] Commit and push the approved plan set.
- [ ] Start **slice 1 only** after approval, using its detailed plan.
- [ ] Do not start slices 2, 3, 3b, or 4 early. They have dependencies and gates.

**The first command after approval is not `cargo build`. It is the first task in `2026-10-25-yashy-r1-install-first-cli.md`.**
