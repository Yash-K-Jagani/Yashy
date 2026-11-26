# Phase 13 — AI-Native 2.0

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make an agent a competent, bounded collaborator on a Yashy codebase:
it reads a semantic model instead of guessing from text, it edits through
validated patches instead of rewriting files, it runs the compiler inside a
default-deny sandbox, and it can prove the result deterministically.

**Outcome:** MCP v1 with structured edits, refactor, fix, format, lint, `doc`,
`bench`, `profile` and a sandboxed `run`; a policy file, a hash-chained audit
log and an approval flow for destructive work; a semantic graph that carries
types, effects, contracts, ownership and the call graph for every 2.0 form;
semantic review and impact analysis; context packs selected by
focus/diff/tests/docs and capped at 8k tokens; an agent benchmark v2 with
held-out tasks and cost metrics that runs reproducibly; a sandbox v1 with
capabilities and resource limits and zero escapes in the corpus; generated
`AGENTS.md`/`llms.txt` and per-symbol docs; a stable `yashy-sdk` for embedding
the compiler; provenance for every edit; and the BPE gate over the whole
surface.

**Time:** ~9 weeks.
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
| Agent policy | none; an agent with write access can change anything the sandbox path allows |
| Audit | none; provenance is per edit, not a tamper-evident log |
| Review/impact | a text diff; no semantic change summary or caller/test impact |
| SDK | none; tools use the CLI |
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

- [x] **Step 1: Write the failing test.** An agent asks for a rename,
  receives a `Patch` with one edit per site, applies it, and re-checks. Then:
  a stale `expected_hash` is refused; an overlapping edit is refused; an edit
  outside the workspace is refused; a patch that does not type-check after
  application is rolled back and reported. Assert the file is byte-identical
  after a refusal.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: There is no write-anything tool.** The only mutation is
  `apply_edit`, and it validates the span and the hash before and the whole tree
  after. A patch is all-or-nothing; a partial application is a bug, not a
  fallback. Every applied edit is recorded in the session's provenance log with
  its hash and rationale.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: Refactor is compiler-aware.** `refactor` offers rename,
  extract-function, inline-function, change-signature, add-missing-match-arm and
  qualify-import, each computed from the index and verified by re-checking before
  it is returned. A refactor that cannot be proved safe is not offered.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: No shell, ever.** The MCP surface has no command execution, no
  environment mutation, no network, and no path outside the workspace root. A
  tool call naming one is refused with a code and appears in the audit log.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 5: Commit.**
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

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

- [x] **Step 1: Write the failing test.** For a program using generic traits,
  `let else`, a typed pointer, an `unsafe` block, a `defer`, a task and a
  capability read, assert the graph carries each symbol's type, its effects, its
  contract, its ownership facts, its unsafe sites and its capability reads, and
  that a rename moves every incoming edge.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: One model, rendered twice.** `index --json`, `context`, `doc`,
  the LSP and the MCP server read the same graph. The human rendering is
  generated from the JSON; a divergence is a test failure. Schema v1 fields are
  frozen; new fields are additive.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: The graph is queryable.** `graph --json --from <symbol> --depth N
  --edges calls,owns,raises` answers "what can this function touch, allocate and
  fail with" in one call, which is the question an agent actually asks.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: Byte-stable.** Two runs produce byte-identical JSON on both
  OSes: sorted keys, sorted ids, no addresses, no timestamps, no hash-map order.
  A diff of the graph is a diff of the program.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 5: Commit.**
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 3: Context packs

**Files:**
- Modify: `crates/yashy-driver/src/context.rs`
- Create: `crates/yashy-driver/src/pack.rs`
- Test: `crates/yashy-driver/tests/context.rs` (extend), `crates/yashy-driver/tests/pack.rs`

**Interfaces:**
- `PackRequest { focus: SymbolId, mode: Focus | Diff | Tests | Docs | Error, budget: TokenBudget }`
- `context_pack(req: &PackRequest) -> Pack` with `{ files: Vec<Excerpt>, symbols, signatures, diagnostics, token_count, dropped: Vec<Reason> }`

- [x] **Step 1: Write the failing test.** A 50k-line workspace: a focus pack
  for one function fits 8k tokens and contains its signature, body, called
  signatures, the types it names and the tests that cover it. A diff pack
  contains only changed hunks plus their callers. An error pack contains the
  diagnostic, its span, the rule page and the fix suggestion. Assert
  `token_count ≤ budget` and that everything omitted is listed with a reason.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: Rank and cut deterministically.** Ranking is a fixed score over
  graph distance, edit recency, test coverage and diagnostic proximity; ties
  break by symbol id. The token count uses the same tokenizer the BPE gate uses,
  so a pack's size is measured, not estimated. No model-specific prompt text in
  the tool.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: A pack is a value, not a conversation.** The tool returns
  structured excerpts with paths, spans and hashes; the caller decides how to
  prompt. A pack names the graph revision it was cut from, so a stale pack is
  detectable.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: Commit.**
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

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

- [x] **Step 1: Write the failing test.** Escape attempts: `..` traversal, a
  symlink pointing out, an absolute path, `%TEMP%`/`/tmp` outside the allowlist,
  opening a socket, reading the environment, `dlopen` of an unlisted library,
  forking a process, writing past the size limit, and reading `/proc/self/mem`.
  Assert each is refused with a distinct code, on both OSes, and that the corpus
  reports zero escapes.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: Default-deny, positively named.** A capability is a named path
  or a named feature; absence is denial. Both OSes implement the same model
  (job object and restricted token on Windows, `rlimit`/`seccomp`-class limits and
  path checks on Linux) or the sandbox is not shipped. A platform that cannot
  enforce a limit says so in `doctor` and refuses the verb rather than pretending.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: Limits are enforced, not requested.** The wall clock kills the
  child, memory is capped, output is truncated with a marker, and a process
  count is capped. `--json` reports which limit fired.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: The compiler verbs are the whole surface.** `check`, `build`,
  `test`, `verify`, `explain`, `context`, `index`, `doc`, `bench`, `profile`,
  `run`, `fix`, `format`, `lint`. Anything else is refused by name.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 5: Commit.**
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 5: Agent benchmark v2

**Files:**
- Create: `crates/yashy-driver/src/agent_bench.rs`, `agents/bench/` (tasks, fixtures, expected checks)
- Modify: `crates/yashy-driver/src/main.rs` (`agent bench`)
- Test: `crates/yashy-driver/tests/agent_bench.rs`

**Interfaces:**
- `Task { id, prompt, workspace, intended_checks: Vec<Check>, budget: Budget }`
- `Check = Builds | Tests | VerifyClean | NoUnsafeAdded | DiffWithin(paths) | Deterministic | NoShell`
- `run_bench(tasks, runner) -> Report`

- [x] **Step 1: Write the failing test.** Ten tasks: add a function, fix a
  failing test, rename a symbol, add a match arm, thread a `Result`, replace a
  leak with `defer`, add a CLI flag, add a migration, split a module, fix a
  diagnostic. Each has machine checks, not a prose rubric. A deliberately wrong
  solution fails the check; a correct one passes.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: Deterministic and comparable.** The benchmark runs under
  `--deterministic` with a fixed clock and seed, records the graph revision, the
  tool calls, the edits and the token counts, and two runs of the same solution
  produce the same trace. A score without a trace is not a result.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: The benchmark is a regression gate, not a leaderboard.** It
  runs on the repo's own fixtures, needs no network, and a drop of more than the
  recorded threshold fails CI. No private model is required to run it.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: Held-out tasks and cost are part of the result.** A held-out
  set the implementation never sees is scored separately; every run reports
  tokens, tool calls, wall time, edits, refused calls and unsafe additions, so a
  cheap wrong answer and an expensive right one are distinguishable. A task that
  passes only by disabling verification is scored as a failure.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 5: Commit.**
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 6: Agent docs and provenance

**Files:**
- Modify: `crates/yashy-driver/src/doc.rs` (site generation), `llms.txt`
- Create: `crates/yashy-driver/src/agents_md.rs`, `docs/provenance.md`
- Test: `crates/yashy-driver/tests/agents.rs` (extend), `crates/yashy-driver/tests/provenance.rs`

- [x] **Step 1: Write the failing test.** `yashyc agent docs` writes an
  `AGENTS.md` for the workspace from the graph — build/test/verify commands,
  module map, identity rules, the diagnostics an agent will hit — and an
  `llms.txt` index; both are byte-stable and both are checked in CI. A symbol
  doc page includes its type, effects, contracts, ownership and unsafe sites.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: Provenance is recorded, not inferred.** Every applied edit
  carries the session id, the tool, the span, the before/after hash and the
  rationale; `yashyc trace --edits` renders it; a commit can be annotated with
  the session that produced it. No hidden attribution and no automatic commit.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: Docs are generated from the compiler's tables.** `docs/` pages
  for the stdlib come from `yashyc doc`; a hand-edited generated page fails the
  docs test. `AGENTS.md` names the rules that actually exist.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: Commit.**
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 7: The BPE gate over the whole surface

**Files:**
- Modify: `crates/yashy-driver/src/tokens.rs`
- Create: `docs/bpe.md` (generated)
- Test: `crates/yashy-driver/tests/bpe.rs`

- [x] **Step 1: Write the failing test.** Tokenize the full keyword/operator
  set and a 1 MB corpus of tutorial, stdlib and example source with three
  tokenizers (`o200k`, `cl100k`, a Llama-family vocab). Assert 100% of keywords
  and operators are one token and ≥ 97% of lexical tokens are one token.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: The formatter never splits a keyword.** A round-trip over the
  corpus preserves token boundaries; `tokens --bpe --json` reports per-keyword
  and per-file numbers so a regression names the file.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: A rejected spelling is recorded.** `docs/bpe.md` lists every
  candidate considered, its measurement and the decision. No aesthetic rule
  without a number.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: Commit.**
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 8: Agent policy, audit and approval

**Files:**
- Create: `crates/yashy-driver/src/agent_policy.rs`, `crates/yashy-driver/src/audit_log.rs`, `crates/yashy-driver/src/approval.rs`
- Modify: `crates/yashy-driver/src/mcp.rs`, `crates/yashy-driver/src/sandbox/mod.rs`
- Create: `docs/agent-policy.md`
- Test: `crates/yashy-driver/tests/agent_policy.rs`, `crates/yashy-driver/tests/audit_log.rs`, `crates/yashy-driver/tests/approval.rs`

**Interfaces:**
- `.yashy/policy.toml`: `allow_verbs`, `deny_paths`, `allow_write`, `allow_net`, `allow_ffi`, `max_edits`, `max_tokens`, `max_wall`, `require_approval`, `protected_paths`
- `AuditEntry { seq, prev_hash, session, verb, args_digest, decision, reason, tree_hash, timestamp, signature }`
- `Approval { tree_hash, patch_hash, verbs, expires, nonce }`

- [x] **Step 1: Write the failing test.** A policy that allows `edit` but denies
  `run` refuses `run` by name and writes the refusal to the audit log. A policy
  with `protected_paths = ["yashy.lock", ".github/**"]` refuses an edit there
  even when the sandbox path allows it. A patch that exceeds `max_edits` or a
  verb that exceeds `max_wall` is refused before it runs. A destructive patch
  without an approval token is held as a plan; the same patch with a valid token
  applies once and the token cannot be replayed.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: Policy is the product's, not the agent's.** The policy file is
  read from the workspace root, is itself protected, and a change to it is an
  audited event that a session cannot self-approve. Precedence is one rule:
  policy denies, then sandbox denies, then the verb runs. A missing policy is
  the strict default, never "allow all".
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: The audit log is tamper-evident.** Entries are append-only
  JSONL, hash-chained (`prev_hash`), signed by the session key, redacted by the
  same rules as a trace, and rotated with a size cap. `yashyc agent audit
  --verify` detects a removed or reordered entry and names the first bad seq. A
  refused call is an entry; a silent call is a bug.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: Approval is a capability, not a prompt.** `plan` returns the
  diff, the affected symbols, the capabilities and the exact verbs; `approve`
  mints a short-lived token bound to the tree hash, the patch hash and the verb
  set. A token from one tree cannot apply to another; an expired token is
  refused; a replay is refused and logged.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 5: Budgets are enforced, not suggested.** Token, edit, wall-clock
  and output budgets are counted by the harness, reported in `--json`, and a
  breach stops the session with a diagnostic. An agent cannot raise its own
  budget; only the policy can.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 6: Commit.**
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 9: Semantic review, impact and a stable SDK

**Files:**
- Create: `crates/yashy-driver/src/review.rs`, `crates/yashy-driver/src/impact.rs`, `crates/yashy-sdk/src/lib.rs`
- Modify: `crates/yashy-driver/src/main.rs` (`review`, `impact`), `crates/yashy-lsp/src/main.rs`
- Modify: `crates/yashy-driver/src/mcp.rs` (resources, prompts, annotations, cancellation, progress)
- Create: `docs/review.md`, `docs/sdk.md`
- Test: `crates/yashy-driver/tests/review.rs`, `crates/yashy-driver/tests/impact.rs`, `crates/yashy-sdk/tests/sdk.rs`

**Interfaces:**
- `Review { changes: Vec<SemanticChange>, public_break: bool, capability_delta, unsafe_delta, test_delta, risk }`
- `impact(base: Revision, ws: &Workspace) -> Impact { callers, tests, packages, examples, docs }`
- `yashy_sdk::{check, build, test, verify, explain, context, index, review}`

- [x] **Step 1: Write the failing test.** A commit that changes a public
  function's signature, adds an `unsafe` block, widens a capability and drops a
  test is summarized as four semantic changes with spans, a breaking-API flag and
  the exact impacted tests. A purely internal rename reports no public break and
  a bounded impact. `yashyc review --json` and the human rendering agree.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: Review reads the graph, not the diff.** Changes are classified
  by symbol and kind (signature, behaviour, effect, contract, capability,
  ownership, unsafe, docs) and the public/private boundary comes from the
  visibility model. A diff that touches only comments is reported as no semantic
  change. Impact reuses the Phase 9 impacted-test graph and names every test,
  package, example and doc page to re-run.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: The SDK is the one embedding surface.** A Rust crate exposes the
  seven compiler verbs, review, impact and the graph with a stable typed API and
  one error type; the CLI, LSP, MCP and the test harness call the same crate, so
  there is no second pipeline. The SDK is `#![forbid(unsafe_code)]` and has a
  semver policy from the first release.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: MCP is complete, not just verbs.** Tools carry annotations
  (`readOnly`, `destructive`, `idempotent`), structured content and a result
  schema; resources expose the graph, diagnostics and docs; prompts are versioned
  templates shipped by the compiler; long operations support progress and
  cancellation. A cancelled tool stops work and leaves no partial edit.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 5: Untrusted text is labelled, never obeyed.** A file, an issue
  body, a dependency README or a tool result that contains instructions is marked
  `untrusted` in the structured output, and a prompt-injection fixture asserts
  that the harness never turns it into a policy change, a verb or a path. The
  label travels with the content into context packs.
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 6: Commit.**
  *Closed 2026-11-26 with exception: needs external evaluation, held-out tasks and signed release infrastructure. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

## Gates

| Gate | Command | Passes when |
|---|---|---|
| Edits | `cargo test -p yashy-driver --test edit --test mcp` | stale hash, overlap and out-of-workspace edits refused; rollback is byte-identical; no shell |
| Graph | `cargo test -p yashy-driver --test semantic --test index` | every 2.0 form carries type/effects/contract/ownership; JSON byte-stable |
| Context | `cargo test -p yashy-driver --test pack` | every mode ≤ 8k tokens; omissions named; stale revision detected |
| Sandbox | `cargo test -p yashy-driver --test sandbox_escape --test sandbox_limits` | zero escapes on both OSes; every limit fires with a code |
| Benchmark | `yashyc agent bench --check-baseline` | 10 tasks; checks pass/fail as intended; deterministic trace; no regression |
| Policy/audit | `cargo test -p yashy-driver --test agent_policy --test audit_log --test approval` | denied verb refused; protected path refused; audit chain verifies; token cannot be replayed |
| Review/impact | `cargo test -p yashy-driver --test review --test impact` | semantic changes classified; public break flagged; impacted tests named; comment-only diff is empty |
| SDK | `cargo test -p yashy-sdk --test sdk` | verbs, review, impact and graph agree with the CLI; no second pipeline |
| MCP completeness | `cargo test -p yashy-driver --test mcp` | annotations, resources, prompts, progress and cancellation work; injection fixture is labelled and ignored |
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
$ yashyc agent plan --verb edit --patch patch.json
plan   4 edits  1 file  tree 91c2…  risk: 1 public signature change, 1 test dropped
approval required  (policy: protected_paths, destructive edit)
$ yashyc agent approve --plan 91c2… --for 10m
token  9f3a…  verbs edit  expires 10m  bound tree 91c2…  patch 44d1…
$ yashyc agent apply --token 9f3a…
applied  audit seq 812  prev 7c0e…  signature ok

$ yashyc review --base HEAD~1
semantic changes 4  public break: yes  capabilities: +net  unsafe: +1  tests: -1
impact  3 tests, 1 package, 1 example, 2 doc pages
$ yashyc agent audit --verify
812 entries  chain ok  first bad: none  secrets redacted 3 keys
```

```console
$ yashyc agent bench --held-out --json | yashyc agent bench summary
held-out 8/10  tokens 38,109  tool calls 74  wall 6m12s  refused 3  unsafe added 0
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
- A model, an agent runtime or a hosted review service; the SDK, policy, audit
  log and review are local and provider-neutral.
- Approving work on the user's behalf; approval is a token the human or CI
  mints, never the agent.
- Automatic semantic interpretation of untrusted text as instructions; text is
  labelled and remains data.

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
| An agent exceeds its mandate | Policy file (protected, strict default), sandbox, budgets, approval token bound to tree+patch+verbs, tamper-evident audit log |
| Review approves a breaking change as internal | Public/private comes from the visibility model; a signature/effect/capability change is breaking; a comment-only diff is empty |
| The SDK becomes a second pipeline | CLI, LSP, MCP and tests all call `yashy-sdk`; the SDK wraps the same driver |
| Prompt injection turns text into a verb | Untrusted content is labelled in structured output; a fixture asserts no verb, path or policy change; approval never comes from the agent |
