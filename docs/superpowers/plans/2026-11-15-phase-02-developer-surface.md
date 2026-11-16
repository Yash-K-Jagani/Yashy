# Phase 2 — Developer Surface

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the compiler pleasant to live in: an editor that knows the whole
language, a debugger that stops on a line, a profiler that names the hot
allocation, diagnostics that can fix themselves, and a catalog that answers
every code.

**Outcome:** The full LSP surface, machine-applicable diagnostics (`fix`,
`--sarif`), `lint`, `coverage`, `new`, `task`, `repl`, a DWARF/CodeView
debugger, sampling and allocation profilers, `doc --serve`, `inspect`, `trace`,
`tokens`, an error-catalog completeness gate, two editor integrations, and a
drift test for `docs/architecture.md`.

**Time:** ~8 weeks.
**Depends on:** Phase 1 gate.
**Spec:** [`../specs/2026-11-15-yashy-2.0-design.md`](../specs/2026-11-15-yashy-2.0-design.md) §2.1, §6.4, §6.8, §6.9.

## Why this phase

1.0's tooling reads code well and changes nothing. The LSP has diagnostics,
hover, definition, completion and semantic tokens; it has no references,
rename, code actions, signature help, inlay hints, formatting, symbols,
hierarchy, or pull diagnostics. `profile` counts calls; there is no sampler.
`run --debug` prints a task table and writes an LLDB script. There is no
debugger, no REPL, no `fix`, no `lint`, no coverage, no `new`.

That surface is where a developer spends the day, and it is where an agent will
later get its edit verbs. Building `suggestions[]` once — in the diagnostic
model, the CLI, the LSP and the MCP server — is cheaper than building four
different edit paths. This phase is also the first consumer of the Phase 1
daemon and the Phase 1 parser recovery.

## Starting point

| Fact | 1.0 value |
|---|---|
| LSP | diagnostics, hover, definition, completion, semantic tokens |
| Diagnostics | labels, notes, `docs_url`; no machine-applicable edit |
| Error catalog | `docs/errors/<CODE>.md` exists; nothing fails on a missing page |
| Debugger | none; `run --debug` writes an LLDB script |
| Profiler | counter-based, per function |
| Formatter | canonical, idempotent for the 1.0 grammar |
| Linter | none (warnings only) |
| Coverage | none |
| Scaffolding | `init` only |
| REPL | none |
| Architecture doc | written in the 2.0 planning pass; no drift test |

## Deliverables

### Task 1: The LSP surface

**Files:**
- Modify: `crates/yashy-lsp/src/server.rs`, `crates/yashy-lsp/src/lib.rs`, `crates/yashy-lsp/src/main.rs`
- Create: `crates/yashy-lsp/src/references.rs`, `rename.rs`, `actions.rs`, `symbols.rs`, `hierarchy.rs`, `hints.rs`, `format.rs`
- Create: `crates/yashy-lsp/tests/conformance.rs`, `crates/yashy-lsp/tests/transcripts/*.jsonrpc`
- Test: `crates/yashy-driver/tests/lsp.rs` (extend)

**Interfaces:**
- `LspModel` — one snapshot built from `yashy-query`: symbols, types, effects, spans, doc comments, references.
- `ConformanceCase { name, transcript: PathBuf, expect: Vec<JsonRpcExpectation> }`
- `run_transcript(case: &ConformanceCase) -> Result<(), Vec<String>>`

- [ ] **Step 1: Write the failing transcript.** A file with a struct, a
  method, a generic function and an incomplete expression. Assert `references`,
  `rename` (prepare + apply), `signatureHelp`, `inlayHint`, `documentSymbol`,
  `workspaceSymbol`, `callHierarchy`, `typeHierarchy`, `foldingRange`,
  `selectionRange`, `codeLens`, `documentHighlight`, `codeAction` and
  `textDocument/formatting` each answer with a stable span.
- [ ] **Step 2: Build one model, not fourteen queries.** `LspModel` is
  refreshed from the daemon (Phase 1) after each edit; every feature reads it.
  A feature that needs a new fact adds it to the model so `index` and `mcp` can
  see it too.
- [ ] **Step 3: Make rename safe.** Rename refuses when the new name collides,
  when a macro expansion created the binding, or when the symbol is outside the
  workspace. Every edit is a `WorkspaceEdit` with byte spans and the expected
  document version.
- [ ] **Step 4: Pull and push diagnostics.** Keep push diagnostics for
  compatibility; add `textDocument/diagnostic` with result ids and unchanged
  reports. Delta semantic tokens use the same ids.
- [ ] **Step 5: Two editors, one transcript.** Ship `editors/vscode` (a thin
  `vscode-languageclient` extension) and `editors/neovim` (built-in LSP
  config). CI runs the transcript harness and a headless smoke test for each.
- [ ] **Step 6: Commit.** `git commit -m "feat(lsp): complete the language server surface"`.

### Task 2: Machine-applicable diagnostics

**Files:**
- Modify: `crates/yashy-common/src/diagnostic.rs` (additive `suggestions[]`, stable `id`)
- Modify: `crates/yashy-common/src/codes.rs` (rule id per code)
- Create: `crates/yashy-driver/src/fix.rs`, `crates/yashy-driver/src/sarif.rs`
- Modify: `crates/yashy-driver/src/main.rs` (`fix`, `--sarif`)
- Modify: `docs/cli.md` (schema v1 additive fields)
- Test: `crates/yashy-driver/tests/diagnostics_json.rs`, `crates/yashy-driver/tests/fix.rs`

**Interfaces:**
- `Suggestion { span, replacement: String, applicability: MachineApplicable | MaybeIncorrect, message }`
- `apply_suggestions(source: &str, suggestions: &[Suggestion]) -> Result<String, FixError>`
- `to_sarif(diagnostics: &[Diagnostic]) -> SarifLog`

- [ ] **Step 1: Write the failing test.** A program with a name typo, a missing
  `move`, and a wrong argument order. Assert `check --json` carries one
  `suggestions[]` per fixable diagnostic, `yashyc fix` rewrites the file, and a
  second `check` is clean. Assert the same suggestion applies through the LSP
  code action.
- [ ] **Step 2: One edit model.** A suggestion is a byte span, replacement text
  and an applicability class. The CLI, LSP and (Phase 13) MCP apply the same
  struct; none of them formats text on its own.
- [ ] **Step 3: Apply safely.** Suggestions are applied back-to-front,
  non-overlapping, and only to a file whose content hash matches the one the
  diagnostic was computed from. `--dry-run` prints the patch; `--json` emits
  it. A conflicting fix is refused, never guessed.
- [ ] **Step 4: SARIF.** Emit §2.1.0 with rules, levels, physical locations and
  fixes. CI uploads it; a schema test pins the shape.
- [ ] **Step 5: Commit.**

### Task 3: Lint and formatter completeness

**Files:**
- Create: `crates/yashy-driver/src/lint.rs`, `crates/yashy-lint/src/lib.rs`
- Modify: `crates/yashy-fmt/src/lib.rs` (new grammar from Phase 4 lands here too)
- Create: `docs/lints.md` (generated), `crates/yashy-lint/tests/*.rs`
- Test: `crates/yashy-driver/tests/lint.rs`, `crates/yashy-driver/tests/fmt_check.rs` (extend)

**Interfaces:**
- `Lint { code: &'static str, group: LintGroup, rule: RuleId, check: fn(&HirProgram, &mut LintBag) }`
- `LintGroup::{Correctness, Style, Complexity, Perf, Security, Deprecated}`
- `yashyc lint [--group <g>] [--deny <code>] [--json]`

- [ ] **Step 1: Write the failing test.** An unused binding, a shadowed name, a
  `switch` that could be an `if let`, a `while` with an invariant, and a
  discarded `Result`. Assert each gets a code, a group and a suggestion; assert
  `--deny` changes the exit code.
- [ ] **Step 2: Lints are data.** A lint is registered once with a code, a
  group, a rule id and a check over HIR. It is off, warn or deny by manifest or
  flag. `yashyc lint --list --json` is the machine form; `docs/lints.md` is
  rendered from it.
- [ ] **Step 3: The formatter owns layout.** The formatter is total over the
  grammar, idempotent, comment-preserving and byte-stable across OSes. It never
  splits a keyword or operator token (the BPE rule), and `--check` reports
  every file that would change.
- [ ] **Step 4: Commit.**

### Task 4: The error-catalog gate

**Files:**
- Modify: `crates/yashy-common/src/codes.rs` (rule id, page path, summary)
- Modify: `docs/errors.md`, `docs/errors/*.md`
- Create: `crates/yashy-driver/tests/error_catalog.rs`
- Modify: `crates/yashy-driver/src/main.rs` (`explain --all`)

- [ ] **Step 1: Write the failing test.** For every code in the registry,
  assert: a page exists, the page's first heading contains the code, the
  catalog lists it, `explain --json` returns `title`/`summary`/`page`/
  `docs_url`, and at least one test or fixture triggers it.
- [ ] **Step 2: Make the catalog generated.** `yashyc explain --all --json` is
  the machine form; `docs/errors.md` is rendered from it plus the pages. A code
  without a page fails the build; a page without a code fails the docs test.
- [ ] **Step 3: Rule ids.** Every code names the rule it enforces (`OWN-3`,
  `ALIAS-2`, `DET-4`, …). Phase 9 turns this into the full traceability matrix.
- [ ] **Step 4: Commit.**

### Task 5: `new`, `task`, `repl`, `doc --serve`

**Files:**
- Create: `crates/yashy-driver/src/scaffold.rs`, `crates/yashy-driver/src/task.rs`, `crates/yashy-driver/src/repl.rs`, `crates/yashy-driver/src/doc_server.rs`
- Modify: `crates/yashy-driver/src/main.rs`
- Create: `crates/yashy-driver/tests/new.rs`, `crates/yashy-driver/tests/task.rs`, `crates/yashy-driver/tests/repl.rs`, `crates/yashy-driver/tests/doc_server.rs`
- Add templates under `templates/{bin,lib,http,cli}`

- [ ] **Step 1: Write the failing `new` test.** `yashyc new hello --template bin`
  creates a project whose first `yashyc test` passes in under a minute on a cold
  machine; `--list` is the machine form; an unknown template is a diagnostic,
  not a panic.
- [ ] **Step 2: `task` is a named command, not a shell.** `yashy.toml` gains
  `[tasks] test = "yashyc test"`, `fmt = "yashyc fmt -w ."`. `yashyc task <name>`
  runs the named compiler verb sequence. It never executes an arbitrary shell
  string; a task is a list of verbs and arguments, and `--json` lists them.
- [ ] **Step 3: A REPL with honest limits.** The REPL parses and checks a
  snippet, then evaluates it in a persistent HIR interpreter for the supported
  subset (values, functions, structs, `Option`/`Result`, no FFI, threads, or
  filesystem writes). Unsupported input says so and points at `yashyc run`.
  State is retained between lines; each line reports its type.
- [ ] **Step 4: `doc --serve`.** Serve the rendered docs over a loopback HTTP
  server with a search index and a `/json` endpoint. No write access, no
  network bind beyond loopback, `--port 0` prints the chosen port.
- [ ] **Step 5: Commit.**

### Task 6: The debugger

**Files:**
- Modify: `crates/yashy-codegen/src/*` (DWARF on ELF/Mach-O, CodeView/PDB on Windows)
- Create: `crates/yashy-driver/src/debug.rs`, `crates/yashy-runtime/debug/pretty_printers.py`
- Modify: `crates/yashy-driver/src/main.rs` (`debug`)
- Create: `crates/yashy-driver/tests/debug.rs`
- Docs: `docs/debugging.md`

- [ ] **Step 1: Write the failing test.** Compile a program with a function,
  a struct, a `switch` and an async task. Assert the binary has a line table and
  a type table for every user type, that `yashyc inspect --addr <symbol>`
  returns a source span, and that a scripted debugger session stops on the
  expected line and prints the expected value.
- [ ] **Step 2: Emit debug info from the same IR.** Every user function, local,
  parameter and type gets a `!dbg` location; the span comes from HIR, not from a
  re-parse. Optimised builds keep line tables; `--debug` builds keep full
  variable info.
- [ ] **Step 3: `yashyc debug` selects a backend.** LLDB where available (Linux,
  macOS, Windows when installed), `cdb` on Windows otherwise; a missing backend
  is a diagnostic with an install hint. The command writes a temporary script,
  launches the program with the user's arguments, and cleans up.
- [ ] **Step 4: Make the async state visible.** A task view renders its state,
  its scope, and the await point; pretty printers render `string`, arrays, maps
  and `Option`/`Result` as values, not addresses.
- [ ] **Step 5: Commit.**

### Task 7: Profilers and coverage

**Files:**
- Modify: `crates/yashy-driver/src/profile.rs`, `crates/yashy-codegen/src/fn_gen.rs`
- Create: `crates/yashy-driver/src/coverage.rs`, `crates/yashy-driver/src/sample.rs`
- Create: `crates/yashy-driver/tests/profile_sampling.rs`, `crates/yashy-driver/tests/coverage.rs`
- Docs: `docs/profiling.md`, `docs/coverage.md`

- [ ] **Step 1: Write the failing test.** A program with a hot loop, a cold
  function and one deliberate leak. Assert the sampler attributes ≥ 80% of
  samples to the hot function, the allocation profiler names the leaking site
  with a source span, and `yashyc coverage` reports the executed lines as
  lcov and JSON.
- [ ] **Step 2: Sampling is platform-native.** Linux `perf_event_open` (or
  `perf record` when permitted), Windows ETW/`DbgHelp` stack walking, macOS
  `sample`; when none is available, say so and fall back to the counter
  profiler. Symbols come from the debug info Phase 2 already emits.
- [ ] **Step 3: Allocation profiling has no hidden hooks.** The allocator
  interface records size, site and a bounded stack; `--alloc-report` is the
  same data in table form. Leaks are reported at exit with the allocation span.
- [ ] **Step 4: Coverage is source-based.** Compile with clang's
  `-fprofile-instr-generate -fcoverage-mapping`, run the test suite, merge
  profiles, and render per-file/per-line counts. `--fail-under` gates CI.
- [ ] **Step 5: Commit.**

### Task 8: `inspect`, `trace`, `tokens`

**Files:**
- Create: `crates/yashy-driver/src/inspect.rs`, `crates/yashy-driver/src/trace.rs`, `crates/yashy-driver/src/tokens.rs`
- Modify: `crates/yashy-driver/src/main.rs`
- Create: `crates/yashy-driver/tests/inspect.rs`, `crates/yashy-driver/tests/trace.rs`, `crates/yashy-driver/tests/tokens.rs`

- [ ] **Step 1: Write the failing test.** `inspect --json` reports a symbol's
  signature, effects, contracts, layout (when Phase 11A lands) and source span;
  `inspect --addr` maps an address to a span. `trace --events` emits a JSON
  event per allocation, free, task start/end and I/O with a span id, in a
  deterministic order.
- [ ] **Step 2: `tokens --bpe` is a measurement, not an opinion.** For a file or
  a corpus, report tokens per keyword/operator and corpus totals for `o200k`,
  `cl100k` and a Llama-family tokenizer. The tokenizer tables are vendored as
  data (no network). Phase 4 uses it to audit new syntax; Phase 13 gates it.
- [ ] **Step 3: Document the architecture drift test.** `cargo test -p
  yashy-driver --test architecture` asserts that `docs/architecture.md` lists
  every workspace crate and every dispatch verb, and that the phase table in
  the master plan names every phase file that exists.
- [ ] **Step 4: Commit.**

### Task 9: Editor integrations

**Files:**
- Create: `editors/vscode/{package.json,src/extension.ts,README.md}`
- Create: `editors/neovim/{init.lua,README.md}`
- Create: `editors/README.md`
- Test: CI job `editors` (headless smoke for both)

- [ ] **Step 1: VS Code.** Syntax highlighting from the lexer's token set,
  LSP client, debug adapter configuration that calls `yashyc debug`, task
  provider for `yashyc task`, and a status item for `yashyc doctor`.
- [ ] **Step 2: Neovim.** `vim.lsp.start` config, treesitter grammar generated
  from the same token set, `:make` mapped to `yashyc check`, and a DAP config.
- [ ] **Step 3: One grammar source.** Both editors read a generated
  `yashy.tmLanguage.json` and `grammar.js` from `crates/yashy-lexer` keywords, so
  a new keyword cannot be highlighted in one editor and not the other.
- [ ] **Step 4: Commit.**

## Gates

| Gate | Command | Passes when |
|---|---|---|
| LSP conformance | `cargo test -p yashy-lsp --test conformance` | every transcript feature answers with a stable span; no panic on an incomplete file |
| Two editors | `editors` CI job | VS Code and Neovim each complete, hover, rename, format and stop at a breakpoint in the fixture |
| Machine fixes | `cargo test -p yashy-driver --test fix` | every fixable diagnostic applies; a conflicting fix is refused; a stale hash is refused |
| Schema | `cargo test -p yashy-driver --test diagnostics_json --test schema_diff` | v1 fields unchanged; `suggestions[]` and `id` are additive; SARIF validates |
| Error catalog | `cargo test -p yashy-driver --test error_catalog` | every code has a page, a catalog entry, an `explain --json` record and a triggering test |
| First test | `yashyc new hello && cd hello && yashyc test` | under 60 s on a cold machine, both OSes |
| Debugger | `cargo test -p yashy-driver --test debug` | a source breakpoint stops on the expected line; a struct prints as a value; `inspect --addr` maps back |
| Profilers | `cargo test -p yashy-driver --test profile_sampling` | ≥ 80% of samples on the hot function; the leaking site is named with a span |
| Coverage | `cargo test -p yashy-driver --test coverage` | lcov and JSON agree; `--fail-under` gates |
| Architecture | `cargo test -p yashy-driver --test architecture` | crates and verbs in the doc match the workspace |
| No regression | `cargo test --workspace` | green on Windows and Linux |
| No P0 | `docs/triage.md` | no P0 |

## Demo

```console
$ yashyc new hello --template bin
created hello (bin, edition 2027)
$ cd hello && yashyc test
1 test ................... ok  (0.18 s)

$ yashyc lint --json | yashyc fix --stdin
fixed 2 diagnostics in main.ya

$ yashyc debug main.ya --break main.ya:12
stopped at main.ya:12  total = 41
(lldb) print order
Order { id: 7, items: 3, total: 41 }
```

```console
$ yashyc profile main.ya --sample
samples 1,204   self 87.3%  run
  87.3%  run        main.ya:8
   9.1%  parse      main.ya:21
$ yashyc coverage --fail-under 90
lines 94.2% (163/173)  functions 100% (12/12)
```

## Out of scope

- Language forms (Phase 4); the LSP formats the 1.0 grammar and gains each new
  form in the phase that adds it.
- Signed registry and installers (Phase 3).
- The traceability matrix (Phase 9); this phase adds rule ids only.
- A GUI debugger; `yashyc debug` drives an existing backend.
- Remote debugging over a network.

## Risks

| Risk | Mitigation |
|---|---|
| Fourteen LSP features, fourteen models | One `LspModel` per snapshot; a feature adds a fact to the model, never a private query |
| Rename corrupts code | Versioned `WorkspaceEdit`, byte spans, collision refusal, macro-origin refusal |
| A fix applies to the wrong text | Suggestions carry the source hash and apply back-to-front; a stale hash is refused |
| The REPL promises more than it can run | A documented HIR subset; unsupported input names the boundary and points at `yashyc run` |
| The debugger only works on Linux | `yashyc debug` selects LLDB or `cdb`; the gate runs on Windows and Linux |
| Sampling needs privileges | Fall back to the counter profiler with a diagnostic naming the missing capability |
| A lint becomes a second verifier | Lints are style/correctness advice over HIR; no lint changes whether a program builds |
