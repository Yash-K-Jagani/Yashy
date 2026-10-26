# Yashy Super-Language Master Plan v2

> **For agentic workers:** This is the execution index. The v2 spec is the product
> and engineering design; this plan is the order, gates, and cut list; each slice
> plan is a unit of work. Work one slice at a time, keep `main` green, and do not
> start a later slice until the current slice's gate is met or an exception is
> recorded.

**Status:** Draft for review, 2026-10-25.
**Source of truth for product direction:** `docs/superpowers/specs/2026-10-25-yashy-super-language-v2-design.md`.
**Phased execution map:** `docs/superpowers/plans/2026-10-25-yashy-super-language-implementation-phases.md`.
**Capacity:** one full-time maintainer.
**Immediate sequence:** slices 1 → 2 → 3 → 3b → 4.
**Code start:** only after the spec, this master plan, the phased plan, and the slice
plans are approved.

---

## 1. How to use this plan

There are four layers:

1. **The v2 spec** defines the language identity, the six workstreams, the metrics,
   the feature catalog, and the release phases.
2. **The phased implementation plan** defines the phase order, steps, gates, and
   definitions of ready from approval through 2.0.
3. **This master plan** defines the execution order, the gates, the cut list, and
   the decisions.
4. **The slice plans** define the files, tests, and commands for one unit of work.

Rules:

- Read the spec section that owns a slice before starting it.
- One slice at a time. No parallel compiler slices.
- A slice ends with a gate command on a clean checkout, a demo, tests, docs, and a
  commit.
- A new idea goes into the catalog or the next release's plan, not into the current
  slice.
- A missed gate gets a written exception with a reason, an owner, and a date.
- Every feature pays rent: tests, docs, diagnostics, and a machine-readable form.
- No code before approval. The spec and this plan are not implementation
  authorization by themselves.

---

## 2. Where Yashy is today

The compiler is no longer a prototype. The language and toolchain that exist today
are enough to build real native programs.

### 2.1 What exists

- **Compiler:** lexer, parser, semantic checker, LLVM IR codegen, C runtime.
  Ownership is linear and reference-counted; there is no GC and no borrow checker.
- **Language:** `Result`, structs, enums, pattern matching, strings, slices, maps,
  closures, generics, threads, mutexes, channels, C interop, and checked
  arithmetic.
- **Tooling:** `yashy build`, `check`, `run`, `test`, `init`, `doctor`, and `fmt`.
  `run` forwards `-- <args>` to the child; `test` discovers ordinary
  `tests/**/*.ya` programs; `--release` defaults to `-O3`.
- **Standard library:** `alloc`, `io`, `math`, `os`, `string`, `path`, `JSON`, and
  `rand` are real. `fs`, `env`, and `process` document builtins.
- **Release engineering:** Windows installer and packaging exist. CI runs the full
  test suite on Ubuntu and Windows. A scheduled benchmark smoke job reports compile
  time, binary size, and runtime on Windows.
- **Docs:** README, tutorial, language spec, vision, and this plan set.

### 2.2 What is missing or weak

| Gap | Why it matters | Slice | Status |
|---|---|---|---|
| Linux release artifact and `install.sh` | A fresh Linux user cannot install Yashy. | 1 | Done |
| First-CLI guide and examples | There is no path from install to a working tool. | 1 | Done |
| `args()`, `now()`, `getcwd()`, `sleep(ms)` | A real CLI cannot read arguments or time work. | 2 | Done |
| "Did you mean," error codes, JSON diagnostics | Errors are correct but do not teach or integrate. | 3 | Done |
| Benchmark gate and stored baselines | Performance can regress silently. | 3 | Done |
| Top-three feature-row performance fixes | Array, debug, and tuple rows are 3.76×, 3.42×, 1.79×. | 3b | Open |
| Package-manager design | The required R2 feature has no design. | 4 | Open |
| Fast dev loop (`-O0`, cache, watch) | The edit/test loop is the tax on every feature. | 5 | Open |
| Safety harnesses (fuzz, sanitizers, audit) | "Safe" is not yet evidenced. | 6 | Open |
| LSP completion/rename, `fix`, `doc` | Editors and agents cannot finish the loop. | 7 | Open |
| HTTP, logging, CLI parser | Real services and tools need them. | 8 | Open |
| Package manager MVP | Dependencies are required for R2. | 9 | Open |
| Three flagship tools | The language has no public proof. | 10 | Open |
| AI index/context, MCP, sandbox, agent benchmark | Agents still guess. | 11 | Open |
| 1.0 hardening (macOS, registry, docs, debugger) | Adoption and trust. | 12 | Open |

Slice 1 is complete; its task ledger is
`.superpowers/sdd/2026-10-25-yashy-r1-install-first-cli/progress.md`.

Stack traces, macOS, traits, Unicode, cross-compilation, and async are later by
design. They are not "forgotten"; they are in the spec's release phases.

---

## 3. Language identity (do not violate)

Every plan in this set must preserve the language the spec describes:

- **No tracing GC.** Ownership is linear; values are released by scope, `return`,
  `break`, and `continue`, or by explicit free.
- **No borrow checker.** The checker enforces linearity and moves, not lifetimes.
- **No exceptions.** Failure is a `Result`, a panic, or a process exit.
- **No implicit coercion.** Integer, float, and string conversions are explicit.
- **No operator overloading.** Operators are fixed by the language.
- **No class inheritance.** Composition, structs, enums, and traits (later).
- **No power-user procedural macros.** Declarative macros are the limit.
- **C interop is explicit.** `extern` and `unsafe` mark the boundary.

These are not preferences to revisit while implementing a slice. If a slice appears
to need one of them, the slice is wrong, not the identity.

---

## 4. The v2 operating model

The v2 spec adds six workstreams. They are lenses on the same compiler, runtime, and
toolchain, not six separate roadmaps.

| Workstream | What it owns | Primary metric |
|---|---|---|
| **Speed** | Compile, build, test, runtime, binary size, dev loop | Hello < 1 s; incremental < 200 ms; runtime ≤ 1.2× C |
| **Safety** | Ownership, checks, `unsafe`, concurrency, FFI, supply chain, sandbox, reproducibility | Zero known UB; all `unsafe` audited; sandbox escape tests |
| **DX** | Diagnostics, LSP, formatter, test runner, package manager, debugger, docs | 90% errors with code + span + fix; first test < 1 min |
| **UX** | Install, upgrade, uninstall, CLI, completions, man pages, docs, playground | Clean install to first program < 5 min |
| **Features** | Language, stdlib, tooling, platforms, AI | Every feature pays rent |
| **AI-native** | JSON, index, context, fix, MCP, sandbox, agent benchmark | 90% agent task success at 1.0 |

The sequencing is **Balanced**:

1. R1/R2/R2.5: speed, safety evidence, machine-readable diagnostics, AI/editor
   tooling, package manager.
2. 1.0/1.1: stability and the big language features.
3. 2.0: async, self-hosting, ecosystem, scale.

The binding rule:

> **Every feature ships with tests, docs, diagnostics, and a machine-readable form.
> No exceptions.**

Speed and AI-friendliness compound: every later feature is cheaper to build and
safer to use when builds are fast and errors are structured.

---

## 5. North-star gates

These are the execution gates. The full metric table is in spec §7.

| Area | Gate | Release |
|---|---|---|
| First run | Clean install to first program < 5 min | R1 |
| First test | First test < 1 min after `init` | R1 |
| Compile | Hello < 1 s | R1 |
| Compile | 10k-line project < 5 s | 1.0 |
| Incremental | Rebuild < 200 ms | R2 |
| Tests | 100 tests < 2 s | R2 |
| Runtime | Scalar ≤ 1.2× C; feature rows ≤ 2× C | 1.0 |
| Binary | Hello < 500 KB; < 10% regression per release | R1/1.0 |
| Safety | Zero known UB; 100% `unsafe` audited; fuzz/sanitizers green | Every release |
| DX | 90% common errors with code + span + fix | R2 |
| AI | JSON for core tools; context ≤ 8k tokens; agent benchmark 90% | R1/R2.5/1.0 |
| Ecosystem | Package manager MVP; registry; 3 flagship tools | R2/1.0 |
| Reliability | Windows/Linux CI; macOS at 1.0; no P0 | Every release |

**Gate policy:**

- PRs report; scheduled/manual runs gate.
- Thresholds: compile +20%, binary +10%, runtime +20%.
- A row worse than 2× C needs a recorded exception.
- No performance claim without a benchmark.
- No safety claim without a test or an audit.

---

## 6. Release roadmap (execution view)

The spec is the product source of truth. This table is the execution view: what
must ship, what proves it, and what gets cut first.

| Release | User outcome | Must ship | Gate | Cut first |
|---|---|---|---|---|
| **R1 — 0.2.0 "It works"** (month 3) | Maya installs Yashy and builds a real CLI tool in minutes. | Windows + Linux installers; `doctor` + `--json`; first-CLI guide; `args`/`now`/`getcwd`/`sleep`; error codes + "did you mean"; `check --json`/`test --json`; benchmark gate; top-three fixes (3b); package-manager spike; `AGENTS.md`/`llms.txt`; safety baseline. | Clean Windows/Linux install to running CLI under five minutes (full guide under thirty); `yashy test` on a fresh `init`; benchmark gate; JSON schema; no known UB; one external tester completes the guide. | macOS, completions, man pages, package-manager install, version manager. Do **not** cut installer, `run`/`test`/`init`, core stdlib, guide, or benchmark gate. |
| **R1.5 — 0.2.x "It's fast"** (month 4) | The edit/check/test loop stops being the tax. | `run`/`test` `-O0`; `build` `-O2`; `--release` `-O3`; fast linker; content cache; `watch`; parallel tests; fuzz/sanitizers; `audit --unsafe`. | Incremental < 200 ms; 100 tests < 2 s; sanitizer CI green; fuzz green 7 days. | `watch`, parallel tests, fast linker (keep cache and `-O0`). |
| **R2 — 0.3.0 "It's usable"** (months 4–7) | Maya shares a project with a git dependency; Dev writes an automation script and a small HTTP service. | Package manager MVP; HTTP/log/cli/net/test/bench/fuzz; LSP completion/references/rename/signature; `fix`/`doc`/stack traces; M9; precompiled stdlib; three flagship tools; dependency audit. | Git-dependency project builds/tests from a clean checkout; automation script processes files + JSON; HTTP service serves and shuts down; LSP conformance; M9 benchmark; three tools (or two with an exception). | Third flagship tool (keep two), `fix` extras, `doc` extras, `bench`/`fuzz` helpers. Do **not** cut package manager, HTTP, LSP completion, M9, or the first flagship tool. |
| **R2.5 — 0.4.0 "It's AI-native"** (months 7–9) | An agent can inspect, edit, test, and fix a project from structured facts. | `index`/`outline`/`graph`/`search`/`context`; MCP v0; sandbox v0; agent benchmark v0; schema stabilization; AI docs. | Context ≤ 8k tokens; MCP conformance (or exception); sandbox escape tests; schema diff CI; benchmark v0 runs (or exception). | MCP v0, agent benchmark v0, `search`, `graph` (keep index, outline, context, sandbox v0). |
| **1.0 "It's stable"** (months 9–12) | Sam builds and deploys a small backend; external users adopt without maintainer help. | Semver; registry + signing; macOS; HTTP hardening; book/API/cookbook; debugger/profiler; playground; package audit; reproducible/signed releases; LSP code actions; `lint`/coverage/`task`; MCP v1; sandbox v1; schema freeze; agent benchmark 90%. | All spec §7 metrics; no P0; reproducible; signed; 90% agent benchmark. | Playground, profiler, coverage, `lint`, `task`; do not cut registry, macOS, reproducibility, MCP v1, sandbox v1, schema freeze, or agent benchmark. |
| **1.1 "It's complete"** (months 12–24) | Ana cross-compiles for embedded; Lee learns from the book and playground; library authors use traits and Unicode. | Traits/interfaces; generic constraints; associated types; trait objects; iterators; declarative macros + `expand`; Unicode; `--target`; `no_std`; WASM decision; embedded example. | Each feature has tests/docs/diagnostics/JSON; cross-compile CI; `no_std` example; Unicode conformance; macro expansion tests. | WASM implementation, trait objects, embedded support (keep traits, iterators, macros, Unicode, cross-compilation, `no_std`). |
| **2.0 "It's a super language"** (months 24–36) | Yashy is a credible general-purpose language with an ecosystem, learners, and a self-hosted compiler. | Async; self-hosting; ecosystem/frameworks; registry at scale; learner path; SIMD; const generics; MCP v2. | Self-hosted compiler builds stdlib and passes the suite; async conformance; ecosystem showcase; parity; no P0. | SIMD, const generics, ecosystem frameworks (keep async, self-hosting, registry at scale). |

**The one-sentence priority:** make the first release genuinely usable and legible
before making the language bigger.

**R1 includes slice 3b.** The top-three performance fixes are not optional before
0.2.0. If a fix slips, record the exception and the measured reason.

**R2 and R2.5 are separate phases.** R2 does not absorb R2.5. If R2 exceeds 12
weeks, cut from its cut list.

---

## 7. The first execution phase (slices 1, 2, 3, 3b, 4)

These are the first implementation plans. The planned slices are 1, 2, 3, 3b,
and 4; 3b is the required performance slice between 3 and 4. They run in order:
**1 → 2 → 3 → 3b → 4**. Each is a two-week slice unless noted.

| Slice | Outcome | Depends on | Detailed plan |
|---|---|---|---|
| **1. Install + first CLI** | A fresh Windows or Linux machine installs a prebuilt Yashy, runs `yashy doctor`, and completes a first CLI project. Linux release artifact and `install.sh` exist. | Nothing. The first-CLI guide reads standard input, not `args()`. | `2026-10-25-yashy-r1-install-first-cli.md` |
| **2. Runtime stdlib gaps** | `args()`, `getcwd()`, `now()`, and `sleep(ms)` work on Windows and Linux, with ownership and leak tests. The guide gains an arguments section. | Slice 1 for the guide; independent for runtime work. | `2026-10-25-yashy-r1-runtime-stdlib.md` |
| **3. Diagnostics + benchmarks + AI v0** | "Did you mean," error codes, `check --json`, `test --json`, `explain`, and a benchmark gate with stored baselines. | Slice 1 CI; slice 2 runtime for timing rows. | `2026-10-25-yashy-r1-diagnostics-benchmarks.md` |
| **3b. Top-three performance fixes** | The three worst benchmark rows improve; no row is worse than 2× without an exception. | Slice 3 baseline. | Specified in this section; detailed plan written after slice 3. |
| **4. Package-manager spike** | A decision-complete design plus a throwaway prototype for path and git dependencies. No registry, no stable CLI commitment. | Slice 2 (program args for the demo) and slice 3 (benchmark discipline). | `2026-10-25-yashy-r2-package-manager-spike.md` |

### 7.1 Slice 1 — Install and first CLI

**Goal:** a stranger on Windows or Linux installs Yashy, runs a CLI, and knows how
to build and test.

**Deliverables:**

- Windows installer: prebuilt binary, checksum verification, PATH setup, uninstall,
  `doctor`.
- Linux installer: prebuilt tarball + `install.sh`, checksum, PATH, uninstall,
  `doctor`.
- `yashy doctor --json`.
- `yashy init` writes `AGENTS.md` and a README with build/test/run commands.
- First CLI guide: stdin `wc`, adapted from the existing `wc.ya`/`wc_words.ya`.
- Quickstart documentation and `llms.txt`.
- Release workflow builds Windows and Linux artifacts.

**Tests:**

- Clean-VM install/uninstall on Windows and Linux.
- Checksum verification fails on a tampered artifact.
- PATH is set and removed correctly.
- `doctor` matrix covers missing clang, wrong version, broken PATH.
- The first CLI guide runs end-to-end.
- `init` writes `AGENTS.md` and the README.

**Gate:** clean install on both platforms; first program in under five minutes
(full guide under thirty); `doctor` + `--json`; uninstall is clean; checksum
works; guide is tested; `AGENTS.md`/`llms.txt` are tested; CI is green.

**Cut order:** macOS, shell completions, man pages, package-manager install,
version manager.

**Known risks:** Application Control blocks freshly linked/copied binaries on the
development host; pad launch copies with 4096 random bytes. WSL builds use
`CARGO_TARGET_DIR=/tmp/yashy-target-linux` to avoid clobbering the Windows `target`.
The Linux artifact must be built on `ubuntu-22.04` and tested on the oldest
supported image.

### 7.2 Slice 2 — Runtime and stdlib gaps

**Goal:** the first CLI and normal programs can use arguments, time, the working
directory, and sleep without `YASHY_PATH`.

**Deliverables:**

- `args()` excludes the program path.
- `getcwd()` returns `""` on failure.
- `now()` returns milliseconds since the Unix epoch; `sleep(ms)` blocks for at
  least `ms` milliseconds (`ms <= 0` is a no-op).
- Runtime entry signature updated if needed.
- Bundled stdlib discovery works without `YASHY_PATH`.
- Windows arguments use C `char**` code-page encoding; `getcwd()` returns UTF-8.
  The difference is documented.
- Stdlib docs for the new functions.

**Tests:**

- Arguments with spaces, quotes, and non-ASCII characters on Windows and Linux.
- `getcwd()` success and failure.
- `now()` wall-clock behavior and `sleep(ms)` duration.
- No `YASHY_PATH` in the environment.
- Runtime signature tests, recursion, and `-> i64`.
- Leak tests at zero live allocations.

**Gate:** tests pass on Windows and Linux; docs exist; no `YASHY_PATH` requirement.

**Cut order:** timezone/date formatting, HTTP, logging.

**Known risk:** the runtime entry-signature change can break existing programs.
Keep the Yashy-level `fn main()` parameterless; add hidden `argc/argv` only at the
LLVM entry; run the full e2e suite.

### 7.3 Slice 3 — Diagnostics, benchmarks, and AI contract v0

**Goal:** errors teach; performance is measured; agents can read results.
**Estimate:** 12–15 days; split into 3a/3c if it exceeds 15.

**Deliverables:**

- Stable error codes (`E....`).
- "Did you mean" for identifiers, fields, methods, imports, and missing `?`.
- `yashy check --json` and `yashy test --json` with schema v1.
- `yashy explain <code>`.
- Diagnostic corpus tests that assert code, span, and fix where available.
- Benchmark gate: compile, binary size, and runtime.
- `key=value` benchmark artifacts with no dependencies.
- Baselines for Windows and Linux.
- Top-three performance fixes plan.
- `AGENTS.md` and `llms.txt` AI contract v0.

**Tests:**

- JSON schema tests for `check` and `test`.
- Diagnostic corpus: 90% of common errors have code + span.
- Benchmark gate runs on PR (report-only) and scheduled/manual (hard gate).
- `explain` returns the catalog entry.
- Artifacts parse with a shell script.

**Gate:** JSON schema v1; 90% common errors; benchmark gate; `explain`;
`AGENTS.md`/`llms.txt`; docs.

**Cut order:** LSP completion/rename, `fix`, `index`, MCP, sandbox.

**Known risk:** schema churn. Freeze v1 at R1; additive changes only within 0.x.

### 7.4 Slice 3b — Top-three performance fixes

**Goal:** close the three worst benchmark rows before 0.2.0.

**Start condition (recorded 2026-10-26):** slice 3's diagnostics work and the
benchmark gate are green on Windows and Linux, and the three fixes below are
chosen from slice 3's baseline. Do not start 3b before both hold.

**Chosen (slice 3 plan, Task 8):** array mapping (3.76× published, 3.62× fresh),
debug formatting (3.42× / 3.31×), tuples (1.79× / 1.52×). Order: array mapping,
debug formatting, tuples. Checked arithmetic and the map-walk copy are not among
them; the slice 3 plan records why.

**Deliverables:**

- Array mapping: 3.76× → target ≤ 2×.
- Debug formatting: 3.42× → target ≤ 2×.
- Tuples: 1.79× → target ≤ 1.5×.
- Before/after benchmarks on Windows and Linux.
- No regressions in other rows.
- A short design note for each fix.

**Gate:** each row improves; no row > 2× without a recorded exception; benchmark
gate green.

**Cut order:** M9, PGO/LTO, SIMD (later releases).

**Known risk:** optimization breaks semantics. Differential tests and the full
suite are mandatory.

### 7.5 Slice 4 — Package manager spike

**Goal:** de-risk dependencies and design a package manager that is safe for
agents.

**Deliverables:**

- `yashy.toml` manifest and `yashy.lock`.
- Path and git dependencies.
- The dependency's `src/` is the import root; name collisions are errors.
- `yashy pkg add`, `remove`, `tree`, and `audit`; all support `--json`.
- No build scripts.
- Pinned commits; the checksum design is recorded for the R2 MVP.
- Offline/cache mode.
- Registry design document; not implemented.
- Tests for path/git deps, lockfile reproducibility, collisions, offline, audit,
  and JSON.

**Gate:** a path and a git dependency build and test; lockfile is reproducible; a
collision produces a clear error; no build scripts; audit works; `--json` and
`--offline` work; docs exist.

**Cut order:** registry, semver resolution, publishing, signing (1.0).

**Known risk:** resolver scope creep. Path/git only; no registry.

### 7.6 Order and gates

| Order | Slice | Gate |
|---|---|---|
| 1 | Install + first CLI | Clean install, first program, doctor + JSON, uninstall, checksum, guide, AGENTS/llms |
| 2 | Runtime + stdlib | args/getcwd/now/sleep, no `YASHY_PATH`, docs |
| 3 | Diagnostics + benchmarks + AI v0 | JSON schema, 90% errors, benchmark gate, AGENTS/llms |
| 3b | Top-three performance fixes | Rows improve, no > 2× without exception |
| 4 | Package manager spike | Path/git, lockfile, collision, no build scripts, audit, `--json`, `--offline` |

No slice starts before the previous slice's gate is met or an exception is
recorded. No code starts before the spec, this master plan, and the slice plans are
approved.

### 7.7 Execution notes

- The full driver suite takes roughly 5 minutes. Run the targeted suite during a
  slice and the full suite at the gate.
- `cargo fmt --all --check` has pre-existing drift in untouched files. Format
  touched files; do not let unrelated drift block a slice.
- Windows Application Control may block freshly linked or copied binaries; pad
  launch copies with 4096 random bytes. Never ship the padding.
- WSL builds use `CARGO_TARGET_DIR=/tmp/yashy-target-linux` to avoid clobbering
  the Windows `target`.
- Keep the Yashy-level `fn main()` surface stable. Hidden ABI details do not leak
  into syntax.

---

## 8. Post-R1 slices (5–12)

Post-R1 work is planned, not started. It begins after the first execution phase is
complete and reviewed. Slices 5–9 are two-week slices; slices 10–12 are
phase-sized and may split into multiple two-week units.

| Slice | Release | Outcome | Gate |
|---|---|---|---|
| **5. Fast dev loop** | R1.5 | `run`/`test` `-O0`; `build` `-O2`; `--release` `-O3`; fast linker; content cache; `watch`; parallel tests. | Incremental < 200 ms; 100 tests < 2 s; no semantic change. |
| **6. Safety hardening** | R1.5 | Bounds/overflow policy; `unsafe` audit; fuzz; ASan/UBSan/TSan; reproducible prototype. | Zero known UB; sanitizer CI green; fuzz 7 days. |
| **7. LSP, fix, doc** | R2 | Completion, references, rename, signature help; `fix`; `doc`; stack traces; code actions. | LSP conformance; fix corpus; doc coverage; panic traces. |
| **8. HTTP, logging, stdlib** | R2 | HTTP client/server; `log`; `cli`; `net`; `test`/`bench`/`fuzz`; JSON improvements; precompiled stdlib; M9 escape analysis; service guide. | Real HTTP service with tests; logging guide; CLI parser examples; M9 benchmark. |
| **9. Package manager MVP** | R2 | Manifest/lock finalization; path/git; add/remove/update/tree/audit; offline; no build scripts; local registry prototype. | Real package with a dependency; lockfile reproducible; audit; offline. |
| **10. Three flagship tools** | R2 | CLI tool; automation/data tool; small HTTP service; tests, docs, artifacts. | All three build/run on Windows/Linux; docs; benchmarks. |
| **11. AI index, context, MCP** | R2.5 | `index`/`outline`/`graph`/`search`/`context`; MCP v0; sandbox v0; agent benchmark v0; schema stabilization. | Context ≤ 8k tokens; MCP conformance; sandbox escape tests; schema diff CI; benchmark v0. |
| **12. 1.0 hardening** | 1.0 | macOS; registry/signing; reproducible builds; book/API/cookbook; debugger/profiler; playground; LSP code actions; `lint`/coverage/`task`; MCP v1; sandbox v1; schema freeze; agent benchmark 90%. | All spec §7 metrics; no P0; reproducible; signed; 90% agent benchmark. |

**Dependency note:** slice 11 (AI) depends on the index/LSP/diagnostic work from
slices 3 and 7, and on the package graph from slice 9. Slice 12 depends on all
previous slices. Do not start slice 11 before slice 9's graph is stable.

---

## 9. AI-native contract (execution summary)

The full contract is spec §13. This is the execution checklist.

### 9.1 Normative rules

1. Human and JSON output carry the same facts.
2. JSON has a `schema_version` and a stable shape.
3. Every diagnostic has a span; every coded diagnostic has a code and a docs URL.
   R1 targets 90% of the common corpus; 1.0 requires 100%.
4. Every auto-fixable diagnostic has structured edits.
5. Exit codes: `0` success, `1` diagnostics/tests failed, `2` usage/config error,
   `3` internal error.
6. No interactive prompt in `--json` or non-TTY mode.
7. No secrets in output.
8. No network access unless the command is explicitly a network command.
9. No code execution unless the command is `run`, `test`, `bench`, or `mcp` run.
10. Every command documents its schema in `yashy help --json`.

### 9.2 Commands and schemas

| Command | JSON | Release |
|---|---|---|
| `yashy check --json` | Diagnostics | R1 |
| `yashy test --json` | Tests + diagnostics | R1 |
| `yashy doctor --json` | Environment + toolchain | R1 |
| `yashy explain --json` | Error catalog entry | R1 |
| `yashy build --json` | Artifacts + timing | R1.5 |
| `yashy index --json` | Symbols/imports/tests | R2.5 |
| `yashy outline --json` | Symbols | R2.5 |
| `yashy graph --json` | Module/dependency graph | R2.5 |
| `yashy context --json` | Context pack | R2.5 |
| `yashy fix --json` | Edits + status | R2 |
| `yashy doc --json` | API docs | R2 |
| `yashy audit --json` | Dependencies + unsafe | R2 |
| `yashy mcp` | MCP tools | R2.5 v0, 1.0 v1 |

### 9.3 Index, context, and explain

- `yashy index` writes `.yashy/index.json` or stdout.
- The index contains packages, modules, symbols, signatures, docs, imports, tests,
  entry points, and diagnostics.
- `yashy context <symbol>` builds a small pack: definition, signature, docs,
  callers, callees, tests, diagnostics. `--max-tokens` caps it.
- `yashy explain <code>` is the human form; `--json` is the agent form.
- Context packs are deterministic for the same index and query.

### 9.4 MCP and sandbox

`yashy mcp` exposes `check`, `build`, `test`, `fmt`, `explain`, `context`,
`search`, `index`, and a **sandboxed** `run`. It never exposes arbitrary shell
execution.

The sandbox is default-deny:

```toml
[capabilities]
fs = ["read:src/**", "write:build/**"]
net = []
env = ["CI"]
process = false
```

- Resource limits: CPU, memory, output, file size.
- Deterministic mode: fixed time, fixed RNG seed, fixed env.
- Escape tests are adversarial and part of CI.
- Sandbox v0 is R2.5; v1 is 1.0.

### 9.5 Docs and agent benchmark

- `AGENTS.md`: build/test/run commands, style, error handling, package rules,
  sandbox rules, docs links.
- `llms.txt`: compact canonical map.
- `docs/ai/patterns.md`, `docs/ai/anti-patterns.md`, `docs/errors/`.
- Examples are tested; URLs are stable.
- The agent benchmark has 10 tasks, held-out variants, and measures success, time,
  tokens, tool calls, human interventions, and sandbox violations.
- Target: 90% success at 1.0; trend at R2.5.

---

## 10. Speed program (execution)

### 10.1 Budgets

| Budget | Target | Release |
|---|---|---|
| Hello compile | < 1 s | R1 |
| 10k-line compile | < 5 s | 1.0 |
| Incremental rebuild | < 200 ms | R1.5 |
| 100 tests | < 2 s | R1.5 |
| Scalar runtime | ≤ 1.2× C | 1.0 |
| Feature runtime | ≤ 2× C per row | 1.0 |
| Hello binary | < 500 KB | R1 |
| Binary regression | < 10% per release | 1.0 |

### 10.2 Work by release

- **R1:** benchmark gate; compile/binary/runtime baselines; top-three fixes (3b).
- **R1.5:** `run`/`test` `-O0`; `build` `-O2`; `--release` `-O3`; explicit `-O`
  wins; fast linker detection; content cache; `watch`; parallel tests.
- **R2:** M9 escape analysis; precompiled stdlib; parallel codegen; inline runtime
  calls; array/debug/tuple fast paths.
- **1.0:** bounds/overflow elision where provable; devirtualization; PGO/LTO;
  sustained gates.
- **1.1:** cross-compile build pipeline; `no_std` size profile.
- **2.0:** SIMD; self-hosting performance; ecosystem-scale builds.

### 10.3 Dev optimization decision

- `yashy run` defaults to `-O0`.
- `yashy test` defaults to `-O0`.
- `yashy build` defaults to `-O2`.
- `yashy build --release` defaults to `-O3`.
- An explicit `-O` always wins.
- The behavior is documented and tested. It is a deliberate change from v1.

### 10.4 Measurement

- Benchmark suite on Windows and Linux.
- Compile, build, test, runtime, and binary-size baselines per release.
- PRs report; scheduled/manual runs gate.
- Thresholds: compile +20%, binary +10%, runtime +20%.
- A row worse than 2× C needs an exception with an owner and a date.
- The top-three fixes are not optional before 0.2.0.

### 10.5 Risks

- LLVM compile time dominates. Mitigation: `-O0`, cache, parallel codegen.
- Cache invalidation is hard. Mitigation: content hashes, conservative invalidation,
  `cache clean`.
- Fast linker availability varies. Mitigation: detect, fall back, document.
- Optimization regresses correctness. Mitigation: differential tests, benchmark
  gate, sanitizer CI.

---

## 11. Safety program (execution)

### 11.1 Gates

- Zero known UB.
- 100% of `unsafe` blocks audited.
- Fuzz targets green for 7 days.
- ASan/UBSan/TSan green.
- Sandbox escape tests green.
- Reproducible build at 1.0.
- No build scripts in packages.

### 11.2 Work by release

- **R1:** bounds/overflow behavior documented and tested; `unsafe` inventory.
- **R1.5:** fuzz harnesses; sanitizer CI (ASan/UBSan/TSan); `audit --unsafe`.
- **R2:** dependency audit; lockfile; no build scripts.
- **R2.5:** sandbox v0; escape tests.
- **1.0:** sandbox v1; reproducible builds; signed releases; package audit.
- **1.1:** `no_std` safety subset; embedded notes.
- **2.0:** trusted-core invariants; self-hosted safety.

### 11.3 Safety model

- Ownership and reference counting; no GC, no borrow checker.
- No null; `Option[T]` is absence.
- Bounds checks on; debug overflow checks; explicit wrapping/checked/saturating.
- `unsafe` is a small, audited surface.
- FFI is `unsafe`; C strings are checked conversions.
- Packages cannot execute build scripts.
- Sandbox is default-deny.
- Evidence: tests, fuzzing, sanitizers, audits, reproducible builds.

### 11.4 Risks

- "No borrow checker" is read as "unsafe." Mitigation: document the model, prove it
  with tests, keep `unsafe` rare and visible.
- Sandbox escapes. Mitigation: default deny, escape tests, review.
- Supply-chain attacks. Mitigation: no build scripts, lockfile, audit, signing.
- Sanitizer noise. Mitigation: clean baseline and triage.
- Reproducibility across platforms. Mitigation: per-platform first.

---

## 12. DX/UX program (execution)

### 12.1 DX by release

- **R1:** error codes; did-you-mean; `check --json`; `test --json`; `explain`;
  formatter idempotence; `doctor --json`; `init` writes `AGENTS.md`.
- **R1.5:** `watch`; parallel tests; cache visibility; stack traces.
- **R2:** LSP completion/references/rename/signature; `fix`; `doc`; package manager;
  `audit`; completions; man pages; `upgrade`/`uninstall`.
- **R2.5:** `index`; `outline`; `graph`; `search`; `context`; MCP v0.
- **1.0:** LSP code actions/inlay hints; debugger/profiler; coverage; `lint`;
  `task`; playground; book/API/cookbook.
- **1.1:** call/type hierarchy; `expand`; cross-compile diagnostics.

### 12.2 UX by release

- **R1:** Windows/Linux install; checksums; PATH; uninstall; doctor; first CLI;
  quickstart.
- **R2:** completions; man pages; `upgrade`; `env --json`; templates;
  troubleshooting.
- **1.0:** macOS; signed installers; version manager; playground; full docs.
- **1.1:** embedded/WASM install guides; cross-compile toolchains.

### 12.3 DX/UX metrics

- Clean install to first program < 5 min.
- First test < 1 min.
- 90% common errors with code + span + fix.
- LSP conformance in VS Code/Neovim/Zed.
- Formatter idempotence.
- Doctor resolves 90% of known cases.
- Uninstall leaves no residue.
- Completions for four shells.

### 12.4 Risks

- LSP scope creep. Mitigation: capability table, conformance tests, cut list.
- Diagnostics without fixes. Mitigation: prioritize common errors.
- Package manager becomes a second compiler. Mitigation: no build scripts, small
  resolver, path/git first.
- Platform installers are a maintenance tax. Mitigation: tarballs first, package
  managers where demand exists.
- Docs drift. Mitigation: tested examples, docs build gate.

---

## 13. Feature program (execution)

### 13.1 Priorities

| Priority | Meaning | Release |
|---|---|---|
| P0 | Without it, the release is not usable | R1 |
| P1 | Without it, the release is not competitive | R2/R2.5 |
| P2 | Without it, the release is not stable | 1.0 |
| P3 | Without it, the language is not complete | 1.1 |
| P4 | Without it, the language is not a super language | 2.0 |
| Won't | Identity or scope | Never |

### 13.2 P0 — R1

- Installers, `doctor`, first CLI, `args`/`now`/`getcwd`/`sleep`.
- Error codes, did-you-mean, `check --json`, `test --json`, `explain`.
- Benchmark gate, top-three fixes, package-manager spike.
- `AGENTS.md`, `llms.txt`.
- Safety baseline.

### 13.3 P1 — R2/R2.5

- Package manager MVP; HTTP/log/cli/net/test/bench/fuzz; LSP completion/rename;
  `fix`/`doc`/stack traces; M9; precompiled stdlib; three flagship tools; audit.
- AI index/outline/graph/search/context; MCP v0; sandbox v0; agent benchmark v0.

### 13.4 P2 — 1.0

- Semver; registry; macOS; HTTP hardening; book/API/cookbook; debugger/profiler;
  playground; package audit; reproducible/signed releases; LSP code actions;
  `lint`/coverage/`task`; MCP v1; sandbox v1; schema freeze; agent benchmark 90%.

### 13.5 P3 — 1.1

- Traits/interfaces; generic constraints; associated types; trait objects;
  iterators; declarative macros; Unicode; `--target`; `no_std`; WASM decision;
  embedded example; `expand`.

### 13.6 P4 — 2.0

- Async; self-hosting; ecosystem/frameworks; registry at scale; learner path; SIMD;
  const generics; MCP v2.

### 13.7 Top 20 features that make things easier

1. `yashy run` with a fast `-O0` default.
2. `yashy watch`.
3. `yashy test --json` with parallel execution.
4. `yashy fix`.
5. `yashy explain`.
6. `yashy doc`.
7. `yashy init --template`.
8. `yashy add`/`remove`.
9. `yashy audit`.
10. `yashy context`.
11. `yashy outline`.
12. `yashy graph`.
13. `yashy mcp`.
14. Stdlib `cli` parser.
15. Stdlib `log`.
16. Stdlib `http`.
17. Shell completions and man pages.
18. Panic stack traces.
19. Playground.
20. Package manager with no build scripts.

### 13.8 Acceptance

Every feature must have:

- A user story.
- Acceptance tests on Windows and Linux.
- Diagnostics with codes, spans, and fixes where possible.
- A docs page, tested.
- A JSON form if it has observable output.
- A performance note if it touches a budget.
- A safety note if it touches memory, concurrency, FFI, or packages.
- A release note.

### 13.9 Won't do

- Tracing GC.
- Borrow checker.
- Exceptions.
- Implicit coercion.
- Operator overloading.
- Class inheritance.
- Power-user procedural macros.
- Named/default arguments before 2.0.
- A GUI toolkit in the stdlib.

---

## 14. Capacity and cadence

- **One maintainer, full-time.** One risky slice at a time. Documentation,
  packaging, and benchmark work can overlap with a compiler slice only if it does
  not touch the same files.
- **Two-week slices.** Each slice has a written plan, a failing test first where
  the change is behavioral, and a demo at the end.
- **Weekly demo.** Run the slice's acceptance command on a clean checkout and paste
  the output into the plan. "It compiles" is not a demo.
- **Gate review.** Do not start the next slice until the current slice's gate is met
  or the cut list is applied and recorded.
- **Keep `main` green.** Every slice ends with `cargo check --workspace
  --all-targets`, the relevant tests on Windows and Linux, and a commit/push.
- **No silent scope growth.** A new idea goes into the backlog or the next
  release's plan, not into the current slice.
- **Quarterly re-plan.** Review metrics, cut list, and risks; adjust dates, not
  identity.
- **Sustainable pace.** No crunch. The cut list exists so the plan can survive
  reality.

**Execution notes:**

- The full driver suite takes roughly 5 minutes. Run the targeted suite during a
  slice and the full suite at the gate.
- `cargo fmt --all --check` has pre-existing drift in untouched files. Format
  touched files; do not let unrelated drift block a slice.
- Windows Application Control may block freshly linked or copied binaries; pad
  launch copies with 4096 random bytes. Never ship the padding.
- WSL builds use `CARGO_TARGET_DIR=/tmp/yashy-target-linux` to avoid clobbering the
  Windows `target`.
- Keep the Yashy-level `fn main()` surface stable. Hidden ABI details do not leak
  into syntax.

---

## 15. Risk register

| Risk | Likelihood | Impact | Mitigation | Trigger to act |
|---|---|---|---|---|
| Scope creep | High | High | Priorities, cut list, one owner | A slice grows > 20% |
| Linux artifact fails on older glibc | Medium | High | Build on `ubuntu-22.04`; document minimum; test clean container | Installer smoke fails |
| Runtime entry-signature change breaks programs | Medium | High | Keep `fn main()` parameterless; hidden `argc/argv`; full e2e | Any e2e/tutorial change |
| Benchmark noise causes false gates | High | Medium | Median of 3–5; generous thresholds; report-only PR; per-OS baselines | Green build fails only on timing |
| Package spike becomes implementation | High | Medium | Timebox to design + path/git; no registry/publishing/build scripts | Spike adds stable CLI or registry |
| Import/package mapping underspecified | Medium | High | Two-package experiment; dependency `src/` root; collision detection | Two deps export same module |
| macOS packaging slips | Medium | Medium | 1.0 target; portable installer logic; document gap | macOS user blocked before 1.0 |
| Self-hosting/async distract | Medium | High | Explicitly deferred; no work before release | Slice touches self-hosted compiler |
| Windows Application Control | High | Low (dev) | 4096-byte padding helper; never ship | Fresh binary blocked |
| Signing/notarization missing | Medium | Medium | 1.0 work; R1 checksums + instructions | OS refuses unsigned binary |
| Compile speed stalls | Medium | High | `-O0`, cache, parallel codegen, fast linker | Hello > 1 s |
| "No borrow checker" read as unsafe | Medium | High | Document model, tests, audit, sandbox | User report |
| AI schema drift | Medium | High | Versioning, diff CI, freeze at 1.0 | Breaking diff |
| Sandbox escape | Low | Critical | Default deny, escape tests, review | Any escape |
| Supply-chain attack | Medium | High | No build scripts, lockfile, audit, signing | Advisory |
| Agent benchmark overfitting | Medium | Medium | Held-out tasks, public harness | Score jump without tooling change |
| Burnout | Medium | High | Sustainable pace, cut list, breaks | Two missed weeks |
| Docs drift | Medium | Medium | Tested examples, docs build gate | Broken example |

---

## 16. Cut list and invariants

### 16.1 Cut order

- **R1:** macOS, completions, man pages, package-manager install, version manager;
  then `rand`, non-essential suggestions, extra examples, benchmark runtime rows
  (keep compile time and binary size). Keep `now`/`sleep`; they are P0.
- **R1.5:** `watch`, parallel tests, fast linker (keep cache and `-O0`).
- **R2:** third flagship tool (keep two), `fix` extras, `doc` extras,
  `bench`/`fuzz` helpers.
- **R2.5:** MCP v0, agent benchmark v0, `search`, `graph` (keep index, outline,
  context, sandbox v0).
- **1.0:** playground, profiler, coverage, `lint`, `task`.
- **1.1:** WASM implementation, trait objects, embedded support (keep traits,
  iterators, macros, Unicode, cross-compilation, `no_std`).
- **2.0:** SIMD, const generics, ecosystem frameworks (keep async, self-hosting,
  registry at scale).

### 16.2 Do not cut

- Language identity.
- Windows/Linux install.
- First CLI.
- `args`/`getcwd`/`now`/`sleep`.
- Error codes and JSON diagnostics.
- Benchmark gate.
- Top-three performance fixes.
- Package manager spike.
- No build scripts.
- Sandbox default deny.
- Safety gates.
- Spec/master/slice approval before code.

### 16.3 Invariants

1. No tracing GC.
2. No borrow checker.
3. No exceptions.
4. No implicit coercion.
5. No operator overloading.
6. No class inheritance.
7. No power-user procedural macros.
8. No hidden runtime.
9. Deterministic builds.
10. Every feature pays rent: tests, docs, diagnostics, JSON.
11. No performance claim without a benchmark.
12. No agent gets ambient authority.
13. No code before approval.
14. `main` stays a normal Yashy function; hidden ABI details do not leak.
15. Every owned value has one release path; leak tests stay at zero live
    allocations.
16. A release gate is never silently dropped; record the cut and the reason.

---

## 17. Decisions

Settled with the user on 2026-10-25. Each item records the decision and the
alternative that was rejected, so a later reader does not reopen it by accident.

### 17.1 Preserved from v1

1. **Build order: 1 → 2 → 3 → 3b → 4.** Slice 1's guide reads standard input;
   slice 2 adds `args()` and updates the guide.
2. **`args()` excludes the program path.** `args()[0]` is the first user argument
   after `yashy run --`. Rejected: C/Rust-style inclusion, which makes every CLI
   use `args()[1]`.
3. **`getcwd()` lives in `os`.** It is a builtin documented in `os.ya`. Rejected:
   `fs.cwd()`, which confuses process state with file contents.
4. **Fallible builtins use the R1 convention.** `getcwd()` returns `""` on
   failure; `args()` is infallible (empty slice). Builtin `Result` signatures are
   R2 work. Rejected: implementing `Result` in slice 2, which would enlarge the
   checker's type surface and delay the runtime gaps.
5. **The time function is `now()` in module `time`.** Rejected: `time()` as a
   function name, which reads like a module/type.
6. **Benchmark gate.** Report-only on pull requests; hard gate on
   scheduled/manual runs; medians; per-OS baselines. Thresholds: compile +20%,
   binary +10%, runtime +20%, calibrated by slice 3.
7. **Package imports.** Each dependency's `src/` is a module root; imports use the
   module name; a collision is an error naming both packages. Rejected for the R2
   MVP: package-prefixed imports, which need a loader change. Revisit if the
   ecosystem outgrows unique module names.
8. **macOS stays a 1.0 target.** R1 ships `doctor` guidance, not an installer.
9. **Performance fixes are slice 3b.** Choose the top three from slice 3's
   baseline and fix them before 0.2.0. Candidates: checked-arithmetic elision,
   map/array paths, string/debug formatting, interned identifiers, iterative
   parser, one IR buffer. Each fix needs before/after numbers.
10. **Windows arguments use the C `char**` encoding in R1.** The code-page
    limitation is documented; `getcwd()` returns UTF-8 via the wide API. Revisit
    with Unicode strings in 1.1. Rejected: `GetCommandLineW` now, which adds
    `shell32` linking and conversion code.
11. **The package spike uses a hand-written TOML subset.** It preserves the
    offline, dependency-light workspace and covers the R2 MVP schema. The `toml`
    crate is evaluated for the R2 MVP. Provisional until the slice-4 spike reports
    its cost.
12. **Benchmark artifacts are `key=value` lines.** The `yashy-bench` comparator
    has no external dependencies, and the workspace stays offline. JSON can
    replace the format later without changing the gate logic.
13. **Slice 3b is scheduled, not optional.** If a fix slips, record the exception
    and the measured reason in this plan.

### 17.2 New in v2

14. **Balanced sequencing.** Speed + AI-native foundation in R1/R2/R2.5; features
    in 1.0/1.1; async/embedded/WASM/self-hosting in 2.0.
15. **Six workstreams.** Speed, Safety, DX, UX, Features, AI-native.
16. **AI contract is normative.** JSON, codes, spans, fixes, docs, exit codes.
17. **JSON schema v1 at R1; additive within 0.x; frozen at 1.0.**
18. **Dev optimization.** `run`/`test` `-O0`; `build` `-O2`; `--release` `-O3`;
    explicit `-O` wins.
19. **Sandbox.** Default deny; capability manifest; resource limits.
20. **Packages.** No build scripts; lockfile; checksums at R2 MVP; signed registry
    at 1.0.
21. **Agent benchmark.** Public harness; 90% gate at 1.0.
22. **R2 and R2.5 are separate phases.** R2 does not absorb R2.5.
23. **Every feature pays rent.** Tests, docs, diagnostics, and a machine-readable
    form.
24. **No code before approval.** Spec, master, and slice plans first.

### 17.3 Deferred decisions

| Decision | When |
|---|---|
| `defer` acceptance | R2 design, 1.1 ship if explicit |
| WASM implementation | 1.1 decision, 2.0 implementation |
| Async implementation | 1.1 decision, 2.0 implementation |
| Registry federation | 2.0 |
| `yashyup` version manager | 1.0 if demand |
| Exact binary-size budget | R1 baseline, tighten at 1.0 |
| `lint` core vs ecosystem | 1.0 review |
| Named/default arguments | 2.0 review |

---

## 18. Definition of done for this plan set

- The v2 spec is reviewed and approved.
- This master plan is reviewed and approved.
- Slice plans 1, 2, 3, and 4 are updated from the v2 spec and approved.
- The slice 3b section is approved; its detailed plan is written after slice 3's
  baseline.
- The phased implementation plan (`2026-10-25-yashy-super-language-implementation-phases.md`)
  is reviewed and approved.
- Each slice and phase has a gate command that can be run on a clean checkout.
- The first slice can start without unresolved design work.
- No code has started.

**Implementation done means:**

1. Slices 1 → 2 → 3 → 3b → 4 pass their gates.
2. 0.2.0 ships with the R1 must-ship list.
3. Post-R1 work starts only after a review of 0.2.0.
4. Every release meets §5 or records an exception.
5. The agent benchmark, benchmark gate, and safety gate are green at 1.0.

---

## 19. Slice plan index

| Slice | Plan | Status |
|---|---|---|
| 1 | `2026-10-25-yashy-r1-install-first-cli.md` | Implemented and pushed, `d30e1af` |
| 2 | `2026-10-25-yashy-r1-runtime-stdlib.md` | Implemented and pushed, `d30e1af` |
| 3 | `2026-10-25-yashy-r1-diagnostics-benchmarks.md` | Updated from v2 spec §15.3; ready for approval |
| 3b | This master plan, §7.4 | Section approved; detailed plan after slice 3 baseline |
| 4 | `2026-10-25-yashy-r2-package-manager-spike.md` | Updated from v2 spec §15.5; ready for approval |

**Phase plan:**

| Phase | Plan | Status |
|---|---|---|
| 0–7 | `2026-10-25-yashy-super-language-implementation-phases.md` | Written; ready for approval |

**Alignment checklist (completed):**

- Slice 1: added `doctor --json`, `AGENTS.md`/`llms.txt`, checksum/uninstall tests,
  Linux CI artifact, and the v2 gate.
- Slice 2: aligned on `now()` (not `time()`), `sleep(ms)`, Windows code-page note,
  no-`YASHY_PATH` test, and the v2 gate.
- Slice 3: added error codes, `check --json`, `test --json`, `explain`, the JSON
  schema v1, and the AI v0 deliverables.
- Slice 3b: detailed plan is written from the slice 3 baseline; targets are array
  ≤ 2×, debug ≤ 2×, tuples ≤ 1.5×.
- Slice 4: added `--json`, no build scripts, the checksum design, offline mode,
  audit, and the registry design doc.

**Do not start slice 1 until this plan set is approved.**




