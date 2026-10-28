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
| 16 | **The plan is an index plus one file per phase, all written up front and revised at phase start**, with detail falling off with distance | A single monolith covering the whole programme; or plans invented one at a time with no whole-programme view |
| 17 | **A dedicated optimization phase sits between speed and features** (new phase 4), covering both the dev loop (query pipeline, parallel front end) and the generated program (verifier facts as IR attributes, escape promotion, interning, LTO/PGO) | Folding optimization into phase 3; optimizing after the features exist; relying on LLVM alone |
| 18 | **Syntax growth is curated and additive** (new phase 5): eight items, no keyword renames, no attributes, no `?.`, no operator overloading. Every item desugars to existing HIR | A broad modernisation that renames `switch` and unifies bindings; performance-only syntax |
| 19 | **Rust-style OOP without inheritance**: traits are requirement sets, `impl` blocks carry methods, dispatch is static, there is no `Drop` and no operator overloading | Class inheritance; vtables and `dyn` in 1.0; a `Drop` trait replacing `defer` |
| 20 | **The C boundary is a phase of its own** (new phase 3b): bitwise operators, `link_name` and variadic `extern`, `repr(C)` with `sizeof`/`alignof`/`offsetof`, byte buffers, and `Option`/`Result` on the builtins that currently lie. It sits after speed and before optimization | Scattering these across the feature phases; treating FFI as "already done" because `extern "C"` parses; shipping `read_file` returning `""` on error for another three years |
| 21 | **Bitwise is checked, like arithmetic.** `x << 64` and `x << -1` panic with a position; there is no silent masking. `&`, `|`, `^` and `~` cannot overflow, so they have no checked variant. Operands must be the same integer type: `u8 & i64` is an error, not a promotion | Hardware masking of the shift count (silently wrong); implicit widening at the operator; separate `checked_shl`/`wrapping_shl` builtins |
| 22 | **The stdlib is a product, not a stub.** Thirteen modules holding about ten real functions is a documented absence, not a library. Phase 12 owns breadth: collections, text encodings, CSV, glob, terminal, date/time, hashing, entropy and process control | Calling the current stub "the stdlib"; ecosystem-first with no first-party packages; breadth before the language can express it |

---

## 3. Language identity and invariants

The identity rules that survive unchanged from every previous plan:

- No tracing GC. Ownership is linear and reference-counted.
- No borrow checker. Linearity and moves are enforced; lifetimes are not.
- No exceptions. Failure is a `Result`, a panic, or a process exit — and that
  applies to the builtins too, so no builtin answers `""` or `0` for "it failed"
  (§3.2.4).
- No implicit coercion. No operator overloading. No class inheritance.
- No power-user procedural macros. Declarative macros are the limit.
- C interop is explicit, marked, and inventoried — and **describable**: a type
  that crosses the boundary can say what its layout is (§3.2.2).

Two invariants are **promoted** by this plan:

- **A construct the verifier cannot analyse does not ship.** A feature and its
  proof land in the same phase or the feature waits.
- **No unproven claim.** No safety claim without a proof or a test; no speed
  claim without a number from Windows and Linux.

And one is **demoted**, deliberately:

- "As small a design space as Go" is now a *preference* applied per feature. It
  no longer forbids traits, iterators, async, macros, Unicode, named arguments or
  modules. The whitelist in §4.1 is what constrains surface growth instead.

### 3.1 Object model (Rust-style OOP, no inheritance)

The object model is stated here once so it is not inferred from a phase plan.
It is Rust's shape minus the parts that need a borrow checker or a vtable.

1. **One representation per type.** A `struct`, an `enum` or an `object` is the
   whole layout. There is no base type, no subtyping, no field inheritance.
2. **A trait is a requirement set, never a base type.** It declares method
   signatures and nothing else: no fields, no state, no constructors.
3. **`impl` carries behaviour.** `impl Point { .. }` is inherent; `impl Show for
   Point { .. }` satisfies a trait. `object` is a struct and its inherent impl
   written together. An impl must implement exactly the trait's methods, with
   matching receiver kinds.
4. **Dispatch is static in 1.0.** Every trait call names one impl at type-check
   time and is monomorphised. There is no `dyn`, no vtable, no runtime type
   information, so there is no call target the verifier cannot see.
5. **Two receivers.** `self` moves the value in; `self: *mut T` passes its
   address for mutation. There is no shared-borrow receiver, because there are no
   borrows — reading through a pointer receiver is explicit.
6. **No `Drop` trait.** Release is `defer` at scope exit, or `.free()` when the
   value is linear. A trait cannot run code when a value dies, because that code
   would run at an unbounded set of program points the verifier does not model.
7. **No operator overloading.** `+` is addition on numbers; a trait method is
   called by name. This is the §3 identity rule, restated where OOP readers will
   look for `Add`.
8. **Associated functions** have no receiver and are called `Type::name(..)`.
9. **Visibility applies to all of it.** A `pub` trait, method or field is usable
   across modules; everything else is private to its module (phase 6).

Default trait methods are deliberately absent in 1.0: a default body is code
that runs in every impl's context, and its effect and ownership rules need their
own design. A trait method is a signature, and every impl supplies the body.

### 3.2 The native foundation (bitwise, FFI, layout, buffers, honest failure)

Yashy claims to be "safer than C at C's speed". That claim needs the parts of C
that programs actually use, or the claim is about a language nobody can write a
program in. Phase 3b builds those parts; this section states them once.

**1. Bitwise operators.** `&`, `|`, `^`, `~`, `<<`, `>>` and the compound forms
`&=`, `|=`, `^=`, `<<=`, `>>=`. They are defined on every integer type —
`i8`…`i64`, `u8`…`u64`, `usize` — and on `bool` for the non-short-circuit
`&`/`|`/`^` (distinct from `&&`/`||`), and on nothing else: `~x` on a `float` or
a `string` is `E0103`, which names the operand's type. Operands must have the
same type: there is no implicit widening, so `u8 & i64` is `E0104` and the fix is
an explicit `as`.

Shifts are **checked**, matching arithmetic (`+` on `i64::MAX` panics today):

| Expression | Result |
|---|---|
| `x << n` where `0 <= n < bits(x)` | the shift |
| `x << n` where `n >= bits(x)` or `n < 0` | panic, `E0105`, at the operator's position |
| `x >> n` on a signed type | arithmetic shift; the sign bit is replicated |
| `x >> n` on an unsigned type | logical shift; zeroes are shifted in |

There is no `wrapping_shl`. A shift count that comes from data is either bounded
by the verifier's range domain or guarded by the programmer; a program that
masks the count writes `x << (n & 63)`, which says what it means.

`&`, `|`, `^` and `~` are total: they cannot overflow, so they have no checked
form and no diagnostic beyond the type rule.

**2. The C boundary is describable.** Three additions to the existing `extern`
block:

- **`link_name`.** `extern "C" link "m" { fn sqrt_raw(x: f64) -> f64 link_name "sqrt"; }`
  lets a declaration be called by one name and resolve to another symbol. Today
  the declaration's name *is* the symbol, which is why `stdlib/math.ya` computes
  `sqrt` with Newton's method instead of calling the hardware instruction — a
  self-inflicted performance and accuracy cost that this removes.
- **Variadic declarations.** `fn printf(fmt: *const u8, ...) -> i32;` — the `...`
  is legal only as the last parameter of an `extern` declaration, never on a
  Yashy function. Calling one is `unsafe` and the checker counts the fixed
  arguments only.
- **`repr(C)`.** `#[repr(C)]` is not an attribute system; it is one keyword form
  on a `struct`, `object` or fieldless `enum`, written `repr(C) struct P { .. }`.
  It means: lay the type out by the C ABI's rules (declaration order, C
  alignment and padding, `enum` as `int`). Without it, a type keeps Yashy's own
  layout, which is declaration order with natural alignment but no promise about
  padding. A `repr(C)` type may not have a type parameter, and an `enum` with
  payloads may not be `repr(C)` — `E0106`.

`sizeof(T)`, `alignof(T)` and `offsetof(T, field)` are compile-time constants of
type `usize`, usable anywhere a `const` is. `offsetof` on a non-`repr(C)` type is
`E0108`, because the offset is not part of the type's contract. These are what
make it possible to write a binary format, a syscall wrapper, or a binding to a C
library without hard-coding numbers that drift.

**3. Bytes are bytes.** `alloc(n)` counts `int` slots, which makes a file reader
allocate eight times what it needs. `alloc_bytes(n) -> *mut u8` and
`read_bytes(path) -> Result[[]u8, io.Error]` give the byte-level surface; a
`[]u8` view is an ordinary slice and every existing slice method applies. Byte
counts are `usize`; an `int` index into a byte buffer is `E0100` with a note, not
a silent cast.

**4. Failure is a value, including in builtins.** The builtins that today answer
`""` or `0` for "it failed" are changed, in one commit, to say so:

| Today | Becomes | Why |
|---|---|---|
| `read_file(path) -> string` (`""` on error) | `read_file(path) -> Result[string, io.Error]` | a missing file and an empty file are different |
| `write_file(path, body) -> int` (`1`/`0`) | `write_file(path, body) -> Result[void, io.Error]` | `1`/`0` is a boolean pretending to be an error |
| `getenv(name) -> string` (`""` unset) | `getenv(name) -> Option[string]` | unset and empty are different |
| `getcwd() -> string` (`""` on error) | `getcwd() -> Result[string, io.Error]` | same |
| `read_line() -> string` (`""` at EOF) | `read_line() -> Option[string]` | **EOF and an empty line are the same value today** |
| `parse_i64(s) -> int` (`0` on garbage) | `parse_i64(s) -> Option[int]` | `"abc"` and `"0"` are the same value today |
| `parse_f64(s) -> float` | `parse_f64(s) -> Option[float]` | same |

This is a breaking change to the standard library and the tutorial, and it is
deliberately one commit with no deprecation window: 1.0 has not shipped, and a
second way to spell "read a file" would be worse than the churn. The verifier
gains nothing here — `Result` and `Option` are already in its domain — which is
exactly why the change is cheap.

**5. Why this is one phase and not five.** Every item above is the same
sentence from a different side: *a program can say what it means when it talks to
the machine*. A bitmask with no `&` is a `switch` on powers of two; a C struct
with no `repr(C)` is a byte array with hand-computed offsets; a file read that
cannot fail is a lie the compiler repeats. They are also the last features that
must land **before** the optimizer, because bitwise operations, layout and
aliasing are precisely what phase 4's passes reason about.

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
  retrofitted at phase 11, because the shape agents consume is the expensive thing
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
from phases 1–2, which is why the order is verifier-then-speed. Phase 4 then
spends those proofs as *information*: `noalias` where ownership proves
uniqueness, `nonnull` where the type is not optional, `readonly` where the
effect is `pure`. C cannot state those facts; this language derives them.

| Workstream | Mechanism | Gate |
|---|---|---|
| **Dev loop** | Content-addressed cache (source + flags + compiler version), per-function IR reuse, `-O0` for `run`/`test`, `-O2` for `build`, `-O3` for `--release`, parallel test workers, fast-linker detection (`lld`, `mold`, fallback) | Incremental < 200 ms; 100 tests < 2 s; hello < 1 s |
| **Query pipeline** (phase 4) | Hand-written memoised queries (`parse`/`check`/`verify`/`emit`) keyed by content hash, parallel front end, incremental verification of changed functions and their dependents | Incremental check < 50 ms on a 10k-line project; ≥ 2 cores busy; byte-identical output |
| **Compiler rebuild** | Faster workspace linker, fewer generic instantiations, split slow test binaries, cached test binaries, `sccache` where present | Rebuild after touching `sema` < 60 s |
| **Program optimization** (phase 4) | Verifier facts as IR attributes, escape-to-stack/arena promotion, literal interning, inlined runtime fast paths, ThinLTO + PGO, a pass manager with a per-pass regression gate, and `--opt-report` naming why each check or allocation survived | fib ≤ 1.1× C; feature rows ≤ 1.8×; no pass regresses a row by more than 2%; every unelided check explained |

Measurement discipline (kept from the existing gate): `key=value` artifacts,
per-OS baselines, medians of 5, report-only on PRs, hard gate on scheduled and
manual runs. Three additions: a **dev-loop gate** (real project `yashy test`, not
just synthetic rows), a **per-phase regression budget** rather than a
per-release one, and a **per-pass gate** so a new optimization pass cannot buy
one row by selling another.

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

- `yashy doc`; debugger (DWARF + LLDB, async-aware after phase 10); profiler
  integration.
- Stdlib `log`, `trace`, `metrics` for structured program observability.
- Deterministic mode (fixed clock, RNG and environment) for reproducible tests
  and benchmarks.

### 6.4 Schema discipline

Schema v1 freezes at phase 11. Additive changes only. A CI schema-diff job fails
on removal or type change. Contracts, effects, typestate and the proof-trace
record appear in the schema from **phase 1**, so agents see proofs and proof
failures, not just errors — and the shape they consume is fixed before anything
depends on it.

---

## 7. Libraries, packages and the dev environment

| Piece | Phase | Content |
|---|---|---|
| Package manager MVP | 12 | `yashy.toml`, `yashy.lock`, path + git deps pinned to commits, no build scripts, `audit`, `--offline`, local registry prototype |
| Stdlib core | 6–12 | `io`, `fs`, `string` (Unicode), `math`, `JSON`, `time`, `log`, `path`, `rand` |
| First-party packages | 12 | `http` (client/server), `cli`, `net`, `test`, `bench`, `fuzz` |
| Precompiled stdlib | 12 | Stdlib ships as cached IR so `hello` does not re-typecheck it |
| Dev environment | 9–11 | LSP completion/references/rename/signature/code actions, `watch`, `fix`, `doc`, `task`, coverage, `lint`, playground |

**Organising idea: the stdlib is a package.** Versioned, tested, documented and
depended on like any third-party package, so the package manager is dogfooded by
the thing that needs it most, and a stdlib bug is a package bug caught by the same
audit and lockfile machinery.

---

## 8. The phase spine

Fourteen phases in fifteen slices, ~166 weeks (~38 months). Each phase ends with
its gate run on a clean checkout on Windows and Linux, a demo, and a recorded
commit. Phase 1 is split so a usable verifier exists at week 6 rather than only at
week 12, and phase 3b is a letter suffix rather than a renumbering so that every
phase after it keeps its number.

| # | Phase | Outcome | Gate | ~wks |
|---|---|---|---|---|
| **1a** | Verifier core, usable | `crates/yashy-verify` as the single owner of ownership (sema delegates); use-after-free, double-free, leak, `defer`; contracts v1 in **warn** mode; coded diagnostics + `--json`; minimal `--timings`; proof-trace record in schema v1 | Runs on every example without crashing; seeded-bug corpus for ownership/defer 100% caught; false positives mined and fixed | 6 |
| **1b** | Verifier core, enforcing | Bounds, overflow, exhaustiveness re-check; contracts as hard errors; **the warning surface becomes real** (unused binding, unused import, unused parameter, unreachable code, `var` never reassigned) with `-W`/`--deny warnings`; three realistic programs added to the corpus | Seeded-bug corpus 100% caught; zero false positives on stdlib, examples and the three realistic programs; unprovable contracts rejected; every warning has a code and a `--deny` test | 8 |
| 2 | Effects + typestate | `pure`/`io`/`async`/`unsafe`; race-freedom proof; typestate for file/mutex/channel | Effect inference sound; every `unsafe` inventoried; race tests rejected | 10 |
| 3 | Speed foundation | Cache, incremental, `-O0` dev defaults, parallel tests, fast linker, compiler rebuild speed; **closes with a 2-week async spike** (state-machine lowering against the verifier, scheduler sketch, no shipping code) | Incremental < 200 ms; 100 tests < 2 s; hello < 1 s; runtime ≤ 1.2× C; binary < 500 KB; spike notes reviewed | 12 |
| **3b** | Native foundation | Bitwise operators with checked shifts; `link_name` and variadic `extern`; `repr(C)` with `sizeof`/`alignof`/`offsetof`; byte buffers (`alloc_bytes`, `[]u8`); the seven builtins that answer `""`/`0` on failure become `Option`/`Result` | Every bitwise and shift case in the corpus has a diagnostic or a value; `libc.ya` calls `sqrt` through `link_name`; a `repr(C)` struct round-trips through C; no builtin returns a sentinel on failure | 8 |
| 4 | Optimization | Memoised query pipeline (parse/check/verify/emit), parallel front end, incremental verification; verifier facts as IR attributes (`noalias`, `nonnull`, `readonly`, alignment); escape-to-stack/arena promotion; interning and fast paths; inlined runtime calls; ThinLTO + PGO; pass manager with a per-pass gate; `--opt-report` | Incremental check < 50 ms; fib ≤ 1.1× C; feature rows ≤ 1.8×; no pass regresses a row by more than 2%; ASan/UBSan differential corpus clean | 10 |
| 5 | Syntax + ergonomics | Eight additive forms: `if let`/`while let`, `?` on `Option`, trailing block with closure inference, struct/array spread, chained comparison, `for i, x`, default field values, `arena { }`; plus **labeled `break`/`continue`** and **doc comments `///`**. Interpolation (`${…}`) and `const fn` already ship; this phase documents them rather than rebuilding them | Every form desugars to existing HIR; no new keywords; verifier corpus covers the three that touch ownership (`E1002`, `E1003`, `E1004`); a label never changes the meaning of an unlabelled `break` | 10 |
| 6 | Modules + Unicode | Namespacing, visibility, package-prefixed imports; grapheme-correct strings; module queries in the phase 4 pipeline | Large-project import test; Unicode conformance; verifier covers both | 10 |
| 7 | Patterns + named args | Guards, or-patterns, range patterns, nested destructuring; named/default arguments | Every form has verifier rules; exhaustiveness still proven | 8 |
| 8 | Traits, OOP + iterators | Interfaces, Rust-style OOP without inheritance (traits, impl, static dispatch, associated functions `Type::name`, member visibility, no `Drop`), generic constraints, **generic structs and enums**, iterator protocol, comprehensions | Dispatch proven safe; monomorphisation within budget; the OOP conformance program passes and the four excluded forms are rejected; a generic `Pair[T]` monomorphises like a generic function | 15 |
| 9 | Macros | Declarative, hygienic, `expand` | Verifier runs on expanded code; expansion deterministic and inspectable | 8 |
| 10 | Async | Executor, futures, state machines, structured concurrency, cancellation, debugger support. **Capped in writing:** one executor, no async traits, no generic runtime | Conformance suite; no data races proven; `async` effect enforced | 16 |
| 11 | AI-native + observability | `index`/`outline`/`graph`/`search`/`context`, MCP v0, sandbox v0, agent benchmark, debugger, profiler, tracing; **`yashy doc`** over the doc comments phase 5 parses; **`yashy lsp`** as a real subcommand | Context ≤ 8k tokens; escape tests; schema v1 frozen; benchmark runs; `yashy doc` renders every public item and `yashy lsp` answers initialize | 13 |
| 12 | Libraries + packages | Package manager MVP, first-party packages, precompiled stdlib; **stdlib breadth**: `Set`, `Deque`, `PriorityQueue`, `sort_by`, `regex`, `base64`/`hex`, `csv`, `glob`, `ANSI`, `datetime`, `hash`, OS entropy, `process.spawn` | Clean-checkout dependency build; real HTTP service; audit green; every module in `stdlib/` has a documented function with a test | 18 |
| 13 | 1.0 / 2.0 hardening | macOS, signed and reproducible releases, book/playground, registry prototype, self-hosting spike; **`fmt --check`**, **`--target`**, **`cfg`/platform conditionals** | All metrics met; no P0; reproducible build; CI fails on unformatted source; a program branches on platform with `cfg` | 14 |

**Why this order.** The verifier comes first because every later construct is
cheaper to prove at birth than to retrofit; phases 6–10 each extend it. Speed comes
third because the cache must understand the IR the verifier inspects — building it
earlier means rebuilding it. **The native foundation (3b) sits between speed and
optimization because it is the last language surface that must exist before the
optimizer is written**: bitwise operations, `repr(C)` layout and byte buffers are
what phase 4's escape promotion, interning and aliasing attributes reason about,
and an optimizer written before them would be rewritten after. It is also the
phase that repairs the builtin signatures, which is a breaking change and so is
cheapest while the standard library is still small. **Optimization comes fourth
because it is the phase that spends what phases 1–3b produced**: the query
pipeline makes the dev loop incremental, and verifier facts become `noalias`,
`nonnull` and `readonly` in the IR — facts C cannot state. Doing it before the
features means each feature lands on an optimizing, measured backend rather than
on a pipeline that has to be retrofitted underneath it. **Syntax comes fifth
because syntax churn is cheapest before modules, traits and macros exist**: one
parser change, one formatter change, one tutorial update, instead of five.
Modules precede AI-native because `index` and `graph` need a module graph to
describe. Libraries follow async because an HTTP server without it is a toy.

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
- **Plans up front, revised at phase start.** All phase plans are written before
  execution begins, with detail falling off with distance; the plan for the phase
  about to start is revised from the current code. The spec and the index are what
  a revision may not contradict.
- **Quarterly re-plan.** Adjust dates and the cut list, never the identity or the
  whitelist.
- **No silent scope growth.** A new idea goes to the next plan, not the current
  phase.

### 9.1 Cut order

Playground → profiler → coverage/lint → `task` → third-party registry → macOS
signing → `yashy doc` → `--target` → `cfg`/platform conditionals → generic structs
and enums → stdlib breadth (`regex` → `csv` → `glob` → `ANSI` → `datetime` →
`PriorityQueue` → `Deque` → `Set`) → `process.spawn` → OS entropy → variadic FFI
→ async cancellation → macro hygiene edge cases → chained comparison → indexed
`for` → struct spread → `arena` block → typestate for channels → iterator
comprehensions.

**Never cut:** the verifier, gate commands, both-OS CI, the benchmark gate, the
contract/effect schema — **and the native foundation**: bitwise operators,
`link_name`, `repr(C)`/`sizeof`, byte buffers, and the honest `Option`/`Result`
builtin signatures. Cutting those does not shrink the language, it withdraws the
claim that it can replace C. The warning surface is in the same class: a compiler
that never warns is a compiler whose `Warning` severity is decoration.

### 9.2 Definition of done

All fifteen phase gates pass on Windows and Linux; every construct has verifier
rules; schema v1 frozen; agent benchmark runs; 1.0 signed and reproducible; no P0
open.

---

## 10. Risks

| Risk | Mitigation |
|---|---|
| Verifier unsoundness manufactures confidence | Differential testing vs ASan/UBSan; negative test per proof rule; inventoried `unsafe` assumptions |
| Two implementations of one rule | `yashy-verify` is the only judge of ownership; `sema` delegates; a duplicated rule is a defect |
| False positives kill adoption | Zero-false-positive gate on stdlib, examples **and** three realistic programs; a false positive blocks release |
| 30-month drift | Quarterly re-plan; phase gates; cut list; dated exceptions; plan revised per phase, detail falling off with distance |
| Optimization promotes something that escapes | Escape promotion is gated on verifier facts agreeing; ASan/UBSan differential corpus is mandatory; `--no-opt=escape` for bisecting; a per-pass regression gate |
| Syntax growth becomes design-space creep | Eight named items, all desugaring to existing HIR; no renames, no attributes, no `?.`; anything else needs a written exception |
| OOP drifts toward inheritance or vtables | The object model is written in §3.1: one representation, traits without state, static dispatch, no `Drop`, no operator overloading |
| Async scope explosion | Written cap (one executor, no async traits, no generic runtime) plus a 2-week spike at the end of phase 3 |
| Bitwise or FFI semantics drift from C | A differential corpus: the same expression evaluated by Yashy and by a generated C program, compared bit for bit, on both OSes and both integer signednesses |
| The `repr(C)` surface grows into an attribute system | `repr(C)` is one keyword form on three item kinds; there is no `#[...]` syntax and no other `repr`, and a second `repr` is a written exception |
| The builtin-signature change breaks the tutorial and examples | It is one commit with no deprecation window, landing before 1.0; the tutorial harness compiles every block, so a stale example fails the build rather than the reader |
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

**The plan is an index plus one file per phase, all written up front.**
`docs/superpowers/plans/` holds:

- `2026-10-27-yashy-master-plan.md` — the index: the phase spine, the invariants,
  the gates, the cut list, and the status of each phase;
- one file per phase, **all fifteen written at the outset** so the programme can
  be reviewed and costed as one artefact:

  ```text
  2026-10-27-yashy-master-plan.md                    <- the index
  2026-10-27-phase-01a-verifier-core.md
  2026-10-27-phase-01b-verifier-enforcing.md
  2026-10-27-phase-02-effects-typestate.md
  2026-10-27-phase-03-speed-foundation.md
  2026-10-27-phase-03b-native-foundation.md
  2026-10-27-phase-04-optimization.md
  2026-10-27-phase-05-syntax-ergonomics.md
  2026-10-27-phase-06-modules-unicode.md
  2026-10-27-phase-07-patterns-named-args.md
  2026-10-27-phase-08-traits-oop-iterators.md
  2026-10-27-phase-09-macros.md
  2026-10-27-phase-10-async.md
  2026-10-27-phase-11-ai-native-observability.md
  2026-10-27-phase-12-libraries-packages.md
  2026-10-27-phase-13-hardening.md
  ```

A single 166-week file would be wrong by phase 3 and then stop being read, which
is how the previous plan set decayed. An index plus per-phase files keeps each
unit small enough to be read in one sitting. Writing them all up front is the
opposite risk — detail that is wrong by the time it is used — and the mitigation
is explicit: **detail falls off with distance.** Phases 1a–5 are executable to
the step; 6–10 name files and interfaces; 11–13 name deliverables and gates.
Each plan is **revised at phase start**, never rewritten from scratch, and the
revision may not contradict the spec or the index. That keeps "read the active
phase before changing behavior" in `AGENTS.md` true, and makes quarterly
re-plan a two-file edit instead of a rewrite.

Carried forward deliberately from the old set, because it worked: the benchmark
gate with per-OS baselines, schema-v1 JSON diagnostics, coded errors with
`explain`, the both-OS CI requirement, and the rule that a missed gate needs a
written exception.
