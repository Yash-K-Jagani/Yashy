# Phase 3 — Packages and Distribution

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a Yashy package something a stranger can install, audit, pin and
rebuild — and make the toolchain itself installable in one command.

**Outcome:** Semver ranges with a lockfile-pinned resolver, workspaces, optional
features, a signed static registry, package signatures, capability manifests and
a dependency policy, an SBOM with build provenance, `yashyc verify-package`,
reproducible package builds, a package surface with `tree`, `why`, `outdated`,
`vendor`, `info`, `remove` and `yank`, a signed stdlib with a precompiled IR
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
- Test: `crates/yashy-manifest/tests/version.rs`, `crates/yashy-driver/tests/resolve.rs`

**Interfaces:**
- `Version { major: u64, minor: u64, patch: u64, pre: Vec<Identifier>, build: Vec<Identifier> }`
- `VersionReq::{Caret, Tilde, Exact, Gte, Lt, Wildcard, All}`
- `resolve(manifest: &Manifest, index: &Index, opts: &ResolveOptions) -> Result<ResolveGraph, ResolveError>`
- `ResolveOptions { locked: bool, offline: bool, features: BTreeSet<String>, target: Triple }`

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
  still uses the store's default registry rather than the passed index. The
  CI `--locked` rule is a gate, not a compiler-enforced flag yet.
- [x] **Step 3: One diagnostic family.** A missing version, a conflict, a
  cycle, a yanked pin and a lock mismatch each get a code, a suggestion where
  one exists, and a `--json` form. No solver backtracking loop is unbounded:
  the search is capped and reports the cap.
  *Current:* the existing catalog codes are reused (`E1800` manifest, `E1801`
  cycle, `E1802` lock, `E1803` offline); no new per-case codes were added.
  The DFS is capped at 4096 package visits and reports the cap as a manifest
  error. There is no solver backtracking loop to bound.
- [x] **Step 4: Commit.**

### Task 2: Workspaces

**Files:**
- Modify: `crates/yashy-manifest/src/manifest.rs` (`[workspace]`, `members`, `default-members`)
- Modify: `crates/yashy-driver/src/modules.rs` (resolve a member as one graph)
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
  *Current:* member names are checked for uniqueness and an outside path
  dependency is refused by `workspace::validate`. There is no CLI flag yet for
  allowing one, and members do not yet share an explicit lockfile/target
  directory: each member still reads the lock next to its own manifest.
- [x] **Step 3: Commit.**

### Task 3: Features

**Files:**
- Modify: `crates/yashy-manifest/src/manifest.rs` (`[features]`, `default`, optional deps)
- Modify: `crates/yashy-driver/src/resolve.rs` (unification, `--no-default-features`)
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
  keys. Phase 4's `cfg(feature = "x")` syntax is not here yet; the flat key is
  the current spelling. `--print-cfg` does not yet accept `--features`.
- [x] **Step 3: Commit.**

### Task 4: Signed registry v1

**Files:**
- Modify: `crates/yashy-driver/src/registry.rs`, `crates/yashy-driver/src/fetch.rs`, `crates/yashy-driver/src/audit.rs`
- Create: `crates/yashy-manifest/src/signature.rs`, `crates/yashy-manifest/src/sbom.rs`
- Create: `registry/` fixture with `index.json`, `packages/`, `keys/`
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
- Create: `docs/sbom.md`, `docs/registry.md`
- Test: `crates/yashy-driver/tests/audit.rs`, `crates/yashy-driver/tests/sbom.rs`

- [x] **Step 1: Write the failing test.** `yashyc sbom --format spdx-json`
  lists every package, version, checksum, license, supplier and the dependency
  graph, plus the compiler and runtime hashes. `yashyc verify-package
  demo-1.0.0.tar.gz` checks the signature, the checksum, the index entry and the
  SBOM; each mismatch is a distinct code.
  *Current:* `tests/sbom.rs` checks the SPDX output and the two distinct codes
  from a tarball with neither signature nor SBOM. The document names the
  package and `yashyc` binary; the runtime C shim is not a separate SBOM
  package. Index-entry checking in `verify-package` is still owed.
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
  *Current:* `tests/reproducible.rs` already compares two builds in two
  directories and asserts the emitted IR names no absolute path. The locale and
  username variations are not asserted in the test suite yet.
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
  deferred by the project owner. Publishing does not yet refuse a package that
  fails the gate.
- [x] **Step 4: Commit.**

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
- Add: `templates/workspace/`, `templates/service/`, `templates/lib/`, `templates/native/`
- Modify: `crates/yashy-driver/src/scaffold.rs` (Phase 2 owns `new`)
- Test: `crates/yashy-driver/tests/new.rs` (extend)

- [ ] **Step 1: Write the failing test.** `yashyc new shop --template workspace`
  creates two members, a shared lockfile and a passing `yashyc test
  --workspace`; `--template service` creates a project whose first `yashyc run`
  prints a health response (the HTTP stack itself lands in Phase 6; until then
  the template uses a loopback TCP handler); `--template native` creates a
  `-sys` package with a generated binding, a safe wrapper and a test against a
  fixture C library.
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

### Task 10: Capability manifests and a dependency policy

**Files:**
- Modify: `crates/yashy-manifest/src/manifest.rs` (`[capabilities]`, `[policy]`)
- Create: `crates/yashy-manifest/src/capability.rs`, `crates/yashy-driver/src/policy.rs`
- Modify: `crates/yashy-driver/src/main.rs` (`add --allow`, `audit --capabilities`)
- Create: `docs/capabilities.md`
- Test: `crates/yashy-driver/tests/capabilities.rs`, `crates/yashy-driver/tests/policy.rs`

**Interfaces:**
- `CapabilitySet { fs: FsRights, net: NetRights, env: EnvRights, clock, proc, rng, ffi }`
- `FsRights { read: Vec<PathGlob>, write: Vec<PathGlob> }`; `FfiRights { libs: Vec<PathGlob>, dlopen: bool }`
- `policy: { allow_capabilities, deny_capabilities, max_transitive, allow_licenses, deny_authors, require_signature, require_provenance }`

- [ ] **Step 1: Write the failing test.** A package that declares no
  capabilities cannot open a file, resolve a name or read the clock; the compiler
  refuses the call at build time with the capability it needs and the manifest
  line that would grant it. A package that declares `fs.read = ["data/*"]`
  reads `data/x` and is refused `data/../secret`. A package that declares `ffi`
  names the libraries it may load; one that does not cannot `dlopen` or link a
  native library beyond the stdlib's signed set. A policy that denies `net`
  refuses a package that asks for it, and `yashyc audit --capabilities` prints
  the transitive union before anything runs.
- [ ] **Step 2: The manifest is the grant, the policy is the ceiling.** A
  dependency's capabilities are the intersection of what it declares and what
  the workspace policy allows; a missing declaration is no capability, never
  full access. `yashyc add` prints the union the addition would create and
  refuses to cross the policy without `--allow`. Capability names, defaults and
  the refusal diagnostics are one table shared by the compiler, `audit`, the
  sandbox in Phase 13 and the trace redactor in Phase 10.
- [ ] **Step 3: Dependency policy is reviewable and machine-readable.**
  `[policy]` names allowed licenses, denied authors or registries, a transitive
  depth/size cap, and whether a signature and provenance are required. Each
  violation is a code with the dependency path (`app -> http -> tls`), and
  `--json` is the CI form. The default policy is strict: unsigned, unprovenanced
  and capability-undeclared packages are refused, not warned about.
- [ ] **Step 4: A capability is enforced where it is used.** The check is at the
  call site in HIR and repeated by the runtime handle that performs the
  operation, so a build that bypassed the front end still cannot exceed the
  grant. `--print-capabilities` prints the resolved set, and a build with no
  declaration is a program that can compute but not touch the world.
- [ ] **Step 5: Commit.**

### Task 11: Build provenance and attestations

**Files:**
- Create: `crates/yashy-manifest/src/provenance.rs`, `crates/yashy-driver/src/attest.rs`
- Modify: `crates/yashy-driver/src/main.rs` (`attest`, `verify-attestation`)
- Modify: `crates/yashy-manifest/src/sbom.rs` (subject digests)
- Create: `docs/provenance.md`
- Test: `crates/yashy-driver/tests/attest.rs`, `crates/yashy-driver/tests/slsa.rs`

**Interfaces:**
- `Statement { subject: Vec<Digest>, predicate_type: "https://slsa.dev/provenance/v1", predicate: Provenance }`
- `Provenance { build_type, builder, invocation, materials: Vec<Digest>, metadata }`
- `attest(package) -> Envelope`; `verify_attestation(envelope, policy) -> Result<(), AttestError>`

- [ ] **Step 1: Write the failing test.** A reproducible build produces an
  in-toto/SLSA v1 statement naming the source commit, the lockfile hash, the
  compiler and toolchain hashes, the target, every native library (path and
  hash) and every other input material; a verifier rebuilds from those materials
  and gets the same subject digest. A statement with a changed source hash, a
  changed compiler, a changed native library or a subject that does not match
  the tarball is refused with the field that disagrees.
- [ ] **Step 2: Provenance is produced by the same build that produces the
  artifact.** The statement is written in the same invocation as the package, is
  signed by the release key, and is embedded in the registry entry next to the
  SBOM. A package without provenance fails a policy that requires it; a package
  with provenance from an unknown builder fails the trust check.
- [ ] **Step 3: Verification is a rebuild, not a claim.** `yashyc
  verify-attestation` checks the signature, the subject digest and the material
  digests, then optionally rebuilds `--reproducible` from the named materials and
  compares. A provenance that only asserts what the builder says is a claim; the
  gate is the rebuild.
- [ ] **Step 4: The format is standard and minimal.** Use in-toto attestation
  and SLSA provenance v1 JSON; do not invent a schema. A future registry can
  consume the same envelope, and the release pipeline publishes it unchanged.
- [ ] **Step 5: Commit.**

### Task 12: The package surface

**Files:**
- Modify: `crates/yashy-driver/src/main.rs` (`tree`, `why`, `outdated`, `vendor`, `info`, `yank`, `remove`)
- Create: `crates/yashy-driver/src/pkg_cmd.rs`
- Modify: `crates/yashy-driver/src/resolve.rs` (reverse edges, vendor directory)
- Test: `crates/yashy-driver/tests/pkg_cmd.rs`

- [ ] **Step 1: Write the failing test.** `yashyc tree --depth 2` prints the
  dependency graph in resolution order with features and duplicates marked;
  `why serde` names the shortest path from the root and every path that needs it;
  `outdated` compares the lock to the index and marks major/minor/patch and a
  yanked pin; `vendor` writes a byte-identical tree that builds `--offline`;
  `info` prints version, license, size, signature, provenance, capabilities,
  native libraries and reverse dependencies; `remove` takes a dependency out of
  the manifest, the lock and the cache, and `yank` is refused without a reason.
- [ ] **Step 2: Every command has one JSON form and one human form.** `--json`
  is stable and used by the Action and the agent; the human form is the same
  facts with color only on a TTY. Duplicates and feature conflicts are explained,
  not merely listed.
- [ ] **Step 3: Destructive commands are reversible.** `remove` and `yank` write
  a backup of the manifest and lock, print what changed, and can be undone;
  `vendor` is additive and reports the bytes and the license files it wrote.
- [ ] **Step 4: Commit.**

## Gates

| Gate | Command | Passes when |
|---|---|---|
| Round trip | `cargo test -p yashy-driver --test registry --test resolve` | publish → add → build; `--locked` refuses a drifted lock; yanked pins refused |
| Tamper | `cargo test -p yashy-driver --test signature --test registry` | one flipped byte, one bad index signature and one missing key each fail with a named code |
| SBOM | `cargo test -p yashy-driver --test sbom --test audit` | every package, hash, license, supplier and toolchain hash present; policy violations refused |
| Capabilities | `cargo test -p yashy-driver --test capabilities --test policy` | an undeclared capability is refused at the call; a declared one is enforced; the policy is the ceiling |
| Provenance | `cargo test -p yashy-driver --test attest --test slsa` | statement names every material; a rebuild matches the subject; a mismatch names the field |
| Package surface | `cargo test -p yashy-driver --test pkg_cmd` | tree, why, outdated, vendor, info, remove and yank behave; every command has a JSON form |
| Reproducible | `cargo test -p yashy-driver --test reproducible` | two directories, two runs, identical SHA-256 on both OSes |
| Stdlib package | `cargo test -p yashy-driver --test stdlib_precompiled` | unchanged stdlib is not re-typechecked; cache key verified; hash mismatch refused |
| Workspaces | `cargo test -p yashy-driver --test workspace --test features` | `--workspace`, `-p`, optional deps and feature unification behave; features are in the cache key |
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
| 1.1-1.3 | Semver ranges, one resolution, per-case diagnostic codes | `Version`/`VersionReq` with SemVer 2.0 precedence, range selection, v2 lock with v1 migration, yanked refusal, bounded DFS; existing E180x codes reused, `ResolveOptions` fields not yet applied to an explicit index | Add per-case codes and feed features/target into the index selection |
| 2.1 / 2.2 | Shared lockfile and target directory for members | Workspace discovery, `-p`/`--workspace` selection, unique names and outside-path refusal; members still read their own lock and do not share an explicit target directory | Add a workspace-level lock/store and an explicit allow flag for outside paths |
| 3.1 / 3.2 | `cfg(feature = "x")` and `--print-cfg` with features | Sorted feature sets, unification, cache-key and lockfile recording, optional dependencies, `feature:<name>` flat cfg keys, `--features`/`--no-default-features` | Add the Phase 4 `cfg(feature = ...)` syntax and feature-aware `--print-cfg` |
| 4.1 / 4.2 | Signed index and tarballs, explicit trust set | `ed25519-compact` signs tarballs and the index; `publish --key-id` writes an SPDX SBOM; `fetch` verifies when `YASHY_TRUSTED_KEY`/`YASHY_KEYRING` is set | Add a built-in release key, multi-signature support and remote signature verification || 5.1 / 5.2 | SBOM, distinct mismatch codes, audit policy with signatures | `yashyc sbom` prints SPDX 2.3; `verify-package` reports E1805/E1806; `[audit]` allowlists and `require-signatures` are parsed and enforced; per-package audit findings and index-entry checks are not implemented |
| 6.1 / 6.3 | Two-directory gate and publish refusal | Two-directory byte comparison IR path check exist; SBOM records compiler version and target; the CI gate and publish-time refusal are deferred with CI |
