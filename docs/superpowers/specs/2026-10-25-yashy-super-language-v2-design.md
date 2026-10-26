# Yashy Super-Language Design v2 — Fast, Safe, AI-Native

**Date:** 2026-10-25
**Status:** Draft for review
**Author:** Brainstormed with the project owner
**Supersedes:** the roadmap sections of `2026-10-24-yashy-super-language-design.md`.
The language identity, permanent non-goals, and historical decisions remain in force.

---

## 1. Summary

Yashy is a native, statically typed, ownership-based programming language with no
tracing GC, no borrow checker, no exceptions, no implicit coercion, no operator
overloading, and no class inheritance. It compiles to native code through LLVM and
ships as a single binary with no external runtime.

The v1 plan made Yashy **usable**. The v2 plan makes it **excellent**.

The v2 thesis is:

> **Yashy is the fastest way for a human or an AI agent to build a small, fast,
> dependency-free native tool — and it scales to real services.**

"Super language" is not a claim that every feature exists. It is a claim about the
whole experience: fast to compile, fast to run, safe by default, pleasant to learn,
powerful in production, and legible to both people and machines.

The sequencing is **Balanced**:

1. **R1/R2:** make the compiler fast, the diagnostics machine-readable, the tooling
   AI/editor-friendly, and the package manager real.
2. **1.0/1.1:** add the big language features — traits, generic constraints,
   iterators, macros, Unicode, cross-compilation, `no_std`, WASM, embedded.
3. **2.0:** async implementation, self-hosting, ecosystem, and scale.

Safety is a **gate at every release**, not a phase. Performance is a **measured
property**, not a slogan. AI-friendliness is a **compiler and toolchain contract**,
not a chatbot bolted onto the side.

The first execution phase — slices 1, 2, 3, 3b, and 4 — remains the immediate
work. It is expanded in §15 and is the only work that starts before this spec and
the rewritten master plan are approved.

---

## 2. Relationship to the v1 spec and plans

This spec **extends** the v1 design rather than replacing its identity.

| Area | v1 | v2 |
|---|---|---|
| Positioning | CLI-first, performance spine | Fast, safe, AI-native general-purpose language |
| Domains | CLI, automation/data, backend, embedded, learners | Same five, plus the AI-agent user |
| Sequencing | CLI first, then breadth | Balanced: speed + AI-native foundation, then features |
| Performance | Spine across releases | Dedicated workstream with budgets and gates |
| Safety | Implicit in the language design | Dedicated workstream with audits, fuzzing, sandboxing |
| DX | Tooling workstream | Dedicated workstream with measurable targets |
| UX | Installer/doctor | Dedicated workstream covering the whole first-run experience |
| AI | Not a first-class concern | Dedicated workstream with a normative contract |
| Features | Release must-ship lists | Prioritized catalog with user value and acceptance |

The v1 spec's permanent non-goals remain permanent:

- No tracing GC.
- No borrow checker.
- No exceptions.
- No implicit coercion.
- No operator overloading.
- No class inheritance.
- No power-user procedural macros.

Historical specs and plans remain in the repository as history. This spec is the
current product and engineering direction. The master plan is the execution index;
the slice plans are the units of work.

---

## 3. Where Yashy is today (2026-10-25)

Yashy is at **0.1.0**. It is a real language with a real compiler, not a prototype.

### 3.1 Compiler and runtime

- Rust workspace with `yashy-lexer`, `yashy-parser`, `yashy-sema`, `yashy-codegen`,
  `yashy-driver`, and `yashy-lsp`.
- Textual LLVM IR plus `clang` produces native executables.
- Windows and Linux x86-64 are supported. There is no cross-compilation yet.
- No external runtime; the generated binary links the Yashy runtime object.
- The driver has `build`, `check`, `fmt`, `run`, `test`, `init`, and `doctor`.
- `run` forwards arguments after `--`.
- `--release` defaults to `-O3`; an explicit `-O` level overrides it.
- Bundled stdlib discovery works without `YASHY_PATH`.
- A Windows installer and package exist. Linux installation is not yet first-class.
- The benchmark smoke exists on Windows but is not a CI gate.

### 3.2 Language

- Primitive types: `int`, `float`, `bool`, `char`, `string`, `void`.
- Structs, enums, `Option`, `Result`, `?`, arrays, fixed arrays, tuples.
- Pattern matching with guards.
- Generics (types and functions), closures, methods, and objects.
- Modules with `pub` visibility.
- FFI through `extern`.
- Threads, channels, and mutexes.
- Checked arithmetic builtins.

### 3.3 Standard library

- Core modules: `alloc`, `io`, `math`, `os`, `string`.
- Builtins: `read_file`, `write_file`, `read_line`.
- Real modules: `path`, `json`, `rand`.
- Documented but partial: `fs`, `env`, `process`.
- Missing: `args`, `now`, `getcwd`, `sleep`, HTTP, logging, CLI parsing.
- No package manager yet.

### 3.4 Tooling and docs

- Formatter and language server.
- LSP diagnostics, hover, definition, and semantic tokens.
- Editor modes for VS Code, Neovim, and Zed.
- Tutorial, README, language spec, and a vision document.
- No book, API reference site, or playground.

### 3.5 Performance baseline

- Scalar benchmark rows are competitive with the C baselines.
- Feature rows include array mapping at **3.76×**, debug formatting at **3.42×**,
  and tuples at **1.79×** the C baseline.
- `fib` compiles in roughly **259 ms**.
- Benchmarks are not yet a hard gate.

### 3.6 The honest gap

Yashy can build real programs, but it is not yet a language a stranger can adopt
without friction, and it is not yet a language an agent can operate without
guessing. The gap is not "more syntax." The gap is **speed, safety, feedback,
distribution, and machine-readable truth**.

---

## 4. What v2 changes

v2 adds six workstreams. They are not six separate roadmaps; they are six lenses on
the same compiler, runtime, and toolchain.

| Workstream | v1 had | v2 adds |
|---|---|---|
| **Speed** | A performance spine and benchmark smoke | Compile/build/test budgets, incremental cache, fast dev loop, runtime work, binary-size budget, hard gates |
| **Safety** | Ownership/refcounting and checked arithmetic | Bounds/overflow policy, `unsafe` audit, fuzzing, sanitizers, sandboxed execution, supply-chain security, reproducible builds |
| **DX** | Diagnostics, LSP, formatter, test runner | Error codes, JSON output, fixes, completion/rename, watch mode, debugger/profiler, templates, docs |
| **UX** | Installer/doctor | First-run experience, version manager, completions, man pages, upgrade/uninstall, playground, accessibility |
| **Features** | Release must-ship lists | Prioritized catalog with user value, acceptance, and release mapping |
| **AI-native** | Not a first-class concern | Stable JSON schemas, semantic index, context packs, MCP, sandbox, `AGENTS.md`/`llms.txt`, agent benchmark |

The rule that binds them:

> **Every feature ships with tests, docs, diagnostics, and a machine-readable form.
> No exceptions.**

That rule is what makes the language "for humans and AI" instead of "for humans,
with an AI integration later."

---

## 5. Identity and design principles

### 5.1 Non-negotiables

These are not trade-offs to revisit when a feature is inconvenient:

1. **No tracing GC.** Ownership and reference counting are visible.
2. **No borrow checker.** The compiler does not infer a global lifetime graph.
3. **No exceptions.** Errors are `Result` values; panics are explicit.
4. **No implicit coercion.** Conversions are written.
5. **No operator overloading.** Operators mean one thing.
6. **No class inheritance.** Composition and traits/interfaces are the tools.
7. **No hidden runtime.** The generated binary does not require a VM.
8. **No power-user procedural macros.** Declarative macros are allowed; arbitrary
   compile-time code execution is not.

### 5.2 Ten design principles

1. **Explicit over implicit.** If the reader has to guess, the language is wrong.
2. **Deterministic over convenient.** Same input, same output, same binary.
3. **Fast feedback over big batches.** The dev loop is a product feature.
4. **Safe by default, fast by opt-in.** Checks are on unless the programmer
   explicitly asks for the unchecked path.
5. **One obvious way.** Multiple spellings of the same idea are a cost.
6. **Machine-readable everything.** Every human output has a structured form.
7. **No hidden allocation, control flow, or global state.** Cost is visible.
8. **Every feature pays rent.** Tests, docs, diagnostics, and a JSON form.
9. **Measure before optimizing.** A performance claim without a benchmark is a
   rumor.
10. **Humans first, agents equal.** Both need clarity; agents additionally need
    stable schemas, deterministic builds, and a sandbox.

### 5.3 What "safe" means without a borrow checker

Yashy's safety model is:

- **Ownership and reference counting** for heap values.
- **Stack allocation** where escape analysis proves it is safe.
- **Bounds checks** on indexing unless explicitly unchecked.
- **Overflow checks** in debug builds; explicit wrapping/checked/saturating in
  release.
- **`Result` for recoverable errors**, panic for programmer errors.
- **A small, audited `unsafe` surface** for FFI and low-level runtime work.
- **Tests, fuzzing, sanitizers, and reproducible builds** around the trusted core.

This is not "memory safety by theorem." It is "memory safety by construction,
checks, and evidence." The spec is honest about that distinction.

### 5.4 What "fast" means

Fast has five separate budgets:

| Budget | What it measures | Why it matters |
|---|---|---|
| Compile | Time from source to object | The edit/check loop |
| Build | Time from source to runnable artifact | The developer and CI loop |
| Test | Time from change to signal | Confidence and iteration |
| Runtime | Time and memory of the produced program | The user's product |
| Size | Bytes of the produced artifact | Distribution and startup |

A change that improves one budget by hurting another must say so and justify it.

### 5.5 What "AI-native" means

AI-native does **not** mean "an AI writes Yashy." It means:

- The compiler emits **stable, structured facts** about code and errors.
- The language is **explicit and deterministic**, so a model can predict it.
- The toolchain can produce a **small, relevant context pack** for a task.
- An agent can **run, test, and fix** code in a **sandbox** with declared
  capabilities.
- The docs have a **machine-readable canonical form** (`llms.txt`, `AGENTS.md`,
  error catalog, examples).
- The project has an **agent benchmark** that measures whether any of this works.

The agent is a first-class user, but not a privileged one. It gets no ambient
authority the human does not get.

---

## 6. Target users and jobs

Yashy serves six users. The first five are human; the sixth is an AI agent acting
on a human's behalf.

| User | Primary jobs | Current pain | Success looks like |
|---|---|---|---|
| **Maya — CLI/dev tools** | Build small native tools, scripts, and utilities | Install friction, slow dev loop, missing argument/time APIs | One command to install; `yashy run` starts fast; single binary |
| **Dev — automation/data** | Read files, parse JSON, call HTTP, automate tasks | Stdlib gaps, no package manager, weak errors | Script feels as easy as Python, but ships as a native binary |
| **Sam — backend** | HTTP services, concurrency, deployment, observability | No HTTP/logging, no production guide, no profiler | A small service in one file; deployable binary; measurable |
| **Ana — embedded/edge** | Cross-compile, small binaries, deterministic behavior | No cross-compilation, no `no_std`, no embedded story | Cross-compiled binary with a size budget and no runtime |
| **Lee — learner** | Learn programming, build a first project | Jargon, weak first-run docs, no playground | First program in five minutes; errors that teach |
| **Ada — AI agent** | Install, inspect, edit, test, fix, package, explain | No JSON output, no index, no sandbox, no stable docs | Completes a task from docs and errors without a human |

The agent is not a replacement for the human. It is a user whose interface happens
to be a tool call instead of a keyboard.

---

## 7. North-star metrics and quality bar

Metrics are release gates, not aspirations. A release that misses a gate needs a
recorded exception with a reason and a date.

### 7.1 First-run and UX

| Metric | Target | Measurement | Release |
|---|---|---|---|
| Clean install to first program | < 5 minutes | Timed script on clean Windows/Linux/macOS | R1 (Win/Linux), 1.0 (macOS) |
| First test run | < 1 minute after `init` | Timed script | R1 |
| `doctor` resolves common failures | 90% of known cases | Doctor test matrix | R1 |
| Uninstall leaves no residue | 100% of documented paths | Clean-VM test | R1 |
| Shell completions | bash, zsh, fish, PowerShell | Completion tests | R2 |
| Man pages | All public commands | Docs build | R2 |

### 7.2 Speed

| Metric | Target | Measurement | Release |
|---|---|---|---|
| Hello-world compile | < 1 s | Compile benchmark, median of 20 | R1 |
| 10k-line project compile | < 5 s | Synthetic project benchmark | 1.0 |
| Incremental rebuild | < 200 ms | Watch-mode benchmark | R1.5 |
| 100-test suite | < 2 s | Test-runner benchmark | R1.5 |
| Scalar runtime | ≤ 1.2× C baseline | Benchmark suite, median | 1.0 |
| Feature runtime | ≤ 2× C baseline per row | Benchmark suite | 1.0 |
| No row worse than 2× | Without a recorded exception | Benchmark gate | R1 |
| Hello-world binary | < 500 KB | Size benchmark | R1 |
| Binary-size regression | < 10% per release | Size gate | 1.0 |

### 7.3 Safety

| Metric | Target | Measurement | Release |
|---|---|---|---|
| Known undefined behavior | Zero | Issue tracker + sanitizer CI | Every release |
| `unsafe` blocks audited | 100% | `yashy audit --unsafe` report | R1.5 |
| Parser/runtime fuzz targets | Green for 7 days | CI fuzz job | R1.5 |
| ASan/UBSan/TSan CI | Green | Linux CI matrix | R1.5 |
| Reproducible build | Byte-identical on same inputs | `yashy verify --reproducible` | 1.0 |
| Package build scripts | Zero | Manifest schema + audit | R1 (spike), R2 (MVP) |
| Sandbox escape tests | Zero known escapes | Adversarial test suite | R2.5 v0, 1.0 v1 |

### 7.4 Developer experience

| Metric | Target | Measurement | Release |
|---|---|---|---|
| Errors with code + span | 90% R1; 100% 1.0 | Diagnostic corpus | R1, 1.0 |
| LSP completion | Works in VS Code/Neovim/Zed | LSP conformance tests | R2 |
| LSP rename | Works across files | LSP conformance tests | R2 |
| Formatter idempotence | 100% of test corpus | `fmt --check` twice | R1 |
| `yashy fix` safe fixes | 80% of auto-fixable errors | Fix corpus | R2 |
| Panic stack traces | File/line/function | Panic tests | R2 |
| API docs | 100% of public stdlib | `yashy doc` coverage | 1.0 |

### 7.5 AI-native

| Metric | Target | Measurement | Release |
|---|---|---|---|
| JSON output for core tools | `check`, `build`, `test`, `doc`, `index`, `explain` | Schema tests | R1/R2/R2.5 |
| Stable schema version | No breaking change within a minor | Schema diff CI | 1.0 |
| Agent task success | ≥ 90% on the agent benchmark | Automated agent harness | 1.0 |
| Context pack size | ≤ 8k tokens for a typical task | Token count | R2.5 |
| MCP tools | `check`, `build`, `test`, `fmt`, `explain`, `context`, `search`, `run` | MCP conformance tests | R2.5 |
| Sandboxed run | Capability manifest enforced | Escape tests | R2.5 v0, 1.0 v1 |
| Docs for agents | `AGENTS.md`, `llms.txt`, error catalog | Docs build | R1 |

### 7.6 Ecosystem and reliability

| Metric | Target | Measurement | Release |
|---|---|---|---|
| Package manager MVP | Path + git deps, lockfile, audit | End-to-end tests | R2 |
| Registry | Public, signed, searchable | Registry tests | 1.0 |
| Flagship tools | 3 real tools built with Yashy | Repository + docs | R2 |
| CI platforms | Windows + Linux | CI matrix | R1 |
| macOS | Installer + tests | CI matrix | 1.0 |
| P0 bugs at release | Zero | Issue tracker | Every release |

### 7.7 The quality bar

Every feature, in every release, must have:

1. **A user story.** Who needs it and why.
2. **Acceptance tests.** What "done" means, executable.
3. **Diagnostics.** What happens when it is used wrong.
4. **Docs.** A human page and a machine-readable form.
5. **A machine-readable output** where the feature has observable results.
6. **A performance note** if it affects a budget.
7. **A safety note** if it touches memory, concurrency, FFI, or packages.

A feature without these is not done. It is a demo.

---

## 8. Workstream 1 — Speed

### 8.1 Goal

Make the edit/check/test/run loop fast enough that a developer or agent chooses
Yashy for a small tool without thinking about compile time.

### 8.2 Compile-time program

| Work item | Why | Release |
|---|---|---|
| Intern identifiers and reduce allocation in lexer/parser | Front-end is a fixed cost on every build | R1 |
| Iterative parser for deeply nested input | Avoid stack overflows and re-parsing | R1 |
| One IR buffer instead of repeated string concatenation | Codegen allocation pressure | R1 |
| Parallel module checking | Multi-file projects | R2 |
| Precompiled stdlib object cache | Every program links the same stdlib | R2 |
| Content-addressed module cache | Unchanged modules should not be recompiled | R1.5 |
| Parallel codegen per module | LLVM work is parallelizable | R2 |
| Fast linker detection (`lld`, `mold`) | Linking is a large fixed cost | R1.5 |
| `-O0` dev default for `run`/`test` | Optimization dominates the dev loop | R1.5 |
| `-O3` for `--release`, `-O2` for `build` | Match intent to cost | R1.5 |
| `-Os` size profile | Distribution and embedded | 1.0 |
| PGO/LTO for release builds | Runtime wins where they matter | 1.0 |

### 8.3 Build-system program

- **Project graph.** Packages, modules, tests, and examples are nodes.
- **Content-addressed cache.** Keyed by source hash, compiler version, target, and
  flags. Cache hits are visible in `--verbose` and `--json`.
- **Incremental rebuild.** Only invalidated modules are rechecked/recompiled.
- **Watch mode.** `yashy watch` rebuilds and reruns tests on change.
- **Parallel tests.** Tests run in parallel by default; `--test-threads` controls it.
- **Build daemon (later).** A warm process for repeated builds; not required for
  R2.
- **Remote cache (later).** For CI and teams; not required before 1.0.
- **Deterministic cache keys.** No timestamps, paths, or environment leakage.

### 8.4 Runtime program

| Work item | Target row | Release |
|---|---|---|
| M9 escape analysis and stack promotion | Allocations, closures | R2 |
| Copy elision and move-aware codegen | Structs, tuples | R2 |
| Inline runtime calls | Array/map/string operations | R2 |
| Array mapping fast path | 3.76× row | R1 (slice 3b) |
| Debug formatting fast path | 3.42× row | R1 (slice 3b) |
| Tuple representation and access | 1.79× row | R1 (slice 3b) |
| Bounds-check elision where provable | Index-heavy loops | 1.0 |
| Checked-arithmetic elision where provable | Numeric loops | 1.0 |
| LLVM `switch` lowering for matches | Pattern-heavy code | R2 |
| Devirtualization for known types | Method calls | 1.0 |
| SIMD for bulk operations | Strings, arrays | 2.0 |

### 8.5 Binary-size program

- Strip debug symbols in release by default.
- Section garbage collection.
- LTO for release.
- `-Os` size profile.
- Hello-world budget: < 500 KB at R1; tighten to < 300 KB at 1.0 if achievable
  without hurting startup.
- Size regression gate: < 10% per release.

### 8.6 Developer-loop program

The dev loop is a product surface:

- `yashy run` defaults to `-O0` for fast compilation.
- `yashy test` defaults to `-O0` and runs tests in parallel.
- `yashy build` defaults to `-O2`.
- `yashy build --release` defaults to `-O3`.
- An explicit `-O` always wins.
- `yashy watch` rebuilds and reruns on change.
- `yashy check --json` is fast enough for editor-on-save.
- Cache hits and misses are visible in `--verbose` and `--json`.

This is a deliberate behavior change from v1. It must be documented in the CLI
reference and covered by tests.

### 8.7 Measurement and gates

- Benchmark suite runs on Windows and Linux.
- Compile-time, build-time, test-time, runtime, and binary-size baselines are
  stored per release.
- PRs report; scheduled/manual runs gate.
- Thresholds: compile +20%, binary +10%, runtime +20%.
- A row worse than 2× C needs a recorded exception with an owner and a date.
- The top-three runtime fixes are slice 3b and are **not optional** before 0.2.0.

### 8.8 Sequence

| Release | Speed deliverables |
|---|---|
| R1 | Benchmark gate, compile baseline, binary-size baseline, top-three fixes (slice 3b) |
| R1.5 | `run`/`test` `-O0`, `build` `-O2`, `--release` `-O3`, fast linker detection, content cache, watch, parallel tests |
| R2 | Precompiled stdlib, M9, parallel codegen |
| 1.0 | Bounds/overflow elision, devirtualization, PGO/LTO, sustained gates |
| 1.1 | Cross-compilation build pipeline, `no_std` size profile |
| 2.0 | SIMD, self-hosting performance, ecosystem-scale builds |

### 8.9 Risks

- **LLVM compile time dominates.** Mitigation: `-O0` dev, caching, parallel codegen.
- **Incremental cache invalidation is hard.** Mitigation: content hashes, conservative
  invalidation, cache-clear command.
- **Fast linker availability varies.** Mitigation: detect, fall back, document.
- **Optimization work regresses correctness.** Mitigation: differential tests,
  benchmark gate, sanitizer CI.

---

## 9. Workstream 2 — Safety

### 9.1 Goal

Make the safe path the default path, make the unsafe path small and auditable, and
make the evidence visible.

### 9.2 Memory safety

- Ownership and reference counting are the default heap model.
- No null: `Option[T]` is the only absence.
- No use-after-free, double-free, or leak in safe code; tests prove the common
  paths.
- `unsafe` is required for raw pointers, FFI, and low-level runtime work.
- Every `unsafe` block has a reason comment and an audit entry.
- `yashy audit --unsafe` lists file, span, reason, and reviewer.
- Leak tests run in CI for long-lived programs and the test runner.
- A `yashy audit --memory` mode reports allocations in a run (later).

### 9.3 Runtime checks

| Check | Debug | Release | Opt-out |
|---|---|---|---|
| Array bounds | On | On | `unchecked` block |
| Integer overflow | On | Off (wrap) | `wrapping_*`, `checked_*`, `saturating_*` |
| Division by zero | On | On | None |
| Null dereference | Impossible in safe code | Impossible | `unsafe` only |
| Uninitialized read | Compile error | Compile error | None |
| Invalid UTF-8 in `string` | Rejected at boundary | Rejected | `bytes` type |

The exact release behavior must be documented and tested. "It depends" is not a
safety policy.

### 9.4 Concurrency safety

- Threads, channels, and mutexes are the current primitives.
- Shared mutable state requires a mutex or channel; the type system should make
  the unsafe sharing visible.
- `Send`/`Sync`-style marker traits are a 1.1 feature if needed; until then,
  document the rules and test them.
- TSan runs the concurrency test suite on Linux.
- Data-race and deadlock examples are part of the test corpus.
- `yashy run --threads=N` and `--test-threads=N` are deterministic in tests.

### 9.5 FFI safety

- All FFI is inside `unsafe`.
- C strings cross the boundary through explicit, checked conversions.
- ABI tests cover structs, enums, callbacks, and variadics where supported.
- ASan/UBSan run FFI tests.
- A "C interop" guide documents ownership, allocation, and error conventions.
- No unwinding across FFI boundaries; panics abort at the boundary.

### 9.6 Supply-chain safety

- `yashy.toml` + `yashy.lock` are mandatory for packages with dependencies.
- Lockfile pins exact commits; the R2 MVP adds checksums.
- Path and git dependencies are supported first; registry later.
- **No build scripts.** Packages cannot execute arbitrary code at build time.
- `yashy audit` checks lockfile integrity, known advisories (later), and package
  capabilities.
- Registry packages are signed at 1.0.
- `yashy tree --json` exposes the dependency graph to agents and CI.
- Vendoring/offline mode is supported.

### 9.7 Sandboxed execution

`yashy run --sandbox` executes a program with a capability manifest:

```toml
[capabilities]
fs = ["read:src/**", "write:build/**"]
net = []
env = ["CI"]
process = false
```

- Default sandbox: no filesystem writes, no network, no child processes, no
  environment except an allowlist.
- `--allow-fs`, `--allow-net`, `--allow-env`, `--allow-process` are explicit.
- Resource limits: CPU time, memory, output size, file size.
- Deterministic mode: fixed time, fixed randomness seed, fixed environment.
- Escape tests are adversarial and part of CI.
- Sandbox v0 ships in R2; v1 hardens it for 1.0.

### 9.8 Testing and evidence

- Parser fuzzing: random bytes, random tokens, random nesting.
- Runtime fuzzing: arrays, strings, maps, formatting.
- Stdlib fuzzing: JSON, path, fs boundaries.
- ASan, UBSan, TSan on Linux.
- Property tests for collections, strings, and numeric operations.
- Differential tests against C baselines for runtime semantics.
- A `yashy audit --unsafe` report is part of every release checklist.
- A public "known limitations" page lists what is not guaranteed.

### 9.9 Reproducible builds

- Deterministic IR and object layout.
- `SOURCE_DATE_EPOCH` respected.
- No absolute paths, timestamps, hostnames, or random seeds in artifacts.
- `yashy verify --reproducible` builds twice and compares hashes.
- Release artifacts are signed at 1.0.
- The build cache is content-addressed and deterministic.

### 9.10 Sequence and gates

| Release | Safety deliverables |
|---|---|
| R1 | Bounds/overflow behavior documented and tested; unsafe inventory; no known UB |
| R1.5 | Fuzz harnesses, sanitizer CI (ASan/UBSan/TSan), unsafe audit |
| R2 | Dependency audit, package lockfile/no build scripts |
| R2.5 | Sandbox v0, escape tests |
| 1.0 | Sandbox v1, reproducible builds, signed releases |
| 1.1 | `no_std` safety subset, embedded safety notes |
| 2.0 | Formal-ish invariants for the trusted core, self-hosted safety |

### 9.11 Risks

- **"No borrow checker" is read as "unsafe."** Mitigation: document the model,
  prove it with tests, and make `unsafe` rare and visible.
- **Sandbox escapes.** Mitigation: default-deny, capability tests, external review.
- **Supply-chain attacks.** Mitigation: no build scripts, lockfile, audit, signing.
- **Sanitizer noise.** Mitigation: a clean baseline and triage rotation.
- **Reproducibility is hard across platforms.** Mitigation: per-platform verification
  first, cross-platform at 1.0.

---

## 10. Workstream 3 — Developer experience

### 10.1 Goal

Make the compiler explain itself, the editor understand the code, and the test
runner answer "did I break anything?" in seconds.

### 10.2 Diagnostics

The diagnostic contract (1.0):

Every diagnostic has:

- **A stable code** (`E0308`, `W0001`) that never changes meaning.
- **A primary span** with file, line, column, and length.
- **Labels** on related spans.
- **A note** explaining the rule, not just the symptom.
- **A fix** when the compiler knows one, as structured edits.
- **A docs URL** to the error catalog.
- **A JSON form** with the same facts.

R1 assigns codes to 90% of the common corpus and ships the JSON form; 1.0
completes the catalog. An uncoded diagnostic still has a span and a JSON form.

The diagnostic pipeline:

1. Human rendering with color, source snippets, and carets.
2. `--json` rendering with a stable schema.
3. `yashy explain E0308` for the long form.
4. `yashy fix` for safe, mechanical fixes.
5. A diagnostic corpus test that asserts code, span, and fix.

"Did you mean" suggestions are required for:

- Unknown identifiers (near-miss names).
- Wrong argument count/type.
- Missing imports.
- Misspelled fields/methods.
- Missing `?` on a `Result`.
- Unreachable match arms.
- Non-exhaustive matches.

### 10.3 Language server

| Capability | Release |
|---|---|
| Diagnostics | Exists |
| Hover | Exists |
| Go to definition | Exists |
| Semantic tokens | Exists |
| Completion | R2 |
| Find references | R2 |
| Rename | R2 |
| Signature help | R2 |
| Code actions (fix, import) | R2/1.0 |
| Formatting | R1 |
| Inlay hints | 1.0 |
| Workspace symbols | 1.0 |
| Call hierarchy | 1.0 |
| Type hierarchy | 1.1 |

The LSP must be usable in VS Code, Neovim, and Zed without a custom build. A
conformance test suite runs against a headless client.

### 10.4 Formatter

- Deterministic and idempotent: formatting twice is a no-op.
- `yashy fmt --check` for CI.
- Editor integration through LSP.
- No configuration before 1.0; one style is the style.
- A corpus test formats every `.ya` file in the repository twice and compares.

### 10.5 Test runner

- Discovers `tests/**/*.ya` with a `main`.
- Filters by name, file, and tag.
- Runs in parallel by default; `--test-threads` controls it.
- `--fail-fast`, `--watch`, `--json`, `--quiet`, `--verbose`.
- Reports duration per test.
- Benchmarks are separate (`yashy bench`), not mixed into tests.
- Coverage is a 1.0 feature, not R2.
- Property tests and fuzz targets are first-class test kinds at R2.
- R1 tests remain ordinary `tests/**/*.ya` programs with `main`; no `#[test]`
  attributes or new macros. A richer test attribute is a later design question.

### 10.6 Package manager

The package manager is a DX feature and a safety feature. The details are in §17.

At R2 it must support:

- `yashy.toml` manifest with name, version, entry, dependencies, capabilities.
- `yashy.lock` with exact commits (checksums in the R2 MVP).
- Path and git dependencies.
- `yashy add`, `yashy remove`, `yashy update`, `yashy tree`, `yashy audit`.
- Offline/cache mode.
- `--json` for every command.
- No build scripts.

### 10.7 Debugger and profiler

- DWARF on Linux, PDB on Windows.
- `yashy build --debug` emits symbols.
- `yashy debug` prints the lldb/gdb command to run, or launches it if available.
- `yashy profile` runs the program and records a sampling profile where `perf` is
  available; otherwise prints guidance.
- `yashy profile --alloc` reports allocation sites (later).
- `yashy run --trace` prints runtime events for debugging.
- A "debugging" guide covers breakpoints, panics, and FFI.

### 10.8 Watch, templates, CI, docs

- `yashy watch` rebuilds/tests on change.
- `yashy init --template cli|lib|server|wasm|embedded`.
- `yashy init` writes `AGENTS.md` and a README with build/test/run commands.
- A GitHub Actions template runs `yashy check`, `yashy test`, and `yashy build`.
- `yashy doc` generates API docs from doc comments; `--json` for agents.
- `yashy graph` prints the module/dependency graph; `--json` for agents.
- `yashy outline` prints symbols in a file or project.
- `yashy search` searches symbols and docs.

### 10.9 Sequence and metrics

| Release | DX deliverables |
|---|---|
| R1 | Error codes, did-you-mean, `check --json`, `test --json`, `explain`, formatter idempotence |
| R1.5 | `watch`, parallel tests, cache visibility, stack traces |
| R2 | LSP completion/references/rename/signature help, `fix`, `doc`, package manager |
| R2.5 | `index`, `outline`, `graph`, `search`, `context` |
| 1.0 | Code actions, inlay hints, workspace symbols, debugger, profiler, coverage, playground, API docs |
| 1.1 | Call/type hierarchy, macro expansion (`yashy expand`), cross-compile diagnostics |

Metrics: 90% of common errors with code + span at R1, 100% at 1.0; 80% of
auto-fixable errors with safe fixes at R2; LSP conformance; formatter idempotence;
first test < 1 minute.

### 10.10 Risks

- **LSP scope creep.** Mitigation: capability table, conformance tests, R2 cut list.
- **Diagnostics without fixes are still work.** Mitigation: prioritize common errors.
- **Package manager becomes a second compiler.** Mitigation: no build scripts, small
  resolver, path/git first.
- **Debugger integration varies by platform.** Mitigation: document lldb/gdb commands
  first; automate later.

---

## 11. Workstream 4 — User experience

### 11.1 Goal

A stranger can install Yashy, run a program, understand an error, and uninstall it
without reading a wiki.

### 11.2 Install, update, uninstall

| Platform | R1 | 1.0 |
|---|---|---|
| Windows | Prebuilt zip + installer + PATH + checksum | winget/scoop, signed |
| Linux | Prebuilt tarball + install script + PATH + checksum | apt/rpm/AUR where feasible, signed |
| macOS | Not yet | Prebuilt pkg/brew, signed/notarized |

- Installers verify SHA-256 checksums.
- `yashy doctor` checks PATH, clang/LLVM, version, stdlib, and permissions.
- `yashy doctor --json` for agents.
- `yashy upgrade` updates to the latest stable.
- `yashy uninstall` removes binaries, PATH entries, and caches it created (R2;
  R1 ships installer-level uninstall).
- A version manager (`yashyup`) is a 1.0 feature if demand justifies it.
- Offline install is supported from a tarball/zip.
- No Rust toolchain is required for a prebuilt install.

### 11.3 CLI surface

```
yashy build [--release] [-O0..3] [--target T] [--json]
yashy check [--json] [--watch]
yashy run [--release] [--sandbox] [--] args...
yashy test [--release] [--json] [--watch] [--test-threads N] [filter]
yashy bench [--json]
yashy fmt [--check] [--json]
yashy doc [--json]
yashy init [--template T]
yashy add/remove/update/tree/audit
yashy doctor [--json]
yashy explain <code|diagnostic>
yashy fix [--dry-run] [--json]
yashy index [--json]
yashy outline [file]
yashy graph [--json]
yashy context <symbol> [--max-tokens N]
yashy search <query>
yashy mcp
yashy profile
yashy watch
yashy completions <shell>
yashy upgrade
yashy uninstall
yashy env
yashy version
```

Every command has `--help`, and `yashy help <command>` is equivalent. Colors obey
`NO_COLOR` and `--no-color`. `--quiet`, `--verbose`, and `--json` are consistent.

### 11.4 Configuration and environment

- `yashy.toml` is the project configuration.
- `~/.config/yashy/config.toml` is the user configuration.
- `YASHY_*` environment variables are documented and listed by `yashy env`.
- No hidden configuration: `yashy env --json` prints every resolved value.
- Precedence: CLI > env > project > user > default.
- Secrets are never printed; `yashy env` redacts them.

### 11.5 Errors

- Human-first: what happened, why, what to do next.
- No raw LLVM errors unless `--verbose`.
- No stack trace unless the program panicked or `--backtrace` is set.
- Every error has a docs link and a code.
- `yashy explain` is the canonical long form.
- Errors are tested as part of the diagnostic corpus.

### 11.6 Documentation

- **Quickstart:** install, init, run, test, build.
- **Tutorial:** the existing tutorial, updated.
- **Book:** language guide with examples.
- **Cookbook:** CLI, files, JSON, HTTP, tests, packages.
- **API reference:** generated by `yashy doc`.
- **Error catalog:** every code, cause, and fix.
- **Troubleshooting:** doctor, LLVM, PATH, antivirus, proxies.
- **Migration:** from Python/Go/Rust for common patterns.
- **AI docs:** `AGENTS.md`, `llms.txt`, `docs/ai/`.

### 11.7 Playground and accessibility

- A WASM playground at 1.0/1.1, once WASM is viable.
- No color-only signals; `NO_COLOR` respected.
- Plain-text output is complete, not a degraded version.
- Keyboard-first CLI; no interactive prompts in CI.
- Screen-reader-friendly docs; semantic HTML.
- Every example is runnable and tested.

### 11.8 Sequence and metrics

| Release | UX deliverables |
|---|---|
| R1 | Windows/Linux install, checksums, doctor, first CLI, uninstall, quickstart |
| R2 | Completions, man pages, upgrade, `env --json`, templates, troubleshooting |
| 1.0 | macOS, signed installers, version manager, playground, book/API/cookbook |
| 1.1 | Embedded/WASM install guides, cross-compile toolchains |

Metrics: clean install < 5 min; first program < 5 min; doctor resolves 90% of known
cases; uninstall clean; completions for four shells.

### 11.9 Risks

- **Platform installers are a maintenance tax.** Mitigation: prebuilt tarballs first,
  package managers where demand exists.
- **LLVM dependency confuses users.** Mitigation: doctor explains, prebuilt bundles
  clang where licensing allows, and docs cover install.
- **Docs drift.** Mitigation: examples are tested; docs build is a gate.
- **Accessibility is an afterthought.** Mitigation: plain output and `NO_COLOR` from
  R1.

---

## 12. Workstream 5 — Features

### 12.1 Goal

Add the features that remove real friction, in the order that makes the previous
features cheaper. "More features" is not the goal; **fewer reasons to leave** is.

### 12.2 Language catalog

Existing features are not repeated here. This is what is planned.

#### P0 — R1 (must ship)

| Feature | Why it matters |
|---|---|
| `args()`, `now()`, `getcwd()`, `sleep(ms)` | The first CLI cannot be written without them |
| Better "did you mean" diagnostics | The first error is the first impression |
| Exhaustiveness diagnostics for matches | Prevent a class of runtime bugs |
| `?` ergonomics audit | `Result` is the error model; it must be pleasant |

#### P1 — R2 (usable)

| Feature | Why it matters |
|---|---|
| Better generic diagnostics | Generics exist; their errors must be legible |
| Pattern-matching improvements | Fewer nested `if` chains |
| `defer` (evaluate only if it stays explicit) | Cleanup without exceptions |
| Compile-time constant evaluation improvements | Less runtime work, better errors |
| `yashy expand` for macros (design) | Macros must be inspectable before they ship |

#### P2 — 1.0 (stable)

| Feature | Why it matters |
|---|---|
| Pattern matching polish | A general-purpose language needs a complete match |
| Trait-free generic improvements | Better inference and error messages |
| Stable ABI for FFI | Real integrations |
| `defer` (if accepted) | Resource cleanup |
| Compile-time reflection over types (limited) | Serialization and docs without macros |

#### P3 — 1.1 (complete)

| Feature | Why it matters |
|---|---|
| Traits/interfaces | Shared behavior without inheritance |
| Generic constraints/trait bounds | Generic code that states its requirements |
| Associated types | Traits that model containers/iterators |
| Trait objects/dynamic dispatch | Extensible plugin-style code |
| Iterators | The collection/loop vocabulary |
| Declarative macros | Remove boilerplate without arbitrary code execution |
| Unicode strings | The world is not ASCII |
| Cross-compilation (`--target`) | Embedded, CI, distribution |
| `no_std` subset | Embedded and tiny binaries |
| WASM decision | Web/playground/edge |
| Embedded example and guide | Prove the domain |
| `yashy expand` for declarative macros | Inspectable metaprogramming |

#### P4 — 2.0 (super language)

| Feature | Why it matters |
|---|---|
| Async implementation | High-concurrency services |
| SIMD | Bulk data performance |
| Const generics | Fixed-size arrays and low-level code |
| Self-hosting | The language builds itself |
| Full trait coherence/blanket impls | Ecosystem-scale abstraction |
| `no_std` async subset | Embedded networking |

#### Won't do

- Tracing GC.
- Borrow checker.
- Exceptions.
- Implicit coercion.
- Operator overloading.
- Class inheritance.
- Power-user procedural macros.
- Named/default arguments before 2.0 (revisit then).
- A GUI toolkit in the stdlib.

### 12.3 Standard-library catalog

#### P0 — R1

| Module/function | Why it matters |
|---|---|
| `args()` | CLI arguments |
| `now()` / `sleep(ms)` | Timestamps and delays |
| `getcwd()` | Relative paths |
| `fs`, `env`, `process` completion | The documented surface must be real |

**R1 exception.** The four runtime builtins use the existing empty-string/0
convention for failure until builtin `Result` lands in R2. `getcwd()` returns
`""` on failure; it is not a `Result`-returning signature in R1, and the docs
must not claim that it is.

#### P1 — R2

| Module | Why it matters |
|---|---|
| `cli` | Argument parsing without a dependency |
| `log` | Structured logging for tools and services |
| `http` client | Automation and backend calls |
| `http` server | Small services |
| `net` | TCP/UDP basics |
| `test` | Assertions, filters, parallel helpers |
| `bench` | Repeatable benchmarks |
| `fuzz` | First-class fuzz targets |
| `sync` improvements | Channels/mutexes ergonomics |
| `json` improvements | Streaming, typed decode |

#### P2 — 1.0

| Module | Why it matters |
|---|---|
| `serde`-style serialization | Structs to/from JSON and other formats |
| `time` formatting/parsing | Real programs need dates |
| `regex` | Text processing |
| `csv` | Data automation |
| `collections` polish | Maps, sets, deques |
| `task` | A small project task runner |

#### P3 — 1.1

| Module | Why it matters |
|---|---|
| `iter` | Iterator adapters |
| `unicode` | Graphemes, normalization, width |
| `collections` iterators | Consistent vocabulary |

#### P4 — 2.0

| Module | Why it matters |
|---|---|
| `crypto` (vetted algorithms only) | Security-sensitive primitives |
| `simd` | Bulk operations |
| Database drivers | Ecosystem, not core stdlib |
| Web framework | Ecosystem, not core stdlib |

### 12.4 Tooling catalog

#### P0 — R1

| Tool | Why it matters |
|---|---|
| Windows/Linux installer + checksums | Adoption |
| `yashy doctor` + `--json` | First-run success |
| `yashy check --json` / `test --json` | Agent/CI integration |
| Error codes + `yashy explain` | Learnability |
| Benchmark gate | Performance truth |
| Package manager spike | De-risk R2 |

#### P1 — R2/R2.5

| Tool | Why it matters | Release |
|---|---|---|
| Package manager MVP | Dependencies | R2 |
| LSP completion/references/rename/signature help | Editor parity | R2 |
| `yashy fix` | Mechanical errors fixed in one command | R2 |
| `yashy doc` | API docs | R2 |
| `yashy watch` | Fast feedback | R1.5 |
| Parallel test runner | Fast feedback | R1.5 |
| Stack traces | Debugging | R2 |
| `yashy index` / `outline` / `graph` / `search` | Code understanding | R2.5 |
| `yashy context` | Token-efficient agent context | R2.5 |
| `yashy mcp` v0 | Agent integration | R2.5 |
| `yashy audit` | Supply-chain and unsafe visibility | R2 |
| Shell completions + man pages | UX | R2 |
| `yashy upgrade` / `uninstall` | Lifecycle | R2 |
| Three flagship tools | Proof the language works | R2 |

#### P2 — 1.0

| Tool | Why it matters |
|---|---|
| Registry | Ecosystem |
| Debugger/profiler | Production debugging |
| Playground | Try before install |
| `yashy lint` | Project-specific rules |
| Coverage | Test quality |
| `yashy task` | Project automation |
| Signed/reproducible releases | Trust |
| API docs site | Discoverability |

#### P3 — 1.1

| Tool | Why it matters |
|---|---|
| `yashy expand` | Macro transparency |
| Cross-compile toolchain manager | Embedded/distribution |
| `yashy target` | List/install targets |

#### P4 — 2.0

| Tool | Why it matters |
|---|---|
| Self-hosted toolchain | Independence |
| Registry at scale | Ecosystem |
| Package search/docs | Discoverability |

### 12.5 Platform catalog

| Platform | Priority | Release |
|---|---|---|
| Windows x86-64 | P0 | Exists |
| Linux x86-64 | P0 | R1 |
| macOS x86-64/arm64 | P2 | 1.0 |
| Cross-compilation | P3 | 1.1 |
| `no_std` | P3 | 1.1 |
| WASM | P3 decision | 1.1 decision, 2.0 implementation |
| Embedded (ARM/RISC-V) | P3 | 1.1 example, 2.0 support |
| Reproducible builds | P2 | 1.0 |
| Signed releases | P2 | 1.0 |

### 12.6 AI catalog

| Feature | Priority | Release |
|---|---|---|
| JSON diagnostics for `check`/`test` | P0 | R1 |
| Stable error codes + `explain` | P0 | R1 |
| `AGENTS.md` + `llms.txt` | P0 | R1 |
| `index` + `outline` + `graph` | P1 | R2.5 |
| `context` packs | P1 | R2.5 |
| `fix` (safe edits) | P1 | R2 |
| MCP server v0 | P1 | R2.5 |
| Sandbox v0 | P1 | R2.5 |
| Agent benchmark v0 | P1 | R2.5 |
| MCP v1 + sandbox v1 | P2 | 1.0 |
| Stable schema freeze | P2 | 1.0 |
| Agent benchmark gate (90%) | P2 | 1.0 |
| Context for traits/Unicode/macros | P3 | 1.1 |
| Agent-native ecosystem | P4 | 2.0 |

### 12.7 Priority summary

| Priority | Meaning | Release |
|---|---|---|
| P0 | Without it, the release is not usable | R1 |
| P1 | Without it, the release is not competitive | R2 |
| P2 | Without it, the release is not stable | 1.0 |
| P3 | Without it, the language is not complete | 1.1 |
| P4 | Without it, the language is not a super language | 2.0 |
| Won't | Identity or scope | Never |

### 12.8 The top 20 features that make things easier

These are the highest-leverage quality-of-life features, in rough order:

1. `yashy run` with a fast `-O0` dev default.
2. `yashy watch`.
3. `yashy test --json` with parallel execution.
4. `yashy fix`.
5. `yashy explain`.
6. `yashy doc`.
7. `yashy init --template`.
8. `yashy add` / `remove`.
9. `yashy audit`.
10. `yashy context`.
11. `yashy outline`.
12. `yashy graph`.
13. `yashy mcp`.
14. A stdlib `cli` parser.
15. A stdlib `log` module.
16. A stdlib `http` client/server.
17. Shell completions and man pages.
18. Panic stack traces.
19. A playground.
20. A package manager that does not run arbitrary build scripts.

### 12.9 Acceptance for every feature

A feature is done only when:

- Its user story is written.
- Its acceptance tests pass on Windows and Linux.
- Its diagnostics have codes, spans, and fixes where possible.
- Its docs page exists and is tested.
- Its JSON form exists if it has observable output.
- Its performance impact is measured if it touches a budget.
- Its safety impact is documented if it touches memory, concurrency, FFI, or
  packages.
- It is listed in the release notes.

### 12.10 Risks

- **Feature catalog becomes a wish list.** Mitigation: priorities, cut list, one
  owner, release gates.
- **Traits/generics destabilize the type system.** Mitigation: 1.1, design doc,
  conformance suite, no inheritance.
- **Stdlib becomes a framework.** Mitigation: small core, ecosystem for frameworks.
- **Macros hurt AI readability.** Mitigation: declarative only, `yashy expand`,
  stable formatting, docs.
- **Cross-compilation is a tarpit.** Mitigation: one target at a time, CI matrix.

---

## 13. Workstream 6 — AI-native

### 13.1 Goal

An agent should be able to install Yashy, understand a project, make a change, run
the tests, read the errors, and fix them — without a human translating.

### 13.2 The contract (normative)

Every core command obeys these rules:

1. **Human output and JSON output carry the same facts.**
2. **JSON has a `schema_version` and a stable shape.**
3. **Every diagnostic has a span; every coded diagnostic has a stable code and a
   docs URL.** R1 targets 90% of the common corpus; 1.0 requires 100%.
4. **Every auto-fixable diagnostic has structured edits.**
5. **Exit codes are meaningful:** `0` success, `1` diagnostics/tests failed,
   `2` usage/config error, `3` internal error.
6. **No interactive prompt in `--json` or non-TTY mode.**
7. **No secrets in output.**
8. **No network access unless the command is explicitly a network command.**
9. **No code execution unless the command is `run`, `test`, `bench`, or `mcp` run.**
10. **Every command documents its schema in `yashy help --json`.**

### 13.3 JSON schemas

#### Diagnostics

```json
{
  "schema_version": 1,
  "tool": "check",
  "yashy_version": "0.2.0",
  "success": false,
  "diagnostics": [
    {
      "code": "E0308",
      "severity": "error",
      "message": "expected `int`, found `string`",
      "file": "src/main.ya",
      "span": { "start": 42, "end": 47, "line": 3, "column": 12 },
      "labels": [
        { "span": { "start": 42, "end": 47 }, "message": "expected `int`" }
      ],
      "notes": ["numeric operations require `int` or `float`"],
      "fix": {
        "description": "replace with an integer",
        "edits": [
          {
            "file": "src/main.ya",
            "span": { "start": 42, "end": 47 },
            "replacement": "42"
          }
        ]
      },
      "docs_url": "https://yashy.dev/errors/E0308"
    }
  ],
  "summary": { "errors": 1, "warnings": 0 }
}
```

`labels`, `notes`, `fix`, and `docs_url` are omitted when empty or uncoded.
`docs_url` is present only with a code.

#### Semantic index

```json
{
  "schema_version": 1,
  "root": "/path/to/project",
  "packages": [],
  "modules": [],
  "symbols": [
    {
      "name": "main",
      "kind": "fn",
      "file": "src/main.ya",
      "span": { "start": 0, "end": 120 },
      "signature": "fn main() -> void",
      "docs": "Entry point."
    }
  ],
  "imports": [],
  "tests": [],
  "entry_points": ["main"],
  "diagnostics": []
}
```

#### Context pack

```json
{
  "schema_version": 1,
  "query": "parse_config",
  "budget_tokens": 8000,
  "definition": {},
  "callers": [],
  "callees": [],
  "tests": [],
  "docs": [],
  "diagnostics": []
}
```

### 13.4 Semantic index and context packs

- `yashy index` writes `.yashy/index.json` (or stdout with `--json`).
- The index contains packages, modules, symbols, signatures, docs, imports, tests,
  entry points, and diagnostics.
- `yashy outline [file]` is the human form; `--json` is the agent form.
- `yashy graph` exposes the module/dependency graph.
- `yashy search <query>` searches symbols and docs.
- `yashy context <symbol>` builds a **small, relevant context pack**: definition,
  signature, docs, callers, callees, tests, and related diagnostics.
- `--max-tokens` caps the pack; the command reports what it omitted.
- Context packs are deterministic for the same index and query.
- The index is content-addressed and invalidated by source changes.

### 13.5 Explain and fix

- `yashy explain E0308` prints the long-form explanation, examples, and fixes.
- `yashy explain --json` returns the same facts.
- `yashy fix` applies only **safe** edits: typos, missing imports, obvious type
  mismatches, unused imports.
- `yashy fix --dry-run` shows the diff without writing.
- `yashy fix --json` returns the edits and their status.
- `yashy fix` never applies a change that could alter semantics without a clear
  diagnostic and a test.
- Every fix is covered by a before/after corpus test.

### 13.6 MCP server

`yashy mcp` exposes the toolchain over the Model Context Protocol:

| Tool | Purpose | Sandbox |
|---|---|---|
| `yashy_check` | Type-check a project | Read-only |
| `yashy_build` | Build a project | Read-only + build dir |
| `yashy_test` | Run tests | Read-only + temp |
| `yashy_fmt` | Format files | Read/write project |
| `yashy_explain` | Explain an error code | Read-only |
| `yashy_context` | Build a context pack | Read-only |
| `yashy_search` | Search symbols/docs | Read-only |
| `yashy_index` | Build the semantic index | Read-only + index |
| `yashy_run` | Run a program | **Sandboxed** |

Rules:

- The MCP server never exposes arbitrary shell execution.
- `yashy_run` requires an explicit capability manifest.
- Every tool returns structured content and a JSON schema.
- The server is versioned; breaking changes require a new tool name.
- The server logs tool calls locally for debugging, never to a remote service by
  default.

### 13.7 Sandbox and capabilities

The sandbox is the safety boundary for agent-run code.

```toml
[capabilities]
fs = ["read:src/**", "write:build/**"]
net = []
env = ["CI"]
process = false
```

- Default deny for filesystem writes, network, child processes, and environment.
- `--allow-fs`, `--allow-net`, `--allow-env`, `--allow-process` are explicit.
- Resource limits: CPU, memory, output, file size.
- Deterministic mode: fixed time, fixed RNG seed, fixed env.
- Escape tests are adversarial and part of CI.
- A capability violation is a clear diagnostic, not a crash.
- Sandbox v0 is R2; v1 is 1.0.

### 13.8 Documentation for AI

- `AGENTS.md` at the repository root: build/test/run commands, style, error
  handling, package rules, sandbox rules, where to find docs.
- `llms.txt`: a compact, canonical map of the language and toolchain docs.
- `docs/ai/patterns.md`: canonical examples for common tasks.
- `docs/ai/anti-patterns.md`: what not to do, with reasons.
- `docs/errors/`: one page per error code, with examples and fixes.
- Every page has a stable URL and a version.
- Docs are tested: examples compile and run.
- Docs are token-efficient: short sentences, tables, runnable snippets.

### 13.9 Language design for AI

The language choices that help agents are the same choices that help humans:

- **Explicit syntax.** No implicit conversions, no hidden control flow.
- **Deterministic formatting.** One canonical form.
- **Stable error codes.** A model can learn and cite them.
- **No hidden global state.** A function's inputs are visible.
- **No exceptions.** Errors are values.
- **Visible `unsafe`.** The dangerous surface is marked.
- **Inspectable macros.** `yashy expand` shows the expansion.
- **Small context.** Modules and symbols are easy to isolate.
- **One obvious way.** Fewer choices means fewer wrong guesses.

The language must not add "AI-only" syntax. That would make the language worse for
both users.

### 13.10 Agent benchmark

A public benchmark of tasks that an agent should complete with the toolchain:

1. Install Yashy and run hello.
2. Init a CLI project and add argument parsing.
3. Add a path dependency and use it.
4. Fix a type error from `check --json`.
5. Add a test and run it.
6. Refactor a function using `context`.
7. Add an HTTP client call.
8. Package the project and audit it.
9. Explain an error code.
10. Cross-compile (when available).

Measured per task:

- Success rate.
- Wall-clock time.
- Token count.
- Tool calls.
- Human interventions.
- Sandbox violations.

The harness runs in CI with a fixed model and tool budget. Results are public.
Target: **90% success by 1.0**. The benchmark is a regression test for the AI
contract, not a marketing claim.

### 13.11 Security for agents

- Untrusted code is data until it runs in the sandbox.
- No build scripts in packages.
- Lockfile is mandatory; checksums are mandatory in the R2 MVP and later.
- `yashy audit` is part of the agent workflow.
- Destructive commands require `--yes` in non-interactive mode.
- `yashy env` redacts secrets.
- Network access is opt-in.
- Tool output is treated as untrusted text; the MCP server does not execute it.
- The agent benchmark includes a prompt-injection scenario.

### 13.12 Sequence and metrics

| Release | AI deliverables |
|---|---|
| R1 | JSON check/test, error codes, `explain`, `AGENTS.md`, `llms.txt` |
| R2 | `fix`; the AI contract stays stable |
| R2.5 | Index/outline/graph/search, context packs, MCP v0, sandbox v0, agent benchmark v0 |
| 1.0 | MCP v1, sandbox v1, schema freeze, agent benchmark gate 90%, docs for agents |
| 1.1 | Context for traits/Unicode/macros, cross-compile context |
| 2.0 | MCP v2, agent-native ecosystem, self-hosted toolchain |

### 13.13 Risks

- **Schema drift.** Mitigation: schema version, diff CI, freeze at 1.0.
- **Sandbox escape.** Mitigation: default deny, escape tests, external review.
- **Benchmark overfitting.** Mitigation: held-out tasks, public harness, no
  task-specific hacks.
- **Docs drift.** Mitigation: tested examples, docs build gate.
- **Agent misuse.** Mitigation: capabilities, audit, no ambient authority.
- **Token cost.** Mitigation: context budgets, outline/index, short docs.

---

## 14. Release roadmap (Balanced)

The roadmap is a sequence of **phases**, not a 36-month waterfall. Dates are
planning horizons for one full-time maintainer. The order is the commitment; the
dates move when reality moves.

### 14.1 R1 — "It works" (0.2.0, months 1–3)

**Theme:** a stranger can install Yashy, write a CLI, and get a useful error.

**Must ship:**

- Windows and Linux install with checksums, PATH, uninstall, and `doctor`.
- `yashy doctor --json`.
- First CLI guide (stdin `wc`).
- `args()`, `now()`, `getcwd()`, `sleep(ms)`.
- Error codes, "did you mean," `check --json`, `test --json`, `explain`.
- Benchmark gate with compile, binary-size, and runtime baselines.
- Top-three performance fixes (slice 3b) or a recorded exception.
- Package manager spike (path/git, lockfile, audit, no build scripts).
- AI contract v0: `AGENTS.md`, `llms.txt`, JSON schema v1.
- Safety baseline: bounds/overflow behavior documented and tested; `unsafe`
  inventory.

**Gate:** clean install on Windows/Linux; first program in under five minutes
(full guide under thirty); `doctor` + `--json`; uninstall; checksum; JSON schema;
benchmark gate; no known UB.

**Not yet:** macOS, registry, HTTP, traits, Unicode, cross-compilation, LSP
completion, MCP, sandbox.

**Cut order:** macOS, completions, man pages, package-manager install, version
manager.

### 14.2 R1.5 — "It's fast" (0.2.x, months 3–4)

**Theme:** the dev loop stops being the tax.

**Must ship:**

- `yashy run` and `yashy test` default to `-O0`; `build` `-O2`; `--release` `-O3`;
  explicit `-O` wins.
- Fast linker detection (`lld`, `mold`) with fallback.
- Content-addressed cache for modules and runtime.
- `yashy watch`.
- Parallel test runner.
- Fuzz harnesses + ASan/UBSan/TSan CI.
- `yashy audit --unsafe`.

**Gate:** incremental rebuild < 200 ms on the synthetic project; 100 tests < 2 s;
sanitizer CI green; fuzz targets green for 7 days.

**Not yet:** M9, precompiled stdlib, parallel codegen, PGO/LTO, sandbox v0.

### 14.3 R2 — "It's usable" (0.3.0, months 4–7)

**Theme:** dependencies, services, and editors become real.

**Must ship:**

- Package manager MVP: manifest, lock, path/git, add/remove/update/tree/audit,
  offline, `--json`, no build scripts.
- `http` client/server, `log`, `cli`, `net`, `test`, `bench`, `fuzz`.
- LSP completion, references, rename, signature help.
- `yashy fix`, `yashy doc`, stack traces.
- M9 escape analysis.
- Precompiled stdlib, parallel codegen.
- Three flagship tools.
- `yashy audit` for dependencies and unsafe.

**Gate:** a real package with a path/git dependency builds and tests; LSP
conformance; M9 benchmark; three tools shipped, or two with a recorded exception.

**Not yet:** registry, macOS, playground, debugger/profiler, traits, Unicode, AI
index/context, MCP, sandbox, agent benchmark.

**Cut order:** third flagship tool (keep two), `fix` extras, `doc` extras,
`bench`/`fuzz` helpers.

R2 and R2.5 are separate phases. If R2 exceeds 12 weeks, cut from its cut list; do
not absorb R2.5.

### 14.4 R2.5 — "It's AI-native" (0.4.0, months 7–9)

**Theme:** the agent contract becomes real.

**Must ship:**

- `yashy index`, `outline`, `graph`, `search`, `context`.
- Context packs for files and symbols (task packs are 1.0).
- MCP v0.
- Sandbox v0.
- Agent benchmark v0 with held-out tasks.
- Schema stabilization and diff CI.
- `AGENTS.md`/`llms.txt` updated for the new tools.

**Gate:** context pack ≤ 8k tokens; MCP conformance or a recorded exception;
sandbox escape tests; schema diff CI; agent benchmark v0 runs or a recorded
exception, and its trend is improving.

**Not yet:** MCP v1, sandbox v1, schema freeze (1.0), agent benchmark gate (1.0).

### 14.5 1.0 — "It's stable" (months 9–12)

**Theme:** a stranger can bet a project on it.

**Must ship:**

- Semver and a compatibility policy.
- Public registry with signed packages.
- macOS installer and CI.
- HTTP hardening and a production guide.
- Book, API reference, cookbook, error catalog.
- Debugger and profiler.
- Playground.
- Package audit and signed/reproducible releases.
- LSP code actions, inlay hints, workspace symbols.
- `yashy lint`, coverage, `yashy task`.
- MCP v1 and sandbox v1.
- Stable JSON schema and agent benchmark gate (90%).
- Performance gates sustained.

**Gate:** all release metrics in §7; no P0; reproducible build; signed artifacts;
90% agent benchmark.

**Not yet:** traits, Unicode, iterators, macros, cross-compilation, `no_std`,
WASM, async, embedded.

**Cut order:** playground, profiler, coverage, `lint`, `task`; do not cut registry,
macOS, reproducibility, MCP v1, sandbox v1, schema freeze, or agent benchmark.

### 14.6 1.1 — "It's complete" (months 12–24)

**Theme:** the language itself is complete for the five domains.

**Must ship:**

- Traits/interfaces, generic constraints, associated types, trait objects.
- Iterators.
- Declarative macros + `yashy expand`.
- Unicode strings.
- Cross-compilation (`--target`).
- `no_std` subset.
- WASM decision (and implementation if viable).
- Embedded example and guide.
- Context packs for the new features.

**Gate:** each feature has acceptance tests, docs, diagnostics, and a JSON form;
cross-compile CI; `no_std` example; Unicode conformance; macro expansion tests.

**Not yet:** async implementation, self-hosting, SIMD.

### 14.7 2.0 — "It's a super language" (months 24–36)

**Theme:** scale, independence, and ecosystem.

**Must ship:**

- Async implementation (if the 1.1 decision is yes).
- Self-hosting.
- Ecosystem frameworks (web, CLI, data) outside the stdlib.
- Registry at scale.
- Learner path: book, exercises, playground, classroom guide.
- SIMD.
- Const generics.
- MCP v2 and agent-native ecosystem.
- Parity with the G5 roadmap.

**Gate:** self-hosted compiler builds Yashy; async conformance; ecosystem showcase;
performance parity; no P0.

### 14.8 Capacity reality

- One full-time maintainer.
- Two-week slices; weekly demo; gate review at the end of each phase.
- Main stays green; no silent scope growth.
- A phase that misses its gate gets a recorded exception, not a quiet extension.
- Re-plan quarterly; the order survives, the dates move.
- The first execution phase (slices 1, 2, 3, 3b, 4) is the only work that starts
  before this spec and the rewritten master plan are approved.

---

## 15. The first execution phase (slices 1, 2, 3, 3b, 4)

These are the immediate work. The planned slices are 1, 2, 3, 3b, and 4; 3b is
the required performance slice between 3 and 4. They run in order:
**1 → 2 → 3 → 3b → 4**. Slice 3b is before 0.2.0 and is not optional. Each slice
has its own detailed plan in `docs/superpowers/plans/`. The phase-and-step map for
Phases 0–7 is `docs/superpowers/plans/2026-10-25-yashy-super-language-implementation-phases.md`;
it references these slices rather than duplicating them.

### 15.1 Slice 1 — Install and first CLI

**Goal:** a stranger on Windows or Linux installs Yashy, runs a CLI, and knows how
to build and test.

**Deliverables:**

- Windows installer: prebuilt binary, checksum verification, PATH setup, uninstall,
  `doctor`.
- Linux installer: prebuilt tarball + install script, checksum, PATH, uninstall,
  `doctor`.
- `yashy doctor --json`.
- `yashy init` writes `AGENTS.md` and a README with build/test/run commands.
- First CLI guide: stdin `wc`, adapted from the existing `wc.ya`/`wc_words.ya`.
- Quickstart documentation.
- `llms.txt` with canonical links.
- Repository-root `AGENTS.md` for contributors and agents.
- Release workflow builds Windows and Linux artifacts.

**Tests:**

- Clean-VM install/uninstall on Windows and Linux.
- Checksum verification fails on a tampered artifact.
- PATH is set and removed correctly.
- `doctor` matrix covers missing clang, wrong version, broken PATH.
- `doctor --json` parses and matches the human facts.
- The first CLI guide runs end-to-end.
- `init` writes `AGENTS.md` and the README.
- `AGENTS.md` commands run and `llms.txt` links resolve.

**Gate:** clean install on both platforms; first program in under five minutes
(full guide under thirty); `doctor` + `--json`; uninstall is clean; checksum
works; guide is tested; `AGENTS.md`/`llms.txt` are tested; CI is green.

**Cut order:** macOS, shell completions, man pages, package-manager install, version
manager.

**Known risks:** Application Control blocks freshly linked/copied binaries on the
development host; pad launch copies with 4096 random bytes. WSL builds use
`CARGO_TARGET_DIR=/tmp/yashy-target-linux` to avoid clobbering the Windows `target`.

### 15.2 Slice 2 — Runtime and stdlib gaps

**Goal:** the first CLI and normal programs can use arguments, time, the working
directory, and sleep without `YASHY_PATH`.

**Deliverables:**

- `args()` excludes the program path.
- `getcwd()` returns `""` on failure; builtin `Result`-returning signatures are R2.
- `now()` and `sleep(ms)`.
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
- Runtime signature tests.

**Gate:** tests pass on Windows and Linux; docs exist; no `YASHY_PATH` requirement.

**Cut order:** timezone/date formatting, HTTP, logging.

### 15.3 Slice 3 — Diagnostics, benchmarks, and AI contract v0

**Goal:** errors teach; performance is measured; agents can read results.

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
- `AGENTS.md` commands run and `llms.txt` links resolve.

**Gate:** JSON schema v1; 90% common errors; benchmark gate; `explain`;
`AGENTS.md`/`llms.txt`; docs.

**Cut order:** LSP completion/rename, `fix`, `index`, MCP, sandbox.

**Known risk:** schema churn. Freeze v1 at R1; additive changes only within 0.x.

### 15.4 Slice 3b — Top-three performance fixes

**Goal:** close the three worst benchmark rows before 0.2.0.

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

### 15.5 Slice 4 — Package manager spike

**Goal:** de-risk dependencies and design a package manager that is safe for agents.

**Deliverables:**

- `yashy.toml` manifest and `yashy.lock`.
- Path and git dependencies.
- The dependency's `src/` is the import root; name collisions are errors.
- `yashy pkg add`, `remove`, `tree`, `audit`; all support `--json`.
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

### 15.6 Order and gates

| Order | Slice | Gate |
|---|---|---|
| 1 | Install + first CLI | Clean install, first program, doctor + JSON, uninstall, checksum, guide, AGENTS/llms |
| 2 | Runtime + stdlib | args/now/getcwd/sleep, no `YASHY_PATH`, docs |
| 3 | Diagnostics + benchmarks + AI v0 | JSON schema, 90% errors, benchmark gate |
| 3b | Top-three performance fixes | Rows improve, no > 2× without exception |
| 4 | Package manager spike | Path/git, lockfile, collision, no build scripts, audit, `--json`, `--offline` |

No slice starts before the previous slice's gate is met or an exception is
recorded. No code starts before this spec and the rewritten master plan are
approved.

---

## 16. Post-R1 slices (5–12)

Post-R1 work is planned, not started. It begins after the first execution phase
(slices 1, 2, 3, 3b, 4) is complete and reviewed.

Slices 5–9 are two-week slices. Slices 10–12 are phase-sized and may split into
multiple two-week units. R2.5 is delivered by slice 11 (AI index/context/MCP,
sandbox v0, schema stabilization, agent benchmark v0). MCP v1 and sandbox v1 are
part of slice 12 (1.0).

### 16.1 Slice 5 — Fast dev loop

**Goal:** make the edit/check/test loop fast enough to disappear.

**Deliverables:**

- `run`/`test` `-O0`; `build` `-O2`; `--release` `-O3`; explicit `-O` wins.
- Fast linker detection (`lld`, `mold`) and `--linker`.
- Content-addressed module cache.
- `yashy watch`.
- Parallel test runner with `--test-threads`.
- `--verbose`/`--json` cache hit reporting.

**Gate:** incremental rebuild < 200 ms; 100 tests < 2 s; no semantic change.

### 16.2 Slice 6 — Safety hardening

**Goal:** make the safe path provable and the unsafe path small.

**Deliverables:**

- Bounds/overflow policy documented and tested.
- `unsafe` audit report.
- Fuzz harnesses for parser, runtime, JSON, path.
- ASan/UBSan/TSan CI.
- Reproducible build prototype.
- `yashy audit --unsafe`.

**Gate:** zero known UB; sanitizer CI green; fuzz green 7 days.

### 16.3 Slice 7 — LSP, fix, doc

**Goal:** editors and agents can complete, rename, fix, and document code.

**Deliverables:**

- LSP completion, references, rename, signature help.
- `yashy fix` with `--dry-run` and `--json`.
- `yashy doc` with `--json`.
- Stack traces.
- Code actions for common fixes.

**Gate:** LSP conformance; fix corpus; doc coverage; panic traces.

### 16.4 Slice 8 — HTTP, logging, stdlib

**Goal:** Yashy can build a small real service.

**Deliverables:**

- `http` client/server.
- `log`.
- `cli` parser.
- `net` basics.
- `test`/`bench`/`fuzz` helpers.
- JSON improvements.
- Precompiled stdlib cache.
- M9 escape analysis.
- Service guide.

**Gate:** a real HTTP service with tests; logging guide; CLI parser examples;
M9 benchmark.

### 16.5 Slice 9 — Package manager MVP

**Goal:** dependencies are real, safe, and reproducible.

**Deliverables:**

- Manifest/lock finalization.
- Path/git deps.
- `add/remove/update/tree/audit`.
- Offline/cache.
- No build scripts.
- Registry design and local registry prototype.

**Gate:** real package with a dependency; lockfile reproducible; audit; offline.

### 16.6 Slice 10 — Three flagship tools

**Goal:** prove the language on real tools.

**Deliverables:**

- A CLI tool.
- An automation/data tool.
- A small HTTP service.
- Each with tests, docs, and a release artifact.

**Gate:** all three build and run on Windows/Linux; docs; benchmarks.

### 16.7 Slice 11 — AI index, context, MCP

**Goal:** an agent can understand a project without reading every file.

**Deliverables:**

- `yashy index`, `outline`, `graph`, `search`.
- `yashy context` with `--max-tokens`.
- MCP v0 with `check`, `build`, `test`, `fmt`, `explain`, `context`, `search`,
  `run` (sandboxed).
- Sandbox v0 with escape tests.
- Agent benchmark v0.
- Schema stabilization and diff CI.

**Gate:** context pack ≤ 8k tokens; MCP conformance; sandbox escape tests;
schema diff CI; benchmark v0 runs.

### 16.8 Slice 12 — 1.0 hardening

**Goal:** make 1.0 stable and trustworthy.

**Deliverables:**

- macOS installer/CI.
- Registry and signing.
- Reproducible builds.
- Book/API/cookbook.
- Debugger/profiler.
- Playground.
- LSP code actions/inlay hints.
- `lint`, coverage, `task`.
- MCP v1 and sandbox v1.
- Schema freeze; agent benchmark 90%.

**Gate:** all §7 metrics; no P0; reproducible; signed; 90% agent benchmark.

---

## 17. Ecosystem and package manager

### 17.1 Manifest

`yashy.toml` is the only project manifest:

```toml
[package]
name = "my-tool"
version = "0.1.0"
edition = "2026"
entry = "src/main.ya"

[dependencies]
helper = { path = "../helper" }
jsonlib = { git = "https://example.com/jsonlib", rev = "abc123" }

[capabilities]
fs = ["read:src/**", "write:build/**"]
net = []
env = ["CI"]
process = false
```

Rules:

- `name` is unique within a dependency graph.
- `entry` is explicit; no magic `main.ya` discovery beyond the default.
- Dependencies are path or git at R2; registry at 1.0.
- Git dependencies pin a commit or tag; a branch is allowed but recorded in the
  lockfile.
- Capabilities are declarative and enforced by the sandbox.
- No build scripts, ever.

### 17.2 Lockfile

`yashy.lock` records:

- Package name and version.
- Source (path, git URL, registry).
- Exact commit or registry version.
- Checksum.
- Dependency edges.
- Capability summary.

The lockfile is deterministic and committed. `yashy update` changes it
intentionally; `yashy build` never does.

### 17.3 Resolver

- R2: path and git only; exact commits; no semver solver.
- 1.0: registry with semver; minimal version selection or a simple resolver with a
  documented policy.
- One version per package name in the graph; conflicts are errors with a clear
  path.
- The resolver is small, deterministic, and testable.
- No optional dependencies, features, or build scripts before 1.0.
- A dependency's `src/` is the import root; name collisions are errors.

### 17.4 Cache and offline mode

- Content-addressed cache under the user cache directory.
- `yashy build --offline` uses only cached/vendored sources.
- `yashy vendor` copies dependencies into the project (1.0).
- Cache keys include compiler version, target, flags, and source hash.
- `yashy cache clean` is explicit; nothing is deleted silently.

### 17.5 Security

- Checksums are mandatory for git and registry packages in the R2 MVP and later.
  The R1 spike may use commit pinning only; the checksum design is recorded for the
  R2 MVP.
- Registry packages are signed at 1.0.
- `yashy audit` checks lockfile integrity, known advisories (later), capabilities,
  and unsafe usage.
- No build scripts means no arbitrary code execution at install time.
- The registry supports yanking and ownership transfer with an audit log.
- A compromised package can be yanked; existing lockfiles still build offline.
- Agents get `yashy tree --json` and `yashy audit --json`.

### 17.6 Registry (1.0)

- A central, signed index is the default.
- Search, package pages, docs, versions, owners, and yank status.
- Publishing requires a signed artifact and a passing `yashy check`/`yashy test`.
- Names are first-come, with a dispute process.
- The registry never executes package code.
- Federation is a 2.0 question.

### 17.7 Flagship tools

Three real tools prove the language:

1. **A CLI tool** — argument parsing, files, JSON, tests, single binary.
2. **An automation/data tool** — HTTP, JSON, CSV, scheduling.
3. **A small HTTP service** — routing, logging, graceful shutdown, deploy guide.

Each has tests, docs, a release artifact, and a benchmark.

### 17.8 Docs and community

- Quickstart, tutorial, book, cookbook, API reference, error catalog.
- A "showcase" page with the flagship tools.
- A contributing guide and a code of conduct.
- A public roadmap and release notes.
- A discussion forum or GitHub Discussions; no mandatory chat.
- Examples are tested in CI.

---

## 18. Capacity, cadence, and review gates

- **Capacity:** one full-time maintainer. The plan assumes no hired team.
- **Cadence:** two-week slices; weekly demo; a gate review at the end of each phase.
- **Main green:** no merge with a red CI; flaky tests are fixed or quarantined.
- **No silent scope growth:** a new feature goes into the catalog and the cut list,
  not into the current slice.
- **Exception process:** a missed gate gets a written exception with a reason, an
  owner, and a date. It does not get a quiet extension.
- **Quarterly re-plan:** review metrics, cut list, and risks; adjust dates, not
  identity.
- **Plan set:** this spec + the master plan + one plan per slice.
- **Review gates:** spec approved → master approved → slice plan approved → code.
- **Definition of a slice:** a gate, a demo, tests, docs, and a release note.
- **Sustainable pace:** no crunch; the cut list exists so the plan can survive
  reality.

---

## 19. Risks and mitigations

| Risk | Impact | Mitigation | Trigger |
|---|---|---|---|
| Scope creep | Misses every gate | Priorities, cut list, one owner | A slice grows > 20% |
| LLVM dependency | Install friction | Prebuilt bundles, doctor, docs | > 10% install failures |
| Compile speed | Dev loop pain | `-O0`, cache, parallel codegen, linker | Hello > 1 s |
| Safety perception | "No borrow checker = unsafe" | Document model, tests, audit, sandbox | User reports |
| Sandbox escape | Security incident | Default deny, escape tests, review | Any escape |
| Supply-chain attack | Trust loss | No build scripts, lockfile, audit, signing | Advisory |
| AI schema drift | Agent breakage | Versioning, diff CI, freeze | Breaking diff |
| Benchmark overfitting | False confidence | Held-out tasks, public harness | Score jump without tooling change |
| macOS delay | Misses 1.0 | Start CI early, prebuilt tarball first | CI unavailable |
| Unicode complexity | 1.1 delay | Scope to strings, graphemes, width | Spec ambiguity |
| Async complexity | 2.0 delay | Decision at 1.1, implementation at 2.0 | No design |
| Self-hosting | 2.0 delay | Stage it; keep the Rust compiler as bootstrap | Bootstrap fails |
| Burnout | Project stops | Sustainable pace, cut list, breaks | Two missed weeks |
| Adoption | No users | Flagship tools, docs, install UX | No growth |
| Docs drift | Trust loss | Tested examples, docs build | Broken example |

---

## 20. Cut list and invariants

### 20.1 Cut order

- **R1:** macOS, completions, man pages, package-manager install, version manager.
- **R1.5:** `watch`, parallel tests, fast linker (keep cache and `-O0`).
- **R2:** third flagship tool (keep two), `fix` extras, `doc` extras, `bench`/`fuzz`
  helpers.
- **R2.5:** MCP v0, agent benchmark v0, `search`, `graph` (keep index, outline,
  context, sandbox v0).
- **1.0:** playground, profiler, coverage, `lint`, `task`.
- **1.1:** WASM implementation, trait objects, embedded support (keep traits,
  iterators, macros, Unicode, cross-compilation, `no_std`).
- **2.0:** SIMD, const generics, ecosystem frameworks (keep async, self-hosting,
  registry at scale).

### 20.2 Do not cut

- Language identity.
- Windows/Linux install.
- First CLI.
- `args`/`now`/`getcwd`/`sleep`.
- Error codes and JSON diagnostics.
- Benchmark gate.
- Top-three performance fixes.
- Package manager spike.
- No build scripts.
- Sandbox default deny.
- Safety gates.
- Spec/master/slice approval before code.

### 20.3 Invariants

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

---

## 21. Decision log

### 21.1 Preserved from v1

| Decision | Choice |
|---|---|
| Beachhead | CLI and developer tools |
| Performance | Spine across releases |
| Domains | CLI, automation/data, backend, embedded, learners |
| `args()` | Excludes the program path |
| `getcwd()` | Returns `""` on failure |
| Dependency imports | Dependency `src/` is the import root; collisions are errors |
| Benchmark artifacts | `key=value`, no dependencies |
| Top-three fixes | Slice 3b, before 0.2.0, not optional |
| Windows arguments | C `char**` code-page encoding |
| Windows `getcwd()` | UTF-8 |
| Bundled stdlib | Works without `YASHY_PATH` |
| Installer | Prebuilt Yashy + `doctor` + LLVM guidance |
| macOS | 1.0 target |
| Capacity | Solo full-time |

### 21.2 New in v2

| Decision | Choice |
|---|---|
| Sequencing | Balanced: speed + AI-native foundation, then features |
| Workstreams | Speed, Safety, DX, UX, Features, AI-native |
| AI contract | Normative: JSON, codes, spans, fixes, docs, exit codes |
| JSON schema | v1 at R1; additive within 0.x; frozen at 1.0 |
| Dev optimization | `run`/`test` `-O0`; `build` `-O2`; `--release` `-O3`; explicit `-O` wins |
| Sandbox | Default deny; capability manifest; resource limits |
| Packages | No build scripts; lockfile; checksums at R2 MVP; signed registry at 1.0 |
| Agent benchmark | Public harness; 90% gate at 1.0 |
| R2/R2.5 | Separate phases; R2 does not absorb R2.5 |
| Feature priority | P0–P4 + Won't, as in §12 |
| Code start | Only after spec, master, and slice plans are approved |

### 21.3 Deferred decisions

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

## 22. Definition of done for this spec

This spec is done when:

1. It is reviewed and approved by the project owner.
2. The master plan is rewritten from it.
3. Slice plans 1, 2, 3, 3b, and 4 are updated from it.
4. The phased implementation plan is written and approved.
5. The first execution phase has gates, cut orders, and owners.
6. No code has started.

The implementation is done when:

1. Slices 1 → 2 → 3 → 3b → 4 pass their gates.
2. 0.2.0 ships with the R1 must-ship list.
3. Post-R1 phases 2 → 7 pass their gates in order, or record exceptions with owners and dates.
4. Post-R1 work starts only after a review of 0.2.0.
5. Every release meets §7 or records an exception.
6. The agent benchmark, benchmark gate, and safety gate are green at 1.0.

---

## 23. Open questions for review

These are the few choices where the owner's preference changes the plan. The
recommendations are the defaults.

1. **Is `run`/`test` defaulting to `-O0` acceptable?**
   Recommendation: yes. It makes the dev loop fast; `--release` keeps `-O3`; an
   explicit `-O` wins. The behavior change is documented and tested.
2. **Is keeping R2 (0.3 "usable") and R2.5 (0.4 "AI-native") as separate phases
   acceptable?**
   Recommendation: yes. It protects the usable release from the AI-native release
   and keeps the agent contract from blocking packages, HTTP, and editors.
3. **Should `defer` be accepted?**
   Recommendation: evaluate at R2; ship at 1.1 only if it stays explicit and does
   not hide control flow.
4. **Is 90% agent task success at 1.0 realistic?**
   Recommendation: keep it as a 1.0 gate, with a trend target at R2.5. If the
   benchmark is too easy, add held-out tasks rather than lowering the bar.
5. **Should `yashy lint` be core or ecosystem?**
   Recommendation: core at 1.0 if time; otherwise ecosystem. Do not block 1.0 on it.
6. **Is the 500 KB hello-world budget right?**
   Recommendation: yes at R1; tighten to 300 KB at 1.0 only if startup is not hurt.
7. **Should the registry be central or federated?**
   Recommendation: central signed index at 1.0; federation at 2.0.
8. **Should macOS move earlier than 1.0?**
   Recommendation: no. Windows/Linux first; macOS at 1.0 with a prebuilt tarball
   before the installer.

---

*End of spec.*






