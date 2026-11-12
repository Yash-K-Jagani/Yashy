# Triage: P0 / P1 / P2 definitions and the empty-list criterion

Every open issue, bug report, or failed gate is classified P0, P1 or P2.
The classification is a decision, not a vibe: it names the harm, the fix,
and (for a P0) the test that proves the fix.

## Severity definitions

### P0 — ship-stopping

A P0 is anything that:

- produces a **wrong binary** (the program compiles but runs incorrectly),
- **panics on valid input** (a `never_panics` violation),
- is a **security escape** (a sandbox escape, a traversal through the MCP
  server, a `cfg` key misspelling that silently deletes code), or
- **breaks a gate that was green** (a regression on a previously-passing
  gate).

"No P0" means the P0 section of this list is empty **and** the crash corpus
(`never_panics`, `sandbox_escape`, `repro-check`) is green. A P0 is not
closed by relabelling it; it is closed by a fix plus a test, or by an
explicit, dated decision that the behaviour is intended.

### P1 — important, not ship-stopping

A P1 degrades the developer surface but does not produce wrong binaries,
panics, or escapes: a misleading diagnostic, a slow gate within its budget
but trending up, a missing page, a clunky error message, a test gap that
does not hide a real bug.

### P2 — polish

A P2 is cosmetic or optional: wording, formatting drift that does not break
CI, a nicer `--json` shape, a benchmark row that is not regressed.

## The triage list

No P0 and no P1 was open at the end of Task 9. Two findings are open now,
T2 and T3, found while repairing CI and measuring the release note for the
1.0.0 tag and recorded here rather than folded into the release pass. T1
was closed by the Task 10 release pass, which touched the same files
anyway.

| ID | Severity | Issue | Fix | Test | Status |
|---|---|---|---|---|---|
| T2 | P2 | Doc-block attachment is line-ending-sensitive. `skip_line_comment` (`crates/yashy-lexer/src/lexer.rs`) stops only at `\n`, so a comment token on a CRLF file swallows the trailing `\r`: the doc text carries `\r` into `yashy doc` output (the page is not byte-stable across OSes), and the blank-line test in `collect_doc_lines` (`crates/yashy-parser/src/items.rs`) never matches `\r\n\r\n`, because the gap it sees is `\n\r\n` - so `/// first` blank `/// second` attaches both blocks on a CRLF checkout and only the last on LF. Silent in this repository only because `.gitattributes` pins the stdlib sources to LF; a user's CRLF file still diverges. | stop comment scanning at `\r\n` as well, which makes the parser's existing `gap.contains("\r\n\r\n")` check work as written | a parser test that feeds the same source with LF and CRLF endings and asserts one doc block and one `yashy doc` byte stream; not yet written | open |
| T3 | P1 | `yashy test` misses the 100-test budget (`< 2 s`) by more than 600x: 101 tests take 1235 s (measured 2026-11-12, Windows, release build), about 12 s each, serially. Per-test `--timings` attributes 0 ms to load, check, verify and emit and 2512 ms to link; the remaining ~10 s is unattributed host-side per-process cost on that machine (first launch of each freshly built test binary 4 s, second 0.05 s; `clang --version` 1.2 s), so the measurement mixes compiler time with process-scan overhead - but even with the host cost removed the budget is missed by two orders of magnitude. | measure the same fixture on a Linux runner to split host cost from compiler cost, then run test programs concurrently or reuse one launch per binary instead of one per test | a committed y100 fixture recipe and a per-OS timing record; not yet written | open |

### Closed findings

| ID | Severity | Issue | Fix | Test |
|---|---|---|---|---|
| G1 | P0 | `cargo check --workspace --all-targets` failed at HEAD with `E0061`: `8ff1644` added `extra_flags` and `epoch` to `cached_runtime_object` and updated the five production callers, but not the test at `pipeline.rs:2367`. The `check` gate — and therefore the `tests` gate, which compiles the same target — was red on main. | added `&[]` and `None` to the test call site (`crates/yashy-driver/src/pipeline.rs`), commit `6da0af1` | `cargo check --workspace --all-targets` → exit 0; found by `scripts/gates.ps1`'s `check` gate |
| T1 | P2 | Live files still name the old repository: `REPO="Yash-K-Jagani/Otter"` in `scripts/install.sh`, two URLs in `scripts/install.ps1`, the `curl` lines in `README.md` and `docs/quickstart.md`, the installer line in `docs/release-notes-template.md`, and the four doc links in the `yashy init` AGENTS.md template (`crates/yashy-driver/src/main.rs`). They worked only through GitHub's rename redirect; the rename design left the remote URL out of scope (§11) while its own risk check says the `Otter` search must come back empty outside dated records. | swapped every live occurrence to `Yash-K-Jagani/Yashy` (the canonical remote) in the Task 10 release pass | `git grep Otter` over live files returns only README's dated "Formerly Otter" note and CHANGELOG history entries, both of which name the old language, not the remote; the install matrix at the `v1.0.0` tag fetches the download URL directly (Task 10 step 2) |

The first run of the runner is what found G1, which is the argument for
running it nightly: a `--all-targets` compile error sat under a green tree
because the everyday commands (`cargo build`, targeted `cargo test`) do not
compile every test target.

### How to close a P0

1. Reproduce it minimally (a single `.ya` file or a single `cargo test`
   filter).
2. Fix at the narrowest responsible layer — surgical patch, not rewrite.
3. Add a test that fails before the fix and passes after it.
4. Run the affected gate(s) and the full crash corpus.
5. Remove the entry from this list and record the fix commit.

## The crash corpus

These suites must be green before "no P0" can be claimed:

- `cargo test -p yashy-driver --test never_panics` — malformed input yields
  diagnostics, never a Rust panic.
- `cargo test -p yashy-driver --test sandbox_escape` — the MCP/server
  request corpus: every escape attempt denied with the right code, both
  OSes.
- `scripts/repro-check.sh` / `scripts/repro-check.ps1` — two independent
  builds produce byte-identical artifacts.

A red line in any of these is an automatic P0 until proven otherwise.

## The empty-list criterion

"No P0" is claimed only when:

1. the P0 section of this list is empty, and
2. `never_panics`, `sandbox_escape` and `repro-check` are all green, and
3. every gate in `docs/gates.md` passes or carries a dated exception.

A P0 list that has been relabelled to P1 is not empty; a gate that has been
silently dropped is not green. Both are findings, not closures.
