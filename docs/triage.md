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

No P0 and no P1 is open at the end of Task 9. One P2 is open: it is
cosmetic today (the URLs resolve) and is scheduled for the Task 10
release pass, which touches the same files anyway.

| ID | Severity | Issue | Fix | Test | Status |
|---|---|---|---|---|---|
| T1 | P2 | Live files still name the old repository: `REPO="Yash-K-Jagani/Otter"` in `scripts/install.sh`, two URLs in `scripts/install.ps1`, the `curl` lines in `README.md` and `docs/quickstart.md`, the installer line in `docs/release-notes-template.md`, and the four doc links in the `yashy init` AGENTS.md template (`crates/yashy-driver/src/main.rs`). They work today only through GitHub's rename redirect (`git ls-remote https://github.com/Yash-K-Jagani/Otter.git` still resolves). The rename design left the remote URL out of scope (§11) while its own risk check says the `Otter` search must come back empty outside dated records — the design is inconsistent, and the redirect is the thing being relied on. | swap every live occurrence to `Yash-K-Jagani/Yashy` (the canonical remote); Task 10, which edits the README and the release notes anyway | no test pins the spelling (`grep Otter` over `crates/**/*.rs` finds only the `main.rs` template); after the fix the installer's download URL resolves with no redirect | open |

### Closed findings

| ID | Severity | Issue | Fix | Test |
|---|---|---|---|---|
| G1 | P0 | `cargo check --workspace --all-targets` failed at HEAD with `E0061`: `8ff1644` added `extra_flags` and `epoch` to `cached_runtime_object` and updated the five production callers, but not the test at `pipeline.rs:2367`. The `check` gate — and therefore the `tests` gate, which compiles the same target — was red on main. | added `&[]` and `None` to the test call site (`crates/yashy-driver/src/pipeline.rs`), commit `6da0af1` | `cargo check --workspace --all-targets` → exit 0; found by `scripts/gates.ps1`'s `check` gate |

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
