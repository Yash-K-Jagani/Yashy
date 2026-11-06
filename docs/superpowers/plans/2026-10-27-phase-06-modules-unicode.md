# Phase 6 — Modules and Unicode: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make large projects possible (namespacing, enforced visibility, package-prefixed imports, no import cycles) and make strings correct for people who do not write English (grapheme-aware length, indexing and slicing), with the verifier covering both.

**Architecture:** The module loader in `yashy-driver/src/modules.rs` already reads imports into a `SourceMap`; it gains a graph with cycle detection and a visibility table. Name resolution in `sema` already carries `Vec<String>` paths on `HirExpr::Ident`/`Call`, so qualified names are enforcement, not new syntax. Strings stay UTF-8 and refcounted; the runtime gains grapheme-cluster boundaries, and the verifier's bounds domain learns string lengths.

**Tech Stack:** `crates/yashy-driver/src/modules.rs`, `crates/yashy-sema/src/check_expr.rs` (resolution), `crates/yashy-runtime` (C string functions), `crates/yashy-stdlib/yashy/string.ya`.

**Spec:** `docs/superpowers/specs/2026-10-27-yashy-master-design.md` §8 row 6. Index: `docs/superpowers/plans/2026-10-27-yashy-master-plan.md`. Predecessor: `2026-10-27-phase-05-syntax-ergonomics.md`.

## Revision note (2026-11-06)

Written against today's code, not the code the plan was drafted from. Three
findings changed task detail:

- **The module graph already exists.** `yashy-driver/src/modules.rs` walks
  imports depth-first with `Loading`/`Loaded` states, reports a cycle naming
  both modules, and loads a diamond's shared module once; deterministic
  pre-order is preserved. Task 1 is therefore finishing: the `E0802` code,
  the `ModuleGraph` query surface for phase 4, and the self-import / 50-module
  tests — not building the loader.
- **Visibility is already enforced.** `crates/yashy-sema/tests/visibility.rs`
  (11 tests) pins private-item rejection across modules, including fields,
  types, `const fn`, and generics. Task 2 is coding the existing diagnostics
  as `E0801` with catalog pages, plus the `Binding.visibility` record.
- **No grapheme work exists yet.** `crates/yashy-runtime` has no grapheme
  functions and `yashy-stdlib/yashy/string.ya` holds only `concat`. Tasks 4–6
  are greenfield, in plan order.
- **Aliased imports do not exist.** `Item::Import` carries only `path`; there
  is no `as` clause in the parser, lexer, or loader. Task 3 is greenfield.

## Global Constraints

See the master index. The ones that bite here:

- No new runtime dependencies. Unicode tables are vendored data files, not a crate.
- No implicit coercion; a byte index and a grapheme index are different functions, not overloads.
- One rule, one implementation: the module graph is built once, in the driver.
- **The phase 4 query engine must learn module queries here, not be bypassed.** The graph is a query input (`module_graph(entry)`), and every per-file query key gains its imports' hashes; a module edit therefore invalidates its importers and nothing else. Building the graph outside the database would make phase 4's invalidation unsound the first time a project has two files.
- Both operating systems or it is not done.

## Review Focus

1. **A private item reached through a module path is an error, not a warning**, and the message names both the item and the module that hides it.
2. **An import cycle reports once**, at the import that closes the cycle, not once per file.
3. **`len()` keeps its current meaning (bytes).** Changing it silently would break every program that counts bytes, including `examples/wordcount`. Grapheme length is a new function with a new name.
4. **A grapheme index out of range is a verifier error where provable and a runtime fault otherwise**, never a silent truncation.
5. **The Unicode conformance suite is vendored and pinned**, so a build does not depend on the network.

---

### Task 1: A module graph with cycle detection

**Files:**
- Modify: `crates/yashy-driver/src/modules.rs`
- Create: `crates/yashy-driver/tests/modules.rs` additions
- Modify: `crates/yashy-common/src/codes.rs` (`MODULE_CYCLE = "E0802"`), `docs/errors/E0802.md`

**Interfaces:**
- Produces: `pub struct ModuleGraph { pub modules: Vec<Module>, pub edges: Vec<(FileId, FileId)> }`; the loader records each import edge as it walks, so `Db` can expose the graph as a query input. `modules::load` keeps its current return shape and gains a `module_graph` projection instead, so callers do not churn.
- Produces: the graph as a phase 4 query — `Db::module_graph(entry)` — so a change to `b.ya` invalidates `a.ya`'s check when `a` imports `b`, and a change to an unrelated file invalidates nothing.
- Produces: `E0802` at the import that closes a cycle, with the cycle path in the diagnostic message (`a -> b -> a`).

- [x] **Step 1: Write the failing tests** — a two-file cycle is `E0802` once (already reported as an uncoded "import cycle" message; this codes it); a diamond loads `d` once (already pinned by the driver's `modules.rs` suite); a self-import is `E0802`; a 50-module chain resolves; the module order is deterministic across runs.
- [x] **Step 2: Watch them fail.**
- [x] **Step 3: Implement** — DFS with a `visiting` stack and a `loaded` map keyed by canonical path already exist in `SourceLoader`; record the edges alongside, code the cycle diagnostic `E0802`, and project the graph for `Db::module_graph`. Landed as `b839722`.
- [x] **Step 4: Run** `cargo test -p yashy-driver --test modules` → PASS (12/12).
- [x] **Step 5: Commit** — `git commit -m "feat(modules): build the module graph and reject import cycles (E0802)"`

---

### Task 2: Enforced visibility

**Files:**
- Modify: `crates/yashy-sema/src/check_expr.rs` (resolution), `crates/yashy-sema/src/scope.rs` (item visibility)
- Modify: `codes.rs` (`PRIVATE_ITEM = "E0801"`), `docs/errors/E0801.md`

**Interfaces:**
- Produces: `E0801` when a path names an item that is not `pub` and the use is outside its module. Enforcement already exists (`visibility.rs`, 11 tests, message "is private to module"); this task codes those diagnostics and adds the catalog pages.
- Produces: `Binding.visibility: Visibility` (`Pub` / `Private`), defaulting to `Private`.

- [x] **Step 1: Write the failing tests** — a private `fn helper` called from another module is `E0801` (already rejected without a code); the same function called from its own module is fine; a `pub` struct with a private field cannot be constructed from another module.
- [x] **Step 2: Watch them fail.**
- [x] **Step 3: Implement** — resolution already knows the current module and the origin's `public` bit (`ItemOrigin`); attach `E0801` at the two refusal sites and record `Binding.visibility` on declaration. Landed as `223b840`.
- [x] **Step 4: Run** `cargo test -p yashy-sema` → PASS (visibility 12/12), including the existing single-module programs (which are all in the root module and unaffected).
- [x] **Step 5: Commit** — `git commit -m "feat(modules): enforce pub visibility across modules (E0801)"`

---

### Task 3: Package-prefixed imports

**Files:**
- Modify: `crates/yashy-driver/src/modules.rs`
- Modify: `crates/yashy-stdlib/yashy/*.ya` (only if a prefix is added to stdlib imports)
- Modify: `docs/cli.md`

**Interfaces:**
- Produces: `import std.io;` and `import std.io as io;` resolve; `io.print(..)` and `print(..)` both work for an imported module; a name collision is `E0001` with both origins named.

- [x] **Step 1: Write the failing tests** — `import std.io as io; io.print("x");` compiles and prints (via alias `v` in the driver suite — `io` is a keyword and cannot start an expression statement); two modules exporting the same name require a prefix and an unprefixed use is ambiguous (`E0001` on the duplicate definition).
- [x] **Step 2: Watch them fail.**
- [x] **Step 3: Implement** — an alias table per module (`Item::Import.alias`, loader records the alias as the module name); resolution tries the alias head, then the unqualified name. Landed as `739175e`.
- [x] **Step 4: Run** `cargo test -p yashy-parser --test alias -p yashy-sema --test alias -p yashy-driver --test modules` → PASS (13/13 driver).
- [x] **Step 5: Commit** — `git commit -m "feat(modules): support package-prefixed and aliased imports"`

---

### Task 4: Grapheme-aware string operations

**Files:**
- Modify: `crates/yashy-runtime/src/*.c` (cluster boundary scan, `yashy_str_grapheme_count`, `yashy_str_grapheme_index`)
- Modify: `crates/yashy-codegen/src/strings.rs`
- Modify: `crates/yashy-stdlib/yashy/string.ya` (`graphemes`, `char_count`, `substring`)
- Create: `crates/yashy-driver/tests/programs/unicode.ya`

**Interfaces:**
- Produces builtins: `str.graphemes() -> int`, `str.char_at(i: int) -> string`, `str.substring(start: int, end: int) -> string` (grapheme indices).
- Keeps: `str.len()` = bytes, `str.chars()` = bytes as `char`.

- [x] **Step 1: Write the failing test** — `unicode.ya` asserts: `"é".len() == 2`, `"é".graphemes() == 1`, a flag emoji (two code points, one cluster) has `graphemes() == 1`, `char_at(0)` of an emoji returns the whole cluster, and `substring` never splits a cluster. Landed as `7af4830`, value asserted in `crates/yashy-driver/tests/e2e.rs` (`grapheme_strings_count_and_slice_by_cluster`).
- [x] **Step 2: Watch it fail.**
- [x] **Step 3: Implement** — UAX #29 extended segmentation in `runtime.c` against generated property tables (`crates/yashy-runtime/src/grapheme_data.h`, Unicode 18.0); `graphemes()`/`char_at()`/`substring()` methods in sema + codegen; `char_at` out of range faults at run time. GB9c (Indic `Link`) is not implemented and is recorded as such in the C comment. Landed as `7af4830`.
- [x] **Step 4: Run** — `cargo test -p yashy-runtime --test grapheme` PASS (7/7); driver e2e blocked by an environment clang triple mismatch that fails the pre-existing `interning` program identically (CI runs it).
- [x] **Step 5: Commit** — `git commit -m "feat(string): grapheme-aware length, indexing and slicing"`

---

### Task 5: The Unicode conformance suite

> The harness lives in the runtime's suite
> (`crates/yashy-runtime/tests/grapheme_conformance.rs`), not the driver's,
> because `count` is a C entry point the runtime tests already call
> directly. The vendored Unicode 18.0.0 file is pinned at
> `crates/yashy-runtime/tests/data/GraphemeBreakTest.txt`; the 26 GB9c
> (`[9.3]`) cases are excluded by rule citation with the count pinned.

**Files:**
- Create: `crates/yashy-runtime/tests/data/GraphemeBreakTest.txt` (vendored, pinned, with its version in a header comment)
- Create: `crates/yashy-runtime/tests/grapheme_conformance.rs`

**Interfaces:**
- Produces: a test that runs every line of the conformance file through `yashy_str_grapheme_count` and asserts the expected cluster count.

- [x] **Step 1: Write the harness** — parse `÷`/`×` notation, skip comments, and assert on every case.
- [x] **Step 2: Run it** — expect failures; each is a missing break rule.
- [x] **Step 3: Fix the scanner** until the file passes, or record the exact rule set implemented in the test's header and assert only those rules. A partial implementation is allowed; an unstated partial implementation is not. Landed: 827/853 cases assert; the 26 failures are exactly the lines citing rule `[9.3]` (GB9c Indic conjunct joining), excluded by that citation with the count pinned at 26 so a rule-set change is visible.
- [x] **Step 4: Commit** — `git commit -m "test(string): run the vendored Unicode grapheme conformance suite"`

---

### Task 6: The verifier covers modules and strings

**Files:**
- Modify: `crates/yashy-verify/src/pass/bounds.rs`
- Create: `crates/yashy-driver/tests/verify_corpus/{string_oob.ya,string_slice_split.ya}`

**Interfaces:**
- Consumes: `str.graphemes()` as a length source for `char_at`/`substring`.
- Produces: `E1200` for a provably out-of-range grapheme index; a runtime fault otherwise.

- [x] **Step 1: Write the failing tests** — `"abc".char_at(5)` is `E1200`; `s.char_at(i)` in a loop bounded by `s.graphemes()` is accepted.
- [x] **Step 2: Implement** — extend the length lookup: `FixedArray` → literal, `Slice` → tracked `.len()`, `Str` → tracked `.graphemes()` (string literals track cluster counts; `len()` on strings stays bytes). Landed as `7af4830`.
- [x] **Step 3: Run the zero-false-positive loop** from phase 1b Task 7 — `cargo test -p yashy-verify` PASS (111/111); seeded corpus PASS.
- [x] **Step 4: Commit** — `git commit -m "feat(verify): prove string index bounds"`

---

### Task 7: Docs and gate

**Files:** `docs/tutorial/13-modules.md`, `docs/tutorial/08-strings.md` (extended), `AGENTS.md` (layout table gains the module graph), `llms.txt`.

- [ ] **Step 1: Write the tutorial chapters** with runnable blocks (the tutorial harness compiles every block).
- [ ] **Step 2: Run the gate table on both OSes.**
- [ ] **Step 3: Commit and record** — `git commit -m "docs: modules and Unicode, and close phase 6"`.

---

## Gate summary

| Gate | Command | Passes when |
|---|---|---|
| Large-project import test | `cargo test -p yashy-driver --test modules` | 50-module fixture resolves, cycles rejected once |
| Visibility | `cargo test -p yashy-sema` | private cross-module use rejected |
| Unicode conformance | `cargo test -p yashy-runtime --test grapheme_conformance` | every vendored case passes or is explicitly excluded |
| Bytes unchanged | `cargo test -p yashy-driver --test examples` | `wordcount` still counts bytes identically |
| Verifier coverage | the zero-false-positive loop | no new false positive on strings |
| No regression | `cargo test --workspace` | green on Windows and Linux |
