# Phase 3 — Packages and Distribution

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a Yashy package something a stranger can install, audit, pin and
rebuild — and make the toolchain itself installable in one command.

**Outcome:** Semver ranges with a lockfile-pinned resolver, workspaces, optional
features, a signed static registry, package signatures, an SBOM, `yashyc
verify-package`, reproducible package builds, a signed stdlib with a precompiled
IR cache, installers for the three claimed platforms, a GitHub Action, and
templates for a workspace and a service.

**Time:** ~8 weeks.
**Depends on:** Phase 2 gate.
**Spec:** [`../specs/2026-11-15-yashy-2.0-design.md`](../specs/2026-11-15-yashy-2.0-design.md) §6.3, §6.8, §12.

## Why this phase

1.0's resolver is exact-pin only, has no workspaces or features, and its
registry is a directory with no signature, no SBOM and no verification. That is
enough to demonstrate a package and not enough to trust one. A product language
is judged the first time someone runs `yashyc add` on a machine you have never
seen.

The phase also fixes the supply-chain floor for everything after it: every
package the stdlib, HTTP, SQL and AI phases depend on is signed and recorded, so
a later phase cannot quietly add an unverified C library. Reproducibility lands
here because a package build is the first place a timestamp or an absolute path
can make two identical sources produce different bytes.

## Starting point

| Fact | 1.0 value |
|---|---|
| Versions | exact pins only; `yashy.lock` records one version |
| Workspaces | none |
| Features | none |
| Registry | static local directory; `publish`/`search`/`add`; no signature |
| Lockfile | v1, sorted, content-addressed with a hand-written SHA-256 |
| SBOM | none |
| `audit` | manifest and lock policy; no signatures, no advisory database |
| Reproducible build | `--reproducible` flag exists; no two-directory gate |
| Installers | PowerShell/bash scripts; archives per OS |
| CI | no first-party GitHub Action |

## Deliverables

### Task 1: Semver and a pinned resolver

**Files:**
- Create: `crates/yashy-manifest/src/version.rs`, `crates/yashy-manifest/src/req.rs`
- Modify: `crates/yashy-manifest/src/manifest.rs` (accept a range)
- Modify: `crates/yashy-driver/src/resolve.rs`, `crates/yashy-driver/src/fetch.rs`
- Modify: `crates/yashy-manifest/src/lock.rs` (v2, migrate v1 on read)
- Test: `crates/yashy-manifest/tests/version.rs`, `crates/yashy-driver/tests/resolve.rs`

**Interfaces:**
- `Version { major: u64, minor: u64, patch: u64, pre: Vec<Identifier>, build: Vec<Identifier> }`
- `VersionReq::{Caret, Tilde, Exact, Gte, Lt, Wildcard, All}`
- `resolve(manifest: &Manifest, index: &Index, opts: &ResolveOptions) -> Result<ResolveGraph, ResolveError>`
- `ResolveOptions { locked: bool, offline: bool, features: BTreeSet<String>, target: Triple }`

- [ ] **Step 1: Write the failing test.** SemVer 2.0 precedence cases
  (`1.0.0-alpha < 1.0.0-alpha.1 < 1.0.0`), `^1.2` selects `1.9.0` not `2.0.0`,
  `~1.2` selects `1.2.x`, `--locked` fails when the lock disagrees, and a
  yanked version is refused with a diagnostic naming it.
- [ ] **Step 2: Resolve once, pin exactly.** The resolver walks the index with a
  deterministic order (highest satisfying version, ties by name), then writes
  exact versions and checksums to `yashy.lock`. `update` is the only command
  that changes a pin; `add` resolves once and writes the range plus the pin;
  `build`, `run` and `test` never resolve without `--locked` in CI.
- [ ] **Step 3: One diagnostic family.** A missing version, a conflict, a
  cycle, a yanked pin and a lock mismatch each get a code, a suggestion where
  one exists, and a `--json` form. No solver backtracking loop is unbounded:
  the search is capped and reports the cap.
- [ ] **Step 4: Commit.**

### Task 2: Workspaces

**Files:**
- Modify: `crates/yashy-manifest/src/manifest.rs` (`[workspace]`, `members`, `default-members`)
- Modify: `crates/yashy-driver/src/modules.rs` (resolve a member as one graph)
- Modify: `crates/yashy-driver/src/main.rs` (`--workspace`, `-p <member>`)
- Test: `crates/yashy-driver/tests/workspace.rs`

- [ ] **Step 1: Write the failing test.** A workspace with `app`, `lib` and
  `lib/tests`; `yashyc test --workspace` runs all three, `-p app` runs one,
  `yashyc build` builds the default member, and a member can depend on a sibling
  by path without a registry entry.
- [ ] **Step 2: One lockfile, one target directory.** Members share
  `yashy.lock` and the build cache; a member's package name is unique in the
  workspace; a path dependency outside the workspace is refused unless it is
  explicitly allowed.
- [ ] **Step 3: Commit.**

### Task 3: Features

**Files:**
- Modify: `crates/yashy-manifest/src/manifest.rs` (`[features]`, `default`, optional deps)
- Modify: `crates/yashy-driver/src/resolve.rs` (unification, `--no-default-features`)
- Modify: `crates/yashy-driver/src/cfg.rs` (feature keys; `cfg(feature = "x")` is Phase 4 syntax)
- Test: `crates/yashy-driver/tests/features.rs`

- [ ] **Step 1: Write the failing test.** `default = ["std"]`, an optional
  dependency gated by a feature, `--features tls`, `--no-default-features`.
  Assert the feature set is in the cache key, is recorded in the lockfile, and
  that two members requesting different features unify to the union.
- [ ] **Step 2: Features are deterministic.** The selected set sorts, unions
  and is written to the lockfile; the resolver never chooses a feature on its
  own. A feature name is a `cfg` key (Phase 4 reads it); a misspelt feature is
  a diagnostic, never a false condition.
- [ ] **Step 3: Commit.**

### Task 4: Signed registry v1

**Files:**
- Modify: `crates/yashy-driver/src/registry.rs`, `crates/yashy-driver/src/fetch.rs`, `crates/yashy-driver/src/audit.rs`
- Create: `crates/yashy-manifest/src/signature.rs`, `crates/yashy-manifest/src/sbom.rs`
- Create: `registry/` fixture with `index.json`, `packages/`, `keys/`
- Test: `crates/yashy-driver/tests/registry.rs`, `crates/yashy-driver/tests/registry_http.rs`, `crates/yashy-driver/tests/signature.rs`

**Interfaces:**
- `IndexEntry { name, version, yanked, checksum, dependencies, features, license, sbom, published_at }`
- `SignedIndex { entries: Vec<IndexEntry>, key_id: String, signature: Vec<u8> }`
- `Signature { key_id, algorithm: Ed25519, over: Sha256Digest, bytes: Vec<u8> }`
- `verify_tarball(bytes: &[u8], sig: &Signature, keys: &Keyring) -> Result<(), VerifyError>`

- [ ] **Step 1: Write the failing test.** Publish `demo 1.0.0` to a fixture
  registry, `yashyc add demo`, build. Flip one byte in the tarball and assert
  the build refuses with a signature/checksum diagnostic. Replace the index
  signature and assert the same. Remove a key and assert the failure names the
  key id.
- [ ] **Step 2: Sign the index and every tarball.** The index is a signed JSON
  document; each tarball has a detached signature over its SHA-256. A package
  may carry several signatures; one trusted key is enough, and the trust set is
  explicit (`--trusted-key`, a keyring file, or the built-in release key).
- [ ] **Step 3: One vetted primitive.** Use an audited Ed25519 implementation
  (a single recorded RustCrypto dependency, or a vendored RFC 8032 verifier
  with known-answer tests) — decide at revision, record the choice, and never
  write the primitive by hand. SHA-256 stays the implementation already pinned
  by `lock.rs`.
- [ ] **Step 4: Publish is a reviewable artifact.** `yashyc publish` writes a
  tarball, a signature, an index entry and an SBOM to a staging directory and
  prints the exact file list; a hosted service is still out of scope. The
  registry is a static tree a reviewer can diff.
- [ ] **Step 5: Commit.**

### Task 5: SBOM, `verify-package`, and audit policy

**Files:**
- Modify: `crates/yashy-manifest/src/sbom.rs`, `crates/yashy-driver/src/audit.rs`
- Modify: `crates/yashy-driver/src/main.rs` (`sbom`, `verify-package`)
- Create: `docs/sbom.md`, `docs/registry.md`
- Test: `crates/yashy-driver/tests/audit.rs`, `crates/yashy-driver/tests/sbom.rs`

- [ ] **Step 1: Write the failing test.** `yashyc sbom --format spdx-json`
  lists every package, version, checksum, license, supplier and the dependency
  graph, plus the compiler and runtime hashes. `yashyc verify-package
  demo-1.0.0.tar.gz` checks the signature, the checksum, the index entry and the
  SBOM; each mismatch is a distinct code.
- [ ] **Step 2: Audit has teeth.** `yashyc audit` refuses an unsigned package, a
  package whose SBOM is missing or disagrees with the tarball, a license outside
  the policy, and a dependency outside the allowlist. `--json` is the machine
  form; the policy file is `[audit]` in `yashy.toml`.
- [ ] **Step 3: Commit.**

### Task 6: Reproducible package builds

**Files:**
- Modify: `crates/yashy-driver/src/pipeline.rs` (`--reproducible`)
- Modify: `crates/yashy-manifest/src/sbom.rs` (input hashes)
- Test: `crates/yashy-driver/tests/reproducible.rs` (extend)

- [ ] **Step 1: Write the failing test.** Build the same package in
  `C:\a\demo` and `C:\b\demo` (and `/tmp/a`, `/tmp/b`) with `--reproducible`;
  compare SHA-256. Then vary `SOURCE_DATE_EPOCH`, an absolute path, a locale and
  a username; assert the bytes do not change.
- [ ] **Step 2: Remove every non-input.** Fixed build id, sorted symbol and
  section order, no timestamps, path remapping (`--remap-path-prefix`), stable
  hash seeds, LF-normalised archives with fixed mtimes, and a sorted SBOM. The
  compiler version and target remain inputs and are recorded.
- [ ] **Step 3: Gate it in CI.** `yashyc build --reproducible` twice in two
  directories, compare, and fail on a difference. A package that cannot be
  reproduced is not publishable.
- [ ] **Step 4: Commit.**

### Task 7: Installers, Action, self-update

**Files:**
- Create: `scripts/install/winget/*.yaml`, `scripts/install/homebrew/yashy.rb`, `scripts/install/apt/*`, `scripts/install/scoop/yashy.json`, `scripts/install/Dockerfile`
- Create: `.github/actions/setup-yashy/action.yml`
- Modify: `crates/yashy-driver/src/main.rs` (`self-update`)
- Create: `docs/install.md`
- Test: `scripts/install/test-install.ps1`, `scripts/install/test-install.sh`

- [ ] **Step 1: Write the failing test.** On Windows, Linux and macOS: install
  from the archive, run `yashyc doctor`, install from the package manager
  manifest, uninstall, and assert `yashyc` is gone and the cache is untouched.
- [ ] **Step 2: One artifact, three channels.** Every channel downloads the same
  signed archive and verifies its checksum and signature; a manifest that names
  a different hash is a bug. `yashyc self-update` verifies the signature before
  replacing the binary and keeps the previous binary for rollback.
- [ ] **Step 3: The Action is thin.** `yashy/setup-yashy@v1` installs a pinned
  version, verifies it, caches `~/.yashy/cache` keyed by compiler version and
  target, and adds `yashyc` to `PATH`. No shell beyond the documented steps.
- [ ] **Step 4: Commit.**

### Task 8: Workspace and service templates

**Files:**
- Add: `templates/workspace/`, `templates/service/`, `templates/lib/`
- Modify: `crates/yashy-driver/src/scaffold.rs` (Phase 2 owns `new`)
- Test: `crates/yashy-driver/tests/new.rs` (extend)

- [ ] **Step 1: Write the failing test.** `yashyc new shop --template workspace`
  creates two members, a shared lockfile and a passing `yashyc test
  --workspace`; `--template service` creates a project whose first `yashyc run`
  prints a health response (the HTTP stack itself lands in Phase 6; until then
  the template uses a loopback TCP handler).
- [ ] **Step 2: Templates are tested programs.** Every template is built and
  tested in CI on both OSes; a template that does not pass its own test is
  removed. No placeholder that compiles but lies.
- [ ] **Step 3: Commit.**

### Task 9: The stdlib as a signed, precompiled package

**Files:**
- Modify: `crates/yashy-stdlib/` (package manifest, signature), `crates/yashy-driver/src/modules.rs` (stdlib resolution)
- Modify: `crates/yashy-cache/` (precompiled IR key)
- Test: `crates/yashy-driver/tests/stdlib_precompiled.rs`, `crates/yashy-driver/tests/stdlib.rs`

- [ ] **Step 1: Write the failing test.** A clean checkout builds `hello`
  without re-typechecking an unchanged stdlib module; the precompiled IR is keyed
  by compiler version, target and the stdlib package hash; deleting the cache
  falls back to source and produces the same binary; a stdlib whose hash
  disagrees with the lockfile is refused.
- [ ] **Step 2: The stdlib is versioned and signed like any package.** It has a
  manifest and a signature in the release, and the lockfile records its version
  and hash. A locally modified stdlib is a diagnostic, not a silent rebuild.
- [ ] **Step 3: The cache never serves a stale module.** The key includes the
  feature set and the target; a miss is a normal compile; a hit is verified
  against the key before use. A wrong hit is a P0.
- [ ] **Step 4: Commit.**

## Gates

| Gate | Command | Passes when |
|---|---|---|
| Round trip | `cargo test -p yashy-driver --test registry --test resolve` | publish → add → build; `--locked` refuses a drifted lock; yanked pins refused |
| Tamper | `cargo test -p yashy-driver --test signature --test registry` | one flipped byte, one bad index signature and one missing key each fail with a named code |
| SBOM | `cargo test -p yashy-driver --test sbom --test audit` | every package, hash, license, supplier and toolchain hash present; policy violations refused |
| Reproducible | `cargo test -p yashy-driver --test reproducible` | two directories, two runs, identical SHA-256 on both OSes |
| Stdlib package | `cargo test -p yashy-driver --test stdlib_precompiled` | unchanged stdlib is not re-typechecked; cache key verified; hash mismatch refused |
| Workspaces | `cargo test -p yashy-driver --test workspace --test features` | `--workspace`, `-p`, optional deps and feature unification behave; features are in the cache key |
| Install matrix | `scripts/install/test-install.*` | Windows, Linux and macOS install, verify, run `doctor`, uninstall |
| Action | `.github/actions/setup-yashy` in CI | cold install < 60 s; cache hit < 10 s; pinned version verified |
| No regression | `cargo test --workspace` | green on Windows and Linux |
| No P0 | `docs/triage.md` | no P0 |

## Demo

```console
$ yashyc publish --registry ./registry
wrote demo-1.0.0.tar.gz          sha256:9f2c…  signed key:release-2027
wrote index.json                 signed key:release-2027
wrote demo-1.0.0.spdx.json       14 packages, 2 licenses

$ yashyc add demo --registry ./registry
resolved demo ^1.0 -> 1.0.0 (pinned)
$ yashyc build main.ya
built main  (demo 1.0.0, 3 packages, signature ok)

$ yashyc verify-package demo-1.0.0.tar.gz
signature ok  checksum ok  sbom ok  index ok
```

```console
$ yashyc sbom --format spdx-json | yashyc audit --stdin
ok  14 packages, 0 unsigned, 0 policy violations
```

## Out of scope

- A hosted registry service; the registry is a signed static tree.
- A general SAT solver or lockfile-less resolution; the lockfile remains exact.
- Binary caching across machines (Phase 11B).
- Publishing to a public service automatically; publishing is an explicit
  command or a reviewed pull request.
- macOS as a claimed platform until the signed archive gate in Phase 14.

## Risks

| Risk | Mitigation |
|---|---|
| Semver ranges break exact-pin guarantees | The lockfile stays exact; ranges resolve once; `--locked` in CI; `update` is explicit |
| A solver hangs or explodes | Deterministic order, capped search, a diagnostic naming the cap |
| Signature verification is homegrown and wrong | Use a vetted Ed25519 implementation; known-answer and negative tests; no hand-written primitive |
| Reproducibility is claimed but a path leaks | Two-directory gate on both OSes; `SOURCE_DATE_EPOCH`, locale and username variations tested |
| Installers drift from the archive | Every channel downloads the same signed archive and verifies the same hash |
| A feature flag changes the cache key silently | Features are in the key, in the lockfile and in `--print-cfg`; mutation tests prove a miss |
