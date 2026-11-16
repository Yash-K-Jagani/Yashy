# Phase 13 — AI-Native 2.0

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make an agent a competent, bounded collaborator on a Yashy codebase:
it reads a semantic model instead of guessing from text, it edits through
validated patches instead of rewriting files, it runs the compiler inside a
default-deny sandbox, and it can prove the result deterministically.

**Outcome:** MCP v1 with structured edits, refactor, fix, format, lint, `doc`,
`bench`, `profile` and a sandboxed `run`; a semantic graph that carries types,
effects, contracts, ownership and the call graph for every 2.0 form; context
packs selected by focus/diff/tests/docs and capped at 8k tokens; an agent
benchmark v2 that runs reproducibly; a sandbox v1 with capabilities and resource
limits and zero escapes in the corpus; generated `AGENTS.md`/`llms.txt` and
per-symbol docs; provenance for every edit; and the BPE gate over the whole
surface.

**Time:** ~7 weeks.
**Depends on:** Phase 12 gate.
**Spec:** [`../specs/2026-11-15-yashy-2.0-design.md`](../specs/2026-11-15-yashy-2.0-design.md) §6.5, §6.8, §14 (calls 11, 14).

## Why this phase

1.0 already ships the right shape: `index`, `outline`, `graph`, `search`,
`context` and an MCP server with the seven compiler verbs, sandboxed by default.
It is a v0 because it describes text and spans, and because the sandbox is a path
check rather than a capability model. An agent working from text can write a
plausible program that does not type-check; an agent working from a graph can
write one that does.

The phase is late because the graph must include the forms Phase 4 added and the
libraries Phases 5–8 built; a graph written against 1.0's smaller surface would
be rewritten. It is after concurrency and performance because the benchmark and
the sandbox both need the deterministic scheduler and the reports that make an
agent run reproducible.

## Starting point

| Fact | 1.0 value |
|---|---|
| MCP verbs | seven: `check`, `build`, `test`, `verify`, `explain`, `context`, `index` |
| MCP edits | none; no write tool, no patch model |
| Sandbox | v0: default-deny path check, read/write flags |
| Index/graph | names, kinds, spans, imports, call edges; no types, effects or contracts per symbol |
| `context` | budgeted excerpts; no focus/diff/tests/docs modes |
| Agent docs | `llms.txt` hand-written; no `AGENTS.md` generation |
| Agent benchmark | none |
| BPE | unmeasured (Phase 4 audits new forms) |
| Determinism | contract from Phase 4; scheduler from Phase 10 |

## Deliverables

### Task 1: MCP v1 and the structured edit model

**Files:**
- Modify: `crates/yashy-driver/src/mcp.rs`, `crates/yashy-driver/src/sandbox/`
- Create: `crates/yashy-driver/src/edit.rs`, `crates/yashy-driver/src/refactor.rs`
- Create: `docs/mcp.md`
- Test: `crates/yashy-driver/tests/mcp.rs` (extend), `crates/yashy-driver/tests/edit.rs`, `crates/yashy-driver/tests/sandbox_escape.rs` (extend)

**Interfaces:**
- `Edit { file: RelPath, span: ByteSpan, expected_hash: Sha256, replacement: String, rationale: String }`
- `apply_edit(ws: &Workspace, edit: &Edit) -> Result<Applied, EditError>`
- `Patch { edits: Vec<Edit>, expected_tree_hash: Sha256 }`
- New MCP verbs: `edit`, `refactor`, `fix`, `format`, `lint`, `doc`, `bench`, `profile`, `run` (sandboxed)

- [ ] **Step 1: Write the failing test.** An agent asks for a rename,
  receives a `Patch` with one edit per site, applies it, and re-checks. Then:
  a stale `expected_hash` is refused; an overlapping edit is refused; an edit
  outside the workspace is refused; a patch that does not type-check after
  application is rolled back and reported. Assert the file is byte-identical
  after a refusal.
- [ ] **Step 2: There is no write-anything tool.** The only mutation is
  `apply_edit`, and it validates the span and the hash before and the whole tree
  after. A patch is all-or-nothing; a partial application is a bug, not a
  fallback. Every applied edit is recorded in the session's provenance log with
  its hash and rationale.
- [ ] **Step 3: Refactor is compiler-aware.** `refactor` offers rename,
  extract-function, inline-function, change-signature, add-missing-match-arm and
  qualify-import, each computed from the index and verified by re-checking before
  it is returned. A refactor that cannot be proved safe is not offered.
- [ ] **Step 4: No shell, ever.** The MCP surface has no command execution, no
  environment mutation, no network, and no path outside the workspace root. A
  tool call naming one is refused with a code and appears in the audit log.
- [ ] **Step 5: Commit.**

### Task 2: The semantic graph

**Files:**
- Modify: `crates/yashy-driver/src/index.rs`, `crates/yashy-driver/src/main.rs` (`graph`)
- Create: `crates/yashy-driver/src/semantic.rs`
- Test: `crates/yashy-driver/tests/index.rs` (extend), `crates/yashy-driver/tests/semantic.rs`
- Docs: `docs/cli.md` (extend)

**Interfaces:**
- `Symbol { id, kind, name, span, ty: Option<TypeId>, effects, contracts: Vec<ContractId>, ownership: OwnershipFacts, docs: Option<DocId>, unsafe_sites: Vec<Span> }`
- `Edge = Calls | Implements | Uses | Owns | Returns | Raises | ReadsCapability`
- `semantic_graph(ws: &Workspace) -> Graph`

- [ ] **Step 1: Write the failing test.** For a program using generic traits,
  `let else`, a typed pointer, an `unsafe` block, a `defer`, a task and a
  capability read, assert the graph carries each symbol's type, its effects, its
  contract, its ownership facts, its unsafe sites and its capability reads, and
  that a rename moves every incoming edge.
- [ ] **Step 2: One model, rendered twice.** `index --json`, `context`, `doc`,
  the LSP and the MCP server read the same graph. The human rendering is
  generated from the JSON; a divergence is a test failure. Schema v1 fields are
  frozen; new fields are additive.
- [ ] **Step 3: The graph is queryable.** `graph --json --from <symbol> --depth N
  --edges calls,owns,raises` answers "what can this function touch, allocate and
  fail with" in one call, which is the question an agent actually asks.
- [ ] **Step 4: Byte-stable.** Two runs produce byte-identical JSON on both
  OSes: sorted keys, sorted ids, no addresses, no timestamps, no hash-map order.
  A diff of the graph is a diff of the program.
- [ ] **Step 5: Commit.**

### Task 3: Context packs

**Files:**
- Modify: `crates/yashy-driver/src/context.rs`
- Create: `crates/yashy-driver/src/pack.rs`
- Test: `crates/yashy-driver/tests/context.rs` (extend), `crates/yashy-driver/tests/pack.rs`

**Interfaces:**
- `PackRequest { focus: SymbolId, mode: Focus | Diff | Tests | Docs | Error, budget: TokenBudget }`
- `context_pack(req: &PackRequest) -> Pack` with `{ files: Vec<Excerpt>, symbols, signatures, diagnostics, token_count, dropped: Vec<Reason> }`

- [ ] **Step 1: Write the failing test.** A 50k-line workspace: a focus pack
  for one function fits 8k tokens and contains its signature, body, called
  signatures, the types it names and the tests that cover it. A diff pack
  contains only changed hunks plus their callers. An error pack contains the
  diagnostic, its span, the rule page and the fix suggestion. Assert
  `token_count ≤ budget` and that everything omitted is listed with a reason.
- [ ] **Step 2: Rank and cut deterministically.** Ranking is a fixed score over
  graph distance, edit recency, test coverage and diagnostic proximity; ties
  break by symbol id. The token count uses the same tokenizer the BPE gate uses,
  so a pack's size is measured, not estimated. No model-specific prompt text in
  the tool.
- [ ] **Step 3: A pack is a value, not a conversation.** The tool returns
  structured excerpts with paths, spans and hashes; the caller decides how to
  prompt. A pack names the graph revision it was cut from, so a stale pack is
  detectable.
- [ ] **Step 4: Commit.**

### Task 4: Sandbox v1

**Files:**
- Modify: `crates/yashy-driver/src/sandbox/mod.rs`
- Create: `crates/yashy-driver/src/sandbox/{fs,net,proc,limits}.rs`
- Create: `docs/sandbox.md`
- Test: `crates/yashy-driver/tests/sandbox_escape.rs`, `crates/yashy-driver/tests/sandbox_limits.rs`

**Interfaces:**
- `Capabilities { read: Vec<PathBuf>, write: Vec<PathBuf>, net: bool, clock: bool, env: Vec<String>, ffi: Vec<PathBuf>, entropy: bool }`
- `Limits { wall: Duration, cpu: Duration, memory: u64, procs: u32, open_files: u32, output: u64 }`
- `run_sandboxed(cmd: CompilerVerb, caps: &Capabilities, limits: &Limits) -> Result<Outcome, SandboxError>`

- [ ] **Step 1: Write the failing test.** Escape attempts: `..` traversal, a
  symlink pointing out, an absolute path, `%TEMP%`/`/tmp` outside the allowlist,
  opening a socket, reading the environment, `dlopen` of an unlisted library,
  forking a process, writing past the size limit, and reading `/proc/self/mem`.
  Assert each is refused with a distinct code, on both OSes, and that the corpus
  reports zero escapes.
- [ ] **Step 2: Default-deny, positively named.** A capability is a named path
  or a named feature; absence is denial. Both OSes implement the same model
  (job object and restricted token on Windows, `rlimit`/`seccomp`-class limits and
  path checks on Linux) or the sandbox is not shipped. A platform that cannot
  enforce a limit says so in `doctor` and refuses the verb rather than pretending.
- [ ] **Step 3: Limits are enforced, not requested.** The wall clock kills the
  child, memory is capped, output is truncated with a marker, and a process
  count is capped. `--json` reports which limit fired.
- [ ] **Step 4: The compiler verbs are the whole surface.** `check`, `build`,
  `test`, `verify`, `explain`, `context`, `index`, `doc`, `bench`, `profile`,
  `run`, `fix`, `format`, `lint`. Anything else is refused by name.
- [ ] **Step 5: Commit.**

### Task 5: Agent benchmark v2

**Files:**
- Create: `crates/yashy-driver/src/agent_bench.rs`, `agents/bench/` (tasks, fixtures, expected checks)
- Modify: `crates/yashy-driver/src/main.rs` (`agent bench`)
- Test: `crates/yashy-driver/tests/agent_bench.rs`

**Interfaces:**
- `Task { id, prompt, workspace, intended_checks: Vec<Check>, budget: Budget }`
- `Check = Builds | Tests | VerifyClean | NoUnsafeAdded | DiffWithin(paths) | Deterministic | NoShell`
- `run_bench(tasks, runner) -> Report`

- [ ] **Step 1: Write the failing test.** Ten tasks: add a function, fix a
  failing test, rename a symbol, add a match arm, thread a `Result`, replace a
  leak with `defer`, add a CLI flag, add a migration, split a module, fix a
  diagnostic. Each has machine checks, not a prose rubric. A deliberately wrong
  solution fails the check; a correct one passes.
- [ ] **Step 2: Deterministic and comparable.** The benchmark runs under
  `--deterministic` with a fixed clock and seed, records the graph revision, the
  tool calls, the edits and the token counts, and two runs of the same solution
  produce the same trace. A score without a trace is not a result.
- [ ] **Step 3: The benchmark is a regression gate, not a leaderboard.** It
  runs on the repo's own fixtures, needs no network, and a drop of more than the
  recorded threshold fails CI. No private model is required to run it.
- [ ] **Step 4: Commit.**

### Task 6: Agent docs and provenance

**Files:**
- Modify: `crates/yashy-driver/src/doc.rs` (site generation), `llms.txt`
- Create: `crates/yashy-driver/src/agents_md.rs`, `docs/provenance.md`
- Test: `crates/yashy-driver/tests/agents.rs` (extend), `crates/yashy-driver/tests/provenance.rs`

- [ ] **Step 1: Write the failing test.** `yashyc agent docs` writes an
  `AGENTS.md` for the workspace from the graph — build/test/verify commands,
  module map, identity rules, the diagnostics an agent will hit — and an
  `llms.txt` index; both are byte-stable and both are checked in CI. A symbol
  doc page includes its type, effects, contracts, ownership and unsafe sites.
- [ ] **Step 2: Provenance is recorded, not inferred.** Every applied edit
  carries the session id, the tool, the span, the before/after hash and the
  rationale; `yashyc trace --edits` renders it; a commit can be annotated with
  the session that produced it. No hidden attribution and no automatic commit.
- [ ] **Step 3: Docs are generated from the compiler's tables.** `docs/` pages
  for the stdlib come from `yashyc doc`; a hand-edited generated page fails the
  docs test. `AGENTS.md` names the rules that actually exist.
- [ ] **Step 4: Commit.**

### Task 7: The BPE gate over the whole surface

**Files:**
- Modify: `crates/yashy-driver/src/tokens.rs`
- Create: `docs/bpe.md` (generated)
- Test: `crates/yashy-driver/tests/bpe.rs`

- [ ] **Step 1: Write the failing test.** Tokenize the full keyword/operator
  set and a 1 MB corpus of tutorial, stdlib and example source with three
  tokenizers (`o200k`, `cl100k`, a Llama-family vocab). Assert 100% of keywords
  and operators are one token and ≥ 97% of lexical tokens are one token.
- [ ] **Step 2: The formatter never splits a keyword.** A round-trip over the
  corpus preserves token boundaries; `tokens --bpe --json` reports per-keyword
  and per-file numbers so a regression names the file.
- [ ] **Step 3: A rejected spelling is recorded.** `docs/bpe.md` lists every
  candidate considered, its measurement and the decision. No aesthetic rule
  without a number.
- [ ] **Step 4: Commit.**

## Gates

| Gate | Command | Passes when |
|---|---|---|
| Edits | `cargo test -p yashy-driver --test edit --test mcp` | stale hash, overlap and out-of-workspace edits refused; rollback is byte-identical; no shell |
| Graph | `cargo test -p yashy-driver --test semantic --test index` | every 2.0 form carries type/effects/contract/ownership; JSON byte-stable |
| Context | `cargo test -p yashy-driver --test pack` | every mode ≤ 8k tokens; omissions named; stale revision detected |
| Sandbox | `cargo test -p yashy-driver --test sandbox_escape --test sandbox_limits` | zero escapes on both OSes; every limit fires with a code |
| Benchmark | `yashyc agent bench --check-baseline` | 10 tasks; checks pass/fail as intended; deterministic trace; no regression |
| Docs/provenance | `cargo test -p yashy-driver --test agents --test provenance` | generated docs stable; every edit has provenance |
| BPE | `cargo test -p yashy-driver --test bpe` | 100% keywords/operators single-token; corpus ≥ 97% |
| No regression | `cargo test --workspace` | green on Windows and Linux |
| No P0 | `docs/triage.md` | no P0 |

## Demo

```console
$ yashyc mcp serve --allow-read . --allow-write src/
mcp v1  verbs: check build test verify explain context index doc edit refactor
        fix format lint bench profile run   sandbox: default-deny  limits: ok

> edit.rename old:"parse_i64" new:"parse_int" symbols:1
patch 3f8a…  4 edits  1 file  tree-hash 91c2…
apply  ok  recheck ok  verify ok  4.2 ms
```

```console
$ yashyc context --focus src/db.ya:connect --budget 8000 --json
{ "tokens": 6104, "symbols": 18, "files": 4, "dropped": [{"reason":"far","n":31}] }

$ yashyc agent bench --deterministic
10 tasks  9 passed  1 failed   trace 7a1c…  edits 26  tokens 41,208
```

```console
$ yashyc tokens --bpe --json | yashyc tokens summary
keywords 100% (68/68)  operators 100% (41/41)  corpus 98.4%
```

## Out of scope

- A hosted agent service, a model, or a prompt library. Yashy ships the
  interface, the graph and the sandbox.
- Auto-committing or auto-merging an agent's work.
- Shell execution of any kind, including "just for `git`".
- Network access from a sandboxed verb.
- A GUI agent console; the LSP and MCP are the surfaces.
- Per-model prompt tuning inside the compiler.

## Risks

| Risk | Mitigation |
|---|---|
| A structured edit corrupts source | Span + expected hash before, full re-check after, all-or-nothing patch, rollback test |
| The graph becomes a second front end | It is built from the same index and types; one model, two renderings, tested |
| Context packs overflow the budget | Measured with the tokenizer, capped, omissions listed, stale revision refused |
| The sandbox is not a sandbox on one OS | Same model on both OSes or the verb is refused; an escape corpus as the gate |
| The benchmark is a leaderboard | Fixtures in-repo, machine checks, deterministic, a regression threshold, no network |
| Generated docs drift | Rendered from compiler tables; a hand-edited generated page fails the test |
| BPE work becomes aesthetic churn | Every decision records its measurement; only measured rejections |
| An agent's work is unattributable | Provenance on every edit and every commit annotation, opt-in |
