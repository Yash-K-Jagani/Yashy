# Slice 1 — Install + First CLI

> **For agentic workers:** Read the master plan first. Work this slice alone.
> Use test-first steps where behavior changes. Keep the Windows path working
> while adding Linux.

**Status:** Implemented and pushed, commit `d30e1af` on `slice-1-install-first-cli`,
2026-10-26. Windows test execution is delegated to CI; the `ubuntu:22.04`
container acceptance is pending a Docker host.
**Source of truth:** `docs/superpowers/specs/2026-10-25-yashy-super-language-v2-design.md` §11 and §15.1.
**Master plan:** `docs/superpowers/plans/2026-10-25-yashy-super-language-master.md` §7.1.
**Goal:** A fresh Windows or Linux machine installs a prebuilt Yashy, runs
`yashy doctor` and `yashy doctor --json`, completes a first CLI project without a
Rust toolchain, and can uninstall cleanly.
**Depends on:** Nothing. The first-CLI project reads standard input, so it does
not need `args()`; slice 2 adds arguments later.
**Estimate:** 10–12 working days (the v2 AI quickstart and uninstall checks add
roughly two days to the original 8–10).

---

## 1. Scope

### In scope

- A Linux release job that builds the same artifacts as Windows.
- A Unix packaging script and `install.sh` that installs into a prefix, puts
  `yashy` on `PATH`, and runs `yashy doctor`.
- Checksums for release artifacts.
- Installer-level uninstall (`install.sh --uninstall`, `install.ps1 -Uninstall`)
  that removes the install prefix and the marked `PATH` block, idempotently.
- Better `yashy doctor` guidance for a missing or broken LLVM toolchain.
- `yashy doctor --json` with a stable v1 schema.
- `yashy init` writes `AGENTS.md` and a README with build/test/run commands.
- A repo-level `AGENTS.md` and `llms.txt` for contributors and agents.
- A "Build your first CLI in 10 minutes" guide and at least three examples.
- A clean-install smoke test that runs against the packaged artifact in CI,
  including uninstall.
- README and tutorial index updates.

### Not in scope

- macOS packaging (1.0 target; `doctor` still gives `brew` guidance).
- `args()`, `getcwd()`, `now()`, `sleep(ms)` (slice 2).
- The `yashy uninstall` CLI command (R2). R1 ships installer-level uninstall.
- Package manager (slice 4).
- Code signing, notarization, or a hosted download service.
- A `0.2.0` tag. The tag is cut at the R1 gate after slices 2–4; this slice
  proves the mechanism.

---

## 2. Current state

- `.github/workflows/release.yml` has one `windows` job: build, package with
  `scripts/package-release.ps1`, upload `dist/*.zip`, publish on a tag.
- `scripts/package-release.ps1` stages `yashy.exe`, `yashy-stdlib/yashy`,
  `install.ps1`, `README.md`, and `LICENSE` into a zip.
- `scripts/install.ps1` downloads the latest Windows zip (or uses a local
  archive), copies everything to `%LOCALAPPDATA%\Yashy`, adds that directory to
  the user `PATH`, and runs `yashy doctor`.
- `yashy doctor` checks `clang` (runs `--version`) and the bundled stdlib. The
  toolchain error already includes a platform hint from `install_hint()`.
- The stdlib search supports `bin/yashy` + `share/yashy/stdlib` (shared prefix)
  and `bin/yashy` + `bin/yashy-stdlib/yashy` (side by side).
- There is no Linux artifact, no `install.sh`, no first-CLI guide, and no
  examples directory.
- The tutorial is `docs/tutorial/00-intro.md` … `11-functions.md`; it starts
  from a source build, not an installed release.

---

## 3. Design decisions

### 3.1 Package layout

Use one archive layout for every platform:

```text
yashy-<version>-<target>/
  yashy or yashy.exe
  yashy-stdlib/yashy/*.ya
  install.sh or install.ps1
  README.md
  LICENSE
```

The Unix installer installs to a prefix with the conventional split:

```text
$PREFIX/bin/yashy
$PREFIX/share/yashy/stdlib/yashy/*.ya
```

`stdlib_root_near_exe` already resolves `bin/../share/yashy/stdlib`, and the
shared-prefix test now creates `bin` so this layout is covered on Unix.

Default prefix: `${YASHY_PREFIX:-$HOME/.local}`. Do not require `sudo`. Offer
`--prefix /usr/local` for a system install, but do not default to it.

The Windows installer keeps its current layout (`%LOCALAPPDATA%\Yashy` with the
binary and `yashy-stdlib` side by side). Changing it is unnecessary risk.

### 3.2 Linux target and compatibility

Build on `ubuntu-22.04`, not `ubuntu-latest`, so the binary links against an
older glibc. Name the target `x86_64-unknown-linux-gnu`. Document the minimum
glibc in the release notes. Test the artifact in a clean `ubuntu:22.04`
container.

### 3.3 Installer behavior

`install.sh` is POSIX `sh`, not Bash-only, because the guide advertises
`curl … | sh`.

Supported inputs:

- `YASHY_VERSION` (default `latest`).
- `YASHY_PREFIX` (default `$HOME/.local`).
- `--version <v>`, `--prefix <dir>`, `--no-path`, `--no-doctor`, `--help`.
- A local archive via `YASHY_ARCHIVE` for tests and offline installs.

Behavior:

1. Detect `uname -s`/`uname -m`; support Linux x86_64 only in R1. Print a clear
   error for anything else.
2. Resolve the release: `latest` via the GitHub API, or a pinned version.
3. Download the `.tar.gz` and its checksum.
4. Verify SHA-256 before extracting.
5. Extract to a temporary directory.
6. Create `$PREFIX/bin` and `$PREFIX/share/yashy/stdlib`; copy `yashy` and the
   stdlib.
7. Add `$PREFIX/bin` to `PATH` in the user's shell profile (`~/.profile` and
   `~/.bashrc`; `~/.zshrc` if it exists) unless `--no-path`. Make the edit
   idempotent and print exactly what changed.
8. Run `yashy doctor` unless `--no-doctor`. If it fails, print the next step
   (`sudo apt install clang llvm` or the detected distro's equivalent) and keep
   the install; a missing compiler is fixable without reinstalling.
9. Print the next command: `yashy init my-cli`.

`install.ps1` gains checksum verification and the same next-step wording. It
already runs `doctor` and is idempotent.

### 3.4 Checksums

The release workflow writes `SHA256SUMS` containing every archive. Both
installers fetch and verify it. A checksum mismatch is a hard failure. The
checksum protects against a truncated download, not a compromised release; say
so in the release notes rather than overclaiming.

### 3.5 First-CLI project

The guide builds a **line/word/byte counter that reads standard input**, because
`args()` does not exist yet. It is a two-module project adapted from the
existing `crates/yashy-driver/tests/programs/wc.ya` and `wc_words.ya`, so the
guide's code is already covered by the e2e suite:

```yashy
// main.ya
import wc_words;

fn main() {
    let lines = 0;
    let words = 0;
    let bytes = 0;
    let line = read_line();
    while line.len() > 0 {
        lines = lines + 1;
        words = words + count_words(line);
        bytes = bytes + line.len() + 1;
        line = read_line();
    }
    print(lines);
    print(" ");
    print(words);
    print(" ");
    print(bytes);
    println("");
}
```

```yashy
// wc_words.ya
pub fn count_words(line: string) -> int {
    // ... the byte scan from the existing program ...
}
```

`let` is the mutable binding; `const` is immutable. A `[]string` is released
with `.free()`, not a `free_slice` builtin. The program is a real filter
(`cat file | yashy run main.ya`) with no arguments. Slice 2 adds a "read a file
path from `args()`" section.

The guide covers:

1. Install on Windows and Linux.
2. `yashy doctor` and what to do when `clang` is missing.
3. `yashy init wordcount`; what the scaffold contains.
4. The stdin counter; `yashy run`.
5. `yashy test`; add a test program.
6. `yashy build --release -o wordcount`; run the binary directly.
7. Distribute: copy one binary; no runtime.
8. Next steps: arguments (slice 2), JSON, packages.

### 3.6 Examples

Add three examples, each with `yashy.toml`, `main.ya`, and a test:

- `examples/hello/` — the smallest project.
- `examples/wordcount/` — the guide's project.
- `examples/json-report/` — reads a JSON file and prints a summary, exercising
  the real `JSON` module.

CI builds and tests all three on Ubuntu and Windows. The tutorial test already
compiles tutorial snippets; examples get their own small test to avoid turning
the tutorial test into a project test.

---

## 4. Tasks

### Task 1 — Linux packaging script

- [ ] Add `scripts/package-release.sh`:
  - Inputs: `--version`, `--target`, optional `--binary`.
  - Stages the layout in §3.1.
  - Writes `dist/yashy-<version>-<target>.tar.gz`.
  - Refuses to run if the binary or stdlib is missing.
  - Uses `tar -czf` and preserves executable bits.
- [ ] Add `scripts/checksums.sh` (or fold into the package script) that writes
  `dist/SHA256SUMS`.
- [ ] Test locally on WSL Ubuntu with a release binary.

### Task 2 — Linux release job

- [ ] Add a `linux` job to `.github/workflows/release.yml`:
  - `runs-on: ubuntu-22.04`.
  - Install Rust, `clang`, and `llvm`.
  - `cargo build --release -p yashy-driver`.
  - Run `scripts/package-release.sh`.
  - Upload `dist/*.tar.gz` and `dist/SHA256SUMS`.
- [ ] Make the publish step depend on both jobs and upload both archives and
  the checksum file. Do not let one job publish alone.
- [ ] Add a `package-smoke` job that runs the installer against the packaged
  artifact in a clean `ubuntu:22.04` container (or a separate job with a fresh
  prefix) before publishing.

### Task 3 — `install.sh`

- [ ] Add `scripts/install.sh` with the behavior in §3.3.
- [ ] Support running from an extracted archive (the script sits next to
  `yashy`) and from a pipe (download path).
- [ ] Verify SHA-256 against `SHA256SUMS`.
- [ ] Add an idempotent `PATH` edit; do not duplicate lines.
- [ ] Run `yashy doctor`; print the exact next command.
- [ ] Test in WSL Ubuntu:
  - fresh prefix, local archive;
  - rerun (idempotent);
  - missing `clang` (doctor fails, install succeeds);
  - bad checksum (hard failure, no partial install);
  - `--no-path` and `--no-doctor`.
- [ ] Add a shell test script `scripts/test-install.sh` that automates the
  local-archive cases with a temp prefix.

### Task 4 — Windows installer parity

- [ ] Add checksum verification to `scripts/install.ps1`.
- [ ] Use the same next-step wording as `install.sh`.
- [ ] Add a PowerShell test `scripts/test-install.ps1` using `-LocalArchive`
  and a temp prefix.
- [ ] Confirm the Windows zip still installs and `doctor` runs.

### Task 5 — `yashy doctor` guidance

- [ ] Improve `install_hint()` in `crates/yashy-driver/src/toolchain.rs`:
  - Ubuntu/Debian: `sudo apt install clang llvm`.
  - Fedora: `sudo dnf install clang llvm`.
  - Arch: `sudo pacman -S clang llvm`.
  - Windows: keep `winget install LLVM.LLVM`.
  - macOS: keep `brew install llvm`.
  - If the distro is unknown, keep the generic message.
- [ ] Make `doctor` print a final `next:` line when it fails, not only the
  per-check errors.
- [ ] Add a `doctor` test that asserts the Linux hint contains `apt` on Linux
  and the Windows hint contains `winget` on Windows. Keep the existing tests.

### Task 6 — First-CLI guide

- [ ] Add `docs/tutorial/12-first-cli.md`.
- [ ] Write the stdin counter against the real API; compile and run every
  snippet.
- [ ] Include Windows (PowerShell) and Linux shell commands side by side, with
  a macOS source-build note.
- [ ] Add the guide to `docs/tutorial/00-intro.md`'s "what next" and the README.
- [ ] Add a tutorial test that compiles and runs the guide's final program.
  Reuse the existing tutorial test harness; do not add a second one.

### Task 7 — Examples

- [ ] Add `examples/hello/`, `examples/wordcount/`, `examples/json-report/`.
- [ ] Each has `yashy.toml`, `main.ya`, and `tests/smoke.ya`.
- [ ] Add `crates/yashy-driver/tests/examples.rs` that runs `yashy test` in
  each example directory on both platforms.
- [ ] Keep the examples small enough to run in the normal test suite.

### Task 8 — Clean-install acceptance

- [ ] Add a CI job `install-smoke` on Ubuntu and Windows:
  - build/package the artifact;
  - install into a temp prefix with the local archive;
  - `yashy doctor` (with `clang` available);
  - `yashy init demo`;
  - `yashy run src/main.ya` or the scaffold's entry;
  - `yashy test`.
- [ ] Add a Linux container check that runs the packaged binary on
  `ubuntu:22.04` without Rust installed.
- [ ] Record the exact commands and output in the plan when the job passes.

### Task 9 — Docs and release notes

- [ ] Update `README.md`:
  - Windows install: download the zip, run `install.ps1`.
  - Linux install: `curl -fsSL <release>/install.sh | sh` or download the
    tarball and run `./install.sh`.
  - Link to the first-CLI guide.
- [ ] Update `docs/tutorial/00-intro.md` to say a source build is for
  contributors and an installed release is the normal path.
- [ ] Add a release-notes template with: version, targets, checksums, minimum
  glibc, LLVM requirement, install commands, and known gaps (macOS).
- [ ] Do not claim macOS support.

### Task 10 — Uninstall, `doctor --json`, and AI quickstart

- [ ] Add installer uninstall:
  - `install.sh --uninstall` removes the install prefix and the marked `PATH`
    block.
  - `install.ps1 -Uninstall` removes `%LOCALAPPDATA%\Yashy` and the marked user
    `PATH` entry.
  - Both are idempotent and print what they removed.
- [ ] Add `yashy doctor --json` with schema v1:
  - `schema_version`, `tool` (`doctor`), `ok`, `yashy_version`, `target`,
    `clang`, `stdlib`, `path`, and `next`.
  - No color, no interactive prompt, stable exit codes.
  - Human and JSON output carry the same facts.
- [ ] Make `yashy init` write `AGENTS.md` and a README with build/test/run
  commands. The `AGENTS.md` covers project layout, commands, error handling, and
  where the docs live.
- [ ] Add a repo-level `AGENTS.md` and `llms.txt` with canonical links to the
  quickstart, tutorial, CLI reference, and error catalog.
- [ ] Tests:
  - Uninstall removes files and the `PATH` block; a second uninstall succeeds.
  - `doctor --json` parses and contains the expected fields on success and
    failure.
  - `init` writes `AGENTS.md` and the README.
  - Every link in `llms.txt` resolves.

---

## 5. Verification

Run these on a clean checkout:

```text
# Windows
cargo check --workspace --all-targets
cargo test -p yashy-driver --test doctor
cargo test -p yashy-driver --test tutorial
cargo test -p yashy-driver --test examples
pwsh scripts/test-install.ps1

# Linux (WSL Ubuntu or CI)
cargo check --workspace --all-targets
cargo test -p yashy-driver --test doctor
cargo test -p yashy-driver --test tutorial
cargo test -p yashy-driver --test examples
sh scripts/test-install.sh
```

Then the acceptance run:

```text
# Linux container with no Rust
docker run --rm -v "$PWD/dist:/dist" ubuntu:22.04 sh -c '
  apt-get update && apt-get install -y clang llvm curl ca-certificates &&
  YASHY_ARCHIVE=/dist/yashy-<version>-x86_64-unknown-linux-gnu.tar.gz \
    sh /dist/install.sh --prefix /tmp/yashy --no-path &&
  /tmp/yashy/bin/yashy doctor &&
  /tmp/yashy/bin/yashy doctor --json &&
  /tmp/yashy/bin/yashy init /tmp/demo &&
  cd /tmp/demo && /tmp/yashy/bin/yashy test &&
  sh /dist/install.sh --uninstall --prefix /tmp/yashy --no-path &&
  test ! -d /tmp/yashy
'
```

On Windows, run `scripts/test-install.ps1` with a temp prefix and confirm
`yashy doctor`, `yashy doctor --json`, `yashy init`, and `yashy test` all pass,
then run `-Uninstall` and confirm the prefix and marked `PATH` block are gone.

---

## 6. Acceptance criteria

- A clean Ubuntu 22.04 machine with `clang` installed can install from the
  release tarball, run `yashy doctor` and `yashy doctor --json` green, and
  uninstall cleanly.
- A clean Windows machine can do the same from the zip.
- Without `clang`, both installers finish, `doctor` fails with an actionable
  platform-specific command, and the user can fix it without reinstalling.
- `yashy init` + `yashy run` produce a running program in under five minutes from
  a clean install; the full first-CLI guide takes under thirty minutes.
- `yashy init` writes `AGENTS.md` and a README with build/test/run commands.
- `AGENTS.md` and `llms.txt` exist at the repository root and their commands/links
  are tested.
- Installer uninstall removes the install prefix and the marked `PATH` block, and
  a second uninstall is a no-op.
- The three examples build and test in CI on Ubuntu and Windows.
- The release workflow publishes both archives and `SHA256SUMS` from one tag.
- No Rust toolchain is required for any of the above.

---

## 7. Risks and rollback

| Risk | Mitigation |
| --- | --- |
| glibc mismatch | Build on `ubuntu-22.04`; test in the same container; document the minimum. |
| `PATH` edit breaks a shell profile | Only append a marked block; test idempotence; `--no-path` escape hatch. |
| `curl \| sh` cannot find a local script path | Detect piped mode and always download; test both modes. |
| Checksum asset missing | Treat as a hard failure; publish the checksum in the same job as the archive. |
| Windows installer regresses | Keep the existing layout and tests; add checksum verification behind the same flow. |
| Guide drifts from the API | Compile and run the guide's program in the tutorial test. |

**Rollback:** the Linux job and installer are additive. If they fail, the
Windows release still publishes; do not publish a partial release with one
platform missing. Revert the workflow change and keep the packaging scripts for
the next attempt.

---

## 8. Files touched

- `.github/workflows/ci.yml`
- `.github/workflows/release.yml`
- `scripts/package-release.sh` (new)
- `scripts/package-release.ps1`
- `scripts/checksums.sh` (new)
- `scripts/install.sh` (new)
- `scripts/install.ps1`
- `scripts/test-install.sh` (new)
- `scripts/test-install.ps1` (new)
- `crates/yashy-driver/src/toolchain.rs`
- `crates/yashy-driver/src/modules.rs` (nested shared-prefix stdlib)
- `crates/yashy-driver/src/pipeline.rs` (clang 14 opaque pointers)
- `crates/yashy-driver/src/main.rs` (doctor's final `next:` line,
  `doctor --json`, `init` writes `AGENTS.md` + README + `.gitignore`)
- `crates/yashy-driver/tests/doctor.rs` (JSON cases)
- `crates/yashy-driver/tests/init.rs` (extend)
- `crates/yashy-driver/tests/examples.rs` (new)
- `crates/yashy-driver/tests/docs.rs` (new)
- `crates/yashy-driver/tests/tutorial.rs`
- `docs/tutorial/12-first-cli.md` (new)
- `docs/tutorial/00-intro.md`
- `docs/tutorial/11-functions.md`
- `docs/quickstart.md` (new)
- `docs/cli.md` (new)
- `docs/errors.md` (new)
- `AGENTS.md` (new, repository root)
- `llms.txt` (new, repository root)
- `examples/hello/`, `examples/wordcount/`, `examples/json-report/` (new)
- `README.md`
- `docs/release-notes-template.md` (new)

---

## 9. Definition of done

- The acceptance commands in §5 pass on Windows and Linux.
- The release workflow produces both artifacts and checksums from one tag.
- `yashy doctor --json` has a stable v1 schema and tests for success and failure.
- Installer uninstall is tested on both platforms and leaves no marked `PATH`
  block.
- `yashy init` writes `AGENTS.md` and a README; the repo-level `AGENTS.md` and
  `llms.txt` links are tested.
- The first-CLI guide is tested, not just written.
- The master plan's R1 gap table marks slice 1 complete.
- Commit and push; record the commit hash in this plan.
