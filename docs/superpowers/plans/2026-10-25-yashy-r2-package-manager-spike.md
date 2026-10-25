# Slice 4 — Package-Manager Spike

> **For agentic workers:** This is a **spike**, not the R2 implementation.
> Timebox it. The deliverable is a decision-complete design plus a throwaway
> prototype that proves path and git dependencies. Do not add a stable CLI
> surface, a registry, or publishing.

**Status:** Draft for review, 2026-10-25.
**Source of truth:** `docs/superpowers/specs/2026-10-25-yashy-super-language-v2-design.md` §15.5 and §17.
**Master plan:** `docs/superpowers/plans/2026-10-25-yashy-super-language-master.md` §7.5.
**Goal:** Decide how `yashy.toml`, `yashy.lock`, path/git dependencies, module
roots, caching, and the CLI will work, prove the two dependency kinds with a
small prototype, and make the design safe for agents.
**Depends on:** Slice 2 (arguments make the demo useful) and slice 3
(measurement discipline).
**Estimate:** 5–7 working days.
**Decisions confirmed (2026-10-25):** the spike is commit-pinned; checksums are
an R2 MVP gate, not a spike deliverable; every prototype command has `--json`;
`audit` and `--offline` are part of the spike.

---

## 1. Scope

### In scope

- A design document covering manifest, lockfile, resolver, cache, module
  mapping, CLI, security, and the future registry shape.
- A minimal TOML-subset parser for the manifest and lockfile (or a documented
  decision to add a TOML crate in R2).
- A resolver for path and git dependencies, with cycle and name-collision
  detection.
- A git fetcher that pins an exact commit in the lockfile.
- A module-root integration experiment: dependency `src/` as a root, imports by
  module name, collision diagnostics.
- An experimental `yashy pkg` prototype with `add`, `remove`, `resolve`, `build`,
  `test`, `tree`, and `audit`, all supporting `--json`.
- `--offline` support: use the cache, and fail with a clear error on a cache
  miss.
- A manifest rejection for build scripts, post-install hooks, or arbitrary code
  execution, with a clear diagnostic.
- An `audit` command for lockfile integrity, duplicate packages, and the
  no-build-scripts rule.
- A checksum design section for the R2 MVP; no checksum implementation in the
  spike.
- Tests using local path packages and a local git repository.
- A written R2 implementation plan derived from the spike.

### Not in scope

- A central registry implementation, publishing, search, or docs hosting.
- A stable `yashy add` / `yashy build` integration. The prototype is behind
  `yashy pkg` and prints an experimental warning.
- Version solving beyond exact git revs and path packages. No semver ranges in
  R2 MVP.
- Build scripts, post-install hooks, or arbitrary code execution from packages.
- Dependency signing, checksum implementation for path/git packages, or a
  sandbox. The checksum design is recorded for the R2 MVP.
- Workspaces/monorepos, optional dependencies, dev-dependencies, or feature
  flags. Record them as future work.
- Network tests in CI. Use a local git repo; a network smoke is manual.

---

## 2. Current state

- `yashy init` writes `yashy.toml` with only `[package] name` and `version`,
  but nothing reads it. It also writes `main.ya`, `tests/smoke.ya`, `README.md`,
  and `.gitignore`.
- The module loader resolves `import a.b;` to `<root>/a/b.ya` or
  `<root>/a/b/mod.ya`. Roots are `YASHY_PATH` (which replaces the entry
  directory) plus the bundled stdlib. `module_roots(entry)` is called inside
  `modules::load`.
- The workspace deliberately builds offline and has almost no dependencies; the
  LSP has a hand-written JSON parser for that reason. Adding `serde`/`toml` is
  a real decision, not a convenience.
- There is no package cache, lockfile, resolver, or dependency concept.
- The compiler is a single-binary driver; a package manager can start as a
  driver subcommand without a new process.

---

## 3. Design decisions to settle

The design doc must answer each of these with a recommendation and the
alternative it rejected.

### 3.1 Manifest schema

Minimum for R2 MVP:

```toml
[package]
name = "demo"
version = "0.1.0"
entry = "main.ya"          # optional; default main.ya

[dependencies]
util = { path = "../util" }
json = { git = "https://example.com/json.git", rev = "0123456789abcdef" }
```

Rules:

- `name` is required, unique in the graph, and a valid identifier-ish string
  (letters, digits, `_`, `-`; no path separators).
- `version` is required for a package that can be depended on; the root package
  may default to `0.1.0`.
- `entry` defaults to `main.ya` at the project root.
- Dependency tables accept exactly one source: `path`, or `git` with one of
  `rev`/`tag`/`branch`. `rev` is the recommended pin; `branch` is resolved to a
  commit at lock time.
- No `[build]`, no scripts, no optional/dev dependencies in R2 MVP.

### 3.2 Lockfile schema

```toml
version = 1

[[package]]
name = "util"
source = "path+../util"

[[package]]
name = "json"
source = "git+https://example.com/json.git"
rev = "0123456789abcdef0123456789abcdef01234567"
```

- The lockfile is generated by `yashy pkg resolve` and is committed.
- A git dependency is pinned to a full commit SHA. A tag/branch is resolved
  once and recorded as a commit; a later resolve does not move it unless the
  user asks to update.
- A path dependency records the path as written (normalized to `/` in the
  lockfile for portability) and is re-read on every build; it is intentionally
  live.
- The lockfile is the reproducibility contract: a clean checkout with the lock
  builds the same graph without network access for path deps and with only the
  pinned git objects for git deps.

### 3.3 TOML parsing

Two options:

- **A (recommended for the spike):** a hand-written TOML subset parser in the
  driver. Support comments, blank lines, `[table]`, `key = "string"`, and
  inline tables `{ key = "value", ... }`. Reject anything else with a clear
  "unsupported manifest syntax" error. This preserves the offline, dependency-
  light workspace and is enough for path/git dependencies.
- **B:** add the `toml` crate (and likely `serde`). Less parser code, more
  manifest syntax, but a new dependency in the compiler's build graph.

The spike implements A and records the cost. R2 can switch to B if the subset
becomes a maintenance burden; the manifest schema does not change.

The parser and writer get round-trip tests: parse -> write -> parse produces the
same graph; comments are not preserved (document that).

### 3.4 Resolver

- Depth-first, deterministic, dependency-before-dependant order.
- Each package is identified by `(name, source)`; a second package with the
  same name from a different source is a name collision, not a silent
  override.
- A path dependency is canonicalized for cycle detection; the lockfile keeps
  the written path.
- A git dependency is fetched into the cache, checked out at the requested
  commit, and its manifest is read from the checkout. A cycle is reported with
  the chain.
- A missing dependency, a bad manifest, a git failure, or a rev mismatch is a
  diagnostic naming the package and the source.
- No semver solving in R2 MVP. One version per package name. If two packages
  need different versions of the same name, that is an error with the two
  requirements printed; version solving is future work.

### 3.5 Module mapping

The loader needs dependency roots. Two options:

- **A (recommended for R2 MVP):** each dependency's `src/` is added as a module
  root. Imports are by module name (`import util;` resolves to
  `<dep>/src/util.ya`). If two dependencies export the same module name, the
  resolver reports both packages and the module; the user renames one.
- **B:** package-prefixed imports (`import dep.module;`). The loader maps the
  first segment to a package root. This avoids collisions but changes the
  import model and the module loader.

The spike builds a two-package demo under A and records the collision error. The
design doc recommends A for R2 MVP and keeps B as the path if the ecosystem
outgrows unique module names.

Implementation note: add an explicit `extra_roots: &[PathBuf]` parameter to
`modules::load` / `load_and_check` rather than setting `YASHY_PATH`. `YASHY_PATH`
*replaces* the entry-directory root; the package manager needs to *append*
dependency roots. The parameter keeps the environment out of the build and
makes the roots testable.

### 3.6 Cache

- Reuse `YASHY_CACHE_DIR` / the platform cache directory.
- Layout: `<cache>/git/<hash-of-url>/` with a bare or worktree checkout at the
  pinned commit. Use the `git` CLI (`git init`, `git fetch`, `git checkout`) to
  avoid a libgit2 dependency.
- A checkout is keyed by URL + commit. Concurrent fetches of the same key use a
  lock file or a staged directory + rename, so two builds cannot corrupt one
  checkout.
- A cache miss with no network is an error that names the package and the
  commit; a path dependency never needs the network.
- Do not delete the cache automatically in R2 MVP; document `yashy pkg cache
  clean` as future work.

### 3.7 CLI surface

Experimental, under `yashy pkg`:

```text
yashy pkg add path <path> [--name <name>] [--json]
yashy pkg add git <url> [--rev <sha> | --tag <tag> | --branch <branch>] [--json]
yashy pkg remove <name> [--json]
yashy pkg resolve [--offline] [--json]
yashy pkg build [--release] [-o <out>] [--offline] [--json]
yashy pkg test [--offline] [--json]
yashy pkg tree [--json]
yashy pkg audit [--json]
```

- Every command prints `experimental: the package manager is a design spike` on
  stderr.
- `--json` returns `schema_version`, `tool`, `success`, `data`, and
  `diagnostics`. `--json` implies no color and no prompt.
- `--offline` never touches the network; a cache miss is a clear error naming the
  package and commit.
- `add` and `remove` edit `yashy.toml` and then resolve; `remove` also updates the
  lockfile.
- `build`/`test` load the entry with the resolved extra roots; they reuse the
  existing compiler pipeline, not a second one.
- `audit` checks lockfile integrity, duplicate package names, commit pinning, and
  the no-build-scripts rule. It is read-only.
- Reject a manifest with `build`, `scripts`, `postinstall`, or any arbitrary
  command field with a clear diagnostic.
- Do not add `yashy add` or change `yashy build` until R2. The stable CLI stays
  unchanged in R1.

### 3.8 Security

- **No build scripts in R2 MVP.** A dependency is source only. This removes the
  largest supply-chain risk and is a deliberate product position.
- Git dependencies are pinned to a full commit in the lockfile; a tag/branch is
  resolved once and then frozen.
- The resolver verifies that the checkout's `HEAD` is the locked commit before
  reading the manifest.
- Path dependencies are local and trusted; document that a path dependency can
  change under the project without a lockfile update.
- Registry (future): immutable versions, checksums, a signed index, and no
  post-install scripts. Publishing is authenticated and append-only.
- The spike rejects build scripts at parse time and `audit` reports the rule.
- The checksum design for the R2 MVP is recorded (tarball hash, lock entry,
  verification point); it is not implemented in the spike.

### 3.9 Registry shape (design only)

- A static index: package name -> versions -> `{ checksum, tarball, yanked }`.
- `yashy publish` uploads a tarball, verifies the checksum, and appends a
  version; no overwrite.
- `yashy add` with a registry source is R2 stretch / 1.0. The R2 MVP uses path
  and git only.
- Search and docs hosting are 1.0.

---

## 4. Tasks

### Task 1 — Design document

- [ ] Write `docs/superpowers/specs/2026-10-25-yashy-package-manager-design.md`
  covering §3.1–§3.9, with a decision and a rejected alternative for each.
- [ ] Include a worked example: root package + path dependency + git dependency,
  the manifest, the lockfile, the resolved tree, and the build command.
- [ ] Include the failure cases: cycle, name collision, missing path, missing
  git rev, bad manifest, offline cache miss.
- [ ] Include the R2 MVP scope and the explicit non-goals.
- [ ] Get the design reviewed before writing the prototype's stable parts.

### Task 2 — Manifest parser

- [ ] Add `crates/yashy-driver/src/package/manifest.rs` with the TOML subset
  from §3.3.
- [ ] Parse `[package]`, `[dependencies]`, quoted strings, inline tables, and
  comments.
- [ ] Reject unsupported syntax with a message that names the line and the
  construct.
- [ ] Validate names, versions, entry paths, and exactly one source per
  dependency.
- [ ] Tests: the worked example, a missing `name`, a duplicate dependency, a
  dependency with both `path` and `git`, an unsupported array, a malformed
  inline table.
- [ ] Round-trip the manifest through the writer used by `yashy pkg add`.

### Task 3 — Lockfile

- [ ] Add `package/lock.rs` with the schema from §3.2.
- [ ] Write a stable, sorted lockfile (deterministic order) so diffs are small.
- [ ] Parse an existing lockfile and compare it to the resolved graph.
- [ ] Tests: round-trip, a changed git rev, a removed dependency, a lockfile
  with an unknown field, a path normalized to `/`.
- [ ] Decide and document what happens when the lockfile is missing (resolve
  and write) and when it is stale (error with the two graphs, or auto-update?
  Recommend: error and tell the user to run `resolve`).

### Task 4 — Path resolver

- [ ] Resolve a path dependency by reading its `yashy.toml`, recursively.
- [ ] Detect cycles with the chain.
- [ ] Detect duplicate names from different sources.
- [ ] Order dependencies before dependants.
- [ ] Tests: a two-package chain, a diamond (same package via two paths), a
  cycle, a missing path, a name collision.
- [ ] Build the demo with the path dependency only and run it.

### Task 5 — Git resolver

- [ ] Add a git fetcher using the `git` CLI:
  - cache key = URL + commit;
  - staged checkout + rename;
  - verify `HEAD` equals the locked commit.
- [ ] Resolve `rev`, `tag`, and `branch` to a full commit at lock time.
- [ ] Tests with a local repository created in a temp dir:
  - pin a commit, change the branch, confirm the lock still builds the pinned
    commit;
  - a missing rev fails with a clear error;
  - a tag resolves to a commit;
  - a dirty worktree does not affect the cached checkout.
- [ ] Do not require network access in the test suite.

### Task 6 — Module-root integration

- [ ] Add `extra_roots: &[PathBuf]` to `modules::load` and `load_and_check`
  (or a small `LoadOptions`), defaulting to empty at every existing call site.
- [ ] Add each dependency's `src/` as a root, dependencies before the root
  package.
- [ ] Implement the collision diagnostic from §3.5.
- [ ] Experiment: a root package imports `util` from a path dependency and
  builds.
- [ ] Record whether Option A is sufficient or Option B is needed.

### Task 7 — Prototype CLI

- [ ] Add `yashy pkg` with `add`, `remove`, `resolve`, `build`, `test`, `tree`,
  and `audit`.
- [ ] Add `--json` to every command. The object has `schema_version`, `tool`,
  `success`, `data`, and `diagnostics`; `--json` implies no color and no prompt.
- [ ] Add `--offline` to `resolve`, `build`, and `test`. A cache miss is a clear
  error naming the package and commit; no network call is attempted.
- [ ] Reuse the existing build pipeline and options; do not duplicate the
  compiler path.
- [ ] Print the experimental warning on stderr.
- [ ] `add path` and `add git` edit `yashy.toml` and resolve; `remove` edits the
  manifest and the lockfile.
- [ ] `tree` prints the resolved graph and the source of each package.
- [ ] `audit` checks lockfile integrity, duplicate names, commit pinning, and
  reports `build`, `scripts`, and `postinstall` fields; the parser rejects them.
- [ ] Tests: `add path` then `build` then run the binary; `add git` with a
  local repo; `tree` output; `remove` updates both files; `audit` catches a
  build script; `--json` parses for each command; `--offline` fails cleanly on a
  cold cache; a bad manifest exits nonzero with a diagnostic.

### Task 8 — End-to-end demo

- [ ] Create `examples/packages/root/`, `examples/packages/util/`, and a local
  git repo fixture (or a script that creates one) for the git dependency.
- [ ] The root uses a function from `util` and prints a result; the git
  dependency contributes a second function.
- [ ] Build from a clean checkout with the lockfile and run the binary.
- [ ] Run `yashy pkg audit --json` and confirm it reports a clean lockfile.
- [ ] Run `yashy pkg resolve --offline` after the warm cache and confirm it
  succeeds without network; then run it with a cold cache and confirm the error.
- [ ] Record the exact commands and output in the design doc.
- [ ] Do not add the demo to the normal test suite if it needs `git` network;
  use the local fixture.

### Task 9 — R2 implementation plan

- [ ] Turn the design into a task-by-task R2 plan:
  - stable `yashy add`, `remove`, `update`, `tree`, and `audit` integration;
  - manifest/lock parser hardening;
  - resolver errors and `tree`;
  - offline and cache clean;
  - checksum verification;
  - CI clean-checkout job;
  - docs and migration notes.
- [ ] Estimate each task and identify the first three.
- [ ] Record the open questions that remain (semver, workspaces, registry).

### Task 10 — Review and stop

- [ ] Review the design with the user; answer or defer every open question.
- [ ] Confirm the prototype stays behind `yashy pkg` and is not documented as
  stable.
- [ ] Record the decision in the master plan: R2 starts with the package-manager
  MVP, using Option A for module mapping unless the review changes it.
- [ ] Stop. Do not start the R2 implementation in this slice.

---

## 5. Verification

```text
cargo test -p yashy-driver --test packages
cargo test -p yashy-driver --test packages_json
cargo test -p yashy-driver --test e2e
cargo check --workspace --all-targets
```

The demo:

```text
# from a clean checkout of the demo
yashy pkg resolve
yashy pkg tree --json
yashy pkg audit --json
yashy pkg resolve --offline
yashy pkg build --release -o demo
./demo
# expected output
```

Run it on Windows and Linux. The git fixture is local; no network. Parse the
`--json` outputs with a small script or `ConvertFrom-Json`/`jq`.

---

## 6. Acceptance criteria

- The design doc answers manifest, lockfile, resolver, module mapping, cache,
  CLI, security, checksums, and registry shape, each with a decision and a
  rejected alternative.
- A root package with a path dependency and a git dependency resolves, locks,
  builds, and runs from a clean checkout.
- A git dependency is pinned to a full commit; changing the branch does not
  change the locked build.
- A cycle, a name collision, a missing path, a missing rev, and a bad manifest
  each produce a clear error.
- A manifest with `build`, `scripts`, or `postinstall` is rejected with a clear
  diagnostic; `audit` reports the rule.
- `--json` parses for `add`, `remove`, `resolve`, `build`, `test`, `tree`, and
  `audit`, and the human and JSON facts agree.
- `--offline` succeeds with a warm cache and fails cleanly on a cold cache.
- The prototype is behind `yashy pkg` and prints an experimental warning; the
  stable CLI is unchanged.
- The R2 implementation plan exists and is estimated.
- No registry, publishing, checksum implementation, build scripts, or semver
  solving was implemented.

---

## 7. Risks and rollback

| Risk | Mitigation |
| --- | --- |
| The spike becomes the implementation | Timebox; no stable CLI; no registry; stop at the gate. |
| The TOML subset grows | Support only the R2 MVP schema; reject the rest; record the `toml` crate as the R2 alternative. |
| Import collisions | Option A with a clear error; test two dependencies with the same module name; keep Option B as the fallback. |
| Git is unavailable or different on Windows | Use the `git` CLI; test with a local repo; document the requirement; a network smoke is manual. |
| Lockfile drift | Exact commits; verify `HEAD`; tests for a changed branch/rev; deterministic order. |
| Module-root changes break existing builds | `extra_roots` defaults to empty; run the full e2e suite; no environment mutation. |
| Scope creep into semver/workspaces | Explicit non-goals; record them for R2/1.0. |

**Rollback:** the prototype is additive and behind `yashy pkg`. Remove the
subcommand and the design remains. Do not change `yashy build` or the manifest
format in this slice.

---

## 8. Files touched

- `docs/superpowers/specs/2026-10-25-yashy-package-manager-design.md` (new)
- `docs/superpowers/plans/2026-10-25-yashy-r2-package-manager-mvp.md` (new,
  derived)
- `crates/yashy-driver/src/package/mod.rs` (new)
- `crates/yashy-driver/src/package/manifest.rs` (new)
- `crates/yashy-driver/src/package/lock.rs` (new)
- `crates/yashy-driver/src/package/resolve.rs` (new)
- `crates/yashy-driver/src/package/git.rs` (new)
- `crates/yashy-driver/src/package/audit.rs` (new)
- `crates/yashy-driver/src/package/json.rs` (new, if the shared JSON helper is not
  enough)
- `crates/yashy-driver/src/main.rs` (`pkg` subcommand, `--json`, `--offline`)
- `crates/yashy-driver/src/modules.rs` (`extra_roots`)
- `crates/yashy-driver/src/pipeline.rs` (`load_and_check` signature)
- `crates/yashy-driver/tests/packages.rs` (new)
- `crates/yashy-driver/tests/packages_json.rs` (new)
- `crates/yashy-driver/tests/e2e.rs` (call-site updates)
- `examples/packages/` (new demo)
- `README.md` (a short experimental note, or no mention until R2)

---

## 9. Definition of done

- The acceptance commands in §5 pass on Windows and Linux.
- The design doc is reviewed and the open questions are answered or deferred.
- The prototype proves path and git dependencies; it does not become a stable
  feature.
- `--json` and `--offline` are tested; `audit` rejects build scripts and checks
  the lockfile.
- The checksum design is recorded for the R2 MVP; no checksum code ships in the
  spike.
- The R2 implementation plan exists with estimates.
- The master plan records the decision and the R2 starting point.
- Commit and push; record the commit hashes in this plan.
