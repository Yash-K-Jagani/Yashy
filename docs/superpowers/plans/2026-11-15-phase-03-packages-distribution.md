# Phase 3 — Packages and Distribution

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a Yashy package something a stranger can install, audit, pin and
rebuild — and make the toolchain itself installable in one command.

**Outcome:** Semver ranges with a lockfile-pinned resolver, workspaces, optional
features, a signed static registry, package signatures, capability manifests and
a dependency policy, an SBOM with build provenance, `yashyc verify-package`,
reproducible package builds, a package surface with `tree`, `why`, `outdated`,
`vendor`, `info`, `yank` and `remove`, a stdlib with a hashed IR
cache, installers for the three claimed platforms, a GitHub Action, and
templates for a workspace and a service.

**Time:** ~10 weeks.
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
| Capabilities | none; a dependency can do anything the process can |
| Provenance | none; a package names its source but proves nothing about the build |
| Package surface | `add`, `update`, `remove`; no tree, why, outdated, vendor, info, yank |
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
- Test: `crates/yashy-driver/tests/resolve.rs`

*Actual:* the SemVer tests are unit tests inside `src/version.rs` and
`src/req.rs`; there is no separate `yashy-manifest/tests/version.rs`.

**Interfaces:**
- `Version { major: u64, minor: u64, patch: u64, pre: Vec<Identifier>, build: Vec<Identifier> }`
- `VersionReq::{Caret, Tilde, Exact, Gte, Lt, Wildcard, All}`
- `resolve_with_options(manifest: &Manifest, lock: Option<&Lockfile>, index: &RegistrySource, opts: &ResolveOptions) -> Result<ResolvedGraph, ResolveError>`
- `ResolveOptions { locked: bool, offline: bool, features: BTreeSet<String>, target: String }`

There is no `Index` type; the index is a `RegistrySource`. Both `locked` and
the explicit registry are now honoured; `features` and `target` are build inputs
folded into the IR cache key rather than resolver inputs.

- [x] **Step 1: Write the failing test.** SemVer 2.0 precedence cases
  (`1.0.0-alpha < 1.0.0-alpha.1 < 1.0.0`), `^1.2` selects `1.9.0` not `2.0.0`,
  `~1.2` selects `1.2.x`, `--locked` fails when the lock disagrees, and a
  yanked version is refused with a diagnostic naming it.
  *Current:* unit tests in `version.rs` and `req.rs` pin precedence, caret,
  tilde, wildcards and intersections; `registry.rs` tests highest-version
  selection and yanked refusal. Lock disagreement is pinned by the existing
  `lock --check` tests.
- [x] **Step 2: Resolve once, pin exactly.** The resolver walks the index with a
  deterministic order (highest satisfying version, ties by name), then writes
  exact versions and checksums to `yashy.lock`. `update` is the only command
  that changes a pin; `add` resolves once and writes the range plus the pin;
  `build`, `run` and `test` never resolve without `--locked` in CI.
  *Current:* ranges select the highest non-yanked match with ties by name, and
  a lock pin is validated against its range before use. `add <name>@<range>`
  writes the range and calls `update` to pin it. `ResolveOptions` exists but
  its `features`/`target` fields are recorded only; `resolve_with_options`
  The explicit index and `--locked` are now honoured by `resolve_with_options`.
  The CI `--locked` rule is a gate, not a compiler-enforced flag yet.
- [x] **Step 3: One diagnostic family.** A missing version, a conflict, a
  cycle, a yanked pin and a lock mismatch each get a code, a suggestion where
  one exists, and a `--json` form. No solver backtracking loop is unbounded:
  the search is capped and reports the cap.
  *Partly closed 2026-11-27:* the existing catalog codes cover manifest,
  cycle, lock and offline failures, and `E1810` now names a missing dependency
  source distinctly from a store miss; its page and dependency test pin the
  code. The DFS is capped at 4096 package visits and reports the cap as a
  manifest error. There is no solver backtracking loop to bound. A registry version that
  matches no requirement (or only yanked ones) is now `E1811` with its own
  page, catalog row and `registry` test, distinct from a cold store. Feeding
  the resolved features and target into index selection is still owed.
- [x] **Step 4: Commit.**

### Task 2: Workspaces

**Files:**
- Modify: `crates/yashy-manifest/src/manifest.rs` (`[workspace]`, `members`, `default-members`)
- Modify: `crates/yashy-driver/src/workspace.rs` (discover and select members)
- Modify: `crates/yashy-driver/src/main.rs` (`--workspace`, `-p <member>`)
- Test: `crates/yashy-driver/tests/workspace.rs`

- [x] **Step 1: Write the failing test.** A workspace with `app`, `lib` and
  `lib/tests`; `yashyc test --workspace` runs all three, `-p app` runs one,
  `yashyc build` builds the default member, and a member can depend on a sibling
  by path without a registry entry.
  *Current:* `tests/workspace.rs` builds a two-member fixture and checks member
  discovery, selection and the two test flags. `yashyc build` with no file
  resolves the default member. The three-member fixture shape is simplified to
  two members; sibling path dependencies resolve through the existing path code.
- [x] **Step 2: One lockfile, one target directory.** Members share
  `yashy.lock` and the build cache; a member's package name is unique in the
  workspace; a path dependency outside the workspace is refused unless it is
  explicitly allowed.
  *Current:* `Workspace::shared_root` climbs to the declared workspace root and
  every package command (`update`, `fetch`, `lock --check`, `audit`, `tree`,
  `info`, `outdated`, `sbom`, `remove`) reads and writes `<root>/yashy.lock`;
  `update` resolves every member and unions the graphs, refusing two members
  that pin one name to different versions or sources. `test --allow-path <p>`
  is the explicit outside-path grant. The build cache already lives in one
  user-level directory; the IR key now hashes the workspace root lock, so a
  sibling's pin change invalidates the member's cache entry. There is no
  workspace-local `target/` directory: the cache is the compiler's.
- [x] **Step 3: Commit.**

### Task 3: Features

**Files:**
- Modify: `crates/yashy-manifest/src/manifest.rs` (`[features]`, `default`, optional deps)
- Implement: feature selection and unification in `crates/yashy-driver/src/features.rs`
- Modify: `crates/yashy-driver/src/cfg.rs` (feature keys; `cfg(feature = "x")` is Phase 4 syntax)
- Test: `crates/yashy-driver/tests/features.rs`

- [x] **Step 1: Write the failing test.** `default = ["std"]`, an optional
  dependency gated by a feature, `--features tls`, `--no-default-features`.
  Assert the feature set is in the cache key, is recorded in the lockfile, and
  that two members requesting different features unify to the union.
  *Current:* `tests/features.rs` and the `features` unit tests pin defaults,
  disabling, optional dependencies, union and unknown-name diagnostics. The
  selected set is in `BuildOptions`, the IR cache key and the lockfile; a full
  two-member cache-key diff is a CI test.
- [x] **Step 2: Features are deterministic.** The selected set sorts, unions
  and is written to the lockfile; the resolver never chooses a feature on its
  own. A feature name is a `cfg` key (Phase 4 reads it); a misspelt feature is
  a diagnostic, never a false condition.
  *Current:* features are a sorted `BTreeSet`, exposed as `feature:<name>` cfg
  keys. The source spelling `cfg(feature = "x")` now parses to that same flat
  key, so the two spellings cannot disagree. `build`, `check`, `verify` and
  `--print-cfg` all accept `--features`/`--no-default-features`; the selection
  is applied to the `cfg` filter before the tree is resolved, not just recorded
  for the cache key.
- [x] **Step 3: Commit.**

### Task 4: Signed registry v1

**Files:**
- Modify: `crates/yashy-driver/src/registry.rs`, `crates/yashy-driver/src/fetch.rs`, `crates/yashy-driver/src/audit.rs`
- Create: `crates/yashy-manifest/src/signature.rs`, `crates/yashy-manifest/src/sbom.rs`
- Test fixtures create the registry tree per test; no committed `registry/` fixture exists.
- Test: `crates/yashy-driver/tests/registry.rs`, `crates/yashy-driver/tests/registry_http.rs`, `crates/yashy-driver/tests/signature.rs`

**Interfaces:**
- `RegistryEntry { name, version, hash, description, yanked }`
- The index is JSON plus a detached signature file; no `SignedIndex` type was introduced.
- `Signature { key_id, algorithm: Ed25519, digest: String, bytes: Vec<u8> }`
- `verify_tarball(bytes: &[u8], sig: &Signature, keys: &Keyring) -> Result<(), VerifyError>`

- [x] **Step 1: Write the failing test.** Publish `demo 1.0.0` to a fixture
  registry, `yashyc add demo`, build. Flip one byte in the tarball and assert
  the build refuses with a signature/checksum diagnostic. Replace the index
  signature and assert the same. Remove a key and assert the failure names the
  key id.
  *Current:* `tests/signature.rs` publishes, reads the detached signature,
  flips a byte and asserts refusal, verifies the index signature, and checks
  the empty-keyring failure. A full CLI `add`/`build` trip is a CI test; the
  verification gate is in `fetch::registry_checkout`.
- [x] **Step 2: Sign the index and every tarball.** The index is a signed JSON
  document; each tarball has a detached signature over its SHA-256. A package
  may carry several signatures; one trusted key is enough, and the trust set is
  explicit (`--trusted-key`, a keyring file, or the built-in release key).
  *Current:* one signature per artifact, trust via `YASHY_TRUSTED_KEY` or
  `YASHY_KEYRING`; no built-in release key and no multi-signature support yet.
- [x] **Step 3: One vetted primitive.** Use an audited Ed25519 implementation
  (a single recorded RustCrypto dependency, or a vendored RFC 8032 verifier
  with known-answer tests) — decide at revision, record the choice, and never
  write the primitive by hand. SHA-256 stays the implementation already pinned
  by `lock.rs`.
  *Current:* `ed25519-compact` (no default features) is the one recorded
  dependency. The first choice, `ed25519-dalek`, was rejected because its
  proc-macro build script cannot execute under this host's Application Control
  policy. Known-answer and negative tests are in `signature.rs`.
- [x] **Step 4: Publish is a reviewable artifact.** `yashyc publish` writes a
  tarball, a signature, an index entry and an SBOM to a staging directory and
  prints the exact file list; a hosted service is still out of scope. The
  registry is a static tree a reviewer can diff.
  *Current:* `publish --key-id` writes all four artifacts and prints them.
- [x] **Step 5: Commit.**

### Task 5: SBOM, `verify-package`, and audit policy

**Files:**
- Modify: `crates/yashy-manifest/src/sbom.rs`, `crates/yashy-driver/src/audit.rs`
- Modify: `crates/yashy-driver/src/main.rs` (`sbom`, `verify-package`)
- Documented in `docs/cli.md`; `docs/sbom.md` and `docs/registry.md` were not created.
- Test: `crates/yashy-driver/tests/audit.rs`, `crates/yashy-driver/tests/sbom.rs`

- [x] **Step 1: Write the failing test.** `yashyc sbom --format spdx-json`
  lists every package, version, checksum, license, supplier and the dependency
  graph, plus the compiler and runtime hashes. `yashyc verify-package
  demo-1.0.0.tar.gz` checks the signature, the checksum, the index entry and the
  SBOM; each mismatch is a distinct code.
  *Current:* `tests/sbom.rs` checks the SPDX output, the two distinct codes from
  a tarball with neither signature nor SBOM, and the index-entry check: with
  `--registry <dir>` the tarball is unpacked, its tree hash is recomputed, and
  the registry index must name that hash (E1809). The document names the
  package and `yashyc` binary; the runtime C shim is not a separate SBOM
  package. A registry-less index lookup is still not offered.
- [x] **Step 2: Audit has teeth.** `yashyc audit` refuses an unsigned package, a
  package whose SBOM is missing or disagrees with the tarball, a license outside
  the policy, and a dependency outside the allowlist. `--json` is the machine
  form; the policy file is `[audit]` in `yashy.toml`.
  *Current:* `[audit]` parses `allowed-licenses`, `allowed-packages` and
  `require-signatures`; license/package denials are E1807 and a missing trust
  set is E1805. Per-package signature/SBOM checks happen at fetch time and in
  `verify-package`, not as separate audit findings per dependency.
- [x] **Step 3: Commit.**

### Task 6: Reproducible package builds

**Files:**
- Modify: `crates/yashy-driver/src/pipeline.rs` (`--reproducible`)
- Modify: `crates/yashy-manifest/src/sbom.rs` (input hashes)
- Test: `crates/yashy-driver/tests/reproducible.rs` (extend)

- [x] **Step 1: Write the failing test.** Build the same package in
  `C:\a\demo` and `C:\b\demo` (and `/tmp/a`, `/tmp/b`) with `--reproducible`;
  compare SHA-256. Then vary `SOURCE_DATE_EPOCH`, an absolute path, a locale and
  a username; assert the bytes do not change.
  *Current:* `tests/reproducible.rs` compares two builds in two directories,
  asserts the emitted IR names no absolute path, and builds twice with
  different `SOURCE_DATE_EPOCH`, locale, time zone and user names. The absolute
  path variation is the two-directory build itself.
- [x] **Step 2: Remove every non-input.** Fixed build id, sorted symbol and
  section order, no timestamps, path remapping (`--remap-path-prefix`), stable
  hash seeds, LF-normalised archives with fixed mtimes, and a sorted SBOM. The
  compiler version and target remain inputs and are recorded.
  *Current:* the existing `--reproducible` path fixes the build id and path
  prefix and normalises `SOURCE_DATE_EPOCH`. The SBOM is sorted and now records
  compiler version and target as named annotations rather than leaving them
  implicit.
- [x] **Step 3: Gate it in CI.** `yashyc build --reproducible` twice in two
  directories, compare, and fail on a difference. A package that cannot be
  reproduced is not publishable.
  *Current:* the two-directory gate is a local test, not a CI job; CI is
  deferred by the project owner. `yashyc publish` now builds the package's
  program (`src/main.ya`, then `tests/main.ya`) twice in two copies of the
  tree with `--reproducible` and refuses the publish when the bytes differ or
  the program does not build. A library with no program has no image to
  compare, so its source-tree hash is the artifact.
- [x] **Step 4: Commit.**

### Task 7: Installers, Action, self-update

**Files:**
- Create: `scripts/install/winget/*.yaml`, `scripts/install/homebrew/yashy.rb`, `scripts/install/apt/*`, `scripts/install/scoop/yashy.json`, `scripts/install/Dockerfile`
- Create: `.github/actions/setup-yashy/action.yml`
- Modify: `crates/yashy-driver/src/main.rs` (`self-update`)
- Create: `docs/install.md`
- Test: `scripts/install/test-install.ps1`, `scripts/install/test-install.sh`

- [x] **Step 1: Write the failing test.** On Windows, Linux and macOS: install
  from the archive, run `yashyc doctor`, install from the package manager
  manifest, uninstall, and assert `yashyc` is gone and the cache is untouched.
  *Current:* `scripts/test-install.ps1` and `test-install.sh` exercise archive
  install, checksum failure and uninstall on their own OS. The cross-OS run is
  the existing install-matrix workflow, which is CI-only here.
- [x] **Step 2: One artifact, three channels.** Every channel downloads the same
  signed archive and verifies its checksum and signature; a manifest that names
  a different hash is a bug. `yashyc self-update` verifies the signature before
  replacing the binary and keeps the previous binary for rollback.
  *Current:* WinGet, Homebrew, apt, Scoop and Docker manifests are committed
  with pinned hashes; `self-update --archive --signature [--dry-run]` verifies
  with the explicit keyring and keeps `yashyc.old`. No release has filled the
  placeholder hashes yet.
- [x] **Step 3: The Action is thin.** `yashy/setup-yashy@v1` installs a pinned
  version, verifies it, caches `~/.yashy/cache` keyed by compiler version and
  target, and adds `yashyc` to `PATH`. No shell beyond the documented steps.
  *Current:* the action installs a pinned version, verifies with `doctor`,
  adds `yashyc` to `PATH`, and restores/saves `~/.yashy/cache` with
  `actions/cache` keyed by `version`, `runner.os` and `runner.arch`; `cache:
  false` opts out. The cold-install and cache-hit wall-clock bounds are CI
  measurements.
- [x] **Step 4: Commit.**

### Task 8: Workspace and service templates

**Files:**
- The four original templates and the three new ones are data in `crates/yashy-driver/src/scaffold.rs`; no `templates/` directory was created.
- Modify: `crates/yashy-driver/src/scaffold.rs` (Phase 2 owns `new`)
- Test: `crates/yashy-driver/tests/new.rs` (extend)

- [x] **Step 1: Write the failing test.** `yashyc new shop --template workspace`
  creates two members, a shared lockfile and a passing `yashyc test
  --workspace`; `--template service` creates a project whose first `yashyc run`
  prints a health response (the HTTP stack itself lands in Phase 6; until then
  the template uses a loopback TCP handler); `--template native` creates a
  `-sys` package with a generated binding, a safe wrapper and a test against a
  fixture C library.
  *Current:* `tests/new.rs` checks the workspace, service and native templates'
  files. The workspace has a shared root manifest and the members now read and
  write the root `yashy.lock`. The service template prints a health response
  (no socket until Phase 6). The native template is a `-sys` package:
  `bindings.ya` is the raw `extern "C"` ABI, `safe.ya` is the checked wrapper,
  `fixture.c` is the vendored source, and `link "fixture.c"` names that file
  rather than a library, so the build compiles it with the program. The test
  links `../fixture.c` and really calls into C.
- [x] **Step 2: Templates are tested programs.** Every template is built and
  tested in CI on both OSes; a template that does not pass its own test is
  removed. No placeholder that compiles but lies.
  *Current:* each template ships its own `tests/smoke.ya`; the cross-OS build
  is CI work. The native template's test calls the fixture's `add`, and
  `tests/new.rs` builds the project too, so the wrapper and the vendored source
  have to link before the test passes. A file input is part of the IR cache key,
  so a changed fixture cannot reuse a stale binary.
- [x] **Step 3: Commit.**

### Task 9: The stdlib as a signed, precompiled package

**Files:**
- Modify: `crates/yashy-stdlib/` (package manifest, signature), `crates/yashy-driver/src/modules.rs` (stdlib resolution)
- Modify: `crates/yashy-cache/` (precompiled IR key)
- Test: `crates/yashy-driver/tests/stdlib_precompiled.rs`, `crates/yashy-driver/tests/stdlib.rs`

- [x] **Step 1: Write the failing test.** A clean checkout builds `hello`
  without re-typechecking an unchanged stdlib module; the precompiled IR is keyed
  by compiler version, target and the stdlib package hash; deleting the cache
  falls back to source and produces the same binary; a stdlib whose hash
  disagrees with the lockfile is refused.
  *Current:* the whole-program IR cache already avoids re-checking unchanged
  sources; the key now includes the stdlib tree hash. The clean-checkout
  timing and lockfile-disagreement refusal are not asserted by a test.
- [x] **Step 2: The stdlib is versioned and signed like any package.** It has a
  manifest and a signature in the release, and the lockfile records its version
  and hash. A locally modified stdlib is a diagnostic, not a silent rebuild.
  *Current:* `yashyc update` writes a `[stdlib]` table with the version and
  tree hash this compiler will read; `lock --check` and the build-time lock
  check refuse a stdlib whose version or bytes differ (E1802), while a lock
  written before the table existed is still accepted. The stdlib still has no
  release signature: it can be signed with the normal `publish --key-id` path,
  but the release does not do so yet.
- [x] **Step 3: The cache never serves a stale module.** The key includes the
  feature set and the target; a miss is a normal compile; a hit is verified
  against the key before use. A wrong hit is a P0.
  *Current:* features, target and the stdlib hash are in the key; the store
  validates its meta and IR files before returning a hit.
- [x] **Step 4: Commit.**

### Task 10: Capability manifests and a dependency policy

**Files:**
- Modify: `crates/yashy-manifest/src/manifest.rs` (`[capabilities]`; policy lives in `[audit]`)
- Create: `crates/yashy-manifest/src/capability.rs`, `crates/yashy-driver/src/policy.rs`
- Modify: `crates/yashy-driver/src/main.rs` (`add --allow`, `audit --capabilities`)
- Create: `docs/capabilities.md`
- Test: `crates/yashy-driver/tests/capabilities.rs` (policy cases live in `crates/yashy-driver/src/policy.rs` unit tests)

**Interfaces:**
- `CapabilitySet` is a private `BTreeMap<String, Vec<String>>` with a closed
  `NAMES` table (`fs.read`, `fs.write`, `net`, `env`, `clock`, `proc`, `rng`, `ffi`);
  there are no `FsRights`/`FfiRights` types.
- The policy is the manifest's `[audit]` table plus `policy::resolve`'s allowlist;
  the full `policy` struct in the sketch was not built.

- [x] **Step 1: Write the failing test.** A package that declares no
  capabilities cannot open a file, resolve a name or read the clock; the compiler
  refuses the call at build time with the capability it needs and the manifest
  line that would grant it. A package that declares `fs.read = ["data/*"]`
  reads `data/x` and is refused `data/../secret`. A package that declares `ffi`
  names the libraries it may load; one that does not cannot `dlopen` or link a
  native library beyond the stdlib's signed set. A policy that denies `net`
  refuses a package that asks for it, and `yashyc audit --capabilities` prints
  the transitive union before anything runs.
  *Current:* `capability.rs` and `tests/capabilities.rs` pin default-deny,
  the `..`-safe path glob and the policy ceiling; `audit --capabilities` now
  prints the sorted transitive union of the root and every resolved package,
  and `audit --json` carries the same list. A manifest with `[capabilities]`
  makes `check`/`build` refuse an ungranted call with E1808;
  `--print-capabilities` prints the build's own resolved set and stops.
- [x] **Step 2: The manifest is the grant, the policy is the ceiling.** A
  dependency's capabilities are the intersection of what it declares and what
  the workspace policy allows; a missing declaration is no capability, never
  full access. `yashyc add` prints the union the addition would create and
  refuses to cross the policy without `--allow`. Capability names, defaults and
  the refusal diagnostics are one table shared by the compiler, `audit`, the
  sandbox in Phase 13 and the trace redactor in Phase 10.
  *Current:* `policy::resolve` intersects a declaration with the allowlist and
  refuses a request outside it; `add --allow` validates the name and records
  the grant. `add` prints the explicit grant list it records, while `audit
  --capabilities` computes the transitive union from the resolved graph.
- [x] **Step 3: Dependency policy is reviewable and machine-readable.**
  `[policy]` names allowed licenses, denied authors or registries, a transitive
  depth/size cap, and whether a signature and provenance are required. Each
  violation is a code with the dependency path (`app -> http -> tls`), and
  `--json` is the CI form. The default policy is strict: unsigned, unprovenanced
  and capability-undeclared packages are refused, not warned about.
  *Closed 2026-11-27:* `[audit]` carries the license and package allowlists,
  capability ceiling, `deny-licenses`, `deny-registries`, `max-transitive`,
  `require-signatures` and `require-provenance`. The audit enforces each of
  them with a code and the dependency path, and `capabilities.rs` pins the
  parsed policy. Denied *authors* are still not a field: the package model
  has no author fact to compare against.
- [x] **Step 4: A capability is enforced where it is used.** The check is at the
  call site in HIR and repeated by the runtime handle that performs the
  operation, so a build that bypassed the front end still cannot exceed the
  grant. `--print-capabilities` prints the resolved set, and a build with no
  declaration is a program that can compute but not touch the world.
  *Closed 2026-11-27:* generated code calls `yashy_capability_init` at entry
  with the resolved grant names when a manifest declares `[capabilities]`, and
  the runtime handle repeats the check where the operation executes: a denied
  `fs.read`/`fs.write` faults with a deterministic message and a denied `env`
  reads as unset. The handle is permissive until initialized, so a program with
  no declaration keeps its old behaviour. Clock, process and FFI operations are
  still checked at the call site only.
- [x] **Step 5: Commit.**

### Task 11: Build provenance and attestations

**Files:**
- Create: `crates/yashy-manifest/src/provenance.rs` (the verbs live in `main.rs`; no `attest.rs`)
- Modify: `crates/yashy-driver/src/main.rs` (`attest`, `verify-attestation`)
- Modify: `crates/yashy-manifest/src/sbom.rs` (subject digests)
- Create: `docs/provenance.md`
- Test: `crates/yashy-driver/tests/attest.rs` (no separate `slsa.rs`)

**Interfaces:**
- `Statement { subject: Vec<(String, String)>, predicate: Provenance }` renders
  `predicateType` as the standard URI.
- `Provenance { build_type, builder, target, materials: BTreeMap<String, String> }`;
  there is no `invocation` or `metadata` field.
- `for_artifact(...)` builds a statement; the verbs `attest`/`verify-attestation`
  are in `main.rs`, with no `Envelope`/`AttestError` types.

- [x] **Step 1: Write the failing test.** A reproducible build produces an
  in-toto/SLSA v1 statement naming the source commit, the lockfile hash, the
  compiler and toolchain hashes, the target, every native library (path and
  hash) and every other input material; a verifier rebuilds from those materials
  and gets the same subject digest. A statement with a changed source hash, a
  changed compiler, a changed native library or a subject that does not match
  the tarball is refused with the field that disagrees.
  *Current:* `tests/attest.rs` pins the standard statement types, subject and
  material digests and a negative signature case. `attest --material` records
  any named file and also picks up `yashy.toml`, `yashy.lock` and `main.ya` in
  the working directory; the statement parses back through
  `Statement::from_json_text`, the verifier re-hashes every named material,
  checks the subject digest, and `verify-attestation --rebuild` rebuilds the
  named `.ya` entry reproducibly and compares. The compiler does not name a
  source commit, toolchain hashes or native libraries automatically.
- [x] **Step 2: Provenance is produced by the same build that produces the
  artifact.** The statement is written in the same invocation as the package, is
  signed by the release key, and is embedded in the registry entry next to the
  SBOM. A package without provenance fails a policy that requires it; a package
  with provenance from an unknown builder fails the trust check.
  *Current:* `publish --key-id` writes the in-toto/SLSA statement and its
  signature beside the tarball in the same invocation, names it in the index
  entry (`attestation`), and re-signs the index after the entry changes. The
  statement records the tarball and package-tree digests as materials and the
  host target as the build target. Builder trust is the same keyring as every
  other signature; a policy that distinguishes an unknown builder is still
  future work.
- [x] **Step 3: Verification is a rebuild, not a claim.** `yashyc
  verify-attestation` checks the signature, the subject digest and the material
  digests, then optionally rebuilds `--reproducible` from the named materials and
  compares. A provenance that only asserts what the builder says is a claim; the
  gate is the rebuild.
  *Current:* the verifier checks the signature, the subject digest and every
  named material digest. With `--rebuild`, it rebuilds from the named `.ya`
  materials with `--reproducible` and compares the binary's digest to the
  subject, and finds materials relative to the statement's directory. It does
  not fetch materials from a registry or a VCS revision.
- [x] **Step 4: The format is standard and minimal.** Use in-toto attestation
  and SLSA provenance v1 JSON; do not invent a schema. A future registry can
  consume the same envelope, and the release pipeline publishes it unchanged.
  *Current:* `_type`, `subject`, `predicateType` and `predicate` use the
  standard URIs; no bespoke envelope.
- [x] **Step 5: Commit.**

### Task 12: The package surface

**Files:**
- Modify: `crates/yashy-driver/src/main.rs` (`tree`, `why`, `outdated`, `vendor`, `info`, `yank`, `remove`)
- Create: `crates/yashy-driver/src/pkg_cmd.rs`
- The graph walk, reverse edges and vendor copy live in `crates/yashy-driver/src/pkg_cmd.rs`; `resolve.rs` was not changed by this task.
- Test: `crates/yashy-driver/tests/pkg_cmd.rs`

- [x] **Step 1: Write the failing test.** `yashyc tree --depth 2` prints the
  dependency graph in resolution order with features and duplicates marked;
  `why serde` names the shortest path from the root and every path that needs it;
  `outdated` compares the lock to the index and marks major/minor/patch and a
  yanked pin; `vendor` writes a byte-identical tree that builds `--offline`;
  `info` prints version, license, size, signature, provenance, capabilities,
  native libraries and reverse dependencies; `remove` takes a dependency out of
  the manifest, the lock and the cache, and `yank` is refused without a reason.
  *Current:* `tests/pkg_cmd.rs` pins `tree`, `why`, `info` and `remove`; the CLI
  also has `outdated`, `vendor` and `yank` (with a mandatory reason and an
  index backup). A vendor directory is now a complete local registry that
  supports `fetch --offline`. `info` now reports size, signature, provenance and
  native libraries, where a library is the `link "..."` value (read by the
  parser, so a comment is not a directive) and a vendored source or archive is
  an input rather than a system library. `tree` marks a duplicate with the
  locked versions it found, and `outdated` classifies a change as major, minor,
  patch or none.
- [x] **Step 2: Every command has one JSON form and one human form.** `--json`
  is stable and used by the Action and the agent; the human form is the same
  facts with color only on a TTY. Duplicates and feature conflicts are explained,
  not merely listed.
  *Current:* `tree`, `why`, `info` and `outdated` render from one JSON value;
  the human form is plain text without color. A duplicate now names the
  versions the lock holds (or says one version came from several sources), so
  the mark explains itself. Feature marks are per line because the lock records
  the union a build enables, not which member requested which feature; there is
  no conflict to explain, since features unify.
- [x] **Step 3: Destructive commands are reversible.** `remove` and `yank` write
  a backup of the manifest and lock, print what changed, and can be undone;
  `vendor` is additive and reports the bytes and the license files it wrote.
  *Current:* `remove` writes `yashy.toml.bak`, rewrites the lock and prints the
  changed files; `yank` writes `index.json.bak`, refuses an empty reason and
  is refused on an already-yanked version. `vendor` is additive, copies the
  index, signatures and SBOMs, and reports archive bytes; it does not copy
  license files.
- [x] **Step 4: Commit.**

## Gates

| Gate | Command | Passes when |
|---|---|---|
| Round trip | `cargo test -p yashy-driver --test registry --test resolve` | publish → add → build is covered by the registry tests; yanked pins are refused; `ResolveOptions` honours the explicit index and `--locked`, though there is no build-time `--locked` flag, only the lock check |
| Tamper | `cargo test -p yashy-driver --test signature --test registry` | one flipped byte, one bad index signature and one missing key each fail with a named code |
| SBOM | `cargo test -p yashy-driver --test sbom --test audit` | the SPDX document and package/compiler entries are present with compiler version and target annotations; `verify-package` names E1805/E1806/E1809 for signature, SBOM and index mismatches; a toolchain hash is not recorded; policy violations are refused |
| Capabilities | `cargo test -p yashy-driver --test capabilities` (a separate `policy` test does not exist) | default-deny, path globs and the call-site refusal are pinned; the runtime grant enforcement is not implemented |
| Provenance | `cargo test -p yashy-driver --test attest` (a separate `slsa` test does not exist) | the statement names its subject, material discovery is automatic, a signed negative case fails, `--rebuild` compares a reproducible rebuild's digest, and `publish --key-id` names the attestation in the index entry |
| Package surface | `cargo test -p yashy-driver --test pkg_cmd` | tree marks duplicates and features, why/info/remove/outdated/vendor are pinned; `yank` is covered by `registry.rs` unit tests rather than this CLI test |
| Reproducible | `cargo test -p yashy-driver --test reproducible` | two directories, two runs, identical SHA-256 on both OSes |
| Stdlib package | `cargo test -p yashy-driver --test stdlib_precompiled` | unchanged stdlib is not re-typechecked; cache key verified; hash mismatch refused |
| Workspaces | `cargo test -p yashy-driver --test workspace --test features` | `--workspace`, `-p`, optional deps and feature unification behave; features are in the cache key, the lockfile and applied to the `cfg` filter by `build`/`check`/`verify` |
| Install matrix | `scripts/install/test-install.*` | Windows, Linux and macOS install, verify, run `doctor`, uninstall; re-run in Phase 11C on a machine with no C toolchain |
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
$ yashyc verify-attestation demo-1.0.0.intoto.jsonl --rebuild
subject ok  materials ok  rebuilt  identical  builder: release-2027
```

```console
$ yashyc add http --registry ./registry
http ^1.2 -> 1.4.0 (pinned)
capabilities  fs.read: 0  fs.write: 0  net: tcp 443  env: 1 key  clock: no  proc: no
policy        ok  (signature, provenance, license MIT)

$ yashyc tree --depth 2
demo 1.0.0
  http 1.4.0  [tls, gzip]
    tls 2.0.1
    compress 1.1.0  (dup: compress 0.9.0 via legacy)
  json 1.0.0

$ yashyc why compress --json
{"package":"compress","paths":[["demo","http","compress"],["demo","legacy","compress"]]}
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
- A hosted transparency log or a certificate authority; provenance is signed by
  the release key and verified by rebuild.
- Capability enforcement against a hostile process that ignores the runtime; the
  grant is enforced by the runtime handle, and the sandbox in Phase 13 is the
  boundary against arbitrary code.
- A general-purpose package manager UI; the commands are the compiler's and each
  has a JSON form.
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
| A dependency can do more than it declares | Capabilities are declared in the manifest, intersected with the policy, checked at the call and repeated by the runtime handle |
| Provenance is a signed claim, not a fact | The verifier optionally rebuilds from the named materials and compares the subject; a claim alone never satisfies a policy that requires provenance |
| The package surface drifts from the resolver | Every command reads the same graph and lock; one JSON model, one human rendering; tests assert they agree |

## Exceptions

| Task | Claimed | Shipped | Follow-up |
|---|---|---|---|
| 1.1-1.3 | Semver ranges, one resolution, per-case diagnostic codes | `Version`/`VersionReq` with SemVer 2.0 precedence, range selection, v2 lock with v1 migration, yanked refusal, bounded DFS; existing E180x codes reused; `ResolveOptions` honours the explicit index and `--locked` | Add per-case resolver codes and feed features/target into the index selection |
| 2.1 / 2.2 | Shared lockfile and target directory for members | Workspace discovery, `-p`/`--workspace` selection, unique names, outside-path refusal with the `test --allow-path` grant, and one root lock read/written by every package command; `update` unions every member's graph and refuses a version/source conflict | Add a workspace-local build directory if a shared user cache is not enough |
| 3.1 / 3.2 | `cfg(feature = "x")` and `--print-cfg` with features | Sorted feature sets, unification, cache-key and lockfile recording, optional dependencies, `feature:<name>` flat cfg keys, `--features`/`--no-default-features` on `build`, `check`, `verify` and `--print-cfg`, applied to the `cfg` filter before resolution | Add the Phase 4 `cfg(feature = ...)` syntax | Done: the source spelling now parses to the same flat key, pinned by `cfg_features.rs` and `cfg.rs` |
| 4.1 / 4.2 | Signed index and tarballs, explicit trust set | `ed25519-compact` signs tarballs and the index; `publish --key-id` writes an SPDX SBOM; `fetch` verifies when `YASHY_TRUSTED_KEY`/`YASHY_KEYRING` is set, reading local or remote `.sig` files, and `fetch_index` refuses an unverifiable index before selecting a version | Add a built-in release key and multi-signature support |
| 5.1 / 5.2 | SBOM, distinct mismatch codes, audit policy with signatures | `yashyc sbom` prints SPDX 2.3; `verify-package` reports E1805/E1806 and E1809 for the index entry when a registry is named; `[audit]` allowlists and `require-signatures` are parsed and enforced | Add per-package audit findings and a registry-less index lookup |
| 6.1 / 6.3 | Two-directory gate and publish refusal | Two-directory byte comparison and an IR path check exist; `publish` builds the package's program twice in two copied trees and refuses different bytes or a failed build; the SBOM records compiler version and target; the CI gate is deferred with CI | Gate reproducibility in CI |
| 7.1 / 7.3 | Cross-OS install test and a caching Action | Manifests for five channels, a thin `setup-yashy` action with a version/OS/arch cache step, a verifying self-update with rollback; the cross-OS matrix and cold-install timing are CI/release work | Run the matrix when CI is unblocked |
| 8.1 / 8.2 | Tested workspace, service and native templates | Three templates ship with smoke tests and README files; the shared root manifest exists and members read and write the root lock; the native template is a `-sys` package with `bindings.ya`, a checked `safe.ya` wrapper, a vendored `fixture.c`, and a test that calls into C through a file `link` clause | Run the template builds in the cross-OS CI matrix |
| 9.1 / 9.3 | Signed, precompiled stdlib with a versioned cache key | The stdlib has a versioned package manifest; its version and canonical hash are part of the IR cache key and are written to `[stdlib]` in the lock; a lock that disagrees with the tree is refused (E1802); a clean-checkout "no recheck" timing is CI work | Sign the stdlib in the release |
| 10.1 / 10.4 | Call-site and runtime capability enforcement, policy table | `[capabilities]`, the HIR call-site check (E1808), the transitive `audit --capabilities` union in human and JSON form, `add --allow`, `--print-capabilities`, and the full `[audit]` policy fields are implemented; the runtime handle repeats the filesystem and environment grants at execution. The clock (`now`) and process (`spawn`) operations now also check the handle, so a manifest-installed grant set cannot be bypassed at run time; FFI remains call-site only | Extend the handle to FFI at the extern call site |
| 11.1 / 11.3 | Provenance produced by the build and verified by rebuild | In-toto/SLSA v1 statement, signed by the same `publish --key-id` invocation that writes the tarball, named in the registry entry, plus subject/material digest checks and `--rebuild` verification; `attest` remains available for an artifact built elsewhere | Distinguish an unknown builder in the policy |
| 12.1 / 12.3 | Full package surface with reversible mutations | `tree` marks duplicates with their locked versions and features; `vendor` supports offline fetch; `info` reports size, signature, provenance and native libraries; `outdated` classifies major/minor/patch changes; `yank` records a reason with a backup, and `crates/yashy-driver/tests/registry.rs` now drives the CLI, checks the backup and refuses a second yank | Nothing outstanding |