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

- [x] Refactor `benchmarks/ci.ps1` so the measurement functions are reusable
  and it can emit the `key=value` format (`-Output <file>`).
- [x] Add `benchmarks/ci.sh` for Linux with the same rows and format.
- [x] Add a hello-world benchmark program and a ~1,000-line compile-time
  program if the latter is stable enough to gate.
- [x] Run both scripts locally and compare the output shape.
- [x] Keep the C comparison in the report; mark it non-gating.

Measured 2026-10-26: Windows `fib_compile_ms=348.3`, `hello_compile_ms=353.0`,
`large_compile_ms=1683.1`, `fib_run_ms=176.0`, `fib_binary_bytes=57344`,
`hello_binary_bytes=56320`. Linux (WSL) `fib_compile_ms=494.9`,
`hello_compile_ms=513.1`, `large_compile_ms=2547.4`, `fib_run_ms=140.2`,
`fib_binary_bytes=48000`, `hello_binary_bytes=47968`. Two Linux runs spread
+17% on `fib_compile_ms` (the `/mnt/c` filesystem), +2% on hello and +3% on
large, so `large_compile_ms` is stable enough to gate. The Windows local run
has no C row because this host's clang has no MSVC CRT headers; the script
treats the C comparison as context and CI's MSVC environment supplies it.

### Task 6 — Comparator

- [x] Add `crates/yashy-bench` with `compare` and `update` subcommands.
- [x] Add `benchmarks/thresholds.bench`.
- [x] Unit-test the comparator:
  - within threshold passes;
  - a compile regression fails;
  - a binary-size regression fails;
  - a runtime regression fails;
  - a missing row is reported, not silently ignored;
  - a negative or zero baseline is handled without division by zero.
- [x] Add the crate to the workspace and to `cargo check --workspace
  --all-targets`.

Verified 2026-10-26: `cargo test -p yashy-bench` runs 16 tests, all passing.
`compare` on the Linux measurement is all-pass; a copy with
`fib_compile_ms=900.0` (+81.9%) fails with exit 1, `--report-only` exits 0, and
a usage error exits 2. The thresholds file defaults to `compile=20`,
`binary=10`, `runtime=20`, with a `large_compile_ms=25` override.

### Task 7 — Baselines and CI

- [x] Add `benchmarks/baselines/windows.bench` and `linux.bench` from a clean run
  on each OS. Record the toolchain and commit in the file.
- [x] Update `.github/workflows/bench.yml`:
  - Linux and Windows jobs;
  - schedule, manual dispatch, and PR-with-`bench`-label;
  - compare step with the comparator;
  - upload the measurement file and text log;
  - fail on a threshold breach for scheduled/manual runs, report-only on PRs.
- [x] Add a `bench-baseline` manual workflow (or document the local command) to
  regenerate a baseline.
- [x] Document the update process in `benchmarks/README.md`: run on a quiet
  machine, review the diff, commit with the reason.

Seeded 2026-10-26: the Linux baseline is an ext4 run (clang 18.1.8,
`fib_compile_ms=327.2`, `large_compile_ms=2319.4`), the Windows baseline a local
MSVC-free run (clang 23.1.4, `fib_compile_ms=348.3`, `large_compile_ms=1683.1`),
both at commit `fec9b8c`. The comparator passes both and fails a +127% `fib_run_ms`
copy with exit 1. The manual **Bench** dispatch takes *Measure only and upload a
candidate baseline*; the artifact is reviewed and committed, and the README
carries the local `update` command. If a runner's first measurement lands
outside a threshold for an environmental reason, the candidate run replaces the
seeded baseline rather than the gate being widened.

### Task 8 — Top-three performance fixes

- [x] Run the new baseline and the existing feature suite; list the three
  largest gaps that are codegen/runtime issues, not measurement noise.
- [x] Choose the three and record the evidence in this plan. Candidates from
  `docs/performance-improvements.md`: checked-arithmetic elision (the fib
  checked row is ~1.8×), map/array paths, string/debug formatting, interned
  identifiers, iterative parser, one IR buffer.
- [x] Do not implement the fixes in this slice. Hand the three, with their
  baseline numbers and a suggested order, to slice 3b.
- [x] Record the slice 3b start condition in the master plan: diagnostics and
  the gate are green, and the three fixes are chosen.
- [x] Do not claim a speedup without a before/after measurement.

Measured 2026-10-26 with a fresh `run-features.ps1 -Runs 3 -Passes 1` on
Windows (GNU Rust toolchain; this host has no MSVC linker), cross-checked with
the published 9 × 3 table in `benchmarks/README.md`:

| Row | Published | Fresh | Ratio | Chosen |
|---|---|---:|---:|---|
| Array mapping (`map_array`) | 417.1 vs C gcc 110.8 | 445.0 vs 122.8 | 3.76× / 3.62× | yes |
| Debug formatting (`debug_format`) | 1753.6 vs Rust 513.4 | 1931.3 vs 583.4 | 3.42× / 3.31× | yes |
| Tuples (`tuple`) | 133.6 vs gcc 74.7 | 142.1 vs 93.3 | 1.79× / 1.52× | yes |
| Map walking (`map_scan`) | 1065.4 vs Rust 567.3 | 1119.1 vs 625.2 | 1.88× / 1.79× | no — copy by design |
| Fixed-array checked `+` | 192.8 vs clang 16.9 | 195.6 vs 22.2 | 11.4× / 8.8× | no — contract |

The three largest ratios are map_array, debug and map_scan; `map_scan` copies
the map's keys and values into caller-owned arrays, which the README records as
the shape the language chose, not a codegen defect. The third fixable row is
therefore tuples. The checked add's gap is the overflow test (and the reason
`wrapping_*` exists); it is a language-contract item, not one of 3b's three.

Suggested order: array mapping, debug formatting, tuples. Each needs a design
note, before/after numbers on Windows and Linux, and no regression in another
row. No fix is implemented in slice 3.

### Task 9 — Docs

- [x] Update `benchmarks/README.md` with the R1 gate rows, baseline format,
  thresholds, and the update process.
- [x] Update the spec's R1 performance bullet to point at the baseline files.
- [x] Add a short "diagnostics" section to the tutorial or README showing a
  "did you mean" example.
- [x] Update the master plan's gap table.

The README's gate section landed with Task 7 (`abd9575`). This task adds the
spec's `hello_compile_ms`, `hello_binary_bytes` and
`benchmarks/baselines/<os>.bench` pointers, and the tutorial's "Diagnostics that
teach" section with the exact `did you mean` block. The master plan's gate row
is Done (committed in `0772658`). Verified 2026-10-26:
`cargo test -p yashy-driver --test tutorial` → 2 passed.

### Task 10 — Error codes and catalog

- [x] Add `code: Option<&'static str>` to `Diagnostic` (or a small
  `DiagnosticCode` enum) and set it at the emission site.
- [x] Add `crates/yashy-common/src/codes.rs` with the R1 codes and their
  one-line meanings.
- [x] Assign codes to the common corpus: unknown name/type, type mismatch,
  missing/unknown field, missing `?`, const reassignment, moved value,
  non-exhaustive match, wrong argument count.
- [x] Add `docs/errors/<code>.md` with cause, example, and fix.
- [x] Tests: code and span are present for the corpus; codes are not reused;
  an uncoded diagnostic is allowed but counted.
- [x] Do not derive a code from the message text.

Implemented 2026-10-26 (commit `b5fcb11`).

- `Diagnostic` and `TypeError` carry `code: Option<&'static str>`, set where the
  diagnostic is built; `DiagnosticBag::uncoded_count` counts the rest.
- `codes.rs` holds the §3.8 scheme: `E0001` unknown name, `E0002` unknown
  type, `E0100` type mismatch, `E0200` unknown field, `E0201` missing field,
  `E0300` missing `?`, `E0400` const reassignment, `E0500` moved value,
  `E0600` non-exhaustive match, and `E0700` wrong argument count. Each entry
  has a title, a one-line summary, and an embedded `docs/errors/<code>.md`
  page (cause, example, fix). The page is embedded with `include_str!`, so
  `yashy explain` will work offline.
- Human output is `error[CODE]: ...` when coded and the old `error: ...` when
  not. `Diagnostic::docs_url` and `codes::docs_url` point at
  `https://yashy.dev/errors/<code>`.
- The corpus is `crates/yashy-sema/tests/codes.rs`: ten programs covering
  unknown name/type, scalar mismatch, unknown/missing field, wrong argument
  count, moved value, const reassignment, non-exhaustive switch, and missing
  `?`. All ten have a code and a real span; the uncoded duplicate-definition
  case is counted rather than treated as coded.
- The 90% figure is scoped to that corpus (10/10). Parse errors, visibility,
  operator mismatch, and linear-copy errors still have no code; they are
  allowed and counted, not silently treated as complete.
- Verified 2026-10-26: `cargo test -p yashy-common -p yashy-sema` (25 common,
  181 sema, 11 corpus), `cargo test -p yashy-driver --test e2e` (115 passed),
  `cargo test -p yashy-driver --test docs` (3 passed),
  `cargo test -p yashy-driver --test tutorial` (2 passed), and the full
  `cargo test --workspace --no-fail-fast` (98 result lines, all ok).

### Task 11 — JSON diagnostics for `check` and `test`

- [x] Add a small JSON writer helper in `yashy-common` (no new dependency).
- [x] Add `yashy check --json` with `schema_version`, `tool`, `success`,
  `diagnostics`, and `summary`.
- [x] Add `yashy test --json` with `tests` and `summary`, reusing the diagnostic
  object.
- [x] `--json` implies no color and no prompt; exit codes are `0/1/2/3`.
- [x] Tests: schema shape snapshot; human and JSON facts agree on code,
  severity, file, span, and summary; a usage error returns `2`; an internal error
  returns `3`.
- [x] Keep the human output unchanged except for the added code.

Implemented 2026-10-26 (commit `cde33d8`).

- `crates/yashy-common/src/json.rs` is a dependency-free `Json` value tree with
  a pretty printer and centralized string escaping; it has eight unit tests.
  `EXIT_USAGE = 2` and `EXIT_INTERNAL = 3` live beside the existing exit codes.
- `check --json` prints the v1 envelope on stdout: `schema_version`, `tool`,
  `yashy_version`, `success`, `diagnostics`, and `summary {errors, warnings}`.
  A diagnostic has `code` (null when uncoded), `severity`, `message`, `file`,
  `span {start, end, line, column}`, optional `labels`, optional `notes`, and
  `docs_url` only when coded. `fix` is not emitted in R1.
- `test --json` uses the same envelope plus `tests` and a test summary
  (`passed`, `failed`, `skipped`). Compile diagnostics are nested under the
  test that produced them; top-level `diagnostics` is reserved for setup errors
  such as a missing toolchain. A runtime failure gets a synthetic diagnostic
  with the exit status. `--list` and `--json` are mutually exclusive.
- Exit codes: `usage_error` and a missing `check` source now return `2`; a
  missing explicit test root and an unreadable test root return `2`; a test
  temporary directory that cannot be created returns `3`. The human `test`
  progress lines, the human `check` renderer, and the human diagnostics are
  otherwise unchanged. The two e2e tests that asserted `1` for a usage error
  now assert `2`.
- `crates/yashy-driver/tests/diagnostics_json.rs` parses the output with a tiny
  local JSON reader and covers the schema and facts, human/JSON agreement,
  success, mixed test results, usage exit `2`, and internal exit `3`.
- `docs/cli.md` documents the shared exit codes, both `--json` commands, and the
  schema v1 diagnostic and test objects.

Verified 2026-10-26: `cargo test -p yashy-common` (31 unit, 25 integration),
`cargo test -p yashy-driver --test diagnostics_json` (8 passed),
`cargo test -p yashy-driver --test e2e usage_error` (2 passed),
`cargo test -p yashy-driver --test test_runner` (5 passed),
`cargo test -p yashy-driver --test docs` (3 passed),
`cargo check --workspace --all-targets` (clean, GNU Windows), and the full
`cargo test --workspace --no-fail-fast` (99 result lines, all ok, WSL Ubuntu).

### Task 12 — `yashy explain`

- [x] Add `yashy explain <code>`; unknown codes exit `2` with the nearest known
  code if one is close.
- [x] Add `yashy explain --json` with the catalog entry and `docs_url`.
- [x] Tests: every code in `codes.rs` has a catalog page; a known code prints the
  cause/example/fix; an unknown code fails cleanly.
- [x] Link the code in the human diagnostic to the same catalog page.

Implemented 2026-10-26 (commit `58b8522`).

- `yashy explain <code> [--json]` reads `codes::CODES`. The human form prints the
  embedded Markdown page verbatim; `--json` emits `schema_version`, `tool`,
  `yashy_version`, `success`, `code`, `title`, `summary`, `page`, and
  `docs_url`. The two forms share one entry, so their facts cannot drift.
- An unknown code exits `2`. When `codes::closest` finds a near miss, stderr
  says ``unknown error code `E010`; did you mean `E0100`?``; an unrelated string
  gets the same message without a suggestion. `--json` with an unknown code
  still writes the human message to stderr, matching the usage-error behavior of
  `check --json` and `test --json`.
- A coded human diagnostic now ends with `  docs: https://yashy.dev/errors/<code>`
  after any notes. An uncoded diagnostic prints no docs line. The missing-source
  fallback keeps the docs line.
- `crates/yashy-driver/tests/explain.rs` (12 tests) covers every catalog code,
  the human page, the JSON envelope, shared facts, near and unrelated unknown
  codes, missing/extra/unknown arguments, and the coded/uncoded human link.
  `crates/yashy-common/tests/render.rs` (27 tests) pins the docs line, its place
  after notes, and the uncoded case. `e2e.rs` (117 tests) adds the coded docs
  link and two top-level usage tests.
- Also closes Task 11's top-level usage gap: no command and an unknown command
  now exit `2` (`EXIT_USAGE`) instead of `1`.
- Docs: `docs/cli.md` gains the explain command, its schema, and the human
  diagnostic link; `docs/errors.md`, all ten `docs/errors/<code>.md` pages, and
  `docs/tutorial/06-errors.md` show the docs line and name `yashy explain`.
- Ruling: the human link is a final indented `docs:` line, not an OSC 8 terminal
  hyperlink - it survives pipes and logs and is testable in plain output - cost
  if wrong: one extra line on every coded diagnostic.
- Ruling: `explain --json` has its own envelope instead of the diagnostics
  envelope - the command returns a catalog entry, not a compile result - cost if
  wrong: one more documented schema for an agent to learn.

Verified 2026-10-26: `cargo test -p yashy-common --test render` (27 passed),
`cargo test -p yashy-driver --test explain` (12 passed),
`cargo test -p yashy-driver --test e2e` (117 passed),
`cargo test -p yashy-driver --test diagnostics_json` (8 passed),
`cargo test -p yashy-driver --test test_runner` (5 passed),
`cargo test -p yashy-driver --test docs` (3 passed),
`cargo check --workspace --all-targets` (clean, GNU Windows), and the full
`cargo test --workspace --no-fail-fast` in WSL Ubuntu (100 result lines, all ok).
`cargo test -p yashy-common` cannot run its unit-test binary on this Windows host
because Application Control blocks the freshly linked image; the same suite runs
in WSL.

### Task 13 — AI contract v0

- [x] Extend `AGENTS.md` with error catalog links, JSON/`explain` commands, and
  the diagnostic workflow.
- [x] Extend `llms.txt` with the error catalog, JSON schema v1, and `explain`.
- [x] Add tests that the commands in `AGENTS.md` run and every link in
  `llms.txt` resolves.
- [x] Do not add the semantic index, context packs, MCP, or sandbox in this
  slice; those are R2.5.

Implemented 2026-10-26 (commit `4644ec7`).

- `AGENTS.md` gains the `explain`, `diagnostics_json`, `docs`, and `agents` test
  targets plus a "Diagnostics and JSON" section: the human `docs:` link, four
  runnable CLI commands, schema v1 fields, the four exit codes, and a three-step
  error workflow. Its links point at `docs/cli.md#json-diagnostics-schema-v1`,
  `docs/errors.md`, and `docs/errors/E0100.md`.
- `llms.txt` names diagnostic schema v1 and `yashy explain`, adds a Diagnostics
  section with the same commands, and keeps every local link resolvable.
- `crates/yashy-driver/tests/agents.rs` (2 tests) parses the `## Commands` block
  and asserts every `cargo test -p yashy-driver --test <name>` target exists,
  then parses the `## Diagnostics and JSON` block and runs all four CLI commands
  from the repository root. The checks cover schema v1, success, a clean
  `check`, the test list, and the human page. Cargo commands that would recurse
  are validated by target rather than executed; the test doc comment records
  that trade-off.
- The existing `docs.rs` tests `llms_links_resolve` and
  `repo_agents_links_resolve` cover the link requirement.
- No semantic index, context packs, MCP, or sandbox was added; those remain
  R2.5.

Verified 2026-10-26: `cargo test -p yashy-driver --test docs` (3 passed, GNU
Windows), `cargo test -p yashy-driver --test agents --test docs` in WSL Ubuntu
(2 and 3 passed), `cargo check --workspace --all-targets` (clean, GNU Windows),
and the full `cargo test --workspace --no-fail-fast` in WSL Ubuntu (101 result
lines, all ok).

---

## 5. Verification

```text
cargo test -p yashy-common
cargo test -p yashy-sema
cargo test -p yashy-bench
cargo test -p yashy-driver --test e2e
cargo test -p yashy-driver --test diagnostics_json
cargo test -p yashy-driver --test explain
cargo run -p yashy-driver -- check --json examples/hello/main.ya
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
