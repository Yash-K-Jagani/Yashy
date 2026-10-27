# Yashy Master Plan (Index)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn Yashy into a language whose programs are provably safer than C while staying as fast and as pleasant to build as Go, with a compiler that is equally legible to a human and to an agent.

**Architecture:** Eleven phases in twelve slices, ~124 weeks. A separate `yashy-verify` pass proves ownership, effects, typestate and contracts over the typed HIR; a content-addressed cache and `-O0` dev defaults make the loop fast; every later language construct ships with its verifier rules in the same phase.

**Tech Stack:** Rust workspace (11 crates), LLVM via `clang`/`llc`, a C runtime shim, and Yashy-source stdlib. No external prover, no new runtime dependencies.

**Spec:** `docs/superpowers/specs/2026-10-27-yashy-master-design.md` — read it before any phase. It is the authority on *why*; this file is the authority on *what next*; the phase plan is the authority on *how*.

## Global Constraints

Every task's requirements implicitly include this section.

- **No garbage collector, borrow checker, exceptions, implicit coercion, operator overloading, class inheritance, or procedural macros.** Declarative macros are the limit.
- **Ownership is explicit.** `move` transfers; `defer` releases at scope exit; arrays, maps, channels, mutexes and built strings use `.free()`.
- **Errors are values.** `Option[T]` and `Result[T, E]`; `?` propagates. No exceptions.
- **Tests are ordinary programs.** A file under `tests/` is a Yashy program with `main`; exit 0 is a pass. No `#[test]` attributes or test macros in `.ya` files.
- **No new runtime dependencies** without a plan and a reason. `yashy-verify` has zero external crates.
- **A malformed program produces a diagnostic, never a Rust panic.** The `never_panics` suite enforces this.
- **One rule, one implementation.** `yashy-verify` is the only judge of ownership; a rule duplicated across crates is a defect.
- **Every feature ships with tests, docs, diagnostics, and a machine-readable form.**
- **Both operating systems or it is not done:** every gate runs on Windows and Linux.
- **Do not claim macOS support** until a signed and tested archive ships.
- **Format touched files only.** `cargo fmt --all --check` has pre-existing drift; do not reformat unrelated files.
- **No unproven claim.** No safety claim without a proof or a test; no speed claim without a number from both operating systems.
- **No silent scope growth.** A new idea goes into the next phase plan, not the current one.

## Review Focus

The five failure modes most likely to bite a user of phase 1a, most likely first. Each has a test in the task that owns the code.

1. **A function that hands ownership out is reported as a leak.** Returning a pointer, storing it in a struct field, pushing it into an array, or placing it in a map must all count as "released". A false positive here rejects correct programs and is the fastest way to make the verifier hated.
2. **A merge point uses the right join for each question.** "May have been moved" is the **union** of the branches (so a read after a branch that freed is rejected); "released on every path" is the **intersection** (so a leak on the branch that skipped the release is still reported). Getting these backwards produces both false positives and missed leaks.
3. **A contract that mentions a parameter name gets a confusing error when the name is wrong.** The message must name the unknown identifier and the function, not report "cannot prove".
4. **`--no-verify` silently produces an unmarked binary.** The sidecar provenance file and the warning are the only evidence a build was unverified; if they are skipped, the escape hatch is invisible.
5. **`--timings` changes program output.** Timings go to stderr and to the JSON `timings` array only; stdout of `yashy run` must be byte-identical with and without the flag.

---

## How this plan is organised

`docs/superpowers/plans/` holds this index plus **one file per phase**. All twelve phase plans are written up front, so the whole programme can be read, reviewed and costed as one artefact:

```text
2026-10-27-yashy-master-plan.md                    <- this file
2026-10-27-phase-01a-verifier-core.md
2026-10-27-phase-01b-verifier-enforcing.md
2026-10-27-phase-02-effects-typestate.md
2026-10-27-phase-03-speed-foundation.md
2026-10-27-phase-04-modules-unicode.md
2026-10-27-phase-05-patterns-named-args.md
2026-10-27-phase-06-traits-iterators.md
2026-10-27-phase-07-macros.md
2026-10-27-phase-08-async.md
2026-10-27-phase-09-ai-native-observability.md
2026-10-27-phase-10-libraries-packages.md
2026-10-27-phase-11-hardening.md
```

**Detail falls off with distance, deliberately.** Phases 1a–3 are executable to the step: every task names its files, its interfaces and its test. Phases 4–8 name files and interfaces and leave step-level detail to the revision at phase start, because the code they touch does not exist yet. Phases 9–11 name deliverables and gates, because their shape depends on everything before them.

**A phase plan is revised at phase start, never rewritten from scratch.** The revision re-reads the code as it exists that day, keeps the outcome and the gate, and updates task detail. A plan that has drifted from the code is corrected in the revision; the spec and this index are what the revision may not contradict.

**A phase is not started until its predecessor's gate has passed and its plan has been revised.** The exception is a written exception record (see Gate protocol).

## Phase spine

| # | Phase | Outcome | Gate | ~wks | Status |
|---|---|---|---|---|---|
| 1a | Verifier core, usable | `yashy-verify` owns ownership; use-after-free, double-free, leak, `defer`; contracts in **warn** mode; `yashy verify` + `--json` + proof trace; `--timings` | Runs on every example without crashing; ownership/defer corpus 100% caught; false positives mined and fixed | 6 | **plan written** |
| 1b | Verifier core, enforcing | Bounds, overflow, exhaustiveness re-check; contracts as hard errors; three realistic programs in the corpus | Seeded-bug corpus 100% caught; zero false positives on stdlib, examples and the realistic programs; unprovable contracts rejected | 6 | plan written |
| 2 | Effects + typestate | `pure`/`io`/`async`/`unsafe`; race-freedom proof; typestate for file/mutex/channel | Effect inference sound; every `unsafe` inventoried; race tests rejected | 10 | plan written |
| 3 | Speed foundation | Cache, incremental, `-O0` dev defaults, parallel tests, fast linker, compiler rebuild speed; closes with a 2-week async spike | Incremental < 200 ms; 100 tests < 2 s; hello < 1 s; runtime ≤ 1.2× C; binary < 500 KB; spike notes reviewed | 12 | plan written |
| 4 | Modules + Unicode | Namespacing, visibility, package-prefixed imports; grapheme-correct strings | Large-project import test; Unicode conformance; verifier covers both | 10 | plan written |
| 5 | Patterns + named args | Guards, or-patterns, range patterns, nested destructuring; named/default arguments | Every form has verifier rules; exhaustiveness still proven | 8 | plan written |
| 6 | Traits + iterators | Interfaces, generic constraints, iterator protocol, comprehensions | Dispatch proven safe; monomorphisation within budget | 12 | plan written |
| 7 | Macros | Declarative, hygienic, `expand` | Verifier runs on expanded code; expansion deterministic and inspectable | 8 | plan written |
| 8 | Async | Executor, futures, state machines, structured concurrency, cancellation, debugger support. **Cap:** one executor, no async traits, no generic runtime | Conformance suite; no data races proven; `async` effect enforced | 16 | plan written |
| 9 | AI-native + observability | `index`/`outline`/`graph`/`search`/`context`, MCP v0, sandbox v0, agent benchmark, debugger, profiler, tracing | Context ≤ 8k tokens; escape tests; schema v1 frozen; benchmark runs | 12 | plan written |
| 10 | Libraries + packages | Package manager MVP, first-party packages, precompiled stdlib | Clean-checkout dependency build; real HTTP service; audit green | 12 | plan written |
| 11 | 1.0 / 2.0 hardening | macOS, signed and reproducible releases, book/playground, registry prototype, self-hosting spike | All metrics met; no P0; reproducible build | 12 | plan written |

Total ≈ 124 weeks.

**Why this order.** The verifier comes first because every later construct is cheaper to prove at birth than to retrofit; phases 4–8 each extend it. Speed comes third because the cache must understand the IR the verifier inspects. Modules precede AI-native because `index` and `graph` need a module graph. Libraries follow async because an HTTP server without it is a toy.

## Gate protocol

A phase is complete when all five hold, on a clean checkout:

1. The gate command in the phase plan passes on **Windows and Linux**.
2. The phase's demo runs: a command a reader can copy, with its expected output in the plan.
3. `cargo test --workspace` is green on both operating systems.
4. The commit hash is recorded in this table and in the phase plan.
5. The progress ledger at `.superpowers/sdd/<date>-<slice>/progress.md` is current.

A missed gate is recorded as an **exception**: a dated entry in the phase plan naming the gate, the reason, the owner, and when it will be re-run. Silent slips are not permitted; a phase with an open exception cannot be marked complete.

## Cut order

When a phase runs long, cut in this order: playground → profiler → coverage/lint → `task` → third-party registry → macOS signing → async cancellation → macro hygiene edge cases → typestate for channels → iterator comprehensions.

**Never cut:** the verifier, the gate commands, both-OS CI, the benchmark gate, or the contract/effect schema.

## Starting a phase

1. Read the spec §8 row for the phase and the previous phase's plan's "Outcome" section.
2. Read the current code the phase touches; the revision is written from today's code, not from the spec's memory of it.
3. Revise `docs/superpowers/plans/2026-10-27-phase-<id>-<name>.md` in place, keeping its header, outcome and gate.
4. Run its self-review against the spec: every spec requirement in that phase maps to a task.
5. Set this table's Status to `in progress`, then execute with `superpowers:executing-plans` or `superpowers:subagent-driven-development`.
