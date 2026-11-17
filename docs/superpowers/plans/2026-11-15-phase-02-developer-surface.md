# Phase 2 — Developer Surface

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the compiler pleasant to live in: an editor that knows the whole
language, a terminal that answers before the command is finished, a debugger
that stops on a line, a profiler that names the hot allocation, diagnostics
that can fix themselves, and a catalog that answers every code.

**Outcome:** The full LSP surface, machine-applicable diagnostics (`fix`,
`--sarif`), `lint`, `coverage`, `new`, `task`, `repl`, a DWARF/CodeView
debugger, sampling and allocation profilers, `doc --serve`, `inspect`, `trace`,
`tokens`, an error-catalog completeness gate, one generated grammar that serves
VS Code, Neovim, Helix, Zed, Emacs and JetBrains, generated shell completions
and man pages, a TTY-aware output policy, and a drift test for
`docs/architecture.md`.

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

The same is true of the two places a person actually types. 1.0 ships syntax
files for VS Code, Vim and Emacs -- hand-maintained keyword lists that a test
compares against the lexer -- but no LSP client, no debug adapter, no
format-on-save and no generated grammar. The terminal is hand-rolled too: no
completions, no man page, no colour policy, no pager and no clickable path. A
language that is pleasant in the editor but awkward in the shell is only half
installed, and an agent that later drives `yashyc` inherits the same surface.
This phase makes both renderings of the compiler's facts.

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
| Editors | VS Code TextMate grammar and language config, Vim syntax, Emacs mode; no client, DAP, format-on-save or generator |
| Terminal | usage text in `main.rs`; no completions, man page, colour policy, pager or clickable path |
| Grammar drift test | `crates/yashy-lsp/tests/grammar.rs` compares hand-maintained keyword lists; nothing generates them |
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

- [x] **Step 1: Write the failing transcript.** A file with a struct, a
  method, a generic function and an incomplete expression. Assert `references`,
  `rename` (prepare + apply), `documentSymbol`, `foldingRange`, `selectionRange`,
  `documentHighlight`, `codeAction` and `textDocument/formatting` each answer
  with a stable span; `signatureHelp`, `inlayHint`, `workspaceSymbol`,
  `callHierarchy`, `typeHierarchy` and `codeLens` answer too. The conformance
  transcript now covers all 19 features.
- [x] **Step 2: Build one model, not fourteen queries.** `LspModel` is
  rebuilt from the document after each edit; every feature reads it. The model
  is a source-derived symbol/occurrence table, not a snapshot from
  `yashy-query`; that substitution is the exception recorded below.
- [x] **Step 3: Make rename safe.** Rename refuses invalid identifiers and
  collisions with declared names. The model is one open document, so the
  macro-origin and out-of-workspace refusals do not apply yet. Edits are a
  `WorkspaceEdit` with LSP ranges.
- [x] **Step 4: Pull and push diagnostics.** Keep push diagnostics for
  compatibility; add `textDocument/diagnostic`. Result ids and delta semantic
  tokens are still open.
- [ ] **Step 5: Two clients, one transcript.** Wire the VS Code and Neovim
  clients (Task 9 owns their packaging, grammar and debug adapters) and run the
  same transcript through both.
- [x] **Step 6: Commit.**

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
  *Current:* `fix.rs` covers a removed-keyword suggestion, the rewrite, the
  dry run, back-to-front application and the stale-hash refusal. The missing
  `move` case, the wrong-argument-order case, and the LSP code-action
  application are not implemented; the code-action response is still a rename
  stub with an empty edit.
- [x] **Step 2: One edit model.** A suggestion is a byte span, replacement text
  and an applicability class. The CLI and `check --json` use the same struct;
  the LSP code action does not consume it yet.
- [x] **Step 3: Apply safely.** Suggestions are applied back-to-front,
  non-overlapping, and only to a file whose content hash matches the one the
  diagnostic was computed from. `--dry-run` prints the patched file; `--json`
  emits the diagnostic suggestions.
- [x] **Step 4: SARIF.** Emit §2.1.0 with rules, levels and physical locations.
  `fixes` objects are not emitted yet; CI upload and the schema test pin the
  shape that does exist.
- [x] **Step 5: Commit.**

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

- [x] **Step 1: Write the failing test.** An unused binding, a shadowed name
  and a removed keyword are reported with a code and a group. `--deny`, the
  `switch`/`while`/`Result` lints and lint-supplied suggestions are not
  implemented yet; `lint.rs` tests the three lints it has.
  *Current:* six lints now: unused, shadowed, removed `var`, long
  `else if` chains (`LINT-SWITCH`), endless `while true` (`LINT-WHILE`) and an
  ignored fallible call (`LINT-RESULT`). Each has a code, group, message, fix
  hint and a test. The checks are deliberately source-level advice: they do
  not claim the checker's type information.
- [x] **Step 2: Lints are data.** A lint is registered once with a code, a
  group and a check. `yashyc lint --list --json` is the machine form and
  `yashyc lint --docs` renders `docs/lints.md`, which the lint test compares
  byte for byte.
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

- [x] **Step 1: Write the failing test.** For every code in the registry,
  assert: a page exists, the page's first heading contains the code, the
  catalog lists it, `explain --json` returns `title`/`summary`/`page`/
  `docs_url`. The "at least one test or fixture triggers it" assertion is not
  implemented; `error_catalog.rs` checks the page, heading, catalog entry,
  `explain --json` record and the all-codes envelope.
- [x] **Step 2: Make the catalog generated.** `yashyc explain --all --json` is
  the machine form; `docs/errors.md` is rendered from it plus the pages. A code
  without a page fails the build; a page without a code fails the docs test.
- [x] **Step 3: Rule ids.** Every code names the rule it enforces
  (`OWN-3`, `ALIAS-2`, `DET-4`, …) through `codes::rule_for`; `explain --json`
  and `explain --all --json` carry a `rule` field, and a unit test refuses an
  empty or duplicate rule. Phase 9 turns this into the full traceability
  matrix.
- [x] **Step 4: Commit.**

### Task 5: `new`, `task`, `repl`, `doc --serve`

**Files:**
- Create: `crates/yashy-driver/src/scaffold.rs`, `crates/yashy-driver/src/task.rs`, `crates/yashy-driver/src/repl.rs`, `crates/yashy-driver/src/doc_server.rs`
- Modify: `crates/yashy-driver/src/main.rs`
- Create: `crates/yashy-driver/tests/new.rs`, `crates/yashy-driver/tests/task.rs`, `crates/yashy-driver/tests/repl.rs`, `crates/yashy-driver/tests/doc_server.rs`
- Add templates under `templates/{bin,lib,http,cli}`

- [x] **Step 1: Write the failing `new` test.** `yashyc new hello --template bin`
  creates a project whose first `yashyc test` passes in under a minute on a cold
  machine; `--list` is the machine form; an unknown template is a diagnostic,
  not a panic.
- [x] **Step 2: `task` is a named command, not a shell.** `yashy.toml` gains
  `[tasks] test = "yashyc test"`, `fmt = "yashyc fmt -w ."`. `yashyc task <name>`
  runs the named compiler verb sequence. It never executes an arbitrary shell
  string; a task is a list of verbs and arguments, and `--list` names them.
- [x] **Step 3: A REPL with honest limits.** The REPL parses and checks a
  snippet, then evaluates it in a persistent HIR interpreter for the supported
  subset (values, functions, structs, `Option`/`Result`, no FFI, threads, or
  filesystem writes). Unsupported input says so and points at `yashyc run`.
  State is retained between lines; each line reports its type.
  *Current:* implemented as a **compiling** REPL, not a persistent HIR
  interpreter: `repl.rs` keeps accepted declarations as source text, every
  line is checked by the real checker, an expression becomes a one-function
  program printed by `print(...)`, and the value's type is read from the
  checked HIR with `expr_ty`. FFI, `unsafe`, imports, `async`, threads and
  filesystem writes are refused with a pointer to `yashyc run`. The difference
  from the plan is a deliberate one: a second interpreter would be a second
  implementation of the language, and the compiler already compiles a small
  program quickly.
- [x] **Step 4: `doc --serve`.** Serve the rendered docs over a loopback HTTP
  server with a search index and a `/json` endpoint. No write access, no
  network bind beyond loopback, `--port 0` prints the chosen port.
  *Current:* `doc_server.rs` builds the model once, binds only `127.0.0.1`,
  serves `/`, `/json`, `/search?q=...` and `/<module>.md`, and answers `404`
  for anything else. `--port 0` prints the chosen URL. The routing rules are
  unit-tested; `tests/doc_server.rs` also binds a real loopback socket.
- [x] **Step 5: Commit.**

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
  *Current:* not met. There is no `!dbg` metadata in the generated IR, so there
  is no line or type table to assert. `yashyc debug` is tested for backend
  selection and script generation only. Source-line breakpoints are the
  follow-up, not a shipped capability.
- [ ] **Step 2: Emit debug info from the same IR.** Every user function, local,
  parameter and type gets a `!dbg` location; the span comes from HIR, not from a
  re-parse. Optimised builds keep line tables; `--debug` builds keep full
  variable info.
  *Current:* not started. `yashy-codegen` emits no LLVM debug intrinsics or
  metadata, so `-g` at the clang stage would have nothing to attach.
- [x] **Step 3: `yashyc debug` selects a backend.** LLDB where available (Linux,
  macOS, Windows when installed), `cdb` on Windows otherwise; a missing backend
  is a diagnostic with an install hint. The command writes a temporary script,
  launches the program with the user's arguments, and cleans up.
  *Current:* `debug.rs` prefers LLDB on every OS, falls back to `cdb` on
  Windows, exits `2` with an install hint when neither is present, writes both
  script dialects, passes `--break` names and post-`--` arguments, and removes
  the temporary directory after the session.
- [ ] **Step 4: Make the async state visible.** A task view renders its state,
  its scope, and the await point; pretty printers render `string`, arrays, maps
  and `Option`/`Result` as values, not addresses.
  *Current:* the runtime's `YASHY_DEBUG=1` stop handler already prints the live
  task table (id, function, resume index) and the debug script turns it on.
  The await point, scope details and typed pretty-printing are not there: the
  LLDB summary providers are committed under `crates/yashy-runtime/debug/`
  but cannot be attached without a type table.
- [x] **Step 5: Commit.**

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
  *Current:* `inspect --json` reports signature, effects, docs, contracts and
  source span from the index; `inspect --human` prints a clickable position.
  `--addr`, layout and the whole `trace --events` verb are still open, so the
  step stays unchecked.
- [ ] **Step 2: `tokens --bpe` is a measurement, not an opinion.** For a file or
  a corpus, report tokens per keyword/operator and corpus totals for `o200k`,
  `cl100k` and a Llama-family tokenizer. The tokenizer tables are vendored as
  data (no network). Phase 4 uses it to audit new syntax; Phase 13 gates it.
  *Current:* `tokens --bpe <paths>` measures the lexical single-token property
  with Yashy's own lexer and reports per-token-class counts and corpus totals.
  The three external vocabulary tables are not vendored, so the JSON names its
  tokenizer and says it cannot measure them; the Phase 13 gate is unmet rather
  than faked. `yashyc trace --json` now emits the static rule matrix as well.
- [x] **Step 3: Document the architecture drift test.** `cargo test -p
  yashy-driver --test architecture` asserts that `docs/architecture.md` lists
  every workspace crate and every dispatch verb, and that the phase table in
  the master plan names every phase file that exists.
- [x] **Step 4: Commit.**

### Task 9: Editors, one grammar source

**Files:**
- Modify: `crates/yashy-lexer/src/lib.rs`
- Create: `crates/yashy-lexer/src/grammar.rs`
- Create: `crates/yashy-driver/src/grammar.rs`
- Modify: `crates/yashy-driver/src/tokens.rs`, `crates/yashy-driver/src/main.rs`
- Create: `editors/grammars/{yashy.tmLanguage.json,yashy.vim,yashy.el,helix/languages.toml}`
- Create: `editors/grammars/tree-sitter-yashy/{grammar.js,queries/highlights.scm,queries/locals.scm,queries/folds.scm}`
- Modify: `editors/vscode/{package.json,language-configuration.json}`
- Create: `editors/vscode/{src/extension.ts,snippets/yashy.json,README.md}`
- Create: `editors/neovim/{lua/yashy/init.lua,README.md}`
- Create: `editors/zed/{extension.toml,src/lib.rs}`, `editors/jetbrains/README.md`
- Modify: `editors/vim/yashy.vim`, `editors/emacs/yashy.el` (now generated)
- Create: `editors/README.md`
- Test: `crates/yashy-driver/tests/grammar.rs`, `crates/yashy-lsp/tests/grammar.rs` (extend), CI job `editors`

**Interfaces:**
- `GrammarModel { keywords, types, operators, delimiters, comments, docs, literals }`
- `render_textmate(&GrammarModel) -> String`, `render_tree_sitter(&GrammarModel) -> String`, `render_vim`, `render_emacs`
- `yashyc tokens --grammar <dir>` writes every generated file.

- [x] **Step 1: Write the failing test.** Run `yashyc tokens --grammar
  editors/grammars` and assert `git diff --exit-code` is clean; add a keyword to
  `TokenKind::KEYWORDS` and assert generation fails until the files are
  rewritten. Every keyword, operator and delimiter has exactly one highlight
  class, and TextMate, tree-sitter, Vim and Emacs list the same set.
  *Current:* `tests/grammar.rs` compares every committed file's bytes with the
  renderer and builds a model with one extra keyword to prove the files go
  stale. The bash/CI `git diff --exit-code` wrapper is deferred with CI; the
  byte comparison is the same gate inside `cargo test`. The command accepts
  both `editors` and `editors/grammars` as its directory.
- [x] **Step 2: One model, several renderings.** The lexer owns the token
  classes; the generator owns the file formats. Tree-sitter's rule skeleton is
  hand-written once, but its keyword and operator tables are generated from the
  same model. The existing keyword drift test stays as the backstop rather than
  the mechanism.
  *Current:* `GrammarModel` is built from `TokenKind::KEYWORDS`,
  `TokenKind::PUNCTUATION` and the type list; a lexer unit test lexes every
  punctuation spelling back to its own kind, so a new operator fails before a
  grammar is generated. The LSP drift test now targets the
  `keyword.control.yashy` rule so the operator rule's `(?:…)` is not mistaken
  for a keyword list.
- [x] **Step 3: Wire the clients.** VS Code gets a thin
  `vscode-languageclient` extension, a DAP configuration that calls `yashyc
  debug`, a task provider for `yashyc task`, a `doctor` status item, snippets
  and a language configuration. Neovim gets `vim.lsp.start`, the generated
  tree-sitter grammar, `:make` mapped to `yashyc check` and a DAP config.
  Helix, Zed, Emacs and JetBrains get documented LSP/DAP configurations that
  point at `yashyc lsp --stdio`; none of them bundles a second compiler.
  *Current:* all six clients and their configs are committed. `yashyc debug`
  is not implemented yet (Task 6), so the DAP launch entry is declared but
  named as pending in the VS Code README rather than claimed as working.
- [x] **Step 4: Two gated, four documented.** VS Code and Neovim are the gate;
  the other editor configurations are validated when the editor's headless CLI
  is available and are cuttable. Format-on-save, run, test and debug work from
  the same compiler verbs, never from an editor-only script.
  *Current:* the shared LSP transcript is `yashy-lsp/tests/conformance.rs`, and
  the client configs are asserted by `tests/grammar.rs`. Running the transcript
  inside real VS Code and Neovim is a CI/headless job the owner deferred.
- [x] **Step 5: Commit.**

### Task 10: The terminal surface

**Files:**
- Create: `crates/yashy-driver/src/cli_spec.rs`, `term.rs`, `completions.rs`, `man.rs`, `hyperlink.rs`
- Modify: `crates/yashy-driver/src/main.rs` (dispatch, help, colour, pager)
- Create: `scripts/completions/{yashyc.bash,yashyc.zsh,yashyc.fish,yashyc.ps1}` (generated)
- Create: `docs/man/yashyc.1` (generated), `docs/terminal.md`
- Test: `crates/yashy-driver/tests/terminal.rs`, `.../completions.rs`, `.../man.rs`

**Interfaces:**
- `CommandSpec { name, aliases, summary, usage, flags, examples, completion }`
- `Style::{Auto, Always, Never}`, `Pager::{Auto, Never, Command(String)}`
- `yashyc completions <bash|zsh|fish|powershell>`, `yashyc help [verb]`

- [x] **Step 1: Write the failing test.** Pipe `yashyc check` on a file with a
  diagnostic: no ANSI escape, no spinner, no prompt, one line per diagnostic,
  stable order. `--color=always` emits SGR; `NO_COLOR`, `CLICOLOR=0` and
  `TERM=dumb` degrade to ASCII; `COLUMNS=40` wraps. `yashyc`, `yashyc help`,
  `yashyc help build`, `--help` and `-h` all answer, and an unknown verb
  suggests the nearest one and exits `2`.
  *Current:* `term.rs` resolves the policy from `--color`, `NO_COLOR`,
  `CLICOLOR`, `TERM` and `COLUMNS`; `--color=always` colours the header and
  severity, `--color=never` and `--no-color` stay plain, and a long diagnostic
  wraps. `terminal.rs` covers the plain pipe, help, an unknown verb, colour and
  wrapping. A terminal-attached `auto` case is a CI/manual check, not a unit
  assertion.
- [x] **Step 2: One command table.** A verb is declared once with its flags,
  defaults, examples and completion kind. Help, the man page, the shell
  completions and `index --json` render from that table; a flag added in one
  place cannot be missing from another. The table now names every verb in
  `dispatch` (the previous 18-verb gap is closed).
- [x] **Step 3: Shell integration is generated.** `yashyc completions bash`
  (and zsh, fish, PowerShell) prints a script that completes verbs, flags and
  file arguments; the generated files under `scripts/completions/` are
  diff-checked and syntax-checked in CI. `docs/terminal.md` shows the one-line
  install for each shell.
  *Current:* the four scripts are committed under `scripts/completions/` and
  `terminal.rs` compares each one with `render_completions` byte for byte, so
  `cargo test` is the diff check; the same test runs `bash -n` when bash is on
  the host, and the PowerShell file parses under
  `System.Management.Automation.Language.Parser`. `.gitattributes` pins the
  scripts and `docs/lints.md` to LF so the comparison is about the table, not
  the checkout. File arguments are completed by the shell's own filename
  completion once no verb is being typed; the scripts complete verbs and flags.
- [x] **Step 4: Pager and streams.** `explain` pages through `PAGER` only on a
  TTY; `--no-pager` wins; `--json` never pages and never colours. `run`
  forwards stdin, stdout, stderr and signals and returns the program's exit
  code. `watch` redraws only on a TTY and prints one line per diagnostic when
  piped; `test` progress appears only on a TTY and never in `--json`.
  *Current:* `term::page` starts `PAGER` only when stdout is a terminal and the
  policy allows it, and a missing pager falls back to writing the page;
  `--pager`, `--no-pager` and a pipe are tested. `run` inherits all three
  streams and forwards the child's whole exit code (Windows codes are 32-bit,
  so it exits through `std::process::exit`); Ctrl-C reaches the child through
  the shared process group, and a child killed by a signal reports `1` rather
  than re-raising the signal, which is the remaining half of "forwards
  signals". `watch` clears the screen on a TTY and prints `file: code: message`
  one line per diagnostic in a pipe; `test` prints one line per test and the
  `--json` path prints no progress at all. There is no spinner, so there is
  nothing to suppress in `--json` beyond the lines that are already absent.
- [x] **Step 5: Paths are clickable when the terminal can click.** A diagnostic
  path is a `path:line:col` string in plain output and an OSC 8 hyperlink on a
  capable TTY; `--hyperlink auto|always|never` overrides, and JSON never carries
  an escape. The same rule covers a source span printed by `inspect` and
  `trace`.
  *Current:* `term::link` wraps a header in `OSC 8 ;; file:///… :line:col` when
  the policy allows it; `--hyperlink`, `--hyperlink=…` and `--no-hyperlink` are
  global flags; capability is a positive list (`WT_SESSION`, `TERM_PROGRAM`,
  `KITTY_WINDOW_ID`, `VTE_VERSION` ≥ 5000, a known `TERM`). `inspect --human`
  prints `path:line:col: signature` through the same helper, so a click opens
  the declaration. `trace` does not exist yet, so its half of this step waits
  for Task 8.
- [x] **Step 6: Commit.**

## Gates

| Gate | Command | Passes when |
|---|---|---|
| LSP conformance | `cargo test -p yashy-lsp --test conformance` | every transcript feature answers with a stable span; no panic on an incomplete file |
| Editor matrix | `editors` CI job | VS Code and Neovim each complete, hover, rename, format and stop at a breakpoint in the fixture; Helix, Zed, Emacs and JetBrains configurations validate when their headless CLI is present |
| Generated grammar | `cargo test -p yashy-driver --test grammar` | `yashyc tokens --grammar` reproduces every committed file; a new keyword fails until regenerated |
| Terminal | `cargo test -p yashy-driver --test terminal` | piped output is plain and ordered; colour, pager and hyperlink policy hold; every verb is in help, man and all four completions; each script parses |
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

```console
$ yashyc completions bash > ~/.local/share/bash-completion/completions/yashyc
$ yashyc he<TAB>
help       history    hello
$ man yashyc | head -n 3
YASHC(1)                         Yashy                         YASHC(1)
$ yashyc check main.ya | cat
main.ya:7:5: error[E0100]: ...
  (plain text: no colour, no spinner; `main.ya:7:5` is clickable in a TTY)
```

```console
VS Code: "Yashy: Format Document" formats on save;
         F5 starts `yashyc debug` and stops at main.ya:12;
         the status bar shows `yashyc doctor` green.
Neovim:  :make runs `yashyc check`; <leader>df formats;
         the DAP session shows the same stack and the same values.
```

## Exceptions

The Phase 2 surface is partly shipped. This table names what is real and what is
still owed so a later phase does not inherit a false checklist.

| Task | Claimed | Shipped | Follow-up |
|---|---|---|---|
| 1.1–1.3 | All LSP features answer; `LspModel` is a `yashy-query` snapshot; rename refuses macro-origin and out-of-workspace edits | All 19 features answer from a source-derived `LspModel`; rename refuses invalid names and collisions; code actions are a rename stub with an empty edit | Feed the model from the query database; apply `suggestions[]` in a code action; add project-wide rename refusals |
| 2.1 / 2.2 / 2.4 | Name typo, missing `move`, wrong argument order, LSP-applied fix, SARIF `fixes` | Removed-keyword suggestion end to end (JSON, `fix`, dry run, stale hash); SARIF 2.1.0 rules/levels/locations; no `fixes`, no LSP application, no missing-`move` or argument-order suggestion | Add the two suggestion producers, the code-action consumer and SARIF `fixes` |
| 3.1 / 3.2 | `--deny`, `switch`/`while`/`Result` lints, lint suggestions, generated `docs/lints.md` | `--deny <code>`, `--group`, `--docs`, the machine-applicable `var`→`let` suggestion, all six lints (including `switch`/`while`/`Result`) and a generated `docs/lints.md` compared by test | Lints are source-level advice; they do not use the checker's type information |
| 4.1 / 4.3 | Every code has a triggering test or fixture; every code names its rule | Page, heading, catalog, `explain --json` and `--all --json` are tested; `codes::rule_for` gives every code a unique `FAMILY-N` id in the JSON; triggering fixtures are not | Add one triggering fixture per code |
| 5.1 | First test under 60 s on a cold machine | `new` creates a project and its first test passes; the wall-clock bound is not asserted in the unit test | Measure in CI |
| 5.3 / 5.4 | `repl`, `doc --serve` | A compiling REPL with retained declarations, type reporting and honest refusals; a loopback read-only docs server with `/json` and `/search`; no persistent HIR interpreter | Add an HIR interpreter only if compile-per-expression is too slow |
| 6 | DWARF/CodeView debugger, async task view, pretty printers | `yashyc debug` selects LLDB or cdb, writes a symbol-breakpoint script and enables the runtime's live task table; no `!dbg` metadata, so no line table, typed locals or attached pretty printers | Emit debug metadata from HIR spans, then line breakpoints and value printers |
| 7 | Sampling and allocation profilers, source coverage | Not started; `profile` is still the 1.0 counter profiler | Build the profilers and `coverage` |
| 8.1 / 8.2 | `inspect --addr`, contracts, layout, `trace --events`, `tokens --bpe` | `inspect --json` reports contracts and spans; `tokens --bpe` measures Yashy's own tokenizer; `trace --json` emits the static rule matrix | Add `--addr` via a linker map, vendor the three BPE vocabularies, and implement runtime events |
| 9 | One generated grammar, six clients, `editors` CI job | One model drives 11 generated files; all six clients are committed; `tests/grammar.rs` is the byte gate and the LSP conformance transcript is shared; no `editors` CI job yet, and the real-editor run is deferred with CI | Add the `editors` job when CI is unblocked |
| 10.1 / 10.3 / 10.4 / 10.5 | Colour policy, `NO_COLOR`/`CLICOLOR`/`TERM=dumb`/`COLUMNS`, committed completions, pager, hyperlinks | `term.rs` implements the colour, width, pager and OSC 8 policies; `scripts/completions/*` are committed and diff-tested; `docs/terminal.md` documents the install; a signal-killed child reports `1` instead of re-raising the signal | Re-raise the child's signal on Unix |
| Gates | LSP conformance in two editors, debugger and profiler demos, terminal matrix, `editors` job | `cargo check --workspace --all-targets` is green; runtime gates cannot run locally (Windows Application Control) and CI is deferred | Re-run the full gate set when CI is unblocked |

## Out of scope

- Language forms (Phase 4); the LSP formats the 1.0 grammar and gains each new
  form in the phase that adds it.
- Signed registry and installers (Phase 3).
- The traceability matrix (Phase 9); this phase adds rule ids only.
- A GUI editor or a GUI debugger; the extensions are thin clients and
  `yashyc debug` drives an existing backend.
- Remote development over SSH or a network debug adapter; LSP and DAP speak
  over stdio, and a remote setup forwards the same streams.
- A hand-maintained second grammar. Every editor consumes the generated model;
  an editor whose format cannot consume it gets documentation, not a fork.
- A terminal UI framework or a shell; `yashyc` is a compiler, and the terminal
  work is the compiler's own output.

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
| Six editors become six grammars | One `GrammarModel`; keyword and operator tables are generated, the tree-sitter rule skeleton is written once, and CI diffs every generated file |
| Terminal behaviour is only tested by eye | Piped, TTY, `NO_COLOR`, `TERM=dumb`, `COLUMNS` and hyperlink modes are transcript tests; JSON never carries an escape |
| An editor bundles its own compiler | Every client calls `yashyc lsp` and `yashyc debug`; the extension is a protocol adapter, not a second front end |
