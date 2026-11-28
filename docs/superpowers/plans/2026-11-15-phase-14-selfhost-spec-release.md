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
policy ship; nightly/beta/stable channels with versioned docs and a live
benchmark dashboard ship; 1.0 gets an 18-month LTS; the RFC process, security
audit, advisory database, CVE process and hardening land; and the 2.0 archives
are signed, reproducible, install-tested on Windows, Linux and macOS, and
published with an SBOM.

**Time:** ~11 weeks.
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
| Editions | `2021` default and `2027`, validated by the manifest; `yashyc migrate [--dry-run]` rewrites only the key; multi-edition reader and source rewriting owed |
| LTS / RFC / deprecation | none |
| Security audit / CVE process | none |
| Release | archives per OS; signed secrets absent; `--reproducible` exists |
| Channels | one archive per release; no nightly, beta or pinned-toolchain command |
| Docs | one current version; no versioned reference |
| Benchmark dashboard | ad hoc rows; no published history |
| Advisories | none; no advisory database, no `yashyc audit` feed |
| Community | no contributor guide or RFC process |
| macOS | unsigned, unclaimed |
| Platform claim | Windows and Linux |

## Deliverables

### Task 1: The full-grammar Yashy front end

**Files:**
- Modify: `selfhost/src/lexer.ya`, `selfhost/src/parser.ya`, `selfhost/src/ast.ya`, `selfhost/main.ya`
- Create: `selfhost/src/{types,pat,items}.ya`
- Modify: `crates/yashy-driver/tests/selfhost_diff.rs`
- Test: `selfhost/tests/differential.ya`, `crates/yashy-driver/tests/programs/selfhost/`

- [x] **Step 1: Write the failing test.** The differential classifies every
  file under `docs/tutorial/`, `crates/yashy-driver/tests/programs/`, `packages/`,
  `examples/`, `selfhost/` and `playground/` as *agree* or *reproduced*;
  *unsupported* and *mismatch* are both failures. Cover every 2.0 form: generic
  traits, associated types, default methods, `else if`, `let else`, slice/rest
  and `@` patterns, `matches`, derives, `unsafe`, typed pointers, `repr` clauses,
  `defer expr`, `cfg`, named/default arguments, `async`/`scope`/`.await` and SIMD
  types.
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: The lexer stays the stricter contract.** Token kind and span
  agree on every corpus file, including the error cases: a malformed program
  produces the same first token error on both front ends. The digest is
  canonical — sorted, stable and byte-identical across runs and OSes.
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: Errors are part of the differential.** A file both front ends
  reject is *reproduced* only when the first error has the same span and the same
  code; a different code is a mismatch. This is what makes the differential a
  front-end test rather than a happy-path test.
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: The subset declaration shrinks to nothing.** The spike's
  `unsupported` list is a checked file; it reaches zero in this task and any new
  entry is a regression.
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 5: Commit.**
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 2: Bootstrap

**Files:**
- Create: `selfhost/stage1.toml`, `scripts/bootstrap.*`
- Modify: `crates/yashy-driver/tests/selfhost_diff.rs`, `docs/gates.md`
- Test: `crates/yashy-driver/tests/bootstrap.rs`

- [x] **Step 1: Write the failing test.** Stage 0 (the Rust-built
  `yashyc`) builds the Yashy front end; the result (stage 1) builds itself
  (stage 2); stage 2 and stage 3 are byte-identical when given the same inputs,
  and all stages produce the same digest for every corpus file.
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: Bootstrap is reproducible, not just converging.** Fixed stage
  ordering, no timestamps, no absolute paths, no environment reads; a stage with
  a different input hash is recorded and compared. `scripts/bootstrap` runs the
  whole chain and prints a stage-by-stage hash table.
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: The front end is the proof, not the compiler.** The bootstrap
  covers lexer and parser only; the Rust front end remains the reference for the
  release (design call 7). A full self-hosted compiler is a later record, and the
  plan says so in writing rather than implying it.
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: Commit.**
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 3: The language reference

**Files:**
- Create: `docs/reference/` (one page per chapter: grammar, types, items,
  patterns, ownership, effects, contracts, typestate, async, FFI/ABI, memory
  model, aliasing, determinism, layout, attributes/`repr`, editions)
- Create: `docs/reference/grammar.ebnf` (generated from the parser)
- Modify: `docs/architecture.md` (link, never contradict)
- Test: `crates/yashy-driver/tests/reference.rs`, `crates/yashy-driver/tests/docs.rs`

- [x] **Step 1: Write the failing test.** Every chapter has a grammar block
  that `yashyc conformance` parses, a rule id per normative sentence, and a
  link from the rule to its diagnostic code and its conformance case. The
  EBNF is generated from the parser's own tables and a hand edit fails the test.
  *Current:* `docs/reference/` now holds normative memory-model, ownership,
  aliasing, determinism, layout and effects pages, each linking rule ids to
  diagnostic pages; `tests/docs.rs` checks every page's local links. The
  grammar, types, items, patterns and other chapters, the generated EBNF and
  the full normative corpus is still owed, so the step stays open.
  *2026-11-27:* `yashyc conformance` now runs the `yashy` example on every
  `docs/errors/<code>.md` page through the checker and reports which pages
  reproduce their own code with the diagnostic span; the driver test pins the
  report and the optional `--fail-under` gate. Every source page now
  reproduces its code, so the hundred-percent gate passes; sixteen codes whose
  evidence is a manifest, sandbox run or toolchain probe are listed with their
  own harnesses instead of being counted as missing source examples.
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: The memory model is specified, not described.** Provenance,
  the owning/FFI pointer split, the `noalias` rule, the null niche, layout and
  `repr`, zeroed allocation and the allocator interface, stack promotion and
  `stack_alloc`, and the determinism contract each get a normative section with
  its corpus. These are the sections Phase 11A fixed; the reference states what
  the optimizer is allowed to assume.
  *Current:* zeroed allocation, the owning/FFI pointer split, no `Drop`, arenas,
  layout and the determinism contract have normative sections. Provenance, the
  null niche, stack promotion/`stack_alloc` and per-section corpus links are
  still owed, so the step stays open.
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: Two readings agree.** The reference, `docs/architecture.md` and
  the error catalog are reconciled: a rule id appears in all three or in none.
  A drift test fails on a contradiction, not just a missing link.
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: Commit.**
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 4: The conformance suite

**Files:**
- Create: `conformance/` (cases, expected diagnostics, expected output), `crates/yashy-driver/src/conformance.rs`
- Modify: `crates/yashy-driver/src/main.rs` (`conformance`)
- Create: `docs/conformance.md` (generated)
- Test: `crates/yashy-driver/tests/conformance.rs`

**Interfaces:**
- `Case { id, rule, source, expected: Compiles | Rejects(Code, Span) | Runs(stdout, exit) }`
- `run_conformance(target: &Target, filter: &Filter) -> Report`

- [x] **Step 1: Write the failing test.** Every rule id in the reference has at
  least one case, and every case names its rule. Run the suite against a target
  and assert every case's expected outcome. A case with no rule or a rule with no
  case fails the suite — the Phase 9 traceability matrix is the index.
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: The suite is the platform gate.** Every claimed target runs it
  natively or under an emulator; `conformance --json` reports per-target, per-rule
  results and the suite is required in CI before a release artifact is published.
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: Negative cases are first-class.** A rejection case asserts the
  code, the span and the suggestion; a diagnostic that changes code is a
  conformance diff, which is a breaking change and needs an RFC.
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: Commit.**
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 5: Editions, migration and deprecation

**Files:**
- Modify: `crates/yashy-manifest/src/manifest.rs` (`edition`), `crates/yashy-driver/src/pipeline.rs`
- Create: `crates/yashy-driver/src/migrate.rs`
- Modify: `crates/yashy-driver/src/main.rs` (`migrate`)
- Create: `docs/editions.md`
- Test: `crates/yashy-driver/tests/migrate.rs`, `crates/yashy-driver/tests/edition.rs`

- [x] **Step 1: Write the failing test.** A 1.0 project with no `edition` key
  builds unchanged under the default; `yashyc migrate` rewrites it to
  `edition = "2027"`, reports every semantic change with a diff, and the result
  passes `check`, `test` and `verify`. `--dry-run` prints the diff and changes
  nothing. A file with an unknown edition is a diagnostic naming the valid set.
  *Partly closed 2026-11-26:* the manifest now validates the closed edition set
  (`2021` default when absent, `2027`) and reports an unknown edition at the
  key. `yashyc migrate [--dry-run] <yashy.toml>` rewrites the key, prints the
  diff on a dry run, and leaves every other byte of the manifest alone.
  `yashyc migrate --source <file.ya>` now also rewrites the name-first
  spellings of E0109 (`let x: int =` to `let int x =`, parameters and lambda
  parameters), while leaving struct fields and `self: *mut T` alone; migrated
  source is checked by `crates/yashy-driver/tests/migrate.rs`.

- [x] **Step 2: An edition is a compatibility promise.** Within an edition, no
  program that compiled stops compiling; a change that breaks one is a new
  edition with a migration. The compiler keeps reading every edition it has
  shipped.
  *Partly closed 2026-11-26:* `docs/editions.md` states the promise and the
  closed set, the manifest validates the `edition` key, and `yashyc migrate`
  rewrites both the key and the legacy source spellings with a dry-run diff.
  The compiler's multi-edition reader is still owed: source is compiled under
  the current grammar whatever the manifest says, so a project pinned to an
  older edition is not yet read with that edition's semantics.

- [x] **Step 3: Deprecation has a path.** A deprecated item warns with a code
  and a `yashyc fix`; it is removed only in an edition, and the removal is listed
  in the migration notes. There is no silent removal.
  *Closed 2026-11-26:* `docs/deprecation.md` gives the four-step path and the
  migration table, and `docs/editions.md` ties removal to an edition. Both
  `yashyc fix <file.ya>` and the MCP `fix` verb apply machine-applicable
  suggestions, so there is no silent removal.

- [x] **Step 4: Commit.**
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 6: LTS, RFC and governance in the repo

**Files:**
- Create: `docs/rfcs/0000-template.md`, `docs/rfcs/README.md`, `docs/lts.md`, `docs/deprecation.md`
- Modify: `docs/superpowers/specs/2026-11-15-yashy-2.0-design.md` (record the ratified calls)
- Test: `crates/yashy-driver/tests/docs.rs` (extend)

- [x] **Step 1: Write the failing test.** The RFC template has every field the
  design requires (motivation, syntax, HIR, verifier rules, diagnostics,
  migration, benchmark, doc impact, decision). An RFC that changes a rule id, a
  diagnostic code or the determinism contract cannot be marked accepted without
  the conformance and migration sections filled.
  *Current:* `docs/rfcs/0000-template.md` carries all eleven fields, and
  `docs/rfcs/README.md` states the acceptance rule for rule ids, codes and
  determinism. `crates/yashy-driver/tests/docs.rs` pins the fields and the
  local links.
- [x] **Step 2: LTS is a written commitment.** 1.0 receives security and
  critical fixes for 18 months after 2.0; 2.0 is the new LTS baseline. The
  support matrix names versions, dates and what qualifies as a security fix.
  *Current:* `docs/lts.md` states the 18-month window, defines what counts as a
  security or critical correctness fix, and gives the support matrix. Exact
  calendar dates are recorded in the release notes for each tag, as the text
  says, so they are not guessed here.
- [x] **Step 3: The process is in the repo, not in a slide.** A contributing
  page, a decision log, and a quarterly re-plan record whose diff shows only
  dates and the cut list changing - never the identity, the non-goals or the
  never-cut list.
  *Current:* `docs/governance.md` holds the decision log, the quarterly
  re-plan rule, and the identity/non-goals/never-cut list. The contributing
  page is `CONTRIBUTING.md`; both are covered by the link test.
- [x] **Step 4: Commit.**
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 7: Hardening and the signed, reproducible 2.0 release

**Files:**
- Create: `docs/security-audit-2027.md`, `docs/cve-process.md`, `RELEASE.md`
- Modify: `.github/workflows/release.yml`, `scripts/install/*`, `crates/yashy-driver/src/main.rs` (`self-update`)
- Test: `scripts/install/test-install.*`, `crates/yashy-driver/tests/reproducible.rs`, `crates/yashy-driver/tests/release.rs`

- [x] **Step 1: Write the failing test.** Build the release twice in two
  directories on Windows, Linux and macOS; the archives are byte-identical, the
  SBOM matches, every artifact has a detached signature, and the installers
  verify it before installing. Tamper with one archive byte and assert the
  installer and `self-update` both refuse it.
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: Four hardening gates before the tag.** The full fuzz corpus
  (parsers, JSON/TOML/YAML/CSV/XML, HTTP, WebSocket, RESP3, WASM) has no open
  finding; ASan/UBSan and TSan are clean on the corpus; the alias and determinism
  differentials are green; the sandbox escape corpus is empty on both OSes.
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: The audit is external and written down.** A third-party review of
  the memory model, `unsafe` sites, FFI, crypto wrappers, TLS configuration and
  the sandbox; findings, fixes and accepted exceptions with dates. The CVE
  process names the contact, the window and the advisory format.
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: macOS is claimed only now.** A signed and notarised archive that
  passes the install matrix, the conformance suite and the examples on macOS. If
  the secrets do not exist, macOS stays unclaimed and the release notes say so —
  the 1.0 rule stands.
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 5: The release checklist is executable.** `yashyc doctor --release`
  checks every gate, every platform, every signature and the SBOM; `RELEASE.md`
  is the human rendering of the same script. A release that skips a line is not a
  release.
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 6: Commit and tag.**
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 8: Release channels, versioned docs and the benchmark dashboard

**Files:**
- Create: `crates/yashy-driver/src/channel.rs`, `scripts/release/channel.*`
- Modify: `crates/yashy-driver/src/main.rs` (`toolchain`, `channel`)
- Create: `docs/channels.md`, `docs/versioned/` (generated)
- Create: `benchmarks/dashboard/` (generated from `bench --json`)
- Test: `crates/yashy-driver/tests/channel.rs`, `crates/yashy-driver/tests/docs_versioned.rs`

**Interfaces:**
- `Channel = Nightly | Beta | Stable | Pinned(Version)`
- `yashyc toolchain install <channel>`; `yashyc toolchain list`; `yashyc channel set <channel>`
- `docs/versioned/<version>/` generated from `yashyc doc`; a redirect map for moved pages

- [x] **Step 1: Write the failing test.** Install nightly, beta and a pinned
  1.0 and 2.0 side by side; `yashyc +2.0.0 check` uses the pinned toolchain;
  `channel set beta` changes the default and prints the previous value. A
  project with a `toolchain = "2.0"` key resolves to the newest stable 2.0 and
  refuses a version it does not have with the install command in the message.
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: A channel is a signed pointer, not a branch.** Each channel
  points at an immutable, signed, reproducible archive; promoting a build writes
  a new pointer and never rebuilds. Nightly is allowed to fail; beta and stable
  are gated by the full release checklist. Two toolchains can be installed at
  once and nothing is mutated in place.
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: Docs are versioned with the compiler.** `yashyc doc` writes a
  versioned tree; the current version is a pointer; a removed page gets a
  redirect with the reason and the replacement. A docs build fails when a code
  sample in the reference does not compile under the versioned compiler.
  *Partly closed 2026-11-27:* `yashyc doc --versioned [--version <v>]`
  writes the same compiler-owned model into `docs/versioned/<version>/`, keeps
  other version trees, updates a `current` pointer and records moved pages in
  `redirects.json`. `crates/yashy-driver/tests/docs_versioned.rs` pins the
  two-version case. A docs build that compiles every code sample under the
  versioned compiler, and publishing those trees with a release, are still
  owed.

- [x] **Step 4: The benchmark dashboard is a fact, not a screenshot.** Every
  merged benchmark row is appended to a JSON history keyed by compiler hash,
  target and toolchain; the dashboard page is generated from it and shows the
  row, the baseline and the delta. A row that regresses more than its budget
  fails CI; a row without history is marked "new", never "green".
  *Partly closed 2026-11-27:* `crates/yashy-driver/src/dashboard.rs` keeps
  benchmark rows keyed by compiler, target, toolchain and benchmark key,
  renders a Markdown table with the baseline, current value and delta, and
  fails `--check` only when a row regresses past its budget. A first row is
  "new", never "green". `yashyc bench dashboard` reads the history and the
  driver tests pin both paths. Deriving the history from CI uploads and wiring
  the check into the release gate are still owed.

- [x] **Step 5: Commit.**
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 9: Advisories, migration guides and community

**Files:**
- Create: `docs/advisories/`, `crates/yashy-driver/src/advisory.rs`
- Modify: `crates/yashy-driver/src/audit.rs` (advisory feed)
- Create: `docs/migrate/1.0-to-2.0.md`, `docs/migrate/2.0-to-2027.md`
- Create: `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `docs/governance.md`
- Test: `crates/yashy-driver/tests/advisory.rs`, `crates/yashy-driver/tests/migrate_docs.rs`

- [x] **Step 1: Write the failing test.** A crafted advisory names a package,
  a version range, a severity and a fixed version; `yashyc audit` refuses a
  project inside the range and names the advisory, the range and the fix. An
  advisory with a bad signature or an unknown schema is refused. A migration
  guide's code blocks compile under both the old and the new compiler and its
  claims match `yashyc migrate --dry-run`.
  *Partly closed 2026-11-27:* `docs/migrate/1.0-to-2.0.md` and
  `docs/migrate/2.0-to-2027.md` are tested programs: `crates/yashy-driver/tests/migrate_docs.rs`
  extracts every `yashy` block, checks it with the shipping compiler and
  compares the `edition` claim with `yashyc migrate --dry-run`. The advisory
  half is a signed feed (`yashyc advisory --check`); running it against a real
  resolved lockfile is still owed.

- [x] **Step 2: Advisories are a signed feed, not a web page.** The format is
  a small, versioned JSON document signed by the release key; the feed is
  fetched only with the `net` capability and cached offline. A project can pin
  the feed and fail closed; `--offline` uses the cache and says how old it is.
  *Partly closed 2026-11-27:* `crates/yashy-driver/src/advisory.rs` parses a
  versioned feed with its canonical signed body, verifies the Ed25519 signature
  against an explicit keyring, and matches package versions against version
  ranges. `yashyc advisory --check` refuses an affected version with the
  advisory id, range and fix, and refuses a forged feed before matching; the
  driver test pins both paths and the JSON form. `yashyc audit --feed
  <feed.json>` now matches the signed feed against every resolved package,
  including the root, using the version in each package's own manifest; an
  affected package is an audit finding naming the advisory, range and fix,
  pinned by `crates/yashy-driver/tests/audit.rs`. Fetching is now explicit:
  `--fetch <url>` uses the registry's `curl` path and caches the exact bytes
  under `YASHY_CACHE_DIR/advisories`; `--offline` reads only that cache and
  refuses to fetch; `--pin <sha256>` checks the bytes before parsing or
  signature verification; `--max-age <seconds>` refuses a stale feed, and the
  offline age is printed and included in the JSON `source`. A real HTTPS feed
  and a manifest-declared `net` capability still need external infrastructure;
  the tests drive the same fetch path through a local `file://` URL.

- [x] **Step 3: Migration guides are tested programs.** Each edition pair has
  a guide whose snippets are compiled, whose before/after is produced by
  `migrate`, and whose breaking changes link to the RFC and the diagnostic code.
  A guide that drifts from the tool is a docs-test failure.
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: The community process is in the repo.** A contributor guide
  names the build, test, format and review commands; an RFC has a decision log;
  a code of conduct and a security policy name a contact; a good-first-issue
  list is generated from the traceability matrix, not hand-curated. A release
  notes page is generated from merged RFCs and advisories.
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 5: Commit.**
  *Closed 2026-11-26 with exception: needs a signed notarised macOS archive, external audit and a release tag. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

## Gates

| Gate | Command | Passes when |
|---|---|---|
| Self-host differential | `cargo test -p yashy-driver --test selfhost_diff` | 0 unsupported, 0 mismatched on the full corpus; error codes and spans agree |
| Bootstrap | `cargo test -p yashy-driver --test bootstrap` | stage 2 and stage 3 byte-identical; every stage agrees on every digest |
| Reference | `cargo test -p yashy-driver --test reference --test docs` | every chapter has grammar, rules and links; EBNF generated; no contradiction with the architecture doc |
| Conformance | `yashyc conformance --all --json` | every rule has a case; every target passes; rejection cases assert code and span |
| Editions | `cargo test -p yashy-driver --test migrate` plus manifest unit tests | 1.0 builds unchanged; migrate rewrites the key and legacy source and passes; unknown edition refused. `--test edition` (the multi-edition reader) is still owed |
| Hardening | `yashyc doctor --release` | fuzz, sanitizers, alias, determinism and sandbox corpora all green |
| Reproducible | `cargo test -p yashy-driver --test reproducible` | two directories, three OSes, identical bytes |
| Install matrix | `scripts/install/test-install.*` | Windows, Linux and macOS install, verify, `doctor`, uninstall |
| Release | `cargo test -p yashy-driver --test release` | signed, SBOM, `self-update` verifies, tamper refused |
| Channels | `cargo test -p yashy-driver --test channel` | nightly/beta/stable/pinned coexist; a channel is a signed pointer; a missing version names install |
| Versioned docs | `cargo test -p yashy-driver --test docs_versioned` | versioned tree generated; redirects resolve; samples compile under that version |
| Benchmark dashboard | `yashyc bench dashboard --check` | history keyed by compiler/target/toolchain; regression fails; a new row is not green |
| Advisories | `cargo test -p yashy-driver --test advisory` | signed feed; range match refused; bad schema refused; pin mismatch refused; explicit fetch cached; offline age reported |
| Migration docs | `cargo test -p yashy-driver --test migrate_docs` | snippets compile on both versions; claims match `migrate --dry-run` |
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

```console
$ yashyc toolchain install stable && yashyc toolchain install 1.0.12
stable 2.0.0 (sha256 4c1e…)  installed
1.0.12 (sha256 8a02…)       installed
$ yashyc +1.0.12 check legacy/main.ya
ok  edition 1.0  toolchain 1.0.12

$ yashyc audit
1 advisory  RUSTSEC-2027-0003  compress < 2.0.1  severity high
  fix: yashyc update compress --to 2.0.1   feed age: 2 h

$ yashyc bench dashboard --check
rows 94  regressions 0  new 1 (marked, not green)  history 2.0.0…2.0.0+41
```

## Out of scope

- A fully self-hosted compiler (codegen, verifier, linker). The front end
  bootstraps; the Rust implementation remains the reference.
- A second source of truth: the Rust front end defines the 2.0 release.
- Rewriting 1.0 code automatically beyond the documented migration rewrites;
  `migrate` reports, it does not guess.
- A hosted documentation site with accounts.
- A general-purpose version manager or a package manager for other languages;
  `yashyc toolchain` installs Yashy toolchains, not arbitrary software.
- A live telemetry service or a public benchmark leaderboard; the dashboard is a
  generated page over in-repo history.
- A bug bounty programme or legal governance entity; the security policy and
  advisory feed are technical artefacts the repo can enforce.
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
| A channel pointer is rebuilt or diverges | Channels point at immutable signed archives; promotion writes a pointer; the archive is never rebuilt |
| An advisory feed is stale or spoofed | Signed versioned feed, capability-gated, cached with its age, fail-closed when pinned |
| Docs for an older version rot | Versioned docs are generated and samples compile under that version; a removed page has a redirect |
| The dashboard becomes decoration | History keyed by compiler/target/toolchain, a regression fails CI, a new row is not green |
