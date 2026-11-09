# Phase 12 — Libraries and Packages: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a Yashy program able to depend on code it did not write, reproducibly and offline, ship a standard library that is a library rather than thirteen files of prose, and ship enough first-party packages that a real HTTP service is a weekend project rather than a research project.

**Architecture:** A manifest (`yashy.toml`) is parsed by a deliberately small TOML subset reader in a new `yashy-manifest` crate — no dependency, a documented grammar, and a 30-manifest conformance corpus. Resolution is exact-pin only: a lockfile names every package, version, source and content hash, and the resolver refuses anything it cannot pin. Dependencies are compiled from source with the phase 3 cache, so a second build is free. The stdlib becomes a package like any other: versioned, locked, and shipped as cached IR so `hello` does not re-typecheck it. Networking is new C in the runtime — sockets on both platforms — and `http` is a Yashy package on top of it.

**Tech Stack:** `yashy-manifest` (new, zero deps), `yashy-driver/src/{resolve.rs,fetch.rs,audit.rs}`, the phase 3 `yashy-cache`, `yashy-runtime` (Winsock/POSIX sockets, OS entropy, process control, directory iteration, file metadata), `crates/yashy-stdlib/` as a versioned package with the modules from Task 10, and new `packages/{http,cli,net,test,bench,fuzz}`.

**Spec:** `docs/superpowers/specs/2026-10-27-yashy-master-design.md` §7. Index: `docs/superpowers/plans/2026-10-27-yashy-master-plan.md`. Predecessor: `2026-10-27-phase-11-ai-native-observability.md`.

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
4. **A cached build must be byte-identical to a from-source build.** The gate builds the same program cold, warm and with `--stdlib-source`, then compares the linked images after zeroing the only byte a linker fills from the clock (the PE header's `TimeDateStamp`). A cache that changes semantics is worse than no cache. The cache stores emitted IR, not serialised stdlib HIR: the stdlib is not ambient (see Task 4's ruling), so there is no whole-stdlib typecheck to remove.
5. **The HTTP server must be a real server**: it handles keep-alive, chunked requests, a 10 MB body cap, slowloris timeouts, and concurrent connections, and the test suite proves each. "Real HTTP service" in the gate means these tests, not a hello-world handler.
6. **A stdlib function that can fail returns `Result` or `Option`.** Phase 3b changed the seven builtins that answered `""` or `0`; this phase must not reintroduce the pattern in the modules it rewrites. The gate greps the doc blocks for the sentinel phrasing, because a doc block that says "or the empty string" is the same bug with a comment on it.
7. **`regex` returns an error rather than a wrong answer.** An unsupported construct (`(?=`, `\1`, `*?`) is `Err(Unsupported)` at compile time, and exhausting the backtracking budget is `Err(Budget)` at match time. A matcher that silently falls back to a substring search when its pattern is too hard is the failure mode this rule exists to prevent.
8. **Determinism is a stdlib property, not a test-only one.** `env.vars` and `fs.read_dir` are sorted, `hash` and `text` are pure, and nothing reads a clock or an RNG unless its name says so. A program that prints a directory listing must print the same listing twice.

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

- [x] **Step 1: Write the failing tests** — all 15 valid manifests parse to the expected struct; each of the 15 invalid ones produces `E1800` at the right line; a 1 MiB manifest is rejected with a size diagnostic rather than a slow parse; a `git` dependency without `rev` is `E1800` ("a git dependency is pinned to a commit").
- [x] **Step 2: Watch them fail.**
- [x] **Step 3: Implement** — a line-oriented reader with a small state machine; keys are `BTreeMap` so error order is deterministic. The parser is total: every input returns `Result`, never panics, and the fuzz corpus from phase 9's harness gains ten manifest cases.
- [x] **Step 4: Run** `cargo test -p yashy-manifest` → PASS.
- [x] **Step 5: Commit** — `git commit -m "feat(manifest): a small, total TOML subset and yashy.toml (E1800)"`

**Ruling recorded during implementation:** the lockfile format needs
`[[package]]`, and the manifest subset as written has no array-of-tables form.
The reader accepts `[[name]]` (append a table to the array at `name`) and keeps
rejecting a repeated plain `[name]`, so a manifest typo is still caught. The
subset's documented grammar in `docs/cli.md` includes this form. The compiler
and driver also share the crate through the root workspace member list.

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

- [x] **Step 1: Write the failing tests** — round-trip a lockfile byte-identically; changing a manifest version without updating the lock is `E1802` naming both versions; a hand-edited hash that does not match the source is `E1802`; the lockfile is sorted by name then version.
- [x] **Step 2: Watch them fail.**
- [x] **Step 3: Implement** — hashing is a streaming SHA-256 written in the crate (about 120 lines, no dependency), with a test vector from the FIPS 180-4 examples so the implementation is proven against a known answer rather than against itself.
- [x] **Step 4: Run** `cargo test -p yashy-manifest` → PASS.
- [x] **Step 5: Commit** — `git commit -m "feat(manifest): a content-hashed, sorted lockfile (E1802)"`

**Ruling recorded during implementation:** `yashy lock --check` is the whole
command in this task; the generating half of `yashy lock` arrives with
`yashy update` in Task 3, when there is a store to fetch from. `yashy build`
verifies a lock only when one exists, so the existing example projects (which
have a `yashy.toml` and no lock yet) keep building. Registry entries are checked
by name and exact version here; a path dependency's tree is hashed and compared.
The driver test `tests/lock_cli.rs` proves all four behaviours.

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

- [x] **Step 1: Write the failing tests** — a path dependency compiles; a diamond resolves one copy; a cycle is `E1801` with the path; a git dependency at a fixed rev is reproducible across two fetches; `--offline` with a cold store is `E1803`; `--offline` with a warm store builds.
- [x] **Step 2: Watch them fail.**
- [x] **Step 3: Implement** — DFS with a visiting stack for cycles (the phase 6 module-graph pattern, reused); fetching shells out to `git` only when `git` is on `PATH`, and the absence of `git` is a documented diagnostic, not a panic. Fetched trees are read-only in the store.
- [x] **Step 4: Run** `cargo test -p yashy-driver --test dependencies` → PASS.
- [x] **Step 5: Commit** — `git commit -m "feat(pkg): resolve path and pinned git dependencies, offline (E1801, E1803)"`

**Ruling recorded during implementation:** the resolver has no version solver and no
registry: a dependency is a path, a `git`+`rev`, or a registry pin that fails
with `E1803` ("no registry is configured") until Task 9 ships the local
registry. `yashy update` is the only writer of `yashy.lock`; `yashy fetch`
populates the store without writing one. `--offline` is accepted by `build`,
`run`, `fetch` and `update` and never invokes git. The build exposes the entry
directory and every resolved package root through `YASHY_PATH`, which the
import loader already honours, so no loader change was needed. On Windows the
clone is staged at a shallow path and renamed into the keyed store directory;
staging beside the deep destination exceeded the path limit inside git's own
`.git/objects/pack` before the clone could finish. Test
`crates/yashy-driver/tests/dependencies.rs` proves all six behaviours, with a
private `YASHY_PACKAGE_STORE` per test.

---

### Task 4: The precompiled stdlib

**Files:**
- Create: `crates/yashy-stdlib/yashy.toml` (the stdlib is a package: name `stdlib`, version 1.0.0)
- Modify: `crates/yashy-driver/src/{modules.rs,pipeline.rs,main.rs}` (a whole-program emitted-IR cache, `--stdlib-source`, the `ir_cache` and `stdlib_modules` timing rows)
- Modify: `crates/yashy-cache/src/lib.rs` (`open_at`, `root`, atomic `put`)
- Modify: `crates/yashy-common/src/source.rs` (`SourceMap::len`/`is_empty`)
- Modify: `scripts/{package-release.ps1,package-release.sh,install.sh,test-install.ps1}` (ship and install the stdlib manifest)
- Modify: `docs/cli.md` (`--stdlib-source`, `ir_cache=hit|miss`, `stdlib_modules=N`)
- Create: `crates/yashy-driver/tests/stdlib_precompiled.rs`

**Interfaces:**
- Produces: `hello` typechecks only `main.ya` plus the stdlib symbols it imports, not every stdlib module. The loader never made the stdlib ambient — `modules::load_sources` adds only the entry point and its imports — so the plan's premise that every build typechecks the whole stdlib is false. `--timings` reports `stdlib_modules=N` to make the fact checkable: `0` for `hello`, `1` for a program that imports `JSON`.
- Produces: a cache of the program's *emitted IR*, keyed by every module source (path and text), the codegen flags, the compiler version and executable mtime, `IR_CACHE_FORMAT`, the stdlib version, the target triple and the entry name. A hit reuses the IR and prints `ir_cache=hit` with `timing: emit 0 ms`; a miss emits and stores; a cache that cannot be read or written falls back to emitting. The phase 3 `yashy-cache` store is reused, rooted at `YASHY_CACHE_DIR/ir`.
- Produces: a `--stdlib-source` escape hatch that neither reads nor writes the cache, used by the equivalence test.

**Ruling (2026-11-09): the IR cache replaces the plan's timing gate.** The plan asked for the stdlib's HIR to be serialised and for `stdlib_ms` to drop by 80%. Neither premise holds. The stdlib is not ambient, so there is no per-build stdlib typecheck to remove; and a wall-clock threshold is not reproducible on a shared CI machine. What is measurable and deterministic is that the emitted IR was reused rather than produced: `ir_cache=hit`, `timing: emit 0 ms`, and a binary byte-identical to the from-source build apart from the PE header's `TimeDateStamp` — the one field the linker fills from the clock, which the test normalises. The `hello_without_stdlib_ms` bench row is dropped for the same reason: `hello` already loads zero stdlib modules, so the row would measure nothing. The cache applies to a program with no stdlib import too, which is the larger win.

- [x] **Step 1: Write the failing test** — build `examples/json-report` cold, warm and with `--stdlib-source`; assert `ir_cache=miss` then `ir_cache=hit`, `timing: emit 0 ms`, `stdlib_modules=1`, and that the linked images are identical after zeroing the PE `TimeDateStamp`. `hello` reports `stdlib_modules=0`. Bumping the version of a copied stdlib turns a hit into a miss.
- [x] **Step 2: Watch it fail.**
- [x] **Step 3: Implement** — the driver consults the store before `emit` and stores the text on a miss. A miss, an unreadable cache or a failed write all fall back to emitting, because a cache is an optimization and never a requirement. `--stdlib-source` disables the cache entirely; `IR_CACHE_FORMAT` is bumped whenever codegen starts emitting different bytes for the same input.
- [x] **Step 4: Run** `cargo test -p yashy-driver --test stdlib_precompiled` → PASS.
- [x] **Step 5: Commit** — `git commit -m "perf(stdlib): ship the stdlib as cached IR"`

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

### Task 10: Stdlib breadth

**Files:**
- Create: `crates/yashy-stdlib/yashy/{collections,text,regex,csv,glob,term,datetime,hash}.ya`
- Rewrite: `crates/yashy-stdlib/yashy/{args,env,fs,os,process,rand,string,time}.ya` (the eight modules that are currently documentation stubs — see the note below)
- Modify: `crates/yashy-runtime` (C: OS entropy, process spawn/wait/pipes, directory iteration, file metadata, `isatty`, monotonic clock)
- Modify: `crates/yashy-sema/src/check_expr.rs` (the slice method `sort_by`, if it is not already covered by the closure machinery from phase 8)
- Create: `crates/yashy-driver/tests/programs/stdlib/` (one program per module, pinned output)
- Create: `crates/yashy-driver/tests/stdlib_coverage.rs`
- Modify: `crates/yashy-driver/tests/e2e.rs`, `docs/cli.md` (nothing new — say so)

**Interfaces:**

| Module | Adds | Runtime C |
|---|---|---|
| `collections` | `Set[T]` (insert/remove/contains/len/to_array/free), `Deque[T]` (push/pop at both ends, front/back/len/free), `PriorityQueue[T]` (binary heap over `[]T` with a `fn(T, T) -> i64` comparator; `push`, `pop -> Option[T]`, `peek -> Option[T]`), and `slice.sort_by(cmp)` | no |
| `text` | `base64_encode([]u8) -> string`, `base64_decode(string) -> Result[[]u8, text.Error]`, `hex_encode`, `hex_decode` | no |
| `regex` | `Regex::compile(pat) -> Result[Regex, regex.Error]`; `is_match`, `find -> Option[Match]`, `find_all -> []Match`, `replace_all`. Subset: literals, `.`, `*`, `+`, `?`, `[...]` with ranges and `^`, `(...)`, `\|`, `^`, `$`, `\d \w \s` and negations. **No** backreferences, lookaround, lazy quantifiers or Unicode classes — those compile to `Err(Unsupported)`, never to a wrong answer. A backtracking matcher with a step budget; exhaustion is `Err(Budget)` | no |
| `csv` | RFC 4180 reader and writer: quoted fields, doubled quotes, embedded newlines, CRLF and LF both accepted on input, LF on output with an opt-in CRLF | no |
| `glob` | `matches(pattern, path) -> bool` with `*`, `?`, `**`, `[abc]`, `[a-z]`, `[!...]`. **No** brace expansion | no |
| `term` | `is_tty() -> bool`, `color(name)`, `bold()`, `reset()`, `move_to(row, col)`, `clear_screen()`. `color` returns `""` when `NO_COLOR` is set, `TERM=dumb`, or stdout is not a tty | yes (`isatty`) |
| `datetime` | `DateTime` (fields + `offset_minutes`), `now`, `from_unix`, `to_unix`, `add_seconds`, `format(fmt)`, `parse(s, fmt)`. Specifiers `%Y %m %d %H %M %S %f %z %%` only. **No** locale, no timezone database, no DST: a civil time carries a fixed offset, and that limit is documented in the module | yes (clock) |
| `hash` | `fnv1a64`, `xxhash64`, `sha256` (returns a 32-byte `[]u8`; the length is asserted, not implied). The module says in one line that none of these is a password hash | no |
| `rand` | `fill([]u8)`, `u64()`, `range(lo, hi)` by rejection sampling (no modulo bias), `shuffle([]T)`. Replaces the current header, which admits there is no entropy source | yes (`getrandom` / `BCryptGenRandom`) |
| `process` | `Command::new([]string)`, `.env`, `.cwd`, `.capture_stdout`, `.spawn -> Result[Child, io.Error]`; `Child.wait -> Result[int, io.Error]`, `Child.stdout -> Result[string, io.Error]`, `Child.kill`, `Child.id`. **Argv only, never a shell** — that is the injection defence, and it is a test | yes (spawn/wait/pipes) |
| `fs` | `read_dir -> Result[[]DirEntry, io.Error]`, `metadata -> Result[Metadata, io.Error]` (`size: usize`, `is_dir: bool`, `modified_unix: i64`), `create_dir`, `create_dir_all`, `remove_file`, `remove_dir`, `rename`, `copy`, `exists` | yes |
| `path` | `absolute`, `normalize`, `file_stem`, `parent`, `is_absolute` (beside the existing `join`/`basename`/`dirname`/`extension`) | no |
| `env` | `vars() -> []EnvVar` **sorted by name**, `set`, `unset` | yes |
| `io` | `read_all -> Result[[]u8, io.Error]` (stdin), `write_stderr` | yes |
| `time` | `monotonic_ns() -> u64`, `now_unix_ns() -> i64` | yes |
| `os` | `platform() -> Platform`, `arch() -> Arch` (fieldless enums), `pid() -> i64` | yes |
| `string` | `join([]string, sep)`, `pad_start`, `pad_end`, `char_at -> Option[string]` (one grapheme, per phase 6), `to_bytes -> []u8`, `from_bytes -> Result[string, text.Error]` — UTF-8 validated, so invalid input is an error and never a replacement character | no |
| `args` | `all() -> []string`, `get(i) -> Option[string]`, `flag(name) -> bool` for `--name` | no |

- Produces: **no sentinel on failure.** Every function that can fail returns `Result` or `Option` — the phase 3b rule, applied to every module. `grep` for a `-> string` function whose doc block says "or the empty string" is a gate.
- Produces: **every module has at least one `pub fn` with a `///` block and at least one test program.** `crates/yashy-driver/tests/stdlib_coverage.rs` walks `crates/yashy-stdlib/yashy/*.ya` and fails when a module has no documented function or no program under `tests/programs/stdlib/` that imports it. This is the mechanical form of "the stdlib is a product, not a stub".
- Produces: **determinism where it is cheap.** `env.vars` is sorted; `fs.read_dir` is sorted by name; `hash` and `text` are pure; nothing in the stdlib reads a clock or an RNG unless the function's name says so. The programs assert this by running twice and comparing bytes.
- Produces: the eight rewritten modules keep their existing function names where they have one (`math.abs`, `path.join`, `JSON.escape`), so this task adds rather than renames. `math.sqrt` is already a `link_name` wrapper after phase 3b.
- Produces: every module is part of the precompiled stdlib package from Task 4, so `hello` does not pay for `regex` being present.

**Note on the eight stubs.** `alloc`, `args`, `env`, `fs`, `io`, `JSON`, `math`, `os`, `path`, `process`, `rand`, `string`, `time` are thirteen modules today; five of them (`alloc`, `io`, `JSON`, `math`, `path`) hold real code and are left alone. The other eight are prose that describes builtins living in the one shared scope, and their headers say so ("the language has no `Option` in a builtin's signature yet"). Phase 3b removed that excuse. This task gives them the functions they describe, as real functions where the language can express them and as documented builtin re-exports where it cannot.

- [ ] **Step 1: Write the failing tests** — one program per module with pinned output; `Set[i64]` deduplicates and `to_array` is order-independent when compared as a set; `Deque` push/pop at both ends; `PriorityQueue` pops in comparator order; `sort_by` sorts by the closure; base64 and hex round-trip every byte value 0–255; `regex` matches and rejects the documented subset and returns `Err(Unsupported)` for a backreference; `csv` round-trips a field containing a comma, a quote and a newline; `glob` matches `**` across separators and not a `?` against `/`; `term.color` returns `""` under `NO_COLOR`; `datetime` round-trips `%Y-%m-%dT%H:%M:%S%z`; `hash.sha256("abc")` equals the published digest; `rand.range(0, 3)` never returns 3 and the distribution over 100 000 draws is within 1% of uniform; `process` runs a child, captures its stdout and its exit code, and a child that does not exist is `Err`; `fs.read_dir` is sorted; `path.normalize` collapses `..` lexically and says so in its doc block; `env.vars` is sorted; `time.monotonic_ns` never goes backwards over 10 000 calls; `os.platform` matches the build target.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — pure-Yashy modules first (`collections`, `text`, `csv`, `glob`, `hash`, `datetime`'s arithmetic, `regex`), then the runtime additions behind them. The C functions follow the phase 3b convention: a tag plus an out-parameter, never a sentinel. `PriorityQueue` and `Deque` are Yashy structs over `[]T`, so the leak suite sees them like any other array. `regex` compiles to a small instruction list and the matcher is one loop with a step counter, so the budget is a counter, not a timeout.
- [ ] **Step 4: Run** `cargo test -p yashy-driver --test stdlib_coverage --test e2e` and the leak suite → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(stdlib): collections, text, regex, csv, glob, term, datetime, hash, entropy and process control"`

---

### Task 11: Docs and gate

**Files:** `docs/packages.md` (new: manifest grammar, lockfile, audit, registry), `docs/stdlib.md` (new: one section per module, generated from the doc blocks by `yashy doc` so it cannot drift), `docs/tutorial/12-first-cli.md`, `docs/cli.md`, `AGENTS.md` (layout: `packages/`; the stdlib rule that a failure is a `Result`/`Option`, never a sentinel), `llms.txt`.

- [ ] **Step 1: Write the docs** with a worked publish → depend → build cycle.
- [ ] **Step 2: Run the gate table on both OSes, from a fresh clone with an empty cache and no network for the offline row.**
- [ ] **Step 3: Commit and record** — `git commit -m "docs: packages and libraries, and close phase 12"`.

---

## Gate summary

| Gate | Command | Passes when |
|---|---|---|
| Clean-checkout dependency build | fresh clone, empty cache, `yashy build` on `examples/http-service` | succeeds; lockfile unchanged after |
| Real HTTP service | `cargo test -p yashy-driver --test examples http-service` | keep-alive, chunked, caps, timeouts and 50 concurrent clients pass |
| Reproducible | two builds from the same lockfile | byte-identical binaries |
| Precompiled stdlib equivalent | `cargo test -p yashy-driver --test stdlib_precompiled` | cached and from-source images differ only in the PE `TimeDateStamp`; `ir_cache=hit` with `emit 0 ms` |
| Offline | `yashy build --offline` with a warm store | succeeds; with a cold store, `E1803` names the package |
| Audit green | `yashy audit` on the repo's own packages | `ok: true` |
| Stdlib is a product | `cargo test -p yashy-driver --test stdlib_coverage` | every module in `crates/yashy-stdlib/yashy/` has a documented `pub fn` and a test program |
| No sentinel on failure | `cargo test -p yashy-driver --test stdlib_coverage` | no `-> string`/`-> int` stdlib function documents "or the empty string"/"or zero" as its failure value |
| Stdlib is deterministic | `cargo test -p yashy-driver --test e2e` | two runs of every `tests/programs/stdlib/` program produce identical bytes |
| Registry round-trip | `cargo test -p yashy-driver --test registry` | publish → add → build passes |
| No regression | `cargo test --workspace` | green on Windows and Linux |
