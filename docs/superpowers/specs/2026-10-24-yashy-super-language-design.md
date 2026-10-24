# Yashy — CLI-First Roadmap to a Complete Language

**Date:** 2026-10-24
**Status:** Draft for review
**Author:** Brainstormed with the project owner

---

## 1. Summary

Yashy is a native, statically typed language with no external runtime. It already
has a real compiler, a formatter, a language server, a tutorial, generics,
closures, FFI, and threads. It is at version 0.1.0. M0–M8 and M12–M15 are
complete; M9–M11 remain, with M11 partly done.

The next phase is not "add every feature." It is "make the language usable, then
complete it." The first beachhead is **CLI and developer tools**. Performance is a
**spine** that runs through every phase, not a separate phase at the end. The other
four domains — automation/data, backend services, embedded/edge, and
general-purpose/learners — follow in sequence.

> **Positioning:** Yashy is the fastest way to build a small, fast,
> dependency-free native tool.

The first release is the foundation for all five domains, not a rejection of four.

### Decision summary

| Decision | Choice |
|---|---|
| First beachhead | CLI and developer tools |
| Performance | Cross-cutting spine, measured from day one |
| Release order | CLI → automation/data → backend → embedded/edge → general-purpose |
| Package manager | Required; the old "no package manager" non-goal is reversed |
| Self-hosting (M10) | Deferred until 2.0 |
| Async runtime | Deferred; threads first, revisit after HTTP |
| Traits, Unicode strings, iterators | 1.1 program, after the first usable release |
| Cross-compilation, `no_std`, WASM | 1.1 program, after the first usable release |

---

## 2. Where Yashy is today

| Area | Current state |
|---|---|
| **Version** | 0.1.0 |
| **Milestones** | M0–M8 and M12–M15 complete; M9–M11 remain. M11 is partly done: `fmt` and the LSP exist. |
| **Compiler** | Rust workspace; emits textual LLVM IR and links with `clang`; Windows and Linux x86-64; no cross-compilation. |
| **Language** | `int`, `float`, `bool`, `char`, `string`, `void`; structs, enums, `Option`/`Result`, `?`, arrays, fixed arrays, tuples, pattern guards, generics, closures, methods/objects, modules with `pub`, FFI, threads/channels/mutexes, checked arithmetic. |
| **Tooling** | `yashy build`, `yashy check`, `yashy fmt`; LSP with diagnostics, hover, definition, and semantic tokens; editor modes; tutorial. No `run`, `test`, `init`, or `doc`. |
| **Standard library** | `alloc`, `io`, `math`, `os`, `string`; built-in `read_file`, `write_file`, `read_line`. No `fs`, `path`, `args`, `env`, `process`, `time`, `rand`, `JSON`, or HTTP. |
| **Missing for users** | Installer, package manager, cross-compilation, stack traces, LSP completion/rename, benchmark CI. |
| **Performance** | Scalar rows are competitive. Feature rows are slower: array mapping 3.76×, debug formatting 3.42×, tuples 1.79×. `fib` compiles in ~259 ms. Benchmarks are not a CI gate. |
| **Documentation** | README, tutorial, design spec, vision, plans. No book, API reference, website, or playground. |

The language is not missing a compiler. It is missing a product.

---

## 3. The gap

Three gaps keep Yashy from being adopted:

1. **Time to first real program.** A new user needs Rust, LLVM, and manual
   commands. There is no installer, no `yashy run`, no `yashy init`, a tiny
   standard library, and no package manager.
2. **Time to debug.** Panics do not give a useful stack trace. Diagnostics lack
   suggestions. The LSP has no completion or rename.
3. **Time to trust.** Feature-heavy code is slower than it should be, there is no
   benchmark gate, there is no compatibility policy, and there is no package
   ecosystem.

The plan closes these gaps in the order a new user meets them.

---

## 4. Goals

| ID | Goal |
|---|---|
| **G1** | By 1.0, a new user installs Yashy and runs a real CLI tool in under one hour on Windows, Linux, and macOS. |
| **G2** | `yashy run`, `yashy test`, `yashy init`, and `yashy fmt` work on a fresh project. |
| **G3** | The standard library covers 80% of common CLI tasks without third-party packages. |
| **G4** | The package manager can add a path or git dependency and build/test a project; a registry prototype follows. |
| **G5** | Performance targets are met: scalar ≤1.2× C, feature rows ≤2×, 1,000 lines compile in <2 s, hello-world binary <500 KB, zero external runtime dependencies. |
| **G6** | A crash points to its cause in under two minutes: stack traces, suggestions, and LSP completion/rename. |
| **G7** | Documentation covers the tutorial, a book, examples, API reference, and three flagship tools built in Yashy. |
| **G8** | The roadmap covers all five domains in sequence: CLI/dev tools, automation/data, backend services, embedded/edge, general-purpose/learners. |
| **G9** | The language keeps its identity: no GC, no borrow checker, no exceptions, no implicit coercion, no operator overloading, no class inheritance. |
| **G10** | The work stays sustainable for one full-time person: two-week slices, a working build every two weeks, public releases at phase gates, no six-month branches. |

---

## 5. Non-goals for the first release

These are deliberate exclusions from R1, not permanent decisions:

- No self-hosting (M10) before 2.0. It is a credibility milestone, not a user need.
- No async runtime in R1 or R2. Use threads for small services; revisit after HTTP.
- No traits/interfaces, Unicode strings, iterators, or macros in R1. They are
  1.1-program work.
- No cross-compilation, `no_std`, or WASM in R1. They are 1.1-program work.
- No central package registry at scale in R1. R2 ships a path/git package manager;
  1.0 ships a registry.
- No "all five domains in one release." The first release is the foundation for all
  five.
- No rewriting of dated plans or specs. They are historical records; this spec is
  the current roadmap.

The following remain permanent non-goals unless explicitly reversed:

- Garbage collection.
- A borrow checker.
- Exceptions and unwinding as the error model.
- Implicit type coercion.
- Operator overloading.
- Class inheritance.
- Power-user procedural macros.

---

## 6. Target users

| Persona | What they need first | Domain |
|---|---|---|
| **Maya, CLI tool author** | Single binary, fast startup, args/env/files/JSON, tests, package manager, docs. | CLI/dev tools |
| **Dev, automation engineer** | File pipelines, JSON/CSV, HTTP client, scheduling, simple deployment. | Automation/data |
| **Sam, backend developer** | HTTP server, concurrency, logging, package manager, deployment story. | Backend services |
| **Ana, embedded developer** | Cross-compilation, `no_std`, small binaries, deterministic memory. | Embedded/edge |
| **Lee, learner** | Installer, clear errors, tutorial, book, examples, playground. | General-purpose |

The first release serves Maya. R2 serves Maya and Dev. 1.0 serves Sam. The 1.1
program serves Ana and Lee. 2.0 serves everyone.

---

## 7. Strategy: CLI-first, with a performance spine

### Why CLI first

- It is the closest to Yashy's current strengths: native binaries, fast startup,
  zero runtime, C interop, checked arithmetic.
- It is the smallest gap between "compiler project" and "tool someone uses."
- It creates real users and real feedback before the language grows more features.
- Every later domain reuses the same installer, standard library, test runner, and
  package manager.

### Why not breadth-first

A thin slice of all five domains would produce five demos: a CLI hello-world, a toy
HTTP server, a JSON parser, a cross-compiled hello, and a traits example. It would
prove breadth, not usability. For a solo full-time project, it would also delay
every domain that needs a solid foundation.

### Why performance is a spine, not a phase

Performance is a differentiator: zero runtime, fast startup, small binaries. If it
is deferred to the end, the standard library and package ecosystem will be built on
slow primitives. The spine means:

- Every release has a benchmark report.
- Every release has a compile-time and binary-size baseline.
- Performance regressions block a release unless explicitly accepted.
- M9 escape analysis is scheduled in R2, not "someday."

### Why foundation work is included, not first

M11 adoption work and M9 escape analysis are part of the roadmap. M10 self-hosting
is not: it is a large project that does not make a new user's first CLI tool faster.
It moves to 2.0.

---

## 8. Release roadmap

Dates are targets, not promises. Re-plan at each release gate.

| Release | Target | Theme | Domains | Must ship | Gate | Not yet |
|---|---|---|---|---|---|---|
| **0.2.0 — R1** | Month 3 | It works | CLI/dev tools; automation foundation | Installer + `yashy doctor`; `run`, `test`, `init`; stdlib `fs`, `path`, `args`, `env`, `process`, `time`, `rand`, `JSON`; error suggestions; first-CLI guide; benchmark CI; `--release` semantics; top three performance fixes | A fresh Windows/Linux user installs and builds a real CLI tool in under one hour; `yashy test` works; benchmarks report | Package manager, HTTP, traits, Unicode, cross-compilation, macOS installer |
| **0.3.0 — R2** | Month 6 | It's usable | CLI, automation/data, backend foundation | Package manager MVP (path/git, lockfile, `add`); HTTP client and minimal server; logging; LSP completion/rename; `yashy doc`; stack traces; M9 escape analysis; three flagship CLI tools | A project with a git dependency builds and tests; an automation script and a small HTTP service work; crash-to-cause under two minutes; performance targets met | Traits, Unicode, iterators, cross-compilation, `no_std`, WASM, async, self-hosting |
| **1.0.0** | Months 9–12 | It's stable | CLI, automation, backend | Compatibility policy; registry; installers for Windows/Linux/macOS; HTTP server hardening; docs/book; external users; performance CI | External users ship tools without maintainer help; docs cover 80% of CLI tasks; no P0 bugs | Traits, Unicode, iterators, embedded, self-hosting |
| **1.1.0 — program** | Months 12–24 | It's complete | Backend, embedded/edge, general-purpose | Traits/interfaces; Unicode strings; iterators; declarative macros; cross-compilation and `no_std`; WASM decision; async decision; embedded examples | Sub-release gates; the program is complete when the language and platform items above are done | Self-hosting, operator overloading |
| **2.0.0** | Months 24–36 | It's a super language | All | Ecosystem, frameworks, registry at scale, learners, self-hosting, performance parity | Self-hosted compiler; broad ecosystem; docs and playground | — |

### R1 — "It works" (0.2.0, month 3)

**User outcome:** Maya installs Yashy, writes a CLI tool that reads files, parses
JSON, handles arguments and environment variables, and runs tests — without
touching Rust or LLVM.

**Must ship:**

- **Installer.** Prebuilt Yashy binaries for Windows and Linux; adds `yashy` to
  PATH; `yashy doctor` detects `clang`/LLVM and gives install instructions. No Rust
  toolchain required for users. macOS is a 1.0 target.
- **CLI.** `yashy run` (compile and execute), `yashy test`, `yashy init` (project
  scaffold). Keep `build`, `check`, and `fmt`.
- **Test runner.** Test discovery, assertions, filtering, clear exit codes. The
  syntax is specified in the M11 plan and implemented as part of this release.
- **Standard library.** `fs`, `path`, `args`, `env`, `process`, `time`, `rand`,
  `JSON`, on top of the existing `alloc`, `io`, `math`, `os`, `string`. Fallible
  operations return `Result`.
- **Errors.** Suggestions, notes, and "did you mean" for common mistakes. Stack
  traces are R2.
- **Docs.** "Build your first CLI in 10 minutes", updated tutorial, and at least
  three examples.
- **Performance.** Benchmark CI; `--release` implies optimization (explicit `-O`
  overrides); top three feature-row fixes; compile-time and binary-size baselines.
- **Package manager.** Design spike only; implementation starts in R2.

**Sequence (first 90 days):**

- **Weeks 1–2:** baseline, benchmark CI, installer decision, scope freeze.
- **Weeks 3–5:** `yashy run`, `yashy test`, `yashy init`; error suggestions.
- **Weeks 6–8:** `fs`, `path`, `args`, `env`, `process`, `time`, `rand`, `JSON`;
  first-CLI guide.
- **Weeks 9–10:** installer and `doctor`; examples; editor support.
- **Weeks 11–12:** top-three performance fixes, package-manager design spike,
  external testers.
- **Week 13:** release 0.2.0.

This sequence is a target, not a contract. If it slips, use the cut list below; do
not silently drop a gate item.

**If R1 runs long, cut in this order:** `rand`, `time`, non-essential error
suggestions, extra examples. Do not cut the installer, `run`/`test`/`init`,
`fs`/`path`/`args`/`env`/`process`, or the first-CLI guide.

**Gate:**

- A clean Windows or Linux machine goes from installer to a running CLI tool in
  under one hour.
- `yashy test` works on a fresh `yashy init` project.
- The benchmark job reports compile time, binary size, and runtime rows.
- At least one external tester completes the first-CLI guide without maintainer
  help; recruit three.

### R2 — "It's usable" (0.3.0, month 6)

**User outcome:** Maya can share a project with a git dependency, test it, and
ship it. Dev can write an automation script and a small HTTP service.

**Must ship:**

- **Package manager MVP.** `yashy.toml`, `yashy.lock`, path and git dependencies,
  `yashy add`, and integration with `build`/`test`. A central registry is not
  required.
- **HTTP.** Client and a minimal server, enough for a small service. Hardening and
  deployment guidance are 1.0.
- **Logging.** A small, structured logging module.
- **LSP.** Completion and rename, on top of diagnostics, hover, definition, and
  semantic tokens.
- **Documentation.** `yashy doc` and generated API docs for the standard library.
- **Debugging.** Stack traces or equivalent panic locations that identify the
  failing call path.
- **Performance.** M9 escape analysis; feature rows under 2×; scalar under 1.2× C;
  1,000 lines under 2 s; hello-world under 500 KB.
- **Flagship tools.** Three real CLI tools built in Yashy, used to expose standard
  library and tooling gaps.
- **Registry prototype.** A simple index and publish/fetch flow is a stretch goal;
  path/git dependencies are the required part.

**If R2 runs long, cut in this order:** registry prototype, `yashy doc`, third
flagship tool, full stack traces (keep panic locations). Do not cut the package
manager, HTTP client/minimal server, LSP completion, M9 escape analysis, or the
first flagship tool.

**Gate:**

- A project with a git dependency builds and tests from a clean checkout.
- An automation script processes files and JSON end to end.
- A small HTTP service starts, serves a request, and shuts down cleanly.
- A panic points to the failing call path in under two minutes.
- The performance targets in G5 are met or the exceptions are documented.

### 1.0 — "It's stable" (months 9–12)

**User outcome:** Sam can build and deploy a small backend service. External users
can adopt Yashy without maintainer help.

**Must ship:**

- **Stability.** Semantic versioning, a compatibility policy, deprecation warnings,
  and a release cadence.
- **Registry.** A hosted or community registry with package docs and search.
- **Installers.** Windows, Linux, and macOS.
- **HTTP server hardening.** Timeouts, graceful shutdown, basic routing guidance,
  deployment examples.
- **Documentation.** A book, API reference, cookbook, and examples.
- **Performance.** The G5 targets are sustained in CI.
- **Ecosystem.** At least three flagship tools and a small set of community
  packages.

**Gate:**

- External users ship tools without asking the maintainer for help.
- The documentation covers 80% of common CLI tasks.
- No known P0 bugs; the compatibility policy is published.

### 1.1 — "It's complete" (program, months 12–24)

**User outcome:** Ana can cross-compile a small binary for an embedded target. Lee
can learn the language from the book and playground. Library authors can use
traits and Unicode text.

This is a program of work, not one release. It will be split into 1.1, 1.2, and so
on when it is planned. Each sub-release gets its own gate; the list below is the
intended order, not a promise that every item ships together.

**Must ship:**

- **Traits/interfaces.** Static and, where justified, dynamic dispatch; used in the
  standard library.
- **Unicode strings.** A defined string representation and indexing model; no
  pretending byte strings are Unicode.
- **Iterators.** A consistent iteration protocol for arrays, maps, and ranges.
- **Declarative macros.** No procedural macros.
- **Cross-compilation.** `--target` for supported targets.
- **`no_std` subset.** For embedded and small binaries.
- **WASM.** A decision and, if accepted, an implementation path.
- **Async.** A decision: keep threads, add stackless async, or defer again.
- **Embedded examples.** At least one real target build.

**Program gate:**

- Traits are used by the standard library and at least one package.
- Unicode text round-trips through files, JSON, and HTTP.
- Iterators work across the core collections.
- A cross-compiled hello-world runs on a supported target.
- The `no_std` subset builds a small binary.
- Declarative macros are used in at least one standard-library module.
- The WASM and async decisions are recorded with rationale.

### 2.0 — "It's a super language" (months 24–36)

**User outcome:** Yashy is a credible general-purpose language with an ecosystem,
learners, and a self-hosted compiler.

**Must ship:**

- Ecosystem and frameworks.
- Registry at scale.
- Learner path: book, playground, exercises.
- Self-hosting (M10).
- Performance parity with the G5 targets.

**Gate:**

- The self-hosted compiler builds the standard library and passes the suite.
- The ecosystem covers the common tasks in all five domains.
- The learner path is usable without a maintainer.

---

## 9. Workstreams

These run across releases. Each release pulls the slices it needs.

### 9.1 Compiler and runtime performance

- **M9 escape analysis.** Stack promotion for non-escaping allocations; differential
  tests; benchmark before/after.
- **Codegen and runtime.** Inline runtime calls; improve array, map, string, tuple,
  and debug-formatting paths; checked-arithmetic elision where safe; LLVM `switch`
  lowering; one IR buffer; intern identifiers; iterative parser.
- **Build semantics.** `--release` implies optimization; explicit `-O` overrides.
- **Size.** Strip, LTO, and a size profile where appropriate; hello-world budget.
- **Measurement.** Benchmark CI, compile-time baseline, binary-size baseline, and a
  documented exception process.

### 9.2 Tooling and developer experience

- **Installer and doctor.** Prebuilt binaries, PATH setup, LLVM detection and
  install guidance, version pinning, uninstall.
- **CLI.** `run`, `test`, `init`, `doc`; keep `build`, `check`, `fmt`.
- **Test runner.** Discovery, assertions, filtering, exit codes, clear failures.
- **LSP.** Completion, rename, formatting, code actions, signature help, inlay hints.
- **Diagnostics.** Suggestions, notes, "did you mean", panic locations, stack traces.
- **Package manager.** Manifest, lockfile, path/git dependencies, add/remove/update,
  build/test integration.
- **Editor support.** VS Code extension, syntax highlighting, LSP client.
- **Documentation.** Tutorial, book, cookbook, API reference, examples.

### 9.3 Standard library

- **R1:** `fs`, `path`, `args`, `env`, `process`, `time`, `rand`, `JSON`.
- **R2:** logging, HTTP client, minimal HTTP server.
- **1.0:** HTTP server hardening, deployment helpers.
- **1.1 program:** Unicode-aware strings, iterators, traits-based abstractions.
- **Design rules:** no external runtime, explicit ownership, no hidden allocation,
  fallible operations return `Result`, consistent naming, tests and docs for every
  public function.

### 9.4 Language

- **R1:** ownership diagnostics and error quality; no major new language features.
- **R2:** no major new language features; focus on usability and performance.
- **1.1 program:** traits/interfaces, Unicode strings, iterators, declarative
  macros, and any language work needed for cross-compilation.
- **2.0:** self-hosting and any remaining completeness work.
- **Permanent identity:** no GC, no borrow checker, no exceptions, no implicit
  coercion, no operator overloading, no class inheritance.

### 9.5 Ecosystem and adoption

- **Flagship tools.** Three real CLI tools built in Yashy; they drive the standard
  library and tooling backlog.
- **Docs.** First-CLI guide, tutorial, book, cookbook, API reference, examples.
- **Community.** Contributing guide, issue templates, code of conduct, changelog,
  release notes.
- **Distribution.** GitHub Releases, installers, package-manager integrations.
- **Registry.** Prototype in R2; hosted or community registry at 1.0.
- **External testing.** Recruit three testers at R1; at least one completes the
  first-CLI guide. A broader group follows at R2 and 1.0.

### 9.6 Platforms and distribution

- **R1:** Windows and Linux x86-64; installer and `doctor`.
- **R2:** same, with a clean-checkout build path.
- **1.0:** macOS; installers for all three.
- **1.1 program:** cross-compilation, `no_std`, WASM decision.
- **CI:** Windows and Linux from day one; macOS added when the installer target is
  added; benchmark job separate from the correctness matrix.

---

## 10. Success metrics

| Metric | Target | How measured | Release |
|---|---|---|---|
| Install to hello world | <5 minutes | Timed clean-VM install | R1 |
| Real CLI tool | <1 hour | Timed first-CLI guide | R1 |
| `yashy test` on a fresh project | Works | End-to-end test | R1 |
| 1,000-line compile time | <2 seconds | Benchmark job | R1 baseline, R2 target |
| `fib` compile time | <200 ms | Benchmark job | R2 |
| Hello-world binary size | <500 KB | `ls` on release build | R1 baseline, R2 target |
| Yashy runtime dependencies | Zero | `ldd`/dependency inspection | R1 |
| Scalar performance | ≤1.2× C | Benchmark table | R2 |
| Feature-row performance | ≤2× C | Benchmark table | R2 |
| Crash to cause | <2 minutes | Timed debugging exercise | R2 |
| CLI tasks without third-party packages | 80% | Task checklist | R1/R2 |
| Flagship tools | 3 | Built and shipped | R2 |
| Package manager path/git | Works | Clean-checkout test | R2 |
| Installers | Windows/Linux/macOS | Clean-VM tests | 1.0 |
| Documentation coverage | 80% of CLI tasks | Task checklist | 1.0 |
| Traits/Unicode/iterators | Used in stdlib | Test suite | 1.1 program |
| Cross-compiled hello | Runs on target | Target test | 1.1 program |
| Self-hosted compiler | Builds stdlib and passes suite | CI | 2.0 |

---

## 11. Decisions of record

| Area | Decision |
|---|---|
| First beachhead | CLI and developer tools |
| Performance | Cross-cutting spine with CI gates |
| Package manager | Required; reverse the old non-goal. Path/git first, registry later. |
| Self-hosting (M10) | Deferred to 2.0 |
| Async | Deferred; threads first; revisit after HTTP |
| Traits/interfaces | 1.1 program |
| Unicode strings | 1.1 program |
| Iterators | 1.1 program |
| Declarative macros | 1.1 program |
| Cross-compilation/`no_std`/WASM | 1.1 program (may split) |
| Installer strategy | Prebuilt Yashy + `yashy doctor` + LLVM install guidance; bundled toolchain later |
| `--release` | Implies optimization; explicit `-O` overrides |
| Versioning | 0.x until 1.0; breaking changes allowed with migration notes; compatibility policy at 1.0 |
| Dated plans/specs | Keep as history; this spec is the current roadmap |
| Language identity | No GC, no borrow checker, no exceptions, no implicit coercion, no operator overloading, no class inheritance |

---

## 12. Risks and mitigations

| Risk | Mitigation |
|---|---|
| **Scope creep across five domains** | Beachhead-first; each release has a "not yet" list; re-plan quarterly. |
| **Installer depends on LLVM/clang** | `yashy doctor` detects and guides installation; test on clean VMs; bundle a toolchain only if adoption justifies the size. |
| **Prebuilt installer pipeline does not exist** | Add a release workflow that builds and attaches Windows/Linux binaries; test on clean VMs; add code signing where required. |
| **Package manager version resolution is hard** | Start with path/git and a lockfile; no central registry requirement in R2; registry at 1.0. |
| **Feature-heavy code stays slow** | Benchmark CI; M9 escape analysis; top-three regressions each release; explicit exception process. |
| **No borrow checker means memory bugs** | Keep linear ownership diagnostics; document FFI/unsafe boundaries; add tests for use-after-move and double-free diagnostics. |
| **Solo burnout** | Two-week slices; a working build every two weeks; public releases at phase gates; no six-month branches; defer self-hosting. |
| **Adoption stays flat** | Three flagship tools; external testers; first-CLI guide; docs/book; package manager. |
| **Pre-1.0 breakage** | Migration notes for every breaking change; compatibility policy at 1.0. |
| **macOS support is untested** | Add macOS CI when the installer target is added; cross-compilation is not a substitute for testing. |
| **Async/threads do not scale** | Document thread-per-connection limits; revisit async after HTTP; do not promise a runtime. |
| **Unicode is deeper than it looks** | Treat it as a 1.1-program design project; do not claim Unicode support while strings are byte strings. |
| **WASM remains a non-goal** | Make an explicit decision in the 1.1 program; do not leave it ambiguous. |

---

## 13. Open questions

These are for later plans, not blockers for R1:

1. Exact test syntax and discovery rules (reuse the M11 plan).
2. Package manifest format and lockfile semantics.
3. Registry protocol and hosting model.
4. Stack-trace mechanism: unwinding, frame pointers, or symbolization.
5. Unicode string representation and indexing model.
6. Traits design: static only, dynamic dispatch, coherence, object safety.
7. Async: threads only, stackless, or defer again.
8. WASM: target, non-goal, or separate backend.
9. macOS CI and installer distribution.
10. Release version numbers and cadence after 1.0.
11. Self-hosting scope: compiler only or compiler plus standard library.
12. Installer format (MSI, zip, shell script) and code-signing requirements.

---

## 14. Relationship to existing plans

- **M9–M10 plan.** M9 escape analysis is pulled into R2. M10 self-hosting is
  deferred to 2.0. The plan file remains the detailed reference for both; this spec
  changes the ordering and release grouping, not the technical content.
- **M11 adoption plan.** `fmt` and the LSP are done. The remaining tasks — `test`,
  `run`, `init`, `doc`, installer, error quality — are split across R1 and R2.
- **M12–M15.** Already complete; this spec builds on them rather than reopening
  them.
- **Language vision.** The vision's five-domain ambition is preserved. Its old
  ordering and its "no package manager" non-goal are superseded by this roadmap.
- **Design spec.** The language identity and permanent non-goals remain in force.
  This spec does not change the language surface in R1.

---

## 15. Verification and review

This spec is ready for approval when:

- It states the first beachhead and sequences all five domains.
- Each release has a gate and a "not yet" list.
- R1, R2, and 1.0 each have one primary user outcome and a bounded tooling/stdlib
  slice; 1.1 is explicitly a program and may split into smaller releases.
- The first 90 days fit one full-time person with the R1 cut list.
- The metrics are measurable and tied to a release.
- The permanent non-goals and the reversed package-manager decision are explicit.
- The implementation plan for R1 can be derived without reopening the strategy.

After approval, the next artifact is an **R1 implementation plan**, not a plan for
all 36 months. R2 and later phases get their own plans after their predecessors
ship.

---

## 16. Out of scope

- Code changes.
- Detailed task breakdowns and estimates (that is the implementation plan).
- Detailed API design for every standard-library module.
- Registry business model and hosting operations.
- Self-hosting implementation details.
- Async, traits, Unicode, WASM, and declarative-macro design details.
- Marketing, branding, and website design.
- Rewriting dated plans and specs.

---

## 17. Next steps

1. Review and approve this spec.
2. Write the R1 implementation plan with `writing-plans`.
3. Implement R1 in two-week slices.
4. Ship 0.2.0, gather feedback, and re-plan R2.
5. Repeat for each phase.
