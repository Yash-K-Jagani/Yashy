# Phase 10 — Libraries and Packages: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a Yashy program able to depend on code it did not write, reproducibly and offline, and ship enough first-party packages that a real HTTP service is a weekend project rather than a research project.

**Architecture:** A manifest (`yashy.toml`) is parsed by a deliberately small TOML subset reader in a new `yashy-manifest` crate — no dependency, a documented grammar, and a 30-manifest conformance corpus. Resolution is exact-pin only: a lockfile names every package, version, source and content hash, and the resolver refuses anything it cannot pin. Dependencies are compiled from source with the phase 3 cache, so a second build is free. The stdlib becomes a package like any other: versioned, locked, and shipped as cached IR so `hello` does not re-typecheck it. Networking is new C in the runtime — sockets on both platforms — and `http` is a Yashy package on top of it.

**Tech Stack:** `yashy-manifest` (new, zero deps), `yashy-driver/src/{resolve.rs,fetch.rs,audit.rs}`, the phase 3 `yashy-cache`, `yashy-runtime` (Winsock/POSIX sockets), `crates/yashy-stdlib/` as a versioned package, and new `packages/{http,cli,net,test,bench,fuzz}`.

**Spec:** `docs/superpowers/specs/2026-10-27-yashy-master-design.md` §7. Index: `docs/superpowers/plans/2026-10-27-yashy-master-plan.md`. Predecessor: `2026-10-27-phase-09-ai-native-observability.md`.

## Global Constraints

See the master index. The ones that bite here:

- **No build scripts, ever.** A dependency is source; if it needs to run code to build, it is not a Yashy package.
- No new runtime dependencies in the compiler. The TOML subset reader is hand-written and its limits are documented.
- Reproducibility is a property of the lockfile: same lockfile, same bytes, on both OSes.
- Both operating systems or it is not done — including sockets.
- No unproven claim: "clean-checkout build" means a fresh clone with no cache and no network.

## Written cap (this phase)

- **Exact versions only.** `version = "1.2.3"`; no `^`, `~`, `*` or ranges. A version solver is a project of its own and its absence is why resolution is 200 lines instead of 2 000. Ranges may be added later behind a lockfile format bump.
- **No registry hosting.** A registry is a directory or an HTTP-served static index; `file://` is the supported transport. Publishing to a public host is out of scope.
- **No C ABI packages.** A package is Yashy source. FFI stays in the stdlib and in `extern` blocks the verifier inventories.

## Review Focus

1. **A lockfile that does not match the manifest is an error, not a silent re-resolve.** `E1802` names the package that differs and tells the user to run `yashy update`. Silent re-resolution is how a build becomes unreproducible without anyone noticing.
2. **`--offline` never touches the network, including for the index.** `E1803` when a required package is not in the local store; the message names the package and the exact command that would fetch it.
3. **A dependency cycle is a diagnostic, not a stack overflow.** `E1801` with the cycle path, detected before any source is read.
4. **The precompiled stdlib must be byte-identical to compiling it from source.** The gate compiles `hello` both ways and compares the binary's hash. A cache that changes semantics is worse than no cache.
5. **The HTTP server must be a real server**: it handles keep-alive, chunked requests, a 10 MB body cap, slowloris timeouts, and concurrent connections, and the test suite proves each. "Real HTTP service" in the gate means these tests, not a hello-world handler.

---

### Task 1: `yashy.toml` and the manifest crate

**Files:**
- Create: `crates/yashy-manifest/{Cargo.toml,src/lib.rs,src/toml.rs,src/manifest.rs}`
- Modify: `Cargo.toml` (member), `crates/yashy-driver/Cargo.toml`
- Modify: `crates/yashy-common/src/codes.rs` (`MANIFEST_SYNTAX = "E1800"`), create `docs/errors/E1800.md`
- Create: `crates/yashy-manifest/tests/corpus/` (30 manifests: 15 valid, 15 invalid)

**Interfaces:**
- Produces:
  ```rust
  pub struct Manifest {
      pub package: Package,                  // name, version, edition, description, license
      pub dependencies: BTreeMap<String, Dependency>,
      pub dev_dependencies: BTreeMap<String, Dependency>,
  }
  pub enum Dependency { Path { path: String }, Git { url: String, rev: String }, Registry { version: String } }
  ```
- Produces: the supported TOML subset — tables, bare/quoted keys, strings, integers, booleans, arrays of strings, comments. No inline tables, no multi-line strings, no dates. Documented in `docs/cli.md` with a one-screen grammar.
- Produces: `E1800` with line and column for any input outside the subset; a manifest missing `name` or `version` is `E1800` with the missing key named.

- [ ] **Step 1: Write the failing tests** — all 15 valid manifests parse to the expected struct; each of the 15 invalid ones produces `E1800` at the right line; a 1 MiB manifest is rejected with a size diagnostic rather than a slow parse; a `git` dependency without `rev` is `E1800` ("a git dependency is pinned to a commit").
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — a line-oriented reader with a small state machine; keys are `BTreeMap` so error order is deterministic. The parser is total: every input returns `Result`, never panics, and the fuzz corpus from phase 7's harness gains ten manifest cases.
- [ ] **Step 4: Run** `cargo test -p yashy-manifest` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(manifest): a small, total TOML subset and yashy.toml (E1800)"`

---

### Task 2: The lockfile

**Files:**
- Create: `crates/yashy-manifest/src/lock.rs`
- Modify: `crates/yashy-common/src/codes.rs` (`LOCKFILE_MISMATCH = "E1802"`), create `docs/errors/E1802.md`
- Modify: `crates/yashy-driver/src/main.rs` (`lock` subcommand: `--check`)
- Create: `crates/yashy-manifest/tests/lock.rs`

**Interfaces:**
- Produces: `yashy.lock` — a deterministic, sorted file:
  ```toml
  [[package]]
  name = "http"
  version = "0.3.1"
  source = "registry+file:///opt/yashy-registry"
  hash = "sha256:…"
  dependencies = ["net 0.2.0", "string 1.0.0"]
  ```
- Produces: `hash` covers the package's source tree canonically (sorted paths, LF, no mtimes), so it is stable across a re-clone.
- Produces: `yashy lock --check` exits 1 with `E1802` when the lock and manifest disagree; `yashy build` runs the same check before compiling.

- [ ] **Step 1: Write the failing tests** — round-trip a lockfile byte-identically; changing a manifest version without updating the lock is `E1802` naming both versions; a hand-edited hash that does not match the source is `E1802`; the lockfile is sorted by name then version.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — hashing is a streaming SHA-256 written in the crate (about 120 lines, no dependency), with a test vector from the FIPS 180-4 examples so the implementation is proven against a known answer rather than against itself.
- [ ] **Step 4: Run** `cargo test -p yashy-manifest` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(manifest): a content-hashed, sorted lockfile (E1802)"`

---

### Task 3: Resolution, path and git dependencies, offline

**Files:**
- Create: `crates/yashy-driver/src/{resolve.rs,fetch.rs}`
- Modify: `crates/yashy-common/src/codes.rs` (`DEPENDENCY_CYCLE = "E1801"`, `OFFLINE_MISSING = "E1803"`)
- Create: `docs/errors/{E1801,E1803}.md`
- Modify: `crates/yashy-driver/src/main.rs` (`add`, `update`, `fetch`, `--offline`), `docs/cli.md`
- Create: `crates/yashy-driver/tests/dependencies.rs`

**Interfaces:**
- Produces: `resolve(manifest, lock, offline) -> Result<ResolvedGraph, Diagnostic>`; a graph node is a package source directory, already fetched.
- Produces: git dependencies are cloned into the global store keyed by `(url, rev)` and checked out at that commit; a `rev` that does not exist is `E1803`-class with the URL and rev printed.
- Produces: `--offline` never invokes git or the network; a missing package is `E1803` with the fetch command in the note.

- [ ] **Step 1: Write the failing tests** — a path dependency compiles; a diamond resolves one copy; a cycle is `E1801` with the path; a git dependency at a fixed rev is reproducible across two fetches; `--offline` with a cold store is `E1803`; `--offline` with a warm store builds.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — DFS with a visiting stack for cycles (the phase 4 module-graph pattern, reused); fetching shells out to `git` only when `git` is on `PATH`, and the absence of `git` is a documented diagnostic, not a panic. Fetched trees are read-only in the store.
- [ ] **Step 4: Run** `cargo test -p yashy-driver --test dependencies` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(pkg): resolve path and pinned git dependencies, offline (E1801, E1803)"`

---

### Task 4: The precompiled stdlib

**Files:**
- Modify: `crates/yashy-stdlib/` (a `yashy.toml` of its own; version 1.0.0)
- Modify: `crates/yashy-driver/src/{pipeline.rs,main.rs}` (load stdlib IR from the cache when the key matches)
- Modify: `crates/yashy-bench/src/lib.rs` (row `hello_without_stdlib_ms`)
- Create: `crates/yashy-driver/tests/stdlib_precompiled.rs`

**Interfaces:**
- Produces: `hello` typechecks only `main.ya` plus the stdlib symbols it imports, not every stdlib module; the IR for the stdlib comes from the phase 3 cache, keyed by `(compiler version, triple, stdlib version, flags)`.
- Produces: a `--stdlib-source` escape hatch that forces a from-source build, used by the equivalence test.

- [ ] **Step 1: Write the failing test** — compile `examples/hello` with the precompiled stdlib and from source; the two binaries are byte-identical (or differ only in a documented timestamp field, which the test normalises); `--timings` shows `stdlib_ms` drop by ≥ 80%.
- [ ] **Step 2: Watch it fail.**
- [ ] **Step 3: Implement** — the cache entry stores the stdlib's HIR-serialised form (a small, versioned binary encoding) plus its IR; the driver loads it when the key matches and falls back to source otherwise. The encoding is versioned so a compiler upgrade cannot read an old blob.
- [ ] **Step 4: Run** `cargo test -p yashy-driver --test stdlib_precompiled` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "perf(stdlib): ship the stdlib as cached IR"`

---

### Task 5: Sockets in the runtime and the `net` package

**Files:**
- Create: `crates/yashy-runtime/src/net.c`, header additions
- Modify: `crates/yashy-runtime/build.rs` (link `ws2_32` on Windows)
- Create: `packages/net/{yashy.toml,src/net.ya}`
- Create: `crates/yashy-runtime/tests/net.rs`, `crates/yashy-driver/tests/programs/net.ya`

**Interfaces:**
- Produces:
  ```c
  int64_t yashy_tcp_listen(const char *host, int64_t port);
  int64_t yashy_tcp_accept(int64_t listener);
  int64_t yashy_tcp_connect(const char *host, int64_t port);
  int64_t yashy_tcp_read(int64_t fd, uint8_t *buf, int64_t len);   /* 0 = EOF, -1 = error */
  int64_t yashy_tcp_write(int64_t fd, const uint8_t *buf, int64_t len);
  void    yashy_tcp_close(int64_t fd);
  ```
- Produces: `net.listen`, `net.accept`, `net.connect`, `net.read`, `net.write`, `net.close` as linear handles (typestate: `Open`/`Closed`, so double-close is `E1400`).
- Produces: `net` is an `io` effect; a `pure fn` cannot open a socket (`E1300`).

- [ ] **Step 1: Write the failing tests** — loopback connect/accept/echo on both OSes; a closed socket read returns EOF, not a panic; a connect to a closed port returns an error `Result`; the linear handle cannot be used after close (`E1401`); 100 sequential connections leak no handles.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — Winsock and POSIX behind one small `#ifdef` pair in `net.c`; every function returns a value, none aborts; `WSAStartup` runs once from a `pthread_once`/`INIT_ONCE`.
- [ ] **Step 4: Run** `cargo test -p yashy-runtime --test net` → PASS on both OSes.
- [ ] **Step 5: Commit** — `git commit -m "feat(runtime): TCP sockets with linear handles"`

---

### Task 6: The `http` package and a real service

**Files:**
- Create: `packages/http/{yashy.toml,src/{http.ya,server.ya,client.ya}}`
- Create: `examples/http-service/{yashy.toml,src/main.ya,tests/http.ya}`
- Create: `packages/http/tests/` (request parsing, keep-alive, chunked, limits)

**Interfaces:**
- Produces: `http.get(url) -> Result[Response, Error]`, `http.post(url, body, headers)`, and a server: `http.serve(addr, handler)` where `handler: fn(Request) -> Response`.
- Produces: HTTP/1.1 with keep-alive, `Content-Length` and chunked request bodies, a 10 MB body cap, a 10 s header timeout, a 30 s idle timeout, and one thread per connection up to a configurable cap.

- [ ] **Step 1: Write the failing tests** — a request line and headers parse; a chunked body reassembles; an oversized body is `413` and the connection closes; a slowloris client is disconnected at the header timeout; 50 concurrent clients all get responses; keep-alive serves two requests on one connection; the client follows one redirect and reports the final URL.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — request parsing is a byte-level state machine in Yashy (no regex); the handler runs on the connection thread; errors become status codes, never panics. The client sets timeouts and caps the response body like the server caps the request.
- [ ] **Step 4: Run** `cargo test -p yashy-driver --test examples http-service` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(http): a client and a server that survive a real client"`

---

### Task 7: `cli`, `test`, `bench`, `fuzz`

**Files:**
- Create: `packages/cli/`, `packages/test/`, `packages/bench/`, `packages/fuzz/` (each with `yashy.toml`, `src/`, `tests/`)
- Modify: `docs/tutorial/12-first-cli.md` (rebuild the CLI chapter on `cli`)

**Interfaces:**
- Produces: `cli.Args` — flags, values, positionals, `--help` generated from a declaration, and a parse error that exits 2 with usage on stderr.
- Produces: `test.assert_eq`, `test.assert_contains`, `test.fail` as Yashy functions that print a diff and exit non-zero — the library form of what the harness already does.
- Produces: `bench.run(name, iterations, body)` printing `key=value` lines the phase 3 bench parser already understands.
- Produces: `fuzz.run(target, iterations, seed)` with a deterministic corpus directory and a minimiser that keeps the shortest failing input.

- [ ] **Step 1: Write the failing tests** — each package has a `tests/*.ya` program that exercises its public surface and exits 0; `cli`'s `--help` output is asserted; `fuzz` finds a seeded bug in a deliberately broken parser in under 5 000 iterations and minimises it to ≤ 8 bytes.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — each package is ordinary Yashy with a manifest and a lock entry; none is special-cased in the driver, which is the point of Task 4's dogfooding.
- [ ] **Step 4: Run** `cargo test -p yashy-driver --test examples` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(packages): cli, test, bench and fuzz"`

---

### Task 8: `yashy audit`

**Files:**
- Create: `crates/yashy-driver/src/audit.rs`
- Modify: `crates/yashy-common/src/codes.rs` (`AUDIT_FAILURE = "E1804"`), create `docs/errors/E1804.md`
- Modify: `crates/yashy-driver/src/main.rs` (`audit [--json]`), `docs/cli.md`
- Create: `crates/yashy-driver/tests/audit.rs`

**Interfaces:**
- Produces: checks — lockfile matches manifest; every package is exact-pinned; every source is `path`/`git@rev`/`registry`; no package contains a build script (it cannot, by construction, but a stray `build.rs` is reported); every package declares a license; the dependency count and total source bytes are reported.
- Produces: `--json` with `{"schema_version":1,"ok":false,"findings":[{"code","package","detail"}]}`; exit 1 on any finding.

- [ ] **Step 1: Write the failing tests** — a clean project is `ok: true`; a lockfile mismatch is `E1802` surfaced as an audit finding; a git dependency without a rev is `E1800` at manifest parse; a package with no license is a finding; the JSON is stable.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — audit is a thin composition of checks that already exist, plus the license and size report. It never runs code from a dependency.
- [ ] **Step 4: Run** `cargo test -p yashy-driver --test audit` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(pkg): yashy audit (E1804)"`

---

### Task 9: Local registry prototype

**Files:**
- Create: `crates/yashy-driver/src/registry.rs`
- Modify: `crates/yashy-driver/src/main.rs` (`publish`, `search`), `docs/cli.md`
- Create: `crates/yashy-driver/tests/registry.rs`

**Interfaces:**
- Produces: a registry is a directory with `index.json` (`{"packages":[{"name","version","hash","description"}]}`) and one tarball per version, content-addressed.
- Produces: `yashy publish --registry file:///…` writes a tarball plus an index entry atomically (temp file, then rename); `yashy add name@1.0.0 --registry …` resolves through the same resolver as Task 3.
- Produces: publishing over an existing version with a different hash is refused.

- [ ] **Step 1: Write the failing tests** — publish then add in a temp registry; a republished different hash is refused; a corrupted tarball fails its hash check on fetch; an index with a duplicate name+version is a registry error naming both.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — tarballs are written by a minimal `tar` writer (ustar, 100-byte names) in the crate; reading uses the same writer's inverse. No compression in v1 (a cap: a registry entry is a `.tar`, size is the package's own problem).
- [ ] **Step 4: Run** `cargo test -p yashy-driver --test registry` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(pkg): a local registry prototype"`

---

### Task 10: Docs and gate

**Files:** `docs/packages.md` (new: manifest grammar, lockfile, audit, registry), `docs/tutorial/12-first-cli.md`, `docs/cli.md`, `AGENTS.md` (layout: `packages/`), `llms.txt`.

- [ ] **Step 1: Write the docs** with a worked publish → depend → build cycle.
- [ ] **Step 2: Run the gate table on both OSes, from a fresh clone with an empty cache and no network for the offline row.**
- [ ] **Step 3: Commit and record** — `git commit -m "docs: packages and libraries, and close phase 10"`.

---

## Gate summary

| Gate | Command | Passes when |
|---|---|---|
| Clean-checkout dependency build | fresh clone, empty cache, `yashy build` on `examples/http-service` | succeeds; lockfile unchanged after |
| Real HTTP service | `cargo test -p yashy-driver --test examples http-service` | keep-alive, chunked, caps, timeouts and 50 concurrent clients pass |
| Reproducible | two builds from the same lockfile | byte-identical binaries |
| Precompiled stdlib equivalent | `cargo test -p yashy-driver --test stdlib_precompiled` | binary hash equals the from-source build |
| Offline | `yashy build --offline` with a warm store | succeeds; with a cold store, `E1803` names the package |
| Audit green | `yashy audit` on the repo's own packages | `ok: true` |
| Registry round-trip | `cargo test -p yashy-driver --test registry` | publish → add → build passes |
| No regression | `cargo test --workspace` | green on Windows and Linux |
