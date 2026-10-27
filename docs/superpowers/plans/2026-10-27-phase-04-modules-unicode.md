# Phase 4 — Modules and Unicode: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make large projects possible (namespacing, enforced visibility, package-prefixed imports, no import cycles) and make strings correct for people who do not write English (grapheme-aware length, indexing and slicing), with the verifier covering both.

**Architecture:** The module loader in `yashy-driver/src/modules.rs` already reads imports into a `SourceMap`; it gains a graph with cycle detection and a visibility table. Name resolution in `sema` already carries `Vec<String>` paths on `HirExpr::Ident`/`Call`, so qualified names are enforcement, not new syntax. Strings stay UTF-8 and refcounted; the runtime gains grapheme-cluster boundaries, and the verifier's bounds domain learns string lengths.

**Tech Stack:** `crates/yashy-driver/src/modules.rs`, `crates/yashy-sema/src/check_expr.rs` (resolution), `crates/yashy-runtime` (C string functions), `crates/yashy-stdlib/yashy/string.ya`.

**Spec:** `docs/superpowers/specs/2026-10-27-yashy-master-design.md` §8 row 4. Index: `docs/superpowers/plans/2026-10-27-yashy-master-plan.md`. Predecessor: `2026-10-27-phase-03-speed-foundation.md`.

## Global Constraints

See the master index. The ones that bite here:

- No new runtime dependencies. Unicode tables are vendored data files, not a crate.
- No implicit coercion; a byte index and a grapheme index are different functions, not overloads.
- One rule, one implementation: the module graph is built once, in the driver.
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
- Produces: `pub struct ModuleGraph { pub modules: Vec<Module>, pub edges: Vec<(FileId, FileId)> }`; `modules::load` returns it instead of `Vec<Module>`.
- Produces: `E0802` at the import that closes a cycle, with the cycle path in the label (`a -> b -> a`).

- [ ] **Step 1: Write the failing tests** — a two-file cycle is `E0802` once; a diamond (`a` imports `b` and `c`, both import `d`) loads `d` once; a self-import is `E0802`; the module order is deterministic across runs.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — DFS with a `visiting` stack and a `loaded` map keyed by canonical path; the existing `SourceMap` insertion order is preserved so diagnostics do not move.
- [ ] **Step 4: Run** `cargo test -p yashy-driver --test modules` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(modules): build the module graph and reject import cycles (E0802)"`

---

### Task 2: Enforced visibility

**Files:**
- Modify: `crates/yashy-sema/src/check_expr.rs` (resolution), `crates/yashy-sema/src/scope.rs` (item visibility)
- Modify: `codes.rs` (`PRIVATE_ITEM = "E0801"`), `docs/errors/E0801.md`

**Interfaces:**
- Produces: `E0801` when a path names an item that is not `pub` and the use is outside its module.
- Produces: `Binding.visibility: Visibility` (`Pub` / `Private`), defaulting to `Private`.

- [ ] **Step 1: Write the failing tests** — a private `fn helper` called from another module is `E0801`; the same function called from its own module is fine; a `pub` struct with a private field cannot be constructed from another module.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — resolution already knows the current module (the entry file or the imported one); record it on the checker and compare on every cross-module lookup.
- [ ] **Step 4: Run** `cargo test -p yashy-sema` → PASS, including the existing single-module programs (which are all in the root module and unaffected).
- [ ] **Step 5: Commit** — `git commit -m "feat(modules): enforce pub visibility across modules (E0801)"`

---

### Task 3: Package-prefixed imports

**Files:**
- Modify: `crates/yashy-driver/src/modules.rs`
- Modify: `crates/yashy-stdlib/yashy/*.ya` (only if a prefix is added to stdlib imports)
- Modify: `docs/cli.md`

**Interfaces:**
- Produces: `import std.io;` and `import std.io as io;` resolve; `io.println(..)` and `println(..)` both work for an imported module; a name collision is `E0001` with both origins named.

- [ ] **Step 1: Write the failing tests** — `import std.io as io; io.println("x");` compiles and prints; two modules exporting the same name require a prefix and an unprefixed use is ambiguous (`E0001` naming both modules).
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — an alias table per module; resolution tries the alias, then the unqualified name, then fails with both candidates.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(modules): support package-prefixed and aliased imports"`

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

- [ ] **Step 1: Write the failing test** — `unicode.ya` asserts: `"é".len() == 2`, `"é".graphemes() == 1`, a flag emoji (two code points, one cluster) has `graphemes() == 1`, `char_at(0)` of an emoji returns the whole cluster, and `substring` never splits a cluster.
- [ ] **Step 2: Watch it fail.**
- [ ] **Step 3: Implement** — a minimal grapheme-break table generated from `GraphemeBreakProperty.txt` and vendored as a C array; the scanner implements the UAX #29 extended-grapheme-cluster rules that matter for emoji, combining marks, flags and Indic conjuncts.
- [ ] **Step 4: Run** — `cargo test -p yashy-driver --test e2e unicode` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(string): grapheme-aware length, indexing and slicing"`

---

### Task 5: The Unicode conformance suite

**Files:**
- Create: `crates/yashy-runtime/tests/data/GraphemeBreakTest.txt` (vendored, pinned, with its version in a header comment)
- Create: `crates/yashy-driver/tests/unicode_conformance.rs`

**Interfaces:**
- Produces: a test that runs every line of the conformance file through `yashy_str_grapheme_count` and asserts the expected cluster count.

- [ ] **Step 1: Write the harness** — parse `÷`/`×` notation, skip comments, and assert on every case.
- [ ] **Step 2: Run it** — expect failures; each is a missing break rule.
- [ ] **Step 3: Fix the scanner** until the file passes, or record the exact rule set implemented in the test's header and assert only those rules. A partial implementation is allowed; an unstated partial implementation is not.
- [ ] **Step 4: Commit** — `git commit -m "test(string): run the vendored Unicode grapheme conformance suite"`

---

### Task 6: The verifier covers modules and strings

**Files:**
- Modify: `crates/yashy-verify/src/pass/bounds.rs`
- Create: `crates/yashy-driver/tests/verify_corpus/{string_oob.ya,string_slice_split.ya}`

**Interfaces:**
- Consumes: `str.graphemes()` as a length source for `char_at`/`substring`.
- Produces: `E1200` for a provably out-of-range grapheme index; a runtime fault otherwise.

- [ ] **Step 1: Write the failing tests** — `"abc".char_at(5)` is `E1200`; `s.char_at(i)` in a loop bounded by `s.graphemes()` is accepted.
- [ ] **Step 2: Implement** — extend the length lookup: `FixedArray` → literal, `Slice` → tracked `.len()`, `Str` → tracked `.graphemes()`.
- [ ] **Step 3: Run the zero-false-positive loop** from phase 1b Task 7.
- [ ] **Step 4: Commit** — `git commit -m "feat(verify): prove string index bounds"`

---

### Task 7: Docs and gate

**Files:** `docs/tutorial/13-modules.md`, `docs/tutorial/08-strings.md` (extended), `AGENTS.md` (layout table gains the module graph), `llms.txt`.

- [ ] **Step 1: Write the tutorial chapters** with runnable blocks (the tutorial harness compiles every block).
- [ ] **Step 2: Run the gate table on both OSes.**
- [ ] **Step 3: Commit and record** — `git commit -m "docs: modules and Unicode, and close phase 4"`.

---

## Gate summary

| Gate | Command | Passes when |
|---|---|---|
| Large-project import test | `cargo test -p yashy-driver --test modules` | 50-module fixture resolves, cycles rejected once |
| Visibility | `cargo test -p yashy-sema` | private cross-module use rejected |
| Unicode conformance | `cargo test -p yashy-driver --test unicode_conformance` | every vendored case passes or is explicitly excluded |
| Bytes unchanged | `cargo test -p yashy-driver --test examples` | `wordcount` still counts bytes identically |
| Verifier coverage | the zero-false-positive loop | no new false positive on strings |
| No regression | `cargo test --workspace` | green on Windows and Linux |
