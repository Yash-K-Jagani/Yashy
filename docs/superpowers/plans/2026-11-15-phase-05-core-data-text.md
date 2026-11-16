# Phase 5 — Core Data and Text

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the thin stdlib skeleton with the data types and text
formats a real program needs, each with a documented surface, a deterministic
behaviour and a test that would catch a wrong answer.

**Outcome:** `Vec[T]`, `Map[K,V]`, `Set[T]`, `Deque[T]`, `PriorityQueue[T]`,
`Rc[T]`/`Arc[T]`/`Weak[T]`, a visible allocator interface, Unicode
normalisation and case folding, a formatting engine, full JSON/TOML/YAML/CSV/XML,
URL/MIME/UUID/semver, base64/hex, vetted crypto, compression, a time-zone
database, a CSPRNG, and structured log/trace/metrics. No fallible function
answers with a sentinel.

**Time:** ~8 weeks.
**Depends on:** Phase 4 gate.
**Spec:** [`../specs/2026-11-15-yashy-2.0-design.md`](../specs/2026-11-15-yashy-2.0-design.md) §6.2, §6.6, §6.8.

## Why this phase

1.0's `collections` is a set of functions over `[]T`; `JSON` is `escape` and
`quote`. A program cannot build a map that grows, parse a document, hash a
password, compress a response, or format a date in a named zone. Every later
phase — HTTP, SQL, the application stack, the agent tooling — consumes these
types, so they come first and they come as types, not as conventions.

This is also where the determinism contract gets its most visible teeth:
insertion-ordered maps, stable formatting, a fixed hash seed, and a clock that
is a capability rather than an ambient read.

## Starting point

| Module | 1.0 value |
|---|---|
| `collections` | functions over `[]T`, `map[K]V`, no generic `Vec`/`Set`/`Deque` |
| `alloc` | `alloc`, `arena_*`; no allocator interface |
| `string`/`text` | byte/char operations; no normalisation, case folding or segmentation |
| `JSON` | `escape`, `quote`; no parser, no value type |
| `csv` | row split/join; no quoted-field parser |
| `hash` | a hash function; no `Hash` trait |
| `rand` | a PRNG; no CSPRNG |
| `time`/`datetime` | instant and formatting; no time-zone database |
| `regex` | a subset engine |
| crypto/compression | none |
| `log`/`metrics` | line and counter helpers; no structured form |

## Deliverables

### Task 1: Collections as types

**Files:**
- Create: `crates/yashy-stdlib/yashy/collections/{vec,map,set,deque,pq}.ya`
- Modify: `crates/yashy-stdlib/yashy/collections.ya` (re-export the facade)
- Modify: `crates/yashy-codegen/src/maps.rs`, `crates/yashy-codegen/src/fixed.rs`
- Test: `crates/yashy-driver/tests/programs/stdlib/collections/`, `crates/yashy-driver/tests/stdlib_coverage.rs`

**Interfaces:**
- `Vec[T] { fn push(self: *mut Vec[T], T); fn pop(self: *mut Vec[T]) -> Option[T]; fn get(self, int) -> Option[T]; fn len(self) -> int; fn iter(self) -> VecIter[T] }`
- `Map[K, V] { fn insert(self: *mut Map[K,V], K, V) -> Option[V]; fn get(self, K) -> Option[V]; fn remove(self: *mut Map[K,V], K) -> Option[V]; fn iter(self) -> MapIter[K,V] }`
- `Set[T]`, `Deque[T]`, `PriorityQueue[T]` with the same ownership discipline.

- [ ] **Step 1: Write the failing test.** Grow a `Vec` past its first capacity, insert and remove map keys, iterate a `Map` and assert insertion order, drain a `Deque` from both ends, and pop a `PriorityQueue` with equal keys. Assert no leak and no double free.
- [ ] **Step 2: Order is part of the type.** `Map` and `Set` iterate in insertion order; removing and re-inserting moves a key to the end. `PriorityQueue` breaks ties by insertion sequence. There is no unordered iteration and no randomised seed. The order is documented on the type and tested.
- [ ] **Step 3: Allocation is a parameter.** Every collection takes an `Allocator` (Phase 4) or uses the documented global one; growth is amortised and reported by `--alloc-report`. A collection never allocates through a hidden path.
- [ ] **Step 4: The facade is generated.** `collections.ya` re-exports the types; `yashyc doc` renders them from the same doc comments. `stdlib_coverage` fails on a public function with no program.
- [ ] **Step 5: Commit.**

### Task 2: Reference counting and shared values

**Files:**
- Create: `crates/yashy-stdlib/yashy/rc.ya`, `crates/yashy-stdlib/yashy/arc.ya`
- Modify: `crates/yashy-codegen/src/fn_gen.rs` (refcount helpers), `crates/yashy-runtime/`
- Test: `crates/yashy-driver/tests/programs/stdlib/rc/`, `crates/yashy-driver/tests/leaks.rs` (extend)

- [ ] **Step 1: Write the failing test.** `Rc[T]` shared by three owners, one dropped, the value still readable; a `Weak[T]` upgraded after the last `Rc` is `None`; an `Arc[T]` shared across four threads; a cycle broken with `Weak`. Assert the leak report reads zero on every passing program.
- [ ] **Step 2: One refcount, two atomics.** `Rc` uses a non-atomic count; `Arc` uses an atomic count with the Phase 10 orderings. Both use the allocator interface. `.clone()` is explicit; there is no implicit copy and no `Drop`.
- [ ] **Step 3: Cycles are the program's problem, and the tools say so.** `yashyc inspect --rc` lists live strong/weak counts at a breakpoint or at exit; a cycle is reported as a diagnostic aid, not silently collected.
- [ ] **Step 4: Commit.**

### Task 3: Unicode, text and formatting

**Files:**
- Create: `crates/yashy-stdlib/yashy/text/{unicode,normalize,case,grapheme,width}.ya`
- Create: `crates/yashy-stdlib/yashy/format.ya`
- Modify: `crates/yashy-stdlib/yashy/string.ya`
- Test: `crates/yashy-driver/tests/programs/stdlib/text/`, `crates/yashy-driver/tests/programs/stdlib/format/`

- [ ] **Step 1: Write the failing test.** NFC/NFD/NFKC/NFKD round-trips for the Unicode test file, Turkish dotless `i` case folding, a family emoji counted as one grapheme, a CJK string's display width, and `format("{:>8.2}", 3.14159)`.
- [ ] **Step 2: Unicode data is vendored and versioned.** The tables are generated from a pinned Unicode release, recorded in the SBOM, and updated by a script with a diff. No network at build time.
- [ ] **Step 3: Formatting is one engine.** `print`, `format`, `Debug` and the error renderer share width, precision, alignment and float rules. Float formatting is shortest-round-trip and locale-independent. A format string error is a compile-time diagnostic where the string is a literal, and a `Result` otherwise.
- [ ] **Step 4: Commit.**

### Task 4: Encodings — JSON, TOML, YAML, CSV, XML

**Files:**
- Create: `crates/yashy-stdlib/yashy/json/{value,parse,emit,stream}.ya`
- Create: `crates/yashy-stdlib/yashy/{toml,yaml,csv,xml}.ya` (replace the stubs)
- Modify: `crates/yashy-driver/src/cfg.rs`? no; modify `crates/yashy-stdlib/yashy/JSON.ya` (compatibility facade)
- Test: `crates/yashy-driver/tests/programs/stdlib/json/`, `.../toml/`, `.../yaml/`, `.../csv/`, `.../xml/`
- Create: `crates/yashy-driver/tests/fuzz.rs`, `crates/yashy-driver/tests/fuzz/` corpus seeds

**Interfaces:**
- `Json = Null | Bool(bool) | Int(int) | Float(float) | String(string) | Array([]Json) | Object(Map[string, Json])`
- `parse(text: string) -> Result[Json, json.Error]`
- `emit(value: Json, opts: EmitOptions) -> string`

- [ ] **Step 1: Write the failing test.** Parse a document with nested objects, escapes, surrogate pairs, big integers and duplicate keys; emit it and parse it again; assert byte-stable output with a documented key order (insertion order) and no trailing whitespace. Repeat the round-trip for TOML, YAML, CSV (quoted fields, embedded newlines) and XML (entities, namespaces).
- [ ] **Step 2: No sentinel, ever.** `parse` returns `Result`; a missing key is `Option`; a type mismatch is an error with a byte offset and a line/column. `""` and `0` are values, never failures.
- [ ] **Step 3: Streaming where it matters.** A JSON reader can emit tokens without building a DOM, and a writer can stream; the DOM is a convenience over the same parser. A depth limit and a size limit are explicit options, not constants hidden in the parser.
- [ ] **Step 4: Fuzz the parsers.** Seed the corpus with the conformance files; a crash, a hang or a non-terminating parse is a P0. The fuzzer runs in CI for a bounded time and nightly for longer.
- [ ] **Step 5: Commit.**

### Task 5: Internet primitives

**Files:**
- Create: `crates/yashy-stdlib/yashy/{url,mime,uuid,semver,base64,hex}.ya`
- Test: `crates/yashy-driver/tests/programs/stdlib/internet/`

- [ ] **Step 1: Write the failing test.** Parse and re-emit RFC 3986 URLs (userinfo, IPv6 host, percent-encoding, relative references), match MIME types with parameters, generate and parse UUID v4/v7, compare SemVer 2.0 precedence, and round-trip base64/base64url/hex with padding rules.
- [ ] **Step 2: One parser per grammar.** No regex-based URL parsing; each format has a grammar, a table-driven test and a fuzz corpus. A malformed input returns `Result` with the byte offset.
- [ ] **Step 3: Commit.**

### Task 6: Crypto, CSPRNG and compression

**Files:**
- Create: `packages/crypto/` (Yashy package wrapping a pinned C library)
- Create: `packages/compress/` (zlib and zstd wrappers)
- Modify: `crates/yashy-stdlib/yashy/rand.ya` (CSPRNG), `crates/yashy-stdlib/yashy/hash.ya`
- Test: `crates/yashy-driver/tests/programs/stdlib/crypto/`, `.../compress/`, `packages/crypto/tests/`

**Interfaces:**
- `sha256`, `sha512`, `sha3_256`, `hmac_sha256`, `hkdf_sha256`
- `AesGcm`, `ChaCha20Poly1305` (AEAD with a nonce and associated data)
- `Ed25519`, `X25519`; `csprng_bytes(n) -> Result[[]u8, rand.Error]`
- `gzip`, `zlib`, `zstd` compress/decompress with a size limit

- [ ] **Step 1: Write the failing test.** Published known-answer vectors for
  every primitive; an AEAD tamper test (one flipped bit fails authentication);
  an Ed25519 sign/verify round-trip; a gzip/zstd round-trip of 1 MiB of random
  and of repetitive data.
- [ ] **Step 2: Wrap, do not write.** The primitives come from one pinned,
  audited C library (mbedTLS 3.x LTS is the default call; OpenSSL 3 LTS is the
  alternative if mbedTLS cannot be vendored on a target). The wrapper is thin,
  the version and hash are in the lockfile and SBOM, and the security review is
  a gate. No homegrown S-box, no homegrown curve arithmetic.
- [ ] **Step 3: Constant time where it matters.** Tag comparison, padding
  checks and key derivation use the library's constant-time helpers; a test
  measures the timing distribution and fails on a gross leak. This is a
  smoke test, not a proof; the security review is the real gate.
- [ ] **Step 4: Entropy is a capability.** `csprng_bytes` is the only source of
  randomness; under `--deterministic` it reads a fixed seed and reports that it
  did. A library that calls `rand()` behind the language's back is a defect.
- [ ] **Step 5: Commit.**

### Task 7: Time, zones and observability

**Files:**
- Create: `crates/yashy-stdlib/yashy/time/{instant,duration,calendar,zone}.ya`
- Create: `crates/yashy-stdlib/yashy/observability/{log,trace,metrics}.ya`
- Test: `crates/yashy-driver/tests/programs/stdlib/time/`, `.../observability/`

- [ ] **Step 1: Write the failing test.** Convert an instant to `America/New_York` across a DST boundary, format and parse RFC 3339, add a duration across a leap day, and emit a structured log line with fields, a trace span and a counter; assert the JSON is stable and the clock is fixed under `--deterministic`.
- [ ] **Step 2: The zone database ships with the stdlib.** A pinned tzdata release, versioned in the SBOM, with a lookup that returns `Option`, not a default zone. The clock is a capability: `now()` reads it, `--deterministic` fixes it, and a program that reads it is named in `yashyc trace`.
- [ ] **Step 3: Observability has one sink interface.** Log, trace and metrics write through an injectable sink; the default writes to stderr; a test sink captures structured records. No global mutable logger, no hidden buffering that changes order.
- [ ] **Step 4: Commit.**

## Gates

| Gate | Command | Passes when |
|---|---|---|
| Stdlib coverage | `cargo test -p yashy-driver --test stdlib_coverage` | every public function has a doc comment and a program |
| Collections | `cargo test -p yashy-driver --test programs` | grow/remove/iterate/leak tests pass; insertion order is asserted |
| Encodings | `cargo test -p yashy-driver --test programs` | round-trip and malformed-input tests pass for all five formats |
| Crypto KATs | `cargo test -p yashy-driver --test programs` | every primitive matches its published vectors; AEAD tamper fails |
| Determinism | `cargo test -p yashy-driver --test e2e` | two runs of the stdlib corpus produce identical bytes; fixed clock/seed under `--deterministic` |
| No sentinels | `yashyc doc --json` + `stdlib_coverage` | no documented failure value is `""` or `0`; every fallible function is `Result`/`Option` |
| Fuzz | `cargo test -p yashy-driver --test fuzz` (bounded) | no crash, hang or unbounded allocation on the seed corpus |
| No regression | `cargo test --workspace` | green on Windows and Linux |
| No P0 | `docs/triage.md` | no P0 |

## Demo

```console
$ yashyc run examples/json/main.ya
{"name":"yashy","tags":["fast","safe"],"version":"2.0.0"}

$ yashyc run examples/collections/main.ya
map order: alpha, beta, gamma   // insertion order, stable across runs
vec len=3 cap=4

$ yashyc run examples/crypto/main.ya
sha256 ok  hmac ok  aes-gcm ok  ed25519 ok
```

## Out of scope

- A database or HTTP client (Phases 6–7).
- A regex engine rewrite; the existing subset stays, with a documented grammar.
- Locale-specific collation beyond case folding and normalisation.
- An async filesystem API; I/O stays blocking until Phase 10's task layer wraps it.
- Compression formats beyond gzip/zlib/zstd.

## Risks

| Risk | Mitigation |
|---|---|
| A collection changes iteration order between runs | Order is defined and tested; no randomised hash iteration; a fixed seed |
| A parser accepts malformed input | Round-trip plus malformed corpus plus fuzzing; a limit is explicit |
| A wrapper leaks the C library's failure as a sentinel | Every wrapper returns `Result`/`Option`; a doc test rejects `""`/`0` as a failure value |
| Crypto is subtly wrong | Vetted library, KATs, tamper tests, security review, constant-time helpers |
| Unicode data drifts | Pinned release, generated tables, SBOM entry, update script with a diff |
| The clock makes tests flaky | Clock is a capability; `--deterministic` fixes it; tests use the injected clock |
| Memory growth from refcount cycles | `Weak` is documented; `inspect --rc` reports cycles; the leak gate stays zero |
