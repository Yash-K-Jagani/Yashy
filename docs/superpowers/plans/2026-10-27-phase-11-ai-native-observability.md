# Phase 11 — AI-Native Tooling and Observability: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the compiler legible to a machine without making it worse for a person: a symbol index, a context packer with a token budget, an MCP server that exposes only compiler verbs, a sandbox that can be tested for escapes, and program-side observability that is deterministic when a test needs it.

**Architecture:** One rule — **one fact, two renderings**. Every command in this phase builds a JSON model first and renders the human form from it, so the agent and the human cannot drift. The index is computed from the module graph (phase 6), the HIR, and the verifier's facts (phases 1–2), which is why this phase comes after them. The MCP server is a thin adapter over the same in-process functions the CLI calls — never a shell, never a new code path. The sandbox is a capability set attached to a process launch, enforced with OS primitives on each platform, and proven by escape tests that run in CI.

**Tech Stack:** `yashy-driver` (commands), `yashy-hir`, `yashy-verify` (`ProofFacts`, effects, typestate), `yashy-lsp/src/json.rs` (JSON-RPC framing to reuse), `yashy-runtime` (log/trace/metrics, deterministic clock and RNG), and OS primitives: Windows Job Objects + restricted token, Linux rlimits + path allowlist. No new Rust dependencies.

**Spec:** `docs/superpowers/specs/2026-10-27-yashy-master-design.md` §6. Index: `docs/superpowers/plans/2026-10-27-yashy-master-plan.md`. Predecessor: `2026-10-27-phase-10-async.md`.

## Global Constraints

See the master index. The ones that bite here:

- One fact, two renderings: no human-only output path.
- Schema v1 freezes at the end of this phase. Additive changes only; a CI job fails on removal or type change.
- MCP exposes compiler verbs only. Never arbitrary shell execution, never a path outside the workspace root.
- A malformed program produces a diagnostic, never a panic — and the same holds for a malformed MCP request.
- Both operating systems or it is not done; a sandbox that only exists on Linux is not a sandbox.
- No unproven claim: "no escape" means the escape corpus fails to escape, on both OSes.

## Review Focus

1. **`context` must fit its budget or refuse, never truncate silently.** At 8 000 tokens it returns a pack whose own token count is reported; if the requested scope cannot fit, it returns the highest-value subset *and* names what it dropped. Silent truncation is how an agent gets a confident wrong answer.
2. **The index is a fact, not a guess.** Every symbol carries its file, span, signature, effects, contracts and (where known) typestate. A field the compiler does not know is `null`, never omitted and never invented.
3. **The MCP server cannot widen its own permissions.** It runs with the sandbox's default-deny capability set; a request naming a path outside the workspace root is refused by the same code the CLI uses, not by a check in the adapter.
4. **Deterministic mode must cover every source of nondeterminism the language exposes**: clock, RNG, environment, process id, map iteration order, and thread scheduling where observable. A test that passes in deterministic mode and fails otherwise is a bug report, not a flake.
5. **The profiler must not change what it measures.** Instrumented timings are per-function counters with a documented overhead; sample-based profiling is documented per OS and never on by default.

---

### Task 1: `yashy index`

**Files:**
- Create: `crates/yashy-driver/src/index.rs`
- Modify: `crates/yashy-driver/src/main.rs` (`index` subcommand, `--json`), `docs/cli.md`
- Create: `crates/yashy-driver/tests/index.rs`

**Interfaces:**
- Produces: `yashy index [path]` (human table) and `yashy index --json` (`{"schema_version":1,"tool":"yashy","files":[…],"symbols":[…],"imports":[…],"tests":[…]}`).
- Produces: a symbol record:
  ```json
  {"name":"fetch","kind":"fn","file":"src/net.ya","span":{"start":120,"end":240},
   "signature":"fn fetch(url: string) -> Result[string, Error]",
   "effects":{"io":true,"unsafe":false,"async":true},
   "contracts":[{"kind":"requires","text":"url.len() > 0","span":{…}}],
   "typestate":null,"visibility":"pub","docs":"Fetches a URL."}
  ```
- Produces: `tests` lists every `tests/**/*.ya` program with the functions it calls, so an agent can pick a regression test by symbol.

- [ ] **Step 1: Write the failing tests** — an index of `examples/` contains every top-level function, struct, enum, trait and impl method; a `pure` function has `effects.io == false`; a function with a `requires` clause carries it; `--json` is stable across two runs (byte-identical).
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — build the model from `HirProgram` plus the module graph plus `VerifyReport`'s effect map; sort by `(file, span.start)` so output is deterministic without a hash map in the path. The human table is rendered from the same structs.
- [ ] **Step 4: Run** `cargo test -p yashy-driver --test index` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(driver): a symbol index with signatures, effects and contracts"`

---

### Task 2: `outline`, `graph`, `search`

**Files:**
- Create: `crates/yashy-driver/src/{outline.rs,graph.rs,search.rs}`
- Modify: `crates/yashy-driver/src/main.rs`, `docs/cli.md`
- Create: `crates/yashy-driver/tests/outline.rs`

**Interfaces:**
- Produces: `yashy outline <file>` — the file's symbols in source order, one line each (`fn fetch(url: string) -> Result[string, Error]  [io, async]`).
- Produces: `yashy graph [--json|--dot]` — the module graph plus the call graph; `--dot` is Graphviz, `--json` is `{"nodes":[…],"edges":[{"from","to","kind":"import"|"call"}]}`.
- Produces: `yashy search <query>` — substring and identifier-boundary matching over symbols and doc comments, ranked by (exact name, prefix, doc match), deterministic ties broken by file then span.

- [ ] **Step 1: Write the failing tests** — outline order equals source order; a cycle-free graph has one edge per import; `--dot` output parses as DOT (a strict bracket/shape check); search for `fetch` ranks the exact name first; search with no match exits 0 with an empty result, not an error.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — all three consume Task 1's model; `graph` additionally walks call sites from the HIR. No new analysis.
- [ ] **Step 4: Run** `cargo test -p yashy-driver --test outline` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(driver): outline, graph and search"`

---

### Task 3: `yashy context`

**Files:**
- Create: `crates/yashy-driver/src/context.rs`
- Modify: `crates/yashy-driver/src/main.rs`, `docs/cli.md`
- Create: `crates/yashy-driver/tests/context.rs`

**Interfaces:**
- Produces: `yashy context --task "fix the leak in file_cli" [--budget 8000]` → a Markdown pack with: the failing diagnostic, the enclosing function, its callees' signatures, the relevant stdlib signatures, the tests that cover it, and the contracts involved.
- Produces: `--json` with `{"tokens":7421,"budget":8000,"included":[…],"dropped":[…]}`; `tokens` is computed by a documented, versioned estimator (bytes/4 for ASCII, 1 token per non-ASCII char, rounded up), not guessed.
- Produces: exit 0 with a pack when it fits, exit 0 with a partial pack and a non-empty `dropped` when it does not, exit 2 when `--task` names nothing in the workspace.

- [ ] **Step 1: Write the failing tests** — a pack for a known bug contains the diagnostic, the function and the covering test; `tokens ≤ budget` always; requesting a 200-token budget drops the stdlib section first and says so; the same task produces byte-identical output twice.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — relevance is a fixed scoring table (diagnostic span > enclosing function > direct callees > tests > stdlib), not a model call. Sections are added in score order until the next would exceed the budget; each addition's cost is measured before it is added.
- [ ] **Step 4: Run** `cargo test -p yashy-driver --test context` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(driver): a token-budgeted context packer"`

---

### Task 4: MCP v0

**Files:**
- Create: `crates/yashy-driver/src/mcp.rs`, `crates/yashy-driver/tests/mcp.rs`
- Modify: `crates/yashy-driver/src/main.rs` (`mcp` subcommand), `docs/cli.md`
- Reuse: `crates/yashy-lsp/src/json.rs` framing (moved into `yashy-common` if it must be shared — one implementation)

**Interfaces:**
- Produces: `yashy mcp --root <dir>` speaking JSON-RPC 2.0 over stdio; tools `check`, `build`, `test`, `verify`, `explain`, `context`, `index`.
- Produces: every tool returns the schema-v1 payload the CLI returns; a tool error is a JSON-RPC error with `code: -32000` and the diagnostic array in `data`.
- Produces: no tool accepts a path outside `--root`; no tool accepts a shell string; `build` writes only under `<root>/.yashy/` and the requested output.

- [ ] **Step 1: Write the failing tests** — `initialize` advertises exactly the seven tools; `tools/call check` on a bad file returns diagnostics with codes; a path traversal (`../../etc/passwd`) is refused with `-32001` and no file read; a malformed request produces an error response and the server stays alive; an unknown method is `-32601`.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — the adapter parses a request, canonicalises the path, checks containment, calls the same `pub fn` the CLI calls, serialises the model. The server is a loop over stdin lines with a size cap (1 MiB) so a hostile client cannot exhaust memory.
- [ ] **Step 4: Run** `cargo test -p yashy-driver --test mcp` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(mcp): seven compiler verbs over JSON-RPC, sandboxed to the workspace"`

---

### Task 5: Sandbox v0

**Files:**
- Create: `crates/yashy-driver/src/sandbox/{mod.rs,windows.rs,linux.rs}`
- Modify: `crates/yashy-driver/src/main.rs` (`run --sandbox`, `test --sandbox`), `docs/cli.md`
- Create: `crates/yashy-driver/tests/sandbox_escape.rs`, `crates/yashy-driver/tests/sandbox_corpus/` (10 escape attempts)

**Interfaces:**
- Produces: a capability set `{ fs_read: [paths], fs_write: [paths], net: bool, spawn: bool, time_ms: u64, memory_mb: u64, output_bytes: u64 }`, default-deny.
- Produces: `yashy run --sandbox --allow-read . --allow-write ./.yashy --time 5000 --memory 256 prog.ya`; a denied capability is a runtime diagnostic `E1900` with the capability named, and a limit breach is `E1901`.
- Produces: Windows enforcement via a Job Object (memory, process count, kill-on-close) plus a restricted token for the file allowlist; Linux via rlimits plus a path allowlist checked by the runtime's `open`/`socket` wrappers.

- [ ] **Step 1: Write the failing tests** — the escape corpus: absolute path outside root, `..` traversal, symlink out, environment-variable path, `/proc/self/mem`, `fork` bomb, 1 GiB allocation, infinite loop, socket connect, child process spawn. Every one must be denied with `E1900`/`E1901`, and the test asserts the *reason*, not just a non-zero exit.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — the allowlist is resolved to canonical paths at launch; the runtime's file and socket entry points consult it; rlimits/Job Objects cover the resource side. Anything not on the list is denied by default, including `stdout` beyond the output cap.
- [ ] **Step 4: Run** `cargo test -p yashy-driver --test sandbox_escape` → PASS on Windows and Linux.
- [ ] **Step 5: Commit** — `git commit -m "feat(sandbox): default-deny capabilities with escape tests"`

---

### Task 6: The agent benchmark

**Files:**
- Create: `benchmarks/agent/` (10 tasks, each a small project plus a held-out variant), `crates/yashy-bench/src/agent.rs`
- Modify: `.github/workflows/bench.yml` (report-only on PR, hard on schedule)
- Modify: `benchmarks/README.md`

**Interfaces:**
- Produces: `yashy-bench agent --model <id> --tasks benchmarks/agent` runs each task through the MCP server and records `{task, success, tokens_in, tokens_out, tool_calls, sandbox_violations, wall_ms}`.
- Produces: a task passes when its test program exits 0 and the diff touches only the intended file; a held-out variant passes when the same fix generalises.

- [ ] **Step 1: Write the harness and three tasks** — a leak fix, a bounds bug, a failing test. Assert the harness records the five metrics and that a deliberately failing agent scores 0/3.
- [ ] **Step 2: Run** the harness with the reference "agent" (a scripted sequence of MCP calls) → 3/3.
- [ ] **Step 3: Add the remaining seven tasks and their variants**, then record the first full baseline in `benchmarks/agent/baseline.json`.
- [ ] **Step 4: Commit** — `git commit -m "test(bench): a ten-task agent benchmark with held-out variants"`

---

### Task 7: Program observability

**Files:**
- Create: `crates/yashy-stdlib/yashy/log.ya`, `crates/yashy-stdlib/yashy/metrics.ya`
- Modify: `crates/yashy-runtime/src/runtime.c` (structured sink, deterministic clock/RNG/env)
- Modify: `crates/yashy-driver/src/main.rs` (`run --deterministic`, `test --deterministic`)
- Create: `crates/yashy-driver/tests/programs/observability.ya`

**Interfaces:**
- Produces: `log.info("msg", fields)`, `log.error(..)`, `trace.span("name") { .. }`, `metrics.counter("name").add(1)`, `metrics.gauge("name").set(v)` — all emitting JSON lines to stderr, one object per event, with a stable schema.
- Produces: `--deterministic` fixes the clock at a given instant, seeds the RNG, sorts map iteration where observable, and pins the process id field; `YASHY_DETERMINISTIC=1` does the same for a test run.
- Produces: `log` and `metrics` are `io` effects (phase 2 inference already knows builtins; these are added to the table), so a `pure fn` cannot log.

- [ ] **Step 1: Write the failing test** — `observability.ya` runs twice with `--deterministic` and produces byte-identical stderr; a `pure fn` that calls `log.info` is `E1300`; a span reports its duration as a field; metrics flush on exit even after a panic.
- [ ] **Step 2: Watch it fail.**
- [ ] **Step 3: Implement** — the sink is one C function writing one JSON object per line, with a mutex; spans are a stack of (name, start) in thread-local storage; the deterministic clock is a function pointer the runtime reads, so no `#ifdef` in the language.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(stdlib): structured log, trace and metrics with a deterministic mode"`

---

### Task 8: Debugger and profiler integration

**Files:**
- Modify: `crates/yashy-codegen/src/fn_gen.rs` (frame pointers, DWARF names, inline-function records)
- Create: `crates/yashy-driver/src/profile.rs`, `crates/yashy-driver/tests/profile.rs`
- Modify: `docs/cli.md`, `docs/tutorial/17-debugging.md`

**Interfaces:**
- Produces: `yashy run --debug` launches under LLDB with a generated command file that sets a breakpoint on the entry function and prints the async task table (phase 10) on stop; the documented workflow covers both OSes.
- Produces: `yashy profile prog.ya` builds with per-function counters and prints a table sorted by self time, with the instrumentation overhead measured and printed; on Linux it also documents the `perf` workflow with the emitted frame pointers.

- [ ] **Step 1: Write the failing test** — the profiler table names the function that dominates a synthetic workload (≥ 50% self time) and reports its overhead as a number; the LLDB command file is generated and syntactically valid.
- [ ] **Step 2: Watch it fail.**
- [ ] **Step 3: Implement** — counters are a static array indexed by function id, incremented on entry and exit; the report reads them and subtracts nothing (self time = total minus children). No sampling thread, because a sampling profiler that changes scheduling is a different measurement problem.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: debugger workflow and a per-function profiler"`

---

### Task 9: Freeze schema v1

**Files:**
- Create: `docs/schema/v1/` (one JSON Schema file per payload: `check`, `test`, `verify`, `explain`, `index`, `context`, `mcp`, `bench`)
- Create: `crates/yashy-driver/tests/schema_diff.rs`, `.github/workflows/schema.yml`
- Modify: `docs/cli.md`

**Interfaces:**
- Produces: a golden payload per command in `crates/yashy-driver/tests/schema_golden/`; the test validates each against its schema and fails if a field is removed or its type changes.
- Produces: `cargo test -p yashy-driver --test schema_diff` is a required check; additive fields are allowed and recorded in `docs/schema/CHANGELOG.md`.

- [ ] **Step 1: Write the goldens** by running each command on a fixed fixture and committing the output.
- [ ] **Step 2: Write the diff test** — compare against the previous committed schema, fail on removal or type change, allow additions.
- [ ] **Step 3: Run** `cargo test -p yashy-driver --test schema_diff` → PASS.
- [ ] **Step 4: Commit** — `git commit -m "docs(schema): freeze v1 with a CI diff gate"`

---

### Task 10: Docs and gate

**Files:** `docs/tutorial/18-tooling.md`, `docs/cli.md` (all seven commands), `AGENTS.md` (the MCP verbs, the sandbox default), `llms.txt`.

- [ ] **Step 1: Write the chapter** with a worked `context` → `index` → `verify` session.
- [ ] **Step 2: Run the gate table on both OSes.**
- [ ] **Step 3: Commit and record** — `git commit -m "docs: AI-native tooling and observability, and close phase 11"`.

---

## Gate summary

| Gate | Command | Passes when |
|---|---|---|
| Context ≤ 8k tokens | `cargo test -p yashy-driver --test context` | every pack reports `tokens ≤ budget`; drops are named |
| Escape tests | `cargo test -p yashy-driver --test sandbox_escape` | all 10 attempts denied with the right code, both OSes |
| MCP is compiler-only | `cargo test -p yashy-driver --test mcp` | seven tools, traversal refused, malformed request survived |
| Schema v1 frozen | `cargo test -p yashy-driver --test schema_diff` | no removal, no type change |
| Benchmark runs | `yashy-bench agent --tasks benchmarks/agent` | 10 tasks + variants, five metrics recorded |
| Determinism | `cargo test -p yashy-driver --test e2e observability` | byte-identical stderr across runs |
| No regression | `cargo test --workspace` | green on Windows and Linux |
