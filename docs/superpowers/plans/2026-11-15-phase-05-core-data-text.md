# Phase 5 ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â Core Data and Text

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

**Time:** ~10 weeks.
**Depends on:** Phase 4 gate.
**Spec:** [`../specs/2026-11-15-yashy-2.0-design.md`](../specs/2026-11-15-yashy-2.0-design.md) ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â§6.2, ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â§6.6, ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â§6.8.

## Why this phase

1.0's `collections` is a set of functions over `[]T`; `JSON` is `escape` and
`quote`. A program cannot build a map that grows, parse a document, hash a
password, compress a response, or format a date in a named zone. Every later
phase ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â HTTP, SQL, the application stack, the agent tooling ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â consumes these
types, so they come first and they come as types, not as conventions.

This is also where the determinism contract gets its most visible teeth:
insertion-ordered maps, stable formatting, a fixed hash seed, and a clock that
is a capability rather than an ambient read.

## Starting point

| Module | 1.0 value |
|---|---|
| `collections` | functions over `[]T`, `map[K]V`, no generic `Vec`/`Set`/`Deque`; no `enumerate`, `zip` or other adapters |
| Conversions | `as` for numeric casts, `to_int`/`to_char`/`to_string`, `parse_i64`/`parse_f64`, `str_from_*`; no `parse_bool`, radix parsing, or a uniform checked matrix |
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
- `Map[K, V] { fn insert(self: *mut Map[K,V], K, V) -> Option[V]; fn get(self, K) -> Option[V]; fn get_or(self, K, V) -> V; fn update(self: *mut Map[K,V], K, fn(V) -> V); fn remove(self: *mut Map[K,V], K) -> Option[V]; fn contains_key(self, K) -> bool; fn len(self) -> int; fn keys(self) -> []K; fn values(self) -> []V; fn entries(self) -> [](K, V); fn iter(self) -> MapIter[K,V] }`
- `Set[T] { fn insert(self: *mut Set[T], T) -> bool; fn contains(self, T) -> bool; fn remove(self: *mut Set[T], T) -> bool; fn len(self) -> int; fn is_empty(self) -> bool; fn clear(self: *mut Set[T]); fn union(self, Set[T]) -> Set[T]; fn intersection(self, Set[T]) -> Set[T]; fn difference(self, Set[T]) -> Set[T]; fn subset(self, Set[T]) -> bool; fn to_array(self) -> []T; fn iter(self) -> SetIter[T] }`
- `Deque[T]`, `PriorityQueue[T]` with the same ownership discipline.
- `for k, v in map` and `for item in set` use the Phase 4 tuple pattern and the
  insertion order in Step 2.

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

- [x] **Step 1: Write the failing test.** NFC/NFD/NFKC/NFKD round-trips for the Unicode test file, Turkish dotless `i` case folding, a family emoji counted as one grapheme, a CJK string's display width, and `format("{:>8.2}", 3.14159)`.
- [x] **Step 2: Unicode data is vendored and versioned.** The tables are generated from a pinned Unicode release, recorded in the SBOM, and updated by a script with a diff. No network at build time.
  *Current:* Unicode 18.0.0 UCD files (`UnicodeData`, `CaseFolding`,
  `CompositionExclusions`, `EastAsianWidth`, `NormalizationTest`, `ReadMe`) are
  checked in under `crates/yashy-runtime/data/unicode/` with their SHA-256 pins
  in a README. `scripts/gen_unicode_data.py` is an offline generator that emits
  `crates/yashy-runtime/src/unicode_data.h`: full case folding, East Asian
  width, combining classes, fully recursive canonical and compatibility
  decompositions, and the primary composition pairs, plus input hashes. The
  header is embedded, hashed into both runtime cache keys and written beside
  the runtime sources by the driver. The runtime implements NFC/NFD/NFKC/NFKD
  (UAX #15), full case folding and string display width, and the checker
  exposes `normalize_{nfc,nfd,nfkc,nfkd}`, `case_fold` and `display_width` as
  builtins. The stdlib surfaces them as `normalize.ya` and `width.ya`, each with
  a coverage program. The SBOM lists `unicode-character-database` 18.0.0 keyed
  by the header bytes, with a `unicode_version` metadata field.
  `tests/unicode_data.rs` verifies the recorded hashes, validates every fold and
  width lookup against the source, and runs the full `NormalizationTest.txt`
  conformance matrix (20 invariants per row).
- [ ] **Step 3: Formatting is one engine.** `print`, `format`, `Debug` and the error renderer share width, precision, alignment and float rules. Float formatting is shortest-round-trip and locale-independent. A format string error is a compile-time diagnostic where the string is a literal, and a `Result` otherwise.
  *Current:* `crates/yashy-stdlib/yashy/casing.ya` provides ASCII case
  operations with an explicit contract: `to_ascii_upper` / `to_ascii_lower`
  (delegating to the ASCII-only builtins), `is_ascii_upper` / `is_ascii_lower` /
  `is_ascii_alpha`, and `eq_ignore_ascii_case`. Non-ASCII bytes pass through
  unchanged; `stdlib/casing.ya` pins upper/lower, pass-through, predicates and
  equality in its golden with zero leaks. `normalize.ya` now provides the
  Unicode-aware operations (`nfc`/`nfd`/`nfkc`/`nfkd`, `fold`, `eq_ignore_case`)
  and `width.ya` provides `width`/`is_narrow`, both backed by the vendored
  18.0.0 tables and pinned by `stdlib/normalize.ya` and `stdlib/width.ya`. The
  `format("{:>8.2}", ...)` is now one engine: the checker parses a literal
  specifier (`[[fill]align][0][width][.precision][type]`, with `{{`/`}}`
  escapes) at the call site and reports a malformed field, an argument-count
  mismatch (`E0700`) or a type mismatch (`E0100`) before the program runs. The
  lowering shares `as_string` with interpolation and `print`, and uses the
  runtime's `yashy_str_from_f64_prec` (locale-independent '.') and
  `yashy_str_pad` (display-cell width, sign-aware zero padding). `format.ya`
  exposes those primitives as `fixed` and `pad_left`/`pad_right`/`pad_center`,
  with a coverage program and a tutorial example. The default float spelling is
  now the shortest decimal that reads back as the same double (and is
  locale-independent for `float` interpolation and `print`), while the fixed
  precision path is unchanged. Still owed for full unification: a non-literal
  specifier answering `Result`, and extending the width/precision rules to
  `print` and `Debug`. The step stays open.
- [ ] **Step 4: Commit.**

### Task 4: Encodings ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â JSON, TOML, YAML, CSV, XML

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
- [x] **Step 2: No sentinel, ever.** `parse` returns `Result`; a missing key is `Option`; a type mismatch is an error with a byte offset and a line/column. `""` and `0` are values, never failures.
  *Current:* `json_scan.ya` provides `minify` and `validate`, both returning
  `Result`/`Option` with a byte offset on failure and no sentinel answer. The
  coverage program pins whitespace inside strings and bracket balance.
  `tokens(text, Limits)` streams the same grammar into a flat token list with
  byte spans, explicit `max_depth`/`max_bytes` and typed `TooDeep`/`TooLarge`
  errors. `json_value.ya` now builds a real DOM from the same grammar without
  the blocked `[]Json`/`map[string,Json]` shapes: a document is four parallel
  columns (kind, scalar text, parent index, object key), one row per node, with
  children found by parent in insertion order. `parse_doc`/`parse_with_doc`
  answer `DocError` (byte offset, `TooDeep`, `TooLarge`); `get_doc` answers
  `Option[int]`, and `emit_doc` writes compact, insertion-ordered JSON. The
  first duplicate key wins on lookup while both rows are emitted. `free_doc`
  releases the columns. Typed accessors `int_value`, `float_value` and
  `bool_value` answer `Option` and return `None` for a value of the wrong kind.
  Unicode escapes now decode to UTF-8, including the surrogate pairs a family
  emoji needs, and a malformed escape or lone surrogate is a `DocError`. The
  names carry a `doc` prefix because the stdlib modules share one root scope.
  `DocError.Invalid` now carries byte, line and column, and `describe` renders it. The `Json` enum spelling remains owed.
- [ ] **Step 3: Streaming where it matters.** A JSON reader can emit tokens without building a DOM, and a writer can stream; the DOM is a convenience over the same parser. A depth limit and a size limit are explicit options, not constants hidden in the parser.
- [ ] **Step 4: Fuzz the parsers.** Seed the corpus with the conformance files; a crash, a hang or a non-terminating parse is a P0. The fuzzer runs in CI for a bounded time and nightly for longer.
- [ ] **Step 5: Commit.**

### Task 5: Internet primitives

**Files:**
- Create: `crates/yashy-stdlib/yashy/{url,mime,uuid,semver,base64,hex}.ya`
- Test: `crates/yashy-driver/tests/programs/stdlib/internet/`

- [ ] **Step 1: Write the failing test.** Parse and re-emit RFC 3986 URLs (userinfo, IPv6 host, percent-encoding, relative references), match MIME types with parameters, generate and parse UUID v4/v7, compare SemVer 2.0 precedence, and round-trip base64/base64url/hex with padding rules.
  *Current:* `mime.ya` parses `type/subtype` with a `Result` and exposes
  `essence`, `is_text` and `is_json`; `uuid.ya` has `nil`, lowercase canonical
  `is_valid` and `from_bytes` over `text.hex_encode`; `url.ya` parses
  `scheme://host[:port]/path` with `origin`; `semver.ya` parses core versions
  and compares them, including pre-release precedence and build metadata;
  `uuid.ya` has `v4`; `text.ya` has base64url/hex. The coverage programs are
  `stdlib/{url,mime,uuid,semver}.ya`. `url.ya` now parses userinfo, bracketed
  IPv6 hosts with an optional port, query and fragment, and rejects a malformed
  bracket; the golden pins the parsed parts. `url.ya` also has percent-decoding
  and `resolve(base, ref)` for RFC 3986 §5.2 relative references (all five forms
  plus dot-segment removal), pinned in the golden. `uuid.ya` now has `v7()`
  (RFC 9562, 48-bit ms timestamp + version 7 + random), with the coverage
  program checking its canonical form, version nibble and variant. `url.ya`,
  `mime.ya`, `semver.ya` and `text.ya` now carry byte offsets in their error
  variants, with describe helpers pinned in the coverage programs. The step
  stays open only for a fuzz corpus in Step 2.
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
  *Current:* `rand.u64`, `range`, `fill` and `shuffle` draw from the OS entropy
  pool through one runtime entry point, `yashy_random_i64`. Under
  `--deterministic` that entry point replaces the pool with a seeded SplitMix64
  stream (`YASHY_DETERMINISTIC_SEED`, which the driver fixes at
  `1700000000000`), and `rand.deterministic()` reports which source is in use.
  The e2e test `deterministic_mode_seeds_the_random_source` pins two
  byte-identical runs and the `true` report; the ordinary golden pins `false`.
  The `csprng_bytes` spelling and the crypto-backed primitives remain owed on
  Steps 1ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã‚Â¦ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã¢â‚¬Å“2.
- [ ] **Step 5: Commit.**

### Task 7: Time, zones and observability

**Files:**
- Create: `crates/yashy-stdlib/yashy/time/{instant,duration,calendar,zone}.ya`
- Create: `crates/yashy-stdlib/yashy/observability/{log,trace,metrics}.ya`
- Test: `crates/yashy-driver/tests/programs/stdlib/time/`, `.../observability/`

- [ ] **Step 1: Write the failing test.** Convert an instant to `America/New_York` across a DST boundary, format and parse RFC 3339, add a duration across a leap day, and emit a structured log line with fields, a trace span and a counter; assert the JSON is stable and the clock is fixed under `--deterministic`.
- [ ] **Step 2: The zone database ships with the stdlib.** A pinned tzdata release, versioned in the SBOM, with a lookup that returns `Option`, not a default zone. The clock is a capability: `now()` reads it, `--deterministic` fixes it, and a program that reads it is named in `yashyc trace`.
- [x] **Step 3: Observability has one sink interface.** Log, trace and metrics write through an injectable sink; the default writes to stderr; a test sink captures structured records. No global mutable logger, no hidden buffering that changes order.
  *Current:* `observability.ya` provides `event_json`, `emit`, `trace_json`,
  `metric_json` and their `trace`/`metric` emitters; all take a
  `fn(string) -> void` writer. `stderr_sink` and `emit_stderr` are the default
  sink. There is no global logger and no buffering, and
  `stdlib/observability.ya` captures the structured lines in a test function.
  CI must confirm the golden output.
- [ ] **Step 4: Commit.**

### Task 8: Iteration adapters and the conversion matrix

**Files:**
- Create: `crates/yashy-stdlib/yashy/iter/{enumerate,zip,adapters}.ya`
- Create: `crates/yashy-stdlib/yashy/convert.ya`
- Modify: `crates/yashy-stdlib/yashy/iter.ya` (re-export the facade)
- Modify: `crates/yashy-sema/src/check_expr.rs` (method names), `crates/yashy-codegen/src/higher_order.rs`
- Test: `crates/yashy-driver/tests/programs/stdlib/iter/`, `.../convert/`

**Interfaces:**
- `xs.enumerate() -> Enumerate[T]` yields `(int, T)`; `xs.zip(ys) -> Zip[A, B]` yields `(A, B)` and stops at the shorter sequence
- `xs.chunks(int) -> Chunks[T]`, `xs.windows(int) -> Windows[T]`, `xs.take(int) -> Take[T]`, `xs.skip(int) -> Skip[T]`, `xs.rev() -> Rev[T]`, `xs.chain(ys) -> Chain[T]`
- `parse_i64(text) -> Option[int]`, `parse_f64(text) -> Option[float]`, `parse_bool(text) -> Option[bool]`, `parse_int(text, int radix) -> Option[int]`
- `int.to_float() -> float`, `float.to_int() -> int`, `float.round() -> int`, `float.floor() -> int`, `float.ceil() -> int`, `float.trunc() -> int`; `to_string` on every scalar
- `T.try_from(string) -> Result[T, ConvertError>` for the types that need a reason, not just a miss

- [ ] **Step 1: Write the failing test.** `for i, x in xs.enumerate()`
  reports `0, 1, 2` and the elements in order; `xs.zip(ys)` stops at the
  shorter sequence and never reads past it; `xs.chunks(3)` and
  `xs.windows(2)` cover every element exactly once; `take`/`skip`/`rev`/`chain`
  preserve order; a set of 10,000 integers inserts and looks up in expected
  constant time and iterates in insertion order; `map.get_or`, `map.update`,
  `map.entries` and `for k, v in map` agree; `parse_bool("true")` is
  `Some(true)`, `parse_bool("TRUE")` is `None`, and `parse_int("ff", 16)` is
  `Some(255)`.
  *Current:* `crates/yashy-stdlib/yashy/iter_adapters.ya` implements the
  adapter behaviours as generic free functions with callbacks:
  `enumerate`, `zip`, `chunks`, `windows`, `take`, `skip`, `rev` and `chain`.
  `stdlib/iter_adapters.ya` prints a deterministic line per adapter, including
  a `rev`-twice round-trip, and pins the exact bytes in its golden; the e2e
  harness confirms two-run byte equality and zero leaks. `zip` and `chain`
  return a `PairSeq[A,B]` struct (a tuple cannot own a slice). The method
  spellings (`xs.enumerate()`) and `for i, x` over an adapter are still owed:
  `for i, x` is slice-only, a generic struct cannot implement `Iterator[T]`,
  and a slice cannot hold a tuple ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â all compiler changes. Those halves of
  Step 1 and Steps 2ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã‚Â¦ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã¢â‚¬Å“3 stay open.
- [ ] **Step 2: An adapter adds no HIR.** `enumerate`, `zip`, `chunks`,
  `windows`, `take`, `skip`, `rev` and `chain` are ordinary generic structs
  implementing the compiler-known `Iterator[T]` protocol; `for` over one lowers
  to the existing loop. A tuple binding in the head is the Phase 4 tuple
  pattern, and a sequence shorter than the adapter needs ends cleanly.
- [ ] **Step 3: `zip` is total and ordered.** It stops at the shorter input, is
  eager only when the caller asks for an array, and never allocates on the
  per-item path. `enumerate` starts at 0 and never yields a negative index; a
  reversed range yields nothing rather than counting down.
- [x] **Step 4: One conversion matrix, no sentinels.** Every fallible
  conversion returns `Option` or `Result`; every lossy numeric conversion is
  spelled with `round`, `floor`, `ceil` or `trunc`, and `as` stays the
  truncating cast the tutorial documents. A conversion that cannot fail
  (`int.to_float()`, `int.to_string()`) is a plain method. `parse_int` checks
  the radix (`2..=36`), leading sign and overflow, and never wraps. `try_from`
  returns a `ConvertError` that names the input and the expected form.
  *Current:* `crates/yashy-stdlib/yashy/convert.ya` is in place with a
  documented `ConvertError`, `Result`-returning `parse_bool`, `parse_int`,
  `parse_float` and `parse_radix`, plain `int_to_string`/`float_to_string`/
  `bool_to_string`, and explicit `trunc`/`floor`/`ceil`/`round`/`try_from_float`.
  Radix parsing bounds the magnitude at 2147483647 so it never panics, and
  reports `Overflow` instead. `stdlib/convert.ya` is the coverage program. The
  scalar method spellings now exist in the checker: `int.to_float`,
  `float.to_int`, `float.trunc`/`floor`/`ceil`/`round` (all answering `int`,
  lowering to `yashy_f64_round_to`) and `to_string` on every scalar. The
  coverage program pins their bytes, and `docs/stdlib.md` lists the free
  functions they mirror. `try_from` still has only the float form, so the step
  stays open.
- [ ] **Step 5: Formatting consumes the same conversions.** The `format`
  engine in Task 3 formats each scalar through this matrix, so a value printed
  by `print`, `format` and `Debug` has one spelling. A `format` specifier on a
  type with no conversion is a compile-time diagnostic for a literal format
  string.
- [ ] **Step 6: Commit.**
  `git commit -m "feat(stdlib): iteration adapters and a total conversion matrix"`.

## Gates

| Gate | Command | Passes when |
|---|---|---|
| Stdlib coverage | `cargo test -p yashy-driver --test stdlib_coverage` | every public function has a doc comment and a program |
| Collections | `cargo test -p yashy-driver --test programs` | grow/remove/iterate/leak tests pass; insertion order is asserted; `Set`/`Map` methods have programs |
| Iteration and conversions | `cargo test -p yashy-driver --test programs` | `enumerate`/`zip`/`chunks`/`windows`/`take`/`skip`/`rev`/`chain` preserve order; every fallible conversion returns `Option`/`Result`; no sentinel |
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
set 3 in {1, 3, 5} = true

$ yashyc run examples/iter/main.ya
0 alpha  1 beta  2 gamma        // enumerate
(1, "a") (2, "b")              // zip stops at the shorter
parse_int("ff", 16) = 255       // a total conversion

$ yashyc run examples/crypto/main.ya
sha256 ok  hmac ok  aes-gcm ok  ed25519 ok
```

## Out of scope

- A database or HTTP client (Phases 6ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Â¦Ãƒâ€šÃ‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã‚Â¦ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã¢â‚¬Å“7).
- A regex engine rewrite; the existing subset stays, with a documented grammar.
- Locale-specific collation beyond case folding and normalisation.
- An async filesystem API; I/O stays blocking until Phase 10's task layer wraps it.
- Compression formats beyond gzip/zlib/zstd.
- Lazy iterators that allocate or suspend; an adapter is a struct over a
  sequence, and an eager array is the only materialised form.
- Locale-aware number parsing or printing; the matrix is ASCII and
  locale-independent.

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
