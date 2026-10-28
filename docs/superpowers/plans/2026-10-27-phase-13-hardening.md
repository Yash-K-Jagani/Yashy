# Phase 13 — 1.0 Hardening: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn fifteen phases of work into a release someone can install, reproduce, read about, and trust — with macOS actually supported rather than mentioned, with a self-hosting spike that answers the 2.0 question with evidence instead of optimism, and with the three developer surfaces a release needs: a format gate CI can enforce, a `--target` that is not the host by assumption, and `cfg` so a program can be platform-specific.

**Architecture:** This phase adds one small piece of language and the four things a 1.0 needs and a prototype does not: **a third platform** (macOS, signed and notarised, tested by the installer suite), **reproducible builds** (a hash a stranger can recompute), **a release process** (signed artifacts, checksums, changelog, clean-VM installs), and **documentation a newcomer can finish** (book, playground, `yashy doc`). The one language addition is `cfg`, and it is here because it is the last thing a release needs from the front end: without it a program cannot be platform-specific. Beside it, `--target` stops the target triple being an assumption hidden in the emitter, and `fmt --check` gives CI the format gate the rest of the programme has been doing by hand. The self-hosting spike is a design record plus a differential test, deliberately not a compiler.

**Tech Stack:** `.github/workflows/{release,ci,bench}.yml`, `scripts/{package-release,install,test-install,checksums}.*`, `crates/yashy-driver` (`--reproducible`, `fmt --check`, `--target`), `crates/yashy-lexer`/`yashy-parser`/`yashy-driver` (the `cfg` keyword and the pre-resolution filter), `crates/yashy-codegen` (one `target_triple` threaded through), the phase 11 sandbox (playground), the phase 12 registry format (hosted index), and a new `selfhost/` directory for the spike.

**Spec:** `docs/superpowers/specs/2026-10-27-yashy-master-design.md` §9.2, §8 row 13. Index: `docs/superpowers/plans/2026-10-27-yashy-master-plan.md`. Predecessor: `2026-10-27-phase-12-libraries-packages.md`.

## Global Constraints

See the master index. The ones that bite here:

- **Do not claim macOS support until a signed and tested macOS archive ships.** This phase is where that sentence is either satisfied or the claim stays absent.
- No new runtime dependencies.
- Both operating systems or it is not done — now three.
- No unproven claim: "reproducible" means two independent builds produced the same hash, printed by CI.
- Format touched files only.

## Written cap (this phase)

- **The playground compiles on the server, not in the browser.** A WASM compiler is a 2.0 project with its own performance story; the playground is the phase 11 sandbox behind a small HTTP front end.
- **The self-hosting spike parses a subset** — lexing plus parsing of the current grammar, no type checking, no codegen. It is measured against the Rust parser on the corpus and written up, not shipped.
- **No third-party registry hosting.** A read-only static index is published; uploads stay local (`file://`).

## Review Focus

1. **Reproducibility is proven by a stranger, not by the author.** CI builds the release twice, in two directories, on two machines of the same OS, and compares SHA-256 of the archives. Any difference is a bug in path normalisation, timestamps, or map ordering — and the diff is printed.
2. **A signed macOS archive is not the same as a working one.** The installer test runs on a clean macOS VM, with Gatekeeper on, and the tutorial's first three chapters must pass from the installed binary.
3. **The playground is the sandbox's customer, not a second sandbox.** It calls `yashy run --sandbox` with the phase 11 capability set; a request that would escape is refused by the same code the escape corpus tests.
4. **The book's every code block is executed by CI.** A book with a broken example is worse than no book, and the tutorial harness already proves the mechanism.
5. **A P0 is anything that produces a wrong binary, a panic on valid input, or a security escape.** "No P0 open" means the triage list is empty and the crash corpus is clean, not that the list was relabelled.
6. **A disabled `cfg` item is not checked, and that is the feature.** If the filter ran after resolution, `cfg(windows) { win32_call() }` would fail to compile on Linux and the construct would be useless. The test that proves it is a program whose disabled branch calls a function that exists nowhere.
7. **A misspelled cfg key must not delete code.** `cfg(window)` is `E2202`, not `false`. Every other unknown-name rule in the compiler is strict; this one matters more than most, because the symptom is a branch that silently disappears and a program that silently does the wrong thing on one platform.
8. **`fmt --check` must not depend on the checkout's line endings.** `core.autocrlf=true` is the default on Windows and the repository uses LF in the index, so the comparison normalises CRLF before comparing. A gate that fails on a clean tree is a gate people disable.

---

### Task 1: The developer surface — `fmt --check`, `--target`, `cfg`

**Files:**
- Modify: `crates/yashy-driver/src/main.rs` (`fmt --check`; `--target` on `build`/`run`/`check`/`test`; `check --print-cfg`)
- Modify: `crates/yashy-driver/src/pipeline.rs` (`host_target_triple()` becomes `target_triple(options)`, so every caller names its target instead of assuming the host)
- Modify: `crates/yashy-codegen/src/{ir.rs,module.rs}` (the data layout, the triple in the IR header, and the C ABI for `repr(C)` all take the target from the options — no `cfg!(target_os)` in the emitter)
- Modify: `crates/yashy-runtime` (the shim is compiled with `clang --target=<triple>`, not with the host compiler)
- Modify: `crates/yashy-lexer/src/token.rs` (`cfg` keyword; `KEYWORDS` 36 → 37)
- Modify: `crates/yashy-parser/src/{ast.rs,items.rs,stmts.rs}` (`Item::Cfg { key: CfgExpr, item: Box<Item>, span }`, `Stmt::Cfg { key, block, span }`, `CfgExpr::{Key, Not, Any, All}`)
- Modify: `crates/yashy-driver/src/pipeline.rs` (a **cfg filter** runs between parse and resolve, so a disabled item is never resolved, checked or verified)
- Modify: `crates/yashy-common/src/codes.rs` (`UNSUPPORTED_TARGET = "E2200"`, `TARGET_TOOLCHAIN_MISSING = "E2201"`, `UNKNOWN_CFG_KEY = "E2202"`, `CFG_MISPLACED = "E2203"`), create the four pages, modify `docs/errors.md`
- Modify: `crates/yashy-fmt/src/lib.rs`
- Modify: `.github/workflows/ci.yml` (the format gate), `docs/cli.md`
- Create: `crates/yashy-driver/tests/{fmt_check,target,cfg}.rs`, `crates/yashy-driver/tests/programs/cfg_platform.ya`

**Interfaces:**

- Produces: `yashy fmt --check <files…>` writes nothing, prints one line per file that is not already canonical, and exits **1** if any file would change and **0** if none. `--check` and `--write` together is a usage error (exit 2). `--json` emits the same list as `{ "schema_version": 1, "tool": "fmt", "success": bool, "files": [{ "file": …, "changed": bool }] }`, so CI can gate on it without parsing text.
- Produces: `yashy build --target <triple>` (and `run`, `check`, `test`). The supported list is exactly six triples — `x86_64` and `aarch64` for `pc-windows-msvc`, `unknown-linux-gnu` and `apple-darwin` — and an unsupported one is `E2200` listing them. `yashy doctor --target <triple>` reports whether the C toolchain for that triple is present.
- Produces: `E2201` when the cross C compiler is absent, naming the program it looked for. Cross-compiling is **not** made self-contained: a target needs its C toolchain, and pretending otherwise is how a cross build silently links the host's runtime.
- Produces: `cfg(windows)`, `cfg(linux)`, `cfg(macos)`, `cfg(unix)`, `cfg(arch_x86_64)`, `cfg(arch_aarch64)`, `cfg(debug)`, `cfg(release)`, and the combinators `not(…)`, `any(…)`, `all(…)`, as an **item modifier** (`cfg(windows) import win;`), a **statement block** (`cfg(unix) { … }`), and inside a struct's field list.
- Produces: a disabled item is **not resolved, not checked, not verified, not emitted** — that is the whole point, and it is what distinguishes `cfg` from `if false`. The filter runs on the AST before name resolution, so platform-only code that does not compile elsewhere is legal and inert.
- Produces: `E2202` for an unknown key — `cfg(window)` is an error, never a silently false condition. A typo that deletes code is the failure mode this rule exists for, and it is the one case where being strict costs nothing.
- Produces: `E2203` for `cfg` anywhere it cannot apply (on an expression, on a parameter, on a `let`), naming the three positions it is allowed in.
- Produces: `yashy check --print-cfg` prints the active set for the current target, one key per line, sorted — so a build script or a confused user can see what was true.
- Produces: the formatter round-trips all three forms; `cfg` is one keyword and adds no syntax beyond the parenthesis.

- [ ] **Step 1: Write the failing tests** — `fmt --check` exits 1 on a file with a stray space and 0 after `fmt -w`; `--check --write` exits 2; `--json` lists exactly the changed files; `--target aarch64-apple-darwin` on Windows produces an object for that triple (assert the IR header and, where a cross linker exists, the archive); an unsupported triple is `E2200`; a missing cross toolchain is `E2201` with the program named; `cfg(windows) { a() } cfg(linux) { b() }` runs exactly one; the disabled branch may contain a call to a function that does not exist anywhere and still compiles; `cfg(window)` is `E2202`; `cfg` on a `let` is `E2203`; `--print-cfg` lists the expected keys for the host; the formatter round-trips all three positions; `KEYWORDS.len() == 37`.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — `fmt --check` reuses the existing `format` function and compares bytes after normalising CRLF, so a checkout with `core.autocrlf=true` does not report every file. `target_triple` threads one value from the CLI to codegen and to the runtime build; nothing else reads `cfg!(target_os)`. The cfg filter is a pure AST transform with the key set derived from the target triple plus the build profile, and it runs before `resolve` so that "not resolved" is the default for a disabled item. `cfg` is parsed as a modifier by looking one token past the identifier.
- [ ] **Step 4: Run** `cargo test -p yashy-driver --test fmt_check --test target --test cfg --test e2e` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: fmt --check, --target and cfg, so CI can gate formatting and a program can be platform-specific (E2200-E2203)"`

---

### Task 2: macOS build, sign and notarise

**Files:**
- Modify: `.github/workflows/release.yml` (a `macos-14` job: arm64 and x86_64 universal or two archives)
- Modify: `scripts/package-release.sh`, `scripts/test-install.sh`
- Create: `docs/install/macos.md`
- Modify: `README.md`, `AGENTS.md` (the "do not claim macOS" rule is satisfied here, with the date)

**Interfaces:**
- Produces: `yashy-<version>-aarch64-apple-darwin.tar.gz` and `-x86_64-apple-darwin.tar.gz`, each signed with a Developer ID Application certificate, notarised with `notarytool`, and stapled.
- Produces: an install test that runs on a clean `macos-14` runner: unzip, `xattr -d`, run `yashy doctor`, compile and run `examples/hello`, run the first three tutorial chapters.

- [ ] **Step 1: Write the failing job** — the release workflow's macOS job fails because there is no archive yet; the install test asserts the stapled ticket (`spctl -a -vvv`) and the version string.
- [ ] **Step 2: Build it** — the compiler is pure Rust plus a C runtime; the only platform work is `build.rs` for the runtime's macOS branch (already `#ifdef`-ed for POSIX) and the `clang` invocation. Record any real porting work in this plan as it is found.
- [ ] **Step 3: Sign and notarise** — credentials come from repository secrets; a missing secret fails the job with a message naming it, never a silent unsigned artifact.
- [ ] **Step 4: Run the install test** on the clean runner → PASS.
- [ ] **Step 5: Update the claim** — `README.md` and `AGENTS.md` may now say macOS is supported, with the archive name and the date. Not before.
- [ ] **Step 6: Commit** — `git commit -m "feat(release): a signed, notarised, tested macOS archive"`

---

### Task 3: Reproducible builds

**Files:**
- Modify: `crates/yashy-driver/src/{main.rs,pipeline.rs}` (`--reproducible`, `SOURCE_DATE_EPOCH`)
- Modify: `crates/yashy-codegen/src/lib.rs` (deterministic module and symbol ordering; no absolute paths in IR)
- Modify: `crates/yashy-runtime/build.rs` (deterministic object flags)
- Create: `scripts/repro-check.sh`, `scripts/repro-check.ps1`
- Create: `crates/yashy-driver/tests/reproducible.rs`

**Interfaces:**
- Produces: `yashy build --reproducible` sets a fixed timestamp, normalises source paths to a workspace-relative form, sorts every map before emission, and passes `-ffile-prefix-map` to `clang`.
- Produces: `scripts/repro-check` builds twice in two directories and compares SHA-256; exit 1 with the first differing byte offset on failure.

- [ ] **Step 1: Write the failing test** — build `examples/hello` twice in two temp directories; the two binaries differ today (timestamp or path). Assert equality after the fix.
- [ ] **Step 2: Watch it fail**, and record *which* input caused the difference — that list is the actual work.
- [ ] **Step 3: Implement** — the timestamp comes from `SOURCE_DATE_EPOCH` (default: the commit time, from `git log -1 --format=%ct`, falling back to 0); paths are mapped; `HashMap` iteration is replaced by `BTreeMap` anywhere it reaches output.
- [ ] **Step 4: Run** `cargo test -p yashy-driver --test reproducible` and `scripts/repro-check` on both OSes → identical hashes.
- [ ] **Step 5: Commit** — `git commit -m "feat(build): reproducible binaries from a fixed epoch and normalised paths"`

---

### Task 4: The release process

**Files:**
- Modify: `.github/workflows/release.yml` (all three platforms, one draft release, checksums, changelog)
- Modify: `scripts/{checksums.sh,package-release.ps1,package-release.sh,test-install.ps1}`
- Create: `CHANGELOG.md` (Keep a Changelog format, generated from conventional commits with a script), `docs/release.md`
- Create: `.github/workflows/install-matrix.yml`

**Interfaces:**
- Produces: a tagged release publishes: three platforms × two architectures where applicable, `SHA256SUMS`, a signed checksum file (GPG for Linux, Authenticode for Windows, Developer ID for macOS), and a changelog section.
- Produces: `install-matrix.yml` runs the installer on `windows-latest`, `ubuntu-22.04` and `macos-14`, from the published artifacts, on every release and weekly.

- [ ] **Step 1: Write the failing workflow** — the release job produces a draft with no artifacts because the matrix is incomplete; the install matrix fails on macOS (no archive).
- [ ] **Step 2: Implement** — one job builds, one signs, one publishes, one tests; artifacts flow through `upload-artifact`/`download-artifact` so no step rebuilds.
- [ ] **Step 3: Run a dry release** from a pre-release tag → all six installs pass, checksums verify, changelog is present.
- [ ] **Step 4: Commit** — `git commit -m "feat(release): signed artifacts, checksums and an install matrix"`

---

### Task 5: The book

**Files:**
- Create: `book/src/` (mdBook layout: getting started, language guide, verification guide, tooling, packages, cookbook, reference)
- Modify: `docs/tutorial/` (the chapters become book chapters; the tutorial harness keeps running them from one source)
- Create: `docs/tutorial/19-platforms.md` (the chapter Task 1 makes possible: `cfg`, `--target`, what the three platforms guarantee, and what cross-compiling does and does not give you)
- Create: `.github/workflows/book.yml` (build the book, run every code block, deploy on tag)

**Interfaces:**
- Produces: a book whose code blocks are the same files the tutorial harness compiles — no copied snippets. A block that does not compile fails CI.
- Produces: a reference section generated from `yashy index --json` on the stdlib and the language's own definitions, so signatures cannot drift.

- [ ] **Step 1: Write the failing job** — the book build fails because a chapter's block does not compile (use a deliberate error to prove the mechanism), then fix it.
- [ ] **Step 2: Write the book** — six sections, each ending with an exercise whose solution is a test program under `book/tests/`.
- [ ] **Step 3: Run** `cargo test -p yashy-driver --test tutorial` and the book job → PASS.
- [ ] **Step 4: Commit** — `git commit -m "docs: the book, with every code block executed by CI"`

---

### Task 6: The playground

**Files:**
- Create: `playground/` (a static front end and a tiny server that shells out to `yashy run --sandbox`)
- Create: `playground/tests/` (escape attempts reused from phase 11, plus rate limits)
- Modify: `docs/cli.md` (the sandbox flags the playground uses)

**Interfaces:**
- Produces: `POST /run` with `{source, args}` → `{stdout, stderr, exit, diagnostics}`; every run is `--sandbox --allow-read /tmp/playground/<id> --time 2000 --memory 128 --output 65536`.
- Produces: per-IP rate limit, a 64 KB source cap, a 2 s wall clock, and a queue depth cap; a refused request returns `429`, never a hang.

- [ ] **Step 1: Write the failing tests** — the phase 11 escape corpus is replayed through `POST /run`; every attempt is denied; a fork bomb is killed at the process cap; a 64 KB+ source is `413`; 100 requests from one IP hit the rate limit.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — the server is a thin shell around the compiler binary with a temp dir per request; the front end is static HTML/JS with no build step, because a playground that needs a toolchain is not a playground.
- [ ] **Step 4: Run** `cargo test -p yashy-driver --test playground` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(playground): compile and run in the sandbox, not in the browser"`

---

### Task 7: A hosted read-only registry index

**Files:**
- Create: `registry/` (a static index generated from the first-party packages, published to a static host)
- Modify: `crates/yashy-driver/src/registry.rs` (an `https://` read path for a static index)
- Create: `docs/packages.md` (publishing section)

**Interfaces:**
- Produces: `yashy add http --registry https://…` reads `index.json` and a tarball over HTTPS; the hash in the lockfile is the integrity check.
- Produces: publishing stays `file://`; a pull request to the registry repo is how a package is added, reviewed by a human.

- [ ] **Step 1: Write the failing test** — a local HTTP server serving `registry/` is added as a registry; `yashy add` fetches, verifies the hash and builds; a tampered tarball fails the hash check.
- [ ] **Step 2: Implement** — the HTTPS path reuses `net` from phase 12 and requires TLS; if TLS is unavailable on a platform, the command fails with a clear message rather than falling back to plaintext.
- [ ] **Step 3: Commit** — `git commit -m "feat(registry): a read-only hosted index with hash-verified fetches"`

---

### Task 8: The self-hosting spike

**Files:**
- Create: `selfhost/{yashy.toml,src/{lexer.ya,parser.ya,ast.ya}}`
- Create: `selfhost/tests/differential.ya`, `crates/yashy-driver/tests/selfhost_diff.rs`
- Create: `docs/superpowers/design/2027-selfhost-spike.md`

**Interfaces:**
- Produces: a Yashy lexer and parser for the current grammar (no type checking, no codegen) that emits the same AST shape as the Rust parser, serialised to a canonical JSON for comparison.
- Produces: a differential test over `crates/yashy-driver/tests/programs/*.ya` and `examples/*/main.ya`: for every file, both parsers either agree or the Rust parser's error is reproduced.

- [ ] **Step 1: Write the lexer** and a differential test over 20 files; fix mismatches until they pass.
- [ ] **Step 2: Write the parser** and extend the differential test to the full corpus; record the pass rate.
- [ ] **Step 3: Write the design record** — what self-hosting would take (a bootstrap path, a codegen story, the performance cost of a Yashy parser, and the maintenance cost of two implementations of one grammar), and a recommendation for 2.0 with the numbers from Step 2.
- [ ] **Step 4: Commit** — `git commit -m "feat(selfhost): a parser spike with a differential test and a design record"`

---

### Task 9: The all-gates runbook and P0 triage

**Files:**
- Create: `docs/gates.md` (every gate from every phase, its command, its budget, and its owner)
- Create: `scripts/gates.sh`, `scripts/gates.ps1` (run every gate, print a table, exit 1 on any failure)
- Modify: `.github/workflows/ci.yml` (a nightly `gates` job)
- Create: `docs/triage.md` (the P0/P1/P2 definitions and the empty-list criterion)

**Interfaces:**
- Produces: one command that runs the whole gate set on a clean checkout, with a machine-readable `--json` summary.
- Produces: a triage list with every open issue classified; "no P0" means the P0 section is empty and the crash corpus (`never_panics`, `sandbox_escape`, `repro-check`) is green.

- [ ] **Step 1: Write the runbook** by collecting the gate tables from all fifteen phase plans; a missing gate is a finding.
- [ ] **Step 2: Write the runner** and run it on both OSes → every gate green or an explicit, dated exception.
- [ ] **Step 3: Triage** — classify every open issue; fix or downgrade each P0 with a test.
- [ ] **Step 4: Commit** — `git commit -m "docs: the all-gates runbook and the P0 triage"`

---

### Task 10: The 1.0 release

**Files:**
- Modify: `Cargo.toml` (version 1.0.0), `CHANGELOG.md`, `README.md`
- Create: `docs/1.0.md` (what 1.0 promises and what it explicitly does not)

**Interfaces:**
- Produces: a release note that states, with numbers: compile and test times, runtime ratios, binary size, the verifier's corpus results, the false-positive count (zero), and the platform list.
- Produces: `docs/1.0.md`'s "does not promise" list — no dynamic dispatch, no generic traits, no async traits, no procedural macros, no WASM target, no third-party registry hosting. Naming the limits is part of shipping.

- [ ] **Step 1: Run the full gate set** one last time on a clean checkout, all three platforms.
- [ ] **Step 2: Tag `v1.0.0`** and publish through the Task 4 process.
- [ ] **Step 3: Verify the published artifacts** with the install matrix, from the tag, on all three platforms.
- [ ] **Step 4: Commit and record** — `git commit -m "release: 1.0.0"`; record the tag, the hashes and the gate table in this plan and in the index.

---

## Gate summary

| Gate | Command | Passes when |
|---|---|---|
| macOS supported | `install-matrix` on `macos-14` | signed, notarised, stapled; `doctor` and the first three chapters pass |
| Reproducible | `scripts/repro-check` | two independent builds, identical SHA-256, all three OSes |
| Signed releases | `release.yml` dry run | checksums verify; every artifact signed |
| Install matrix | `install-matrix.yml` | six installs from published artifacts, all green |
| Book | `book.yml` | every code block compiles; reference generated from `yashy index` |
| Playground safe | `cargo test -p yashy-driver --test playground` | phase 11 escape corpus denied through `/run` |
| Registry | `cargo test -p yashy-driver --test registry` | hosted index add builds; tampered tarball rejected |
| Self-hosting evidence | `cargo test -p yashy-driver --test selfhost_diff` | pass rate recorded; design record written |
| All gates | `scripts/gates` | every phase gate green, or a dated exception |
| No P0 | `docs/triage.md` | P0 list empty; crash corpus green |
| CI fails on unformatted source | `cargo test -p yashy-driver --test fmt_check` | `fmt --check` exits 1 on a misformatted file, 0 on the repository, and agrees with `fmt -w` |
| Cross-compilation | `yashy build --target <triple>` on each of the six triples | the binary runs on that platform; `E2200`/`E2201` name the unsupported triple and the missing toolchain |
| Platform conditionals | `cargo test -p yashy-driver --test cfg` | exactly one branch is selected; a disabled branch is never resolved; `cfg(window)` is `E2202` |
| Metrics met | `docs/1.0.md` | every number in the release note reproduced by the gate run |
