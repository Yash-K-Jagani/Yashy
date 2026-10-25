# Slice 3 — Diagnostics + Benchmarks

> **For agentic workers:** Read the master plan first. This slice has two
> independent halves: diagnostics and measurement. Land them as separate
> commits so a noisy benchmark cannot block a diagnostic fix.

**Status:** Draft for review, 2026-10-25.
**Source of truth:** `docs/superpowers/specs/2026-10-25-yashy-super-language-v2-design.md` §10.2, §13, and §15.3.
**Master plan:** `docs/superpowers/plans/2026-10-25-yashy-super-language-master.md` §7.3.
**Goal:** Errors teach the fix ("did you mean", notes, stable codes), agents can
read the same facts as JSON, and the benchmark job stores baselines and fails on
large regressions.
**Depends on:** Slice 1's CI; slice 2's runtime for stable timing rows.
**Estimate:** 12–15 working days. If it exceeds 15, split it into **3a**
(suggestions + benchmark gate) and **3c** (error codes + JSON + `explain` + AI
v0), and keep 3b after both.
**Decisions confirmed (2026-10-25):** benchmark artifacts are `key=value` lines
with a dependency-free comparator; the top-three performance fixes are slice 3b
before 0.2.0, not part of this slice; R1 also ships error codes, `check --json`,
`test --json`, `explain`, `AGENTS.md`, and `llms.txt`.

---

## 1. Scope

### In scope

- A suggestion helper (Damerau-Levenshtein, case-insensitive, bounded).
- "Did you mean" for unknown values, functions, types, and struct/enum fields.
- A small set of notes for common mistakes (const reassignment, missing import,
  moved-value use).
- Stable error codes (`E....`) for the common diagnostic corpus, with a catalog
  under `docs/errors/`.
- `yashy check --json` and `yashy test --json` with a versioned schema (v1).
- `yashy explain <code>` for the human form; `yashy explain --json` for the agent
  form.
- Extend `AGENTS.md` and `llms.txt` from slice 1 into the AI contract v0.
- A cross-platform benchmark runner for the R1 rows (compile time, binary size,
  runtime).
- A stored baseline per OS and a comparator with thresholds.
- A CI job that reports on pull requests and gates scheduled/manual runs.
- Documentation for updating baselines and recording exceptions.
- Selection of the top-three performance fixes from the new baseline. The fixes
  themselves are slice 3b, before 0.2.0; slice 3 records the evidence and hands
  them over.

### Not in scope

- Stack traces (R2).
- LSP completion/rename, `fix`, `index`, MCP, and sandbox (R2/R2.5).
- LSP changes; the LSP consumes the same diagnostics, so it benefits
  automatically.
- A full feature-suite gate on every PR. The full `run-features.ps1` suite is
  for a quiet machine; CI uses the small R1 rows.
- Automatic baseline commits. A maintainer updates the baseline deliberately.
- Performance parity with C. R1 measures and fixes the top three; parity is R2.
- A code for every diagnostic. R1 targets 90% of the common corpus; the rest can
  stay uncoded until R2.

---

## 2. Current state

- `Diagnostic` in `crates/yashy-common/src/diagnostic.rs` has `severity`,
  `message`, `file`, `span`, and an optional `label`. `SourceMap::render` prints
  the location, message, source line, caret, and label.
- Unknown-name diagnostics are emitted in several places:
  - `crates/yashy-sema/src/scope.rs` (`move_out`, unknown value);
  - `crates/yashy-sema/src/check_expr.rs` (unknown value, function, type,
    field);
  - `crates/yashy-sema/src/check_pat.rs` (unknown enum/type/field);
  - `crates/yashy-sema/src/const_eval.rs`.
- There is no suggestion helper and no "did you mean" anywhere.
- `benchmarks/ci.ps1` builds Yashy and C versions of `fib`, checks the output,
  and prints compile time, binary size, and run time. It is Windows-only.
- `.github/workflows/bench.yml` runs `ci.ps1` on a schedule and manual dispatch,
  uploads the text log, and does not compare against a baseline.
- `benchmarks/run.ps1` and `run-features.ps1` are the full human-driven suites.
- The README has methodology and current results but no machine-readable
  baseline.

---

## 3. Design decisions

### 3.1 Suggestion helper

Add `crates/yashy-common/src/suggest.rs`:

```rust
pub fn closest<'a>(name: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<String>;
```

- Damerau-Levenshtein distance (insert, delete, substitute, adjacent
  transpose), case-insensitive.
- Accept a suggestion only when the distance is small relative to the length:
  - length ≤ 4: distance ≤ 1;
  - length 5–8: distance ≤ 2;
  - length > 8: distance ≤ 3.
- Return the single best candidate; on a tie, prefer the one with the same
  first character, then alphabetical order. Do not return a suggestion when
  nothing is within the bound.
- Cap the candidate list at a few thousand names; the checker's tables are
  small enough today.

The helper lives in `yashy-common` so the driver's import resolver can use it
later without depending on sema.

### 3.2 Where to suggest

Start with the four highest-value cases:

| Case | Candidate set | Message |
| --- | --- | --- |
| Unknown value in scope | Bindings visible in the current scope chain | `cannot find value \`x\` in this scope` + label `did you mean \`y\`?` |
| Unknown function | Top-level functions, builtins, and imported functions | `cannot find function \`f\`` + label |
| Unknown type | Structs, enums, and built-in types | `cannot find type \`T\`` + label |
| Unknown field | The struct's or enum variant's fields | `struct \`S\` has no field \`f\`` + label |

Do not suggest for:
- an empty name;
- a name that is already an exact match in another scope (the error is about
  visibility, not spelling);
- a name with a distance above the bound.

### 3.3 Notes

Use the existing `label` for the suggestion. If a second, location-free note is
needed, add `notes: Vec<String>` to `Diagnostic`:

- `Diagnostic::new` initializes it empty.
- `SourceMap::render` prints each note as `note: <text>` after the caret line.
- Update the few struct literals (`yashy-common/tests/render.rs`,
  `yashy-codegen/src/lib.rs`, `yashy-sema/src/error.rs`,
  `yashy-driver/src/pipeline.rs`, `yashy-driver/src/modules.rs`).

Start with two notes:
- Assigning to a `const`: `note: use \`let\` for a mutable binding`.
- An unknown name that matches an imported module's exported item: `note: if
  \`x\` comes from \`mod\`, add \`import mod;\``. This is best-effort; if the
  checker does not know the module's exports at that point, skip the note
  rather than guess.

Do not add a note for every error. A note that repeats the message is noise.

### 3.4 Benchmark rows

Keep the R1 gate small and stable:

| Row | How it is measured |
| --- | --- |
| `fib_compile_ms` | `yashy build fib.ya -o fib-yashy --release`, wall clock |
| `fib_binary_bytes` | size of the Yashy `fib` executable |
| `fib_run_ms` | run the Yashy `fib` executable, wall clock |
| `hello_binary_bytes` | size of a hello-world release build |
| `hello_compile_ms` | compile time of hello-world |
| `large_compile_ms` | compile a checked-in ~1,000-line program (optional, if it is stable) |

The C comparison (`clang -O2 fib.c`) stays in the report for context but is not
a gate row; a change in clang's time is not a Yashy regression.

Use the median of 3 runs (5 on a quiet machine). One untimed warm-up. Do not
gate on a single sample.

### 3.5 Baseline format

Use a line-based `key=value` format, one file per OS. It is trivially parseable
without an external crate, diffable, and enough for a gate:

```text
# generated by benchmarks/ci.sh; update deliberately
os=linux
generated_at=2026-10-25T12:00:00Z
commit=<sha>
toolchain=clang 18.1.3
fib_compile_ms=123.4
fib_binary_bytes=24576
fib_run_ms=45.6
hello_compile_ms=80.1
hello_binary_bytes=16384
```

Store baselines in `benchmarks/baselines/windows.bench` and
`benchmarks/baselines/linux.bench`. The file is a normal reviewed artifact; a
baseline update is a commit with the reason in the message. JSON can replace
this later without changing the gate logic; R1 does not need it.

### 3.6 Comparator

Add a small Rust tool, `crates/yashy-bench` (binary, dev-only):

- `yashy-bench compare --baseline <file> --current <file> --thresholds <file>`
  prints a table of row, baseline, current, percent change, and verdict.
- Exits nonzero when a gate row exceeds its threshold.
- `yashy-bench update --current <file> --baseline <file>` writes the baseline
  after a deliberate update.
- Thresholds default to: compile +20%, binary size +10%, runtime +20%. They are
  configurable in `benchmarks/thresholds.bench`.
- Unit tests cover the threshold logic, missing rows, and the percent
  calculation.

Why Rust rather than a PowerShell/bash pair: the comparison logic is the part
that must be identical on both platforms, and a tested Rust binary is less
likely to drift than two scripts. The measurement scripts stay platform-native.

The tool has no external dependencies: it reads the `key=value` format above
with a small hand-written parser. The workspace stays offline, and the parser is
covered by unit tests.

### 3.7 CI behavior

- **Pull request:** run the benchmark only when the PR has the `bench` label.
  Report the table; do not fail the PR. Upload the measurement file.
- **Schedule and manual dispatch:** run on Ubuntu and Windows, compare against
  the stored baseline, and fail on a threshold breach. Upload the measurement
  file and the text log.
- **Main push:** do not auto-commit a baseline. A maintainer runs the
  `bench-baseline` workflow dispatch or the script locally, reviews the diff,
  and commits it.
- **Missing baseline:** report-only, with a warning. A new OS does not fail
  until a baseline exists.

The workflow stays `.github/workflows/bench.yml`; add a Linux job and keep the
Windows job. Use `ubuntu-22.04` and `windows-latest`.

### 3.8 Error codes

- Add a stable code to `Diagnostic`: `code: Option<&'static str>` (or a small
  `DiagnosticCode` enum). Codes are assigned at the emission site, never derived
  from the message text.
- Format: `E` for errors, `W` for warnings, four digits. Examples:
  - `E0001` unknown name
  - `E0002` unknown type
  - `E0100` type mismatch
  - `E0200` missing/unknown field
  - `E0300` missing `?` on a `Result`
  - `W0001` unused import
- R1 covers the common corpus: unknown names, type mismatch, missing field,
  missing `?`, const reassignment, moved value, non-exhaustive match, and wrong
  argument count. A diagnostic without a code is allowed; it must not invent one.
- `docs/errors/<code>.md` contains the cause, a minimal example, and the fix.
- `yashy explain <code>` reads the same catalog; the human and JSON forms share
  the facts.
- Tests assert the code and span, and assert that a code is not reused for a
  different meaning.

### 3.9 JSON schema v1

`yashy check --json` returns:

```json
{
  "schema_version": 1,
  "tool": "check",
  "yashy_version": "0.2.0",
  "success": false,
  "diagnostics": [
    {
      "code": "E0100",
      "severity": "error",
      "message": "expected `int`, found `string`",
      "file": "src/main.ya",
      "span": { "start": 42, "end": 47, "line": 3, "column": 12 },
      "labels": [
        { "span": { "start": 42, "end": 47 }, "message": "expected `int`" }
      ],
      "notes": ["numeric operations require `int` or `float`"],
      "fix": {
        "description": "replace with an integer",
        "edits": [
          {
            "file": "src/main.ya",
            "span": { "start": 42, "end": 47 },
            "replacement": "42"
          }
        ]
      },
      "docs_url": "https://yashy.dev/errors/E0100"
    }
  ],
  "summary": { "errors": 1, "warnings": 0 }
}
```

`yashy test --json` adds:

```json
{
  "tests": [
    {
      "name": "smoke",
      "file": "tests/smoke.ya",
      "status": "passed",
      "duration_ms": 12,
      "diagnostics": []
    }
  ],
  "summary": { "passed": 1, "failed": 0, "skipped": 0 }
}
```

Rules:

- `--json` implies no color and no interactive prompt.
- Exit codes: `0` success, `1` diagnostics/tests failed, `2` usage/config error,
  `3` internal error.
- The schema is v1 for R1; additive changes only within 0.x; frozen at 1.0.
- `labels`, `notes`, and `fix` are optional and omitted when empty. `docs_url` is
  present only when `code` is present.
- `yashy test --json` uses the same envelope as `check --json`, plus `tests`.
- Tests snapshot the shape and the facts, not the message text. Human and JSON
  output must agree on code, severity, file, span, and summary.
- Use a small JSON writer helper in `yashy-common`; do not hand-roll escaping in
  each command.

### 3.10 AI v0 docs

- Extend `AGENTS.md` (created in slice 1): build/test/run commands, project
  layout, style, error handling, package rules (no build scripts), sandbox rules,
  and docs links.
- Extend `llms.txt` (created in slice 1): a compact canonical map of the
  quickstart, tutorial, CLI reference, and error catalog.
- `docs/errors/`: one page per code, with a tested example.
- Keep it short and test the examples. R2.5 adds the semantic index, context
  packs, and MCP; R1 is only the stable facts.

---

## 4. Tasks

### Task 1 — Suggestion helper

- [ ] Add `crates/yashy-common/src/suggest.rs` with `closest` and tests:
  - exact match returns `None` (the caller already handles it);
  - one substitution, insertion, deletion, and adjacent transpose;
  - case-insensitive match;
  - length-dependent bounds;
  - ties prefer the same first character, then alphabetical;
  - an unrelated name returns `None`.
- [ ] Export it from `yashy-common`.
- [ ] Run the tests.

### Task 2 — Unknown value and function suggestions

- [ ] Add a `names()` or `suggest(name)` method to `Scope` that exposes the
  visible binding names without leaking the map.
- [ ] Wire `Scope`'s unknown-value path and the `check_expr.rs` unknown-value
  paths to `closest`.
- [ ] Wire the unknown-function path to the function/builtin table.
- [ ] Add sema tests:
  - `let count = 1; print(cuont);` suggests `count`;
  - `prntln("x")` suggests `println`;
  - a genuinely unrelated name gets no suggestion;
  - a name in an outer scope is found, not suggested.
- [ ] Add a driver test that the rendered diagnostic contains
  `did you mean` and the candidate.

### Task 3 — Type and field suggestions

- [ ] Wire unknown-type errors to the struct/enum/builtin type names.
- [ ] Wire unknown-field errors (struct fields, enum variant fields) to the
  type's fields.
- [ ] Add tests for each case, including a field that exists on a different
  struct (no suggestion if the distance is too large).
- [ ] Keep the existing messages; add the suggestion as a label.

### Task 4 — Notes

- [ ] Decide whether a second note line is needed. If yes, add
  `notes: Vec<String>` to `Diagnostic` and update the struct literals and
  `SourceMap::render`.
- [ ] Add the `const` reassignment note.
- [ ] Add the missing-import note only if the checker can name the module
  reliably; otherwise skip it and record why.
- [ ] Add render tests for one note and for no notes.
- [ ] Do not duplicate the message in a note.

### Task 5 — Cross-platform measurement

- [ ] Refactor `benchmarks/ci.ps1` so the measurement functions are reusable
  and it can emit the `key=value` format (`-Output <file>`).
- [ ] Add `benchmarks/ci.sh` for Linux with the same rows and format.
- [ ] Add a hello-world benchmark program and a ~1,000-line compile-time
  program if the latter is stable enough to gate.
- [ ] Run both scripts locally and compare the output shape.
- [ ] Keep the C comparison in the report; mark it non-gating.

### Task 6 — Comparator

- [ ] Add `crates/yashy-bench` with `compare` and `update` subcommands.
- [ ] Add `benchmarks/thresholds.bench`.
- [ ] Unit-test the comparator:
  - within threshold passes;
  - a compile regression fails;
  - a binary-size regression fails;
  - a runtime regression fails;
  - a missing row is reported, not silently ignored;
  - a negative or zero baseline is handled without division by zero.
- [ ] Add the crate to the workspace and to `cargo check --workspace
  --all-targets`.

### Task 7 — Baselines and CI

- [ ] Add `benchmarks/baselines/windows.bench` and `linux.bench` from a clean run
  on each OS. Record the toolchain and commit in the file.
- [ ] Update `.github/workflows/bench.yml`:
  - Linux and Windows jobs;
  - schedule, manual dispatch, and PR-with-`bench`-label;
  - compare step with the comparator;
  - upload the measurement file and text log;
  - fail on a threshold breach for scheduled/manual runs, report-only on PRs.
- [ ] Add a `bench-baseline` manual workflow (or document the local command) to
  regenerate a baseline.
- [ ] Document the update process in `benchmarks/README.md`: run on a quiet
  machine, review the diff, commit with the reason.

### Task 8 — Top-three performance fixes

- [ ] Run the new baseline and the existing feature suite; list the three
  largest gaps that are codegen/runtime issues, not measurement noise.
- [ ] Choose the three and record the evidence in this plan. Candidates from
  `docs/performance-improvements.md`: checked-arithmetic elision (the fib
  checked row is ~1.8×), map/array paths, string/debug formatting, interned
  identifiers, iterative parser, one IR buffer.
- [ ] Do not implement the fixes in this slice. Hand the three, with their
  baseline numbers and a suggested order, to slice 3b.
- [ ] Record the slice 3b start condition in the master plan: diagnostics and
  the gate are green, and the three fixes are chosen.
- [ ] Do not claim a speedup without a before/after measurement.

### Task 9 — Docs

- [ ] Update `benchmarks/README.md` with the R1 gate rows, baseline format,
  thresholds, and the update process.
- [ ] Update the spec's R1 performance bullet to point at the baseline files.
- [ ] Add a short "diagnostics" section to the tutorial or README showing a
  "did you mean" example.
- [ ] Update the master plan's gap table.

### Task 10 — Error codes and catalog

- [ ] Add `code: Option<&'static str>` to `Diagnostic` (or a small
  `DiagnosticCode` enum) and set it at the emission site.
- [ ] Add `crates/yashy-common/src/codes.rs` with the R1 codes and their
  one-line meanings.
- [ ] Assign codes to the common corpus: unknown name/type, type mismatch,
  missing/unknown field, missing `?`, const reassignment, moved value,
  non-exhaustive match, wrong argument count.
- [ ] Add `docs/errors/<code>.md` with cause, example, and fix.
- [ ] Tests: code and span are present for the corpus; codes are not reused;
  an uncoded diagnostic is allowed but counted.
- [ ] Do not derive a code from the message text.

### Task 11 — JSON diagnostics for `check` and `test`

- [ ] Add a small JSON writer helper in `yashy-common` (no new dependency).
- [ ] Add `yashy check --json` with `schema_version`, `tool`, `success`,
  `diagnostics`, and `summary`.
- [ ] Add `yashy test --json` with `tests` and `summary`, reusing the diagnostic
  object.
- [ ] `--json` implies no color and no prompt; exit codes are `0/1/2/3`.
- [ ] Tests: schema shape snapshot; human and JSON facts agree on code,
  severity, file, span, and summary; a usage error returns `2`; an internal error
  returns `3`.
- [ ] Keep the human output unchanged except for the added code.

### Task 12 — `yashy explain`

- [ ] Add `yashy explain <code>`; unknown codes exit `2` with the nearest known
  code if one is close.
- [ ] Add `yashy explain --json` with the catalog entry and `docs_url`.
- [ ] Tests: every code in `codes.rs` has a catalog page; a known code prints the
  cause/example/fix; an unknown code fails cleanly.
- [ ] Link the code in the human diagnostic to the same catalog page.

### Task 13 — AI contract v0

- [ ] Extend `AGENTS.md` with error catalog links, JSON/`explain` commands, and
  the diagnostic workflow.
- [ ] Extend `llms.txt` with the error catalog, JSON schema v1, and `explain`.
- [ ] Add tests that the commands in `AGENTS.md` run and every link in
  `llms.txt` resolves.
- [ ] Do not add the semantic index, context packs, MCP, or sandbox in this
  slice; those are R2.5.

---

## 5. Verification

```text
cargo test -p yashy-common
cargo test -p yashy-sema
cargo test -p yashy-bench
cargo test -p yashy-driver --test e2e
cargo test -p yashy-driver --test diagnostics_json
cargo test -p yashy-driver --test explain
cargo run -p yashy-driver -- check --json examples/hello
cargo run -p yashy-driver -- test --json
cargo run -p yashy-driver -- explain E0100
cargo check --workspace --all-targets
```

Benchmark smoke:

```text
# Windows
pwsh benchmarks/ci.ps1 -Output benchmarks/build/current-windows.bench
cargo run -p yashy-bench -- compare `
  --baseline benchmarks/baselines/windows.bench `
  --current benchmarks/build/current-windows.bench

# Linux
sh benchmarks/ci.sh benchmarks/build/current-linux.bench
cargo run -p yashy-bench -- compare \
  --baseline benchmarks/baselines/linux.bench \
  --current benchmarks/build/current-linux.bench
```

The CI job must be observed on both OSes: one green run, then a deliberate
regression (e.g., force `-O0` in a branch) to prove the comparator fails.

---

## 6. Acceptance criteria

- A misspelled local, function, type, or field gets a "did you mean" label with
  the right candidate; an unrelated name gets none.
- The `const` reassignment error explains `let`.
- At least 90% of the common error corpus has a stable code and a span; the code
  is present in both human and JSON output.
- `yashy check --json` and `yashy test --json` parse, match the human facts, and
  use exit codes `0/1/2/3` correctly.
- `yashy explain <code>` works for every code in the catalog; an unknown code
  exits `2` cleanly.
- `AGENTS.md` and `llms.txt` exist; their commands run and their links resolve.
- The benchmark job reports compile time, binary size, and runtime on Windows
  and Linux, and stores a machine-readable `key=value` artifact.
- The comparator fails a deliberate regression and passes a normal run.
- A baseline update is a reviewed commit with a recorded reason.
- The top three performance gaps are chosen from measured data and handed to
  slice 3b; slice 3b is scheduled before 0.2.0.
- No diagnostic regresses its existing message or span.

---

## 7. Risks and rollback

| Risk | Mitigation |
| --- | --- |
| Suggestions are noisy or wrong | Length-dependent bounds; top-one only; tests for unrelated names; no suggestion when an exact match exists elsewhere. |
| Adding `notes` touches many literals | Use the existing `label` first; add `notes` only if a second line is truly needed. |
| Benchmark noise fails a good build | Median of 3–5; generous thresholds; report-only on PRs; per-OS baselines; gate only scheduled/manual. |
| CI time grows | Keep the gate to fib + hello; leave the full feature suite for a quiet machine. |
| Baselines go stale | Record the commit and toolchain; update deliberately; a missing baseline is report-only. |
| A performance fix changes behavior | Differential tests and the full suite; measure before/after on the same machine. |

**Rollback:** diagnostics are additive labels; revert a suggestion without
touching the message. The benchmark gate can be switched to report-only by
removing the failing step; keep the measurement artifact and the baseline.

---

## 8. Files touched

- `crates/yashy-common/src/suggest.rs` (new)
- `crates/yashy-common/src/codes.rs` (new)
- `crates/yashy-common/src/json.rs` (new)
- `crates/yashy-common/src/diagnostic.rs` (code field; notes if added)
- `crates/yashy-common/src/lib.rs`
- `crates/yashy-common/src/source.rs` (only if notes are added)
- `crates/yashy-common/tests/render.rs` (only if notes are added)
- `crates/yashy-sema/src/scope.rs`
- `crates/yashy-sema/src/check_expr.rs`
- `crates/yashy-sema/src/check_pat.rs`
- `crates/yashy-sema/src/const_eval.rs`
- `crates/yashy-sema/src/error.rs` (only if notes are added)
- `crates/yashy-driver/src/main.rs` (`check --json`, `test --json`, `explain`)
- `crates/yashy-driver/tests/diagnostics_json.rs` (new)
- `crates/yashy-driver/tests/explain.rs` (new)
- `crates/yashy-bench/` (new crate)
- `Cargo.toml` (workspace member)
- `benchmarks/ci.ps1`
- `benchmarks/ci.sh` (new)
- `benchmarks/thresholds.bench` (new)
- `benchmarks/baselines/windows.bench` (new)
- `benchmarks/baselines/linux.bench` (new)
- `benchmarks/README.md`
- `.github/workflows/bench.yml`
- `docs/errors/` (new)
- `AGENTS.md` (extend; created in slice 1)
- `llms.txt` (extend; created in slice 1)
- `docs/superpowers/specs/2026-10-25-yashy-super-language-v2-design.md`
- `README.md`

---

## 9. Definition of done

- The verification commands in §5 pass.
- The CI benchmark has a green run on Ubuntu and Windows, and a proven
  regression failure.
- The baseline update process is documented and exercised once.
- Error codes cover at least 90% of the common corpus; `check --json` and
  `test --json` match the human facts; `explain` works for every code.
- `AGENTS.md` and `llms.txt` exist and are tested.
- The top-three performance list is recorded with evidence and handed to slice
  3b; the slice 3b start condition is recorded in the master plan.
- The master plan's gap table marks slice 3 complete.
- Commit and push; record the commit hashes in this plan.
