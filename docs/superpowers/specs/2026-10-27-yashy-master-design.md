# Yashy Master Design — Trust-First, Speed-Must, AI-Native for Humans and Agents

**Status:** approved in conversation 2026-10-27; this file is the design record.
**Supersedes:** `specs/history/2026-10-25-yashy-super-language-v2-design.md`,
`specs/history/2026-10-24-yashy-super-language-design.md`,
`specs/history/2026-10-23-yashy-rename-design.md`,
`specs/history/2026-10-16-otter-language-design.md`, and every plan now under
`docs/superpowers/plans/history/`.
**Capacity:** one maintainer, full-time.
**Horizon:** 24–36 months, planned at ~29.
**Companion:** the phase-by-phase master plan (written after this spec is
reviewed) is the only authoritative plan set after this document lands.

---

## 1. Purpose

Yashy should be a language whose programs are **provably safer than the ones you
can write in C, while staying as fast and as pleasant to build as Go** — and
whose compiler, tools and errors are equally legible to a human and to an agent.

The four goals, in the order they win when they conflict (settled 2026-10-27):

1. **Trust is a must.** Static guarantees are never traded for features.
2. **Speed is a must.** A slow loop taxes every later feature. Trust and speed
   are co-equal gates, not a ranking.
3. **AI-native is important, for humans and agents alike.** Not an agent-only
   mode, not a chat feature: the same facts, in two renderings.
4. **Adoption is a consequence.** It improves over time and is never a reason to
   cut 1–3.

### 1.1 Success criteria

| Axis | Measurable success |
|---|---|
| Trust | Every language construct has verifier rules; zero false positives on stdlib, examples and three realistic programs; seeded-bug corpus caught 100%; one implementation per rule |
| Speed | Incremental < 200 ms; 100 tests < 2 s; hello < 1 s; scalar runtime ≤ 1.2× C; feature rows ≤ 2×; hello binary < 500 KB |
| AI-native | Schema v1 frozen and stable; context ≤ 8k tokens; sandbox escape tests green; agent benchmark runs and is published |
| DX | 90% of common errors carry code + span + fix; first test < 1 min after `init`; LSP conformance in two editors |
| Libraries | A new project can serve HTTP, parse CLI arguments and test itself using first-party packages only |
| Reliability | Windows and Linux CI green on every phase; no P0 open at 1.0 |

### 1.2 Starting line (measured 2026-10-27)

| Fact | Value |
|---|---|
| Compiler + runtime | ~61k lines (`sema` 15.9k, `codegen` 12.9k, `driver` 9.6k, `parser` 8.9k, `runtime` 6.1k) |
| Tests | 80 Rust test files, 97 `.ya` programs |
| Compile | hello 318 ms, 1000-line program 2.3 s |
| Existing gates | benchmark gate (compile, binary, runtime), schema-v1 JSON diagnostics, both-OS CI |
| Existing ownership rules | `yashy-sema` already rejects double-move and use-after-move (`ownership.rs`, `scope.rs`); phase 1a moves the decision into `yashy-verify` |
| Timing | no `--timings` flag exists yet; it ships in phase 1a |

---

## 2. Decisions

Settled with the owner on 2026-10-27. Each records the rejected alternative so a
later reader does not reopen it by accident.

| # | Decision | Rejected alternative |
|---|---|---|
| 1 | **Trust and speed are both "must"; adoption is a consequence** | Adoption-first or AI-first ordering |
| 2 | **Strictness = separate verifier pass + opt-in contracts**, not a richer type system | Refinement/dependent types; verifier-free; verifying the compiler instead of programs |
| 3 | **"As small a design space as possible" is a preference, not an invariant** | Design-space minimalism as a hard rule (it would forbid traits, async and macros) |
| 4 | **Verifier surface whitelist: contracts + effects + typestate.** Nothing else without a written exception | Contracts only; per-phase whitelist |
| 5 | **No external SMT solver.** Hand-written decidable domain; unprovable contracts are rejected | Optional Z3; Z3 required |
| 6 | **Global content-addressed build cache**, shared across projects, with a size cap and `yashy cache clean` | Per-project cache only; no cache |
| 7 | **Observability covers both the compiler and the programs it produces** | Compiler-only; program-only |
| 8 | **Hybrid libraries: small stable core + first-party packages**, with the stdlib versioned as a package | Batteries-included stdlib; minimal stdlib with ecosystem-first |
| 9 | **Verifier first, then speed, then features in dependency order** (the trust ratchet) | Feature-first with verification last; alternating tracks |
| 10 | **Every phase carries one experience slice** so no phase is purely language or purely infrastructure | Pure language phases; pure infrastructure phases |
| 11 | **All eight requested features are in scope** over 24–36 months | Deferring async, macros or named arguments to a later plan |
| 12 | **Old plans and specs are archived, not deleted.** "Forget" means "no longer authoritative" | Deleting history |
| 13 | **`yashy-verify` is the only judge of ownership**; `sema` keeps type classification and delegates | Two implementations of one rule; verifier as a second opinion |
| 14 | **`--no-verify` is narrow**: no contracts/effects/typestate, warns, stamps unverified provenance, fatal under `YASHY_REQUIRE_VERIFY=1` | An unrestricted escape hatch |
| 15 | **Phase 1 is split into 1a (usable, warn mode) and 1b (enforcing)**, and `--timings` ships in 1a | One 12-week phase with no artifact until the end; measuring the 30% budget only from phase 3 |
| 16 | **The plan is an index plus one just-in-time file per phase** | A single 124-week plan file written up front |

---

## 3. Language identity and invariants

The identity rules that survive unchanged from every previous plan:

- No tracing GC. Ownership is linear and reference-counted.
- No borrow checker. Linearity and moves are enforced; lifetimes are not.
- No exceptions. Failure is a `Result`, a panic, or a process exit.
- No implicit coercion. No operator overloading. No class inheritance.
- No power-user procedural macros. Declarative macros are the limit.
- C interop is explicit, marked, and inventoried.

Two invariants are **promoted** by this plan:

- **A construct the verifier cannot analyse does not ship.** A feature and its
  proof land in the same phase or the feature waits.
- **No unproven claim.** No safety claim without a proof or a test; no speed
  claim without a number from Windows and Linux.

And one is **demoted**, deliberately:

- "As small a design space as Go" is now a *preference* applied per feature. It
  no longer forbids traits, iterators, async, macros, Unicode, named arguments or
  modules. The whitelist in §4.1 is what constrains surface growth instead.

---

## 4. The verification system

### 4.1 Surface whitelist

The verifier may introduce exactly these three constructs. Anything else needs a
written exception recorded in the master plan, with a reason and a date.

| Construct | Syntax (shape, not final spelling) | Buys |
|---|---|---|
| **Contracts** | `requires` / `ensures` on functions; loop invariants | Pre/postconditions, bounds and overflow where provable |
| **Effects** | `pure`, `io`, `async`, `unsafe` on functions | No hidden IO, no races, no async in pure code |
| **Typestate** | Resource states, e.g. `open` → `closed` on file handles, `locked` → `unlocked` on mutexes | Double-close, use-after-close, double-unlock become compile errors |

### 4.2 Architecture

`crates/yashy-verify` is a pass over the typed AST that `sema` already produces.
It does not replace the type checker; types stay as they are and the verifier
adds proofs on top. This is what makes phase 1 shippable without destabilising
`yashy-sema`.

**Invariant: one rule, one implementation.** `sema` already rejects double-move
and use-after-move today — `crates/yashy-sema/src/ownership.rs` holds
`Linearity` and `Live`/`Moved`, and `scope.rs` joins moved sets across branches.
Two implementations of the same rule will eventually disagree, and then neither
is authoritative. So `yashy-verify` becomes the **only** judge of ownership:
`sema` keeps `linearity_of` (a pure type classification) and delegates every
ownership decision to the verifier. Migration is part of phase 1a, not a later
cleanup, and a rule duplicated in both crates is a defect.

| Property | Mechanism |
|---|---|
| Ownership linearity, use-after-free, double-free, leak | Per-variable ownership state machine over the CFG (owned → moved → freed → dead), joined at merges |
| `defer` discipline | Same machine plus scope-exit edges; double-`defer` and unreachable release are errors |
| Bounds and overflow | Interval + congruence domain; facts outside the domain are **rejected, never assumed** |
| Exhaustiveness | Re-checked after macro expansion |
| Contracts | Discharged in the same domain; unprovable ⇒ compile error |
| Effects | Inferred per function, checked against declared effects; callers inherit |
| Race freedom | Shared values must sit behind mutex/atomic/channel typestate; unproven sharing is an error |
| Typestate | Per-resource state lattice over the same CFG machinery |

**Prover backend:** an internal decidable domain (intervals, congruence,
ownership, effects, typestate). No external solver, no new dependency, predictable
time. The accepted cost: some true programs are rejected because the domain
cannot discharge them. That is the sound direction of failure.

### 4.3 How it runs

- `yashy verify` — standalone, human and `--json` output.
- `yashy build` — runs the verifier by default. An unverified binary is not a
  build.
- `--no-verify` is deliberately narrow, because an unrestricted escape hatch
  becomes the default path and quietly voids the guarantee:
  - legal only for a program that declares **no** contracts, effects or
    typestate;
  - always prints a warning;
  - stamps *unverified* provenance that `yashy inspect` reports and that
    `yashy --version`-style metadata keeps with the binary;
  - `YASHY_REQUIRE_VERIFY=1` (set in CI) turns any use into a hard error.
- Diagnostics get their own code range and reuse schema v1, so an agent receives
  proof failures and errors in one shape.
- **Proof traces:** a rejection explains which fact could not be established and
  where. "Cannot prove" alone is not acceptable output. The trace record is part
  of schema v1 **from phase 1** — it is defined with the diagnostics, not
  retrofitted at phase 9, because the shape agents consume is the expensive thing
  to change later.

### 4.4 Budget and evidence

- Verifier ≤ 30% of `check` wall time; results cached per function. This budget is
  measured from phase 1, which is why a minimal `--timings` (per-phase wall time,
  human and `--json`) ships with 1a instead of waiting for phase 3.
- Seeded-bug corpus: at least one program per property, all must be caught.
- Zero false positives on stdlib, examples, **and three realistic programs**
  ported in phase 1 (`wordcount`, `json-report`, a file-IO CLI). The small
  programs do not exercise aliasing or real control flow, so a corpus of them
  alone would pass trivially and manufacture confidence.
- `unsafe` and FFI boundaries carry explicit, inventoried assumptions, so
  "proven" never silently means "trusted".

### 4.5 Non-goals

- Not a dependent type system. No term-level indices in types.
- Not a theorem prover. Unprovable ≠ false; unprovable ⇒ rejected.
- No runtime reflection API. Read-only metadata lives in `index` at compile time.

---

## 5. The speed program

Trust and speed multiply: every proof the verifier produces is a licence to
delete a runtime check. Bounds and overflow elision in phase 3 depend on proofs
from phases 1–2, which is why the order is verifier-then-speed.

| Workstream | Mechanism | Gate |
|---|---|---|
| **Dev loop** | Content-addressed cache (source + flags + compiler version), per-function IR reuse, `-O0` for `run`/`test`, `-O2` for `build`, `-O3` for `--release`, parallel test workers, fast-linker detection (`lld`, `mold`, fallback) | Incremental < 200 ms; 100 tests < 2 s; hello < 1 s |
| **Compiler rebuild** | Faster workspace linker, fewer generic instantiations, split slow test binaries, cached test binaries, `sccache` where present | Rebuild after touching `sema` < 60 s |
| **Runtime and binary** | Verifier-driven bounds/overflow elision, escape analysis (stack/arena promotion), inlined runtime calls, array/debug/tuple fast paths, string interning, LTO/PGO at release | Scalar ≤ 1.2× C; feature rows ≤ 2×; hello < 500 KB |

Measurement discipline (kept from the existing gate): `key=value` artifacts,
per-OS baselines, medians of 5, report-only on PRs, hard gate on scheduled and
manual runs. Two additions: a **dev-loop gate** (real project `yashy test`, not
just synthetic rows) and a **per-phase regression budget** rather than a
per-release one.

**Cache design:** one content-addressed store under the user profile, shared by
all projects, with a per-project lockfile, a size cap, `yashy cache clean`, and
`yashy cache verify` that recomputes a key. CI never trusts a cache it did not
create.

---

## 6. Observability and the AI-native contract

**Governing rule: one fact, two renderings.** The human output is a rendering of
the JSON model, not a separate code path. That is what makes the tooling serve a
person and an agent equally.

### 6.1 Compiler observability

- `--timings` per phase; cache hit/miss per compilation. The minimal version ships
  in phase 1a so the verifier's 30% budget is measurable from the day the budget
  exists.
- Proof traces for every verifier rejection.
- `yashy cache stats` and `yashy verify --json` summaries.

### 6.2 Tooling for agents

- `index` — symbols, signatures, contracts, effects, typestate, tests, imports.
- `outline`, `graph`, `search`, `context` (≤ 8k tokens).
- MCP v0 exposing `check`, `build`, `test`, `verify`, `explain`, `context`,
  `index` — never arbitrary shell execution.
- Sandbox v0: default-deny capabilities, resource limits, escape tests in CI.
- Agent benchmark: 10 tasks, held-out variants, measuring success, tokens, tool
  calls and sandbox violations. No benchmark, no claim.

### 6.3 Tooling for humans

- `yashy doc`; debugger (DWARF + LLDB, async-aware after phase 8); profiler
  integration.
- Stdlib `log`, `trace`, `metrics` for structured program observability.
- Deterministic mode (fixed clock, RNG and environment) for reproducible tests
  and benchmarks.

### 6.4 Schema discipline

Schema v1 freezes at phase 9. Additive changes only. A CI schema-diff job fails
on removal or type change. Contracts, effects, typestate and the proof-trace
record appear in the schema from **phase 1**, so agents see proofs and proof
failures, not just errors — and the shape they consume is fixed before anything
depends on it.

---

## 7. Libraries, packages and the dev environment

| Piece | Phase | Content |
|---|---|---|
| Package manager MVP | 10 | `yashy.toml`, `yashy.lock`, path + git deps pinned to commits, no build scripts, `audit`, `--offline`, local registry prototype |
| Stdlib core | 4–10 | `io`, `fs`, `string` (Unicode), `math`, `JSON`, `time`, `log`, `path`, `rand` |
| First-party packages | 10 | `http` (client/server), `cli`, `net`, `test`, `bench`, `fuzz` |
| Precompiled stdlib | 10 | Stdlib ships as cached IR so `hello` does not re-typecheck it |
| Dev environment | 7–9 | LSP completion/references/rename/signature/code actions, `watch`, `fix`, `doc`, `task`, coverage, `lint`, playground |

**Organising idea: the stdlib is a package.** Versioned, tested, documented and
depended on like any third-party package, so the package manager is dogfooded by
the thing that needs it most, and a stdlib bug is a package bug caught by the same
audit and lockfile machinery.

---

## 8. The phase spine

Eleven phases in twelve slices, ~124 weeks (~29 months). Each phase ends with its
gate run on a clean checkout on Windows and Linux, a demo, and a recorded commit.
Phase 1 is split so a usable verifier exists at week 6 rather than only at week 12.

| # | Phase | Outcome | Gate | ~wks |
|---|---|---|---|---|
| **1a** | Verifier core, usable | `crates/yashy-verify` as the single owner of ownership (sema delegates); use-after-free, double-free, leak, `defer`; contracts v1 in **warn** mode; coded diagnostics + `--json`; minimal `--timings`; proof-trace record in schema v1 | Runs on every example without crashing; seeded-bug corpus for ownership/defer 100% caught; false positives mined and fixed | 6 |
| **1b** | Verifier core, enforcing | Bounds, overflow, exhaustiveness re-check; contracts as hard errors; three realistic programs added to the corpus | Seeded-bug corpus 100% caught; zero false positives on stdlib, examples and the three realistic programs; unprovable contracts rejected | 6 |
| 2 | Effects + typestate | `pure`/`io`/`async`/`unsafe`; race-freedom proof; typestate for file/mutex/channel | Effect inference sound; every `unsafe` inventoried; race tests rejected | 10 |
| 3 | Speed foundation | Cache, incremental, `-O0` dev defaults, parallel tests, fast linker, compiler rebuild speed; **closes with a 2-week async spike** (state-machine lowering against the verifier, scheduler sketch, no shipping code) | Incremental < 200 ms; 100 tests < 2 s; hello < 1 s; runtime ≤ 1.2× C; binary < 500 KB; spike notes reviewed | 12 |
| 4 | Modules + Unicode | Namespacing, visibility, package-prefixed imports; grapheme-correct strings | Large-project import test; Unicode conformance; verifier covers both | 10 |
| 5 | Patterns + named args | Guards, or-patterns, range patterns, nested destructuring; named/default arguments | Every form has verifier rules; exhaustiveness still proven | 8 |
| 6 | Traits + iterators | Interfaces, generic constraints, iterator protocol, comprehensions | Dispatch proven safe; monomorphisation within budget | 12 |
| 7 | Macros | Declarative, hygienic, `expand` | Verifier runs on expanded code; expansion deterministic and inspectable | 8 |
| 8 | Async | Executor, futures, state machines, structured concurrency, cancellation, debugger support. **Capped in writing:** one executor, no async traits, no generic runtime | Conformance suite; no data races proven; `async` effect enforced | 16 |
| 9 | AI-native + observability | `index`/`outline`/`graph`/`search`/`context`, MCP v0, sandbox v0, agent benchmark, debugger, profiler, tracing | Context ≤ 8k tokens; escape tests; schema v1 frozen; benchmark runs | 12 |
| 10 | Libraries + packages | Package manager MVP, first-party packages, precompiled stdlib | Clean-checkout dependency build; real HTTP service; audit green | 12 |
| 11 | 1.0 / 2.0 hardening | macOS, signed and reproducible releases, book/playground, registry prototype, self-hosting spike | All metrics met; no P0; reproducible build | 12 |

**Why this order.** The verifier comes first because every later construct is
cheaper to prove at birth than to retrofit; phases 4–8 each extend it. Speed comes
third because the cache must understand the IR the verifier inspects — building it
earlier means rebuilding it. Modules precede AI-native because `index` and `graph`
need a module graph to describe. Libraries follow async because an HTTP server
without it is a toy.

**Why the async spike sits in phase 3.** Async is the largest single phase and
lands around month 18, where drift is most likely. Two weeks of design against the
finished verifier — before traits, macros and modules are in the way — is the
cheapest point at which the unknown can be reduced.

---

## 9. Governance

- **Trust ratchet.** No phase adds a construct without its verifier rules.
- **Every phase carries one experience slice**, so there is always a visible win.
- **Gate or exception.** A missed gate gets a written exception with a reason,
  an owner and a date. No silent slips.
- **One slice at a time.** No parallel compiler slices.
- **Plan just-in-time.** The phase plan is written when the phase is about to
  start, from the index and the current code; it is not forecast 30 months ahead.
- **Quarterly re-plan.** Adjust dates and the cut list, never the identity or the
  whitelist.
- **No silent scope growth.** A new idea goes to the next plan, not the current
  phase.

### 9.1 Cut order

Playground → profiler → coverage/lint → `task` → third-party registry → macOS
signing → async cancellation → macro hygiene edge cases → typestate for channels →
iterator comprehensions.

**Never cut:** the verifier, gate commands, both-OS CI, the benchmark gate, or the
contract/effect schema.

### 9.2 Definition of done

All twelve gates pass on Windows and Linux; every construct has verifier rules;
schema v1 frozen; agent benchmark runs; 1.0 signed and reproducible; no P0 open.

---

## 10. Risks

| Risk | Mitigation |
|---|---|
| Verifier unsoundness manufactures confidence | Differential testing vs ASan/UBSan; negative test per proof rule; inventoried `unsafe` assumptions |
| Two implementations of one rule | `yashy-verify` is the only judge of ownership; `sema` delegates; a duplicated rule is a defect |
| False positives kill adoption | Zero-false-positive gate on stdlib, examples **and** three realistic programs; a false positive blocks release |
| 30-month drift | Quarterly re-plan; phase gates; cut list; dated exceptions; plan written per phase, not all at once |
| Async scope explosion | Written cap (one executor, no async traits, no generic runtime) plus a 2-week spike at the end of phase 3 |
| Cache staleness produces a wrong binary | Key covers compiler version, flags, source hash; `cache verify`; CI never trusts a foreign cache |
| Design-space creep | Closed whitelist; anything else needs a written exception |
| Burnout | Visible win per phase; phases shrink instead of slipping |
| Benchmark noise | Medians of 5; per-OS baselines; report-only PRs; hard gate on schedule |

---

## 11. What this supersedes, and how the plan is shaped

All nine files that were in `docs/superpowers/plans/` have moved to
`docs/superpowers/plans/history/`. The four superseded specs moved to
`docs/superpowers/specs/history/`. Nothing is deleted; history stays readable, and
every old plan is explicitly non-authoritative.

**The plan is an index, not a monolith.** `docs/superpowers/plans/` will hold:

- `2026-10-27-yashy-master-plan.md` — the index: the phase spine, the invariants,
  the gates, the cut list, and the status of each phase;
- one file per phase (`…-phase-01a-verifier-core.md`, `…-phase-01b-…`), written
  **just before that phase starts**, not all at the outset.

A single 124-week file would be wrong by phase 3 and then stop being read, which
is how the previous plan set decayed. An index plus a just-in-time phase plan
matches "read the active phase before changing behavior" in `AGENTS.md` and makes
quarterly re-plan a two-file edit instead of a rewrite.

Carried forward deliberately from the old set, because it worked: the benchmark
gate with per-OS baselines, schema-v1 JSON diagnostics, coded errors with
`explain`, the both-OS CI requirement, and the rule that a missed gate needs a
written exception.
