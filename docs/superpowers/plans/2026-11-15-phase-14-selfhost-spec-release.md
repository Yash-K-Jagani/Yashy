# Phase 14 — Self-Hosting, Specification and the 2.0 Release

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the language: a Yashy front end that agrees with the Rust
reference on the whole grammar, a language reference that specifies what the
compiler does (including the memory, aliasing and determinism contracts), a
conformance suite that runs on every claimed platform, an edition with a
migration path, and a signed, reproducible, install-tested 2.0 release.

**Outcome:** The `selfhost/` front end covers the full 2.0 grammar with zero
mismatches and bootstraps a third stage that is byte-identical to the second; a
versioned language reference and a conformance suite (`yashyc conformance`) pass
on every claimed target; `edition = "2027"`, `yashyc migrate` and a deprecation
policy ship; 1.0 gets an 18-month LTS; the RFC process, security audit, CVE
process and hardening land; and the 2.0 archives are signed, reproducible,
install-tested on Windows, Linux and macOS, and published with an SBOM.

**Time:** ~10 weeks.
**Depends on:** Phase 13 gate.
**Spec:** [`../specs/2026-11-15-yashy-2.0-design.md`](../specs/2026-11-15-yashy-2.0-design.md) §6.9, §12, §13, §14.

## Why this phase

Two of the strongest claims in 2.0 are only checkable once the language stops
moving. "The language is specified" means a second implementation can be written
from the document and agree; "the toolchain is trustworthy" means a stranger can
rebuild the release and get the same bytes. Both are last on purpose: a
specification of a moving language is a changelog, and a bootstrap of a changing
grammar is a second job.

The existing `selfhost/` spike is the right starting point and an honest
measurement: a Yashy lexer and parser for a declared subset, with a differential
that classifies every corpus file as *agree*, *reproduced*, *unsupported* or
*mismatch*. Phase 14 grows the subset to the full grammar and makes *unsupported*
zero. It does not make the Yashy front end the source of truth: the Rust front
end stays the reference for the 2.0 release (design call 7).

## Starting point

| Fact | 1.0 value |
|---|---|
| `selfhost/` | lexer + parser for a declared subset; `main.ya` renders a canonical digest |
| Differential | 42 agree / 121 unsupported / 0 mismatched |
| Lexer standard | token kinds and spans agree on every corpus file |
| Bootstrap | none |
| Language reference | docs, tutorial and error catalog; de facto spec |
| Conformance suite | none; `yashyc conformance` absent |
| Editions | none; no `edition` key, no `migrate` |
| LTS / RFC / deprecation | none |
| Security audit / CVE process | none |
| Release | archives per OS; signed secrets absent; `--reproducible` exists |
| macOS | unsigned, unclaimed |
| Platform claim | Windows and Linux |

## Deliverables

### Task 1: The full-grammar Yashy front end

**Files:**
- Modify: `selfhost/src/lexer.ya`, `selfhost/src/parser.ya`, `selfhost/src/ast.ya`, `selfhost/main.ya`
- Create: `selfhost/src/{types,pat,items}.ya`
- Modify: `crates/yashy-driver/tests/selfhost_diff.rs`
- Test: `selfhost/tests/differential.ya`, `crates/yashy-driver/tests/programs/selfhost/`

- [ ] **Step 1: Write the failing test.** The differential classifies every
  file under `docs/tutorial/`, `crates/yashy-driver/tests/programs/`, `packages/`,
  `examples/`, `selfhost/` and `playground/` as *agree* or *reproduced*;
  *unsupported* and *mismatch* are both failures. Cover every 2.0 form: generic
  traits, associated types, default methods, `else if`, `let else`, slice/rest
  and `@` patterns, `matches`, derives, `unsafe`, typed pointers, `repr` clauses,
  `defer expr`, `cfg`, named/default arguments, `async`/`scope`/`.await` and SIMD
  types.
- [ ] **Step 2: The lexer stays the stricter contract.** Token kind and span
  agree on every corpus file, including the error cases: a malformed program
  produces the same first token error on both front ends. The digest is
  canonical — sorted, stable and byte-identical across runs and OSes.
- [ ] **Step 3: Errors are part of the differential.** A file both front ends
  reject is *reproduced* only when the first error has the same span and the same
  code; a different code is a mismatch. This is what makes the differential a
  front-end test rather than a happy-path test.
- [ ] **Step 4: The subset declaration shrinks to nothing.** The spike's
  `unsupported` list is a checked file; it reaches zero in this task and any new
  entry is a regression.
- [ ] **Step 5: Commit.**

### Task 2: Bootstrap

**Files:**
- Create: `selfhost/stage1.toml`, `scripts/bootstrap.*`
- Modify: `crates/yashy-driver/tests/selfhost_diff.rs`, `docs/gates.md`
- Test: `crates/yashy-driver/tests/bootstrap.rs`

- [ ] **Step 1: Write the failing test.** Stage 0 (the Rust-built
  `yashyc`) builds the Yashy front end; the result (stage 1) builds itself
  (stage 2); stage 2 and stage 3 are byte-identical when given the same inputs,
  and all stages produce the same digest for every corpus file.
- [ ] **Step 2: Bootstrap is reproducible, not just converging.** Fixed stage
  ordering, no timestamps, no absolute paths, no environment reads; a stage with
  a different input hash is recorded and compared. `scripts/bootstrap` runs the
  whole chain and prints a stage-by-stage hash table.
- [ ] **Step 3: The front end is the proof, not the compiler.** The bootstrap
  covers lexer and parser only; the Rust front end remains the reference for the
  release (design call 7). A full self-hosted compiler is a later record, and the
  plan says so in writing rather than implying it.
- [ ] **Step 4: Commit.**

### Task 3: The language reference

**Files:**
- Create: `docs/reference/` (one page per chapter: grammar, types, items,
  patterns, ownership, effects, contracts, typestate, async, FFI/ABI, memory
  model, aliasing, determinism, layout, attributes/`repr`, editions)
- Create: `docs/reference/grammar.ebnf` (generated from the parser)
- Modify: `docs/architecture.md` (link, never contradict)
- Test: `crates/yashy-driver/tests/reference.rs`, `crates/yashy-driver/tests/docs.rs`

- [ ] **Step 1: Write the failing test.** Every chapter has a grammar block
  that `yashyc conformance` parses, a rule id per normative sentence, and a
  link from the rule to its diagnostic code and its conformance case. The
  EBNF is generated from the parser's own tables and a hand edit fails the test.
- [ ] **Step 2: The memory model is specified, not described.** Provenance,
  the owning/FFI pointer split, the `noalias` rule, the null niche, layout and
  `repr`, zeroed allocation and the allocator interface, stack promotion and
  `stack_alloc`, and the determinism contract each get a normative section with
  its corpus. These are the sections Phase 11A fixed; the reference states what
  the optimizer is allowed to assume.
- [ ] **Step 3: Two readings agree.** The reference, `docs/architecture.md` and
  the error catalog are reconciled: a rule id appears in all three or in none.
  A drift test fails on a contradiction, not just a missing link.
- [ ] **Step 4: Commit.**

### Task 4: The conformance suite

**Files:**
- Create: `conformance/` (cases, expected diagnostics, expected output), `crates/yashy-driver/src/conformance.rs`
- Modify: `crates/yashy-driver/src/main.rs` (`conformance`)
- Create: `docs/conformance.md` (generated)
- Test: `crates/yashy-driver/tests/conformance.rs`

**Interfaces:**
- `Case { id, rule, source, expected: Compiles | Rejects(Code, Span) | Runs(stdout, exit) }`
- `run_conformance(target: &Target, filter: &Filter) -> Report`

- [ ] **Step 1: Write the failing test.** Every rule id in the reference has at
  least one case, and every case names its rule. Run the suite against a target
  and assert every case's expected outcome. A case with no rule or a rule with no
  case fails the suite — the Phase 9 traceability matrix is the index.
- [ ] **Step 2: The suite is the platform gate.** Every claimed target runs it
  natively or under an emulator; `conformance --json` reports per-target, per-rule
  results and the suite is required in CI before a release artifact is published.
- [ ] **Step 3: Negative cases are first-class.** A rejection case asserts the
  code, the span and the suggestion; a diagnostic that changes code is a
  conformance diff, which is a breaking change and needs an RFC.
- [ ] **Step 4: Commit.**

### Task 5: Editions, migration and deprecation

**Files:**
- Modify: `crates/yashy-manifest/src/manifest.rs` (`edition`), `crates/yashy-driver/src/pipeline.rs`
- Create: `crates/yashy-driver/src/migrate.rs`
- Modify: `crates/yashy-driver/src/main.rs` (`migrate`)
- Create: `docs/editions.md`
- Test: `crates/yashy-driver/tests/migrate.rs`, `crates/yashy-driver/tests/edition.rs`

- [ ] **Step 1: Write the failing test.** A 1.0 project with no `edition` key
  builds unchanged under the default; `yashyc migrate` rewrites it to
  `edition = "2027"`, reports every semantic change with a diff, and the result
  passes `check`, `test` and `verify`. `--dry-run` prints the diff and changes
  nothing. A file with an unknown edition is a diagnostic naming the valid set.
- [ ] **Step 2: An edition is a compatibility promise.** Within an edition, no
  program that compiled stops compiling; a change that breaks one is a new
  edition with a migration. The compiler keeps reading every edition it has
  shipped.
- [ ] **Step 3: Deprecation has a path.** A deprecated item warns with a code
  and a `yashyc fix`; it is removed only in an edition, and the removal is listed
  in the migration notes. There is no silent removal.
- [ ] **Step 4: Commit.**

### Task 6: LTS, RFC and governance in the repo

**Files:**
- Create: `docs/rfcs/0000-template.md`, `docs/rfcs/README.md`, `docs/lts.md`, `docs/deprecation.md`
- Modify: `docs/superpowers/specs/2026-11-15-yashy-2.0-design.md` (record the ratified calls)
- Test: `crates/yashy-driver/tests/docs.rs` (extend)

- [ ] **Step 1: Write the failing test.** The RFC template has every field the
  design requires (motivation, syntax, HIR, verifier rules, diagnostics,
  migration, benchmark, doc impact, decision). An RFC that changes a rule id, a
  diagnostic code or the determinism contract cannot be marked accepted without
  the conformance and migration sections filled.
- [ ] **Step 2: LTS is a written commitment.** 1.0 receives security and
  critical fixes for 18 months after 2.0; 2.0 is the new LTS baseline. The
  support matrix names versions, dates and what qualifies as a security fix.
- [ ] **Step 3: The process is in the repo, not in a slide.** A contributing
  page, a decision log, and a quarterly re-plan record whose diff shows only
  dates and the cut list changing — never the identity, the non-goals or the
  never-cut list.
- [ ] **Step 4: Commit.**

### Task 7: Hardening and the signed, reproducible 2.0 release

**Files:**
- Create: `docs/security-audit-2027.md`, `docs/cve-process.md`, `RELEASE.md`
- Modify: `.github/workflows/release.yml`, `scripts/install/*`, `crates/yashy-driver/src/main.rs` (`self-update`)
- Test: `scripts/install/test-install.*`, `crates/yashy-driver/tests/reproducible.rs`, `crates/yashy-driver/tests/release.rs`

- [ ] **Step 1: Write the failing test.** Build the release twice in two
  directories on Windows, Linux and macOS; the archives are byte-identical, the
  SBOM matches, every artifact has a detached signature, and the installers
  verify it before installing. Tamper with one archive byte and assert the
  installer and `self-update` both refuse it.
- [ ] **Step 2: Four hardening gates before the tag.** The full fuzz corpus
  (parsers, JSON/TOML/YAML/CSV/XML, HTTP, WebSocket, RESP3, WASM) has no open
  finding; ASan/UBSan and TSan are clean on the corpus; the alias and determinism
  differentials are green; the sandbox escape corpus is empty on both OSes.
- [ ] **Step 3: The audit is external and written down.** A third-party review of
  the memory model, `unsafe` sites, FFI, crypto wrappers, TLS configuration and
  the sandbox; findings, fixes and accepted exceptions with dates. The CVE
  process names the contact, the window and the advisory format.
- [ ] **Step 4: macOS is claimed only now.** A signed and notarised archive that
  passes the install matrix, the conformance suite and the examples on macOS. If
  the secrets do not exist, macOS stays unclaimed and the release notes say so —
  the 1.0 rule stands.
- [ ] **Step 5: The release checklist is executable.** `yashyc doctor --release`
  checks every gate, every platform, every signature and the SBOM; `RELEASE.md`
  is the human rendering of the same script. A release that skips a line is not a
  release.
- [ ] **Step 6: Commit and tag.**

## Gates

| Gate | Command | Passes when |
|---|---|---|
| Self-host differential | `cargo test -p yashy-driver --test selfhost_diff` | 0 unsupported, 0 mismatched on the full corpus; error codes and spans agree |
| Bootstrap | `cargo test -p yashy-driver --test bootstrap` | stage 2 and stage 3 byte-identical; every stage agrees on every digest |
| Reference | `cargo test -p yashy-driver --test reference --test docs` | every chapter has grammar, rules and links; EBNF generated; no contradiction with the architecture doc |
| Conformance | `yashyc conformance --all --json` | every rule has a case; every target passes; rejection cases assert code and span |
| Editions | `cargo test -p yashy-driver --test edition --test migrate` | 1.0 builds unchanged; migrate rewrites and passes; unknown edition refused |
| Hardening | `yashyc doctor --release` | fuzz, sanitizers, alias, determinism and sandbox corpora all green |
| Reproducible | `cargo test -p yashy-driver --test reproducible` | two directories, three OSes, identical bytes |
| Install matrix | `scripts/install/test-install.*` | Windows, Linux and macOS install, verify, `doctor`, uninstall |
| Release | `cargo test -p yashy-driver --test release` | signed, SBOM, `self-update` verifies, tamper refused |
| No regression | `cargo test --workspace` | green on Windows and Linux (and macOS once claimed) |
| No P0 | `docs/triage.md` | no P0 |

## Demo

```console
$ scripts/bootstrap
stage 0  yashyc (rust)       hash 4c1e…
stage 1  yashyc (yashy)      hash 9ab2…  matches 412/412 digests
stage 2  yashyc (yashy)      hash 7d10…
stage 3  yashyc (yashy)      hash 7d10…  identical

$ yashyc conformance --all
1,208 cases  214 rules  3 targets  Windows ok  Linux ok  macOS ok
```

```console
$ yashyc migrate --dry-run
edition 1.0 -> 2027
  src/db.ya:41   implicit arena -> arena { }        semantic change: scope
  src/main.ya:7  let x: int = 5 -> let int x = 5     no change
2 files, 1 semantic change, 0 breaking

$ yashyc doctor --release
targets windows/linux/macos ok   signed ok   sbom ok   reproducible ok
fuzz ok   asan/ubsan/tsan ok   alias ok   determinism ok   sandbox ok
no P0   release: yashy 2.0.0
```

## Out of scope

- A fully self-hosted compiler (codegen, verifier, linker). The front end
  bootstraps; the Rust implementation remains the reference.
- A second source of truth: the Rust front end defines the 2.0 release.
- Rewriting 1.0 code automatically beyond the documented migration rewrites;
  `migrate` reports, it does not guess.
- A hosted documentation site with accounts.
- A long-term support promise beyond the 18-month 1.0 window.
- Claiming a platform that has not passed the install matrix and conformance.

## Risks

| Risk | Mitigation |
|---|---|
| The self-hosted front end diverges | The differential is the gate; error codes and spans are compared, not just success |
| Bootstrap converges on a wrong compiler | Stage hashes are recorded; every stage agrees on every digest; the Rust front end is the reference |
| The reference documents the compiler's bugs | Rules are generated from the same tables; conformance cases are the proof; a contradiction fails a test |
| Migration changes semantics silently | Every semantic change is reported with a diff; `--dry-run` is the default demo; a breaking change needs an edition |
| The release is not reproducible | Three OSes, two directories, byte comparison; the SBOM and signatures are inputs |
| macOS is claimed without proof | Signed, notarised archive and the full matrix, or the platform stays unclaimed |
| LTS is a promise nobody can keep | The window is written, the qualifying fixes are named, and the 1.0 branch is tested in CI |
| The 2.0 tag ships with an open P0 | `doctor --release` refuses; the checklist is executable, not prose |
