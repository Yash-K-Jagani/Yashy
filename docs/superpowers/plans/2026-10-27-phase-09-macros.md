# Phase 9 — Macros: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a program write a syntax shorthand once and use it many times, hygienically, deterministically, and — the part that matters — without creating a hole in the verifier.

**Architecture:** A macro is a set of arms, each a pattern over token trees with typed metavariables (`$x:expr`), and each a replacement token stream. Expansion is a pure function from `(macro table, invocation tokens)` to tokens, run **before** parsing an item's body, so the verifier and the type checker see ordinary syntax and need no new rule. Hygiene is scope-set based: an identifier introduced by the macro's template resolves in the macro's definition scope, an identifier substituted from the call site keeps the call site's scope. Every emitted token carries an expansion frame, so a diagnostic inside expanded code points at both the call site and the arm that produced it.

**Tech Stack:** `yashy-lexer` (token trees over the existing `TokenKind`), `yashy-parser` (fragment re-parsing per metavariable kind), a new `crates/yashy-expand` crate (no dependencies), `yashy-common` (`Span` gains an expansion frame), `yashy-verify` (unchanged by design — that is the point).

**Spec:** `docs/superpowers/specs/2026-10-27-yashy-master-design.md` §8 row 9. Index: `docs/superpowers/plans/2026-10-27-yashy-master-plan.md`. Predecessor: `2026-10-27-phase-08-traits-oop-iterators.md`.

## Global Constraints

See the master index. The ones that bite here:

- No procedural macros. Declarative only; a macro is data, never a program that runs at compile time.
- No new runtime dependencies.
- One rule, one implementation: expansion happens once, in `yashy-expand`, before the HIR exists.
- A malformed program produces a diagnostic, never a panic — and a malformed *macro* is the easiest way to get a panic, so this phase owns a dedicated fuzz corpus.
- Both operating systems or it is not done.

## Written cap (this phase)

- **No macro-defining-a-macro.** A `macro` item produced by expansion is `E1600`.
- **No macro-generated imports or `pub` wrappers.** Expansion produces expressions, statements, patterns, types, blocks and items of the six fragment kinds below, and nothing that changes the module graph.
- **Depth 64, expansion size 1 MiB per invocation.** Exceeding either is `E1601`.
- **Fragment kinds are exactly** `expr`, `stmt`, `pat`, `ty`, `ident`, `block`, `item`. `tt` is deliberately absent: a metavariable that can be anything is a metavariable the type checker cannot re-parse deterministically.

## Review Focus

1. **Expansion must not be able to hide an effect.** A macro that expands to `println(..)` inside a `pure fn` is `E1300` at the invocation's span, with the macro name in the note. If the verifier ran before expansion, this would be free; because it runs after, the span mapping is the safety property.
2. **Hygiene is not optional and not approximate.** A template's `let tmp = ..` must not capture a caller's `tmp`, and a caller's `x` passed as `$x` must keep resolving to the caller's `x`. Both directions get a test, and the second is the one implementations forget.
3. **Expansion is deterministic byte for byte.** The same source expands to the same token stream, in the same order, on both operating systems. A `HashMap` iteration in the expander is a defect even if the output happens to be stable today.
4. **A failed match names what it saw.** `E1600` prints the invocation's tokens and the arms' patterns; "no rule matched" with no tokens is a support ticket, not a diagnostic.
5. **Diagnostics survive expansion.** An error in expanded code reports the *user's* line as primary and the expansion path as notes (`in macro `unless` at line 3, expanded from line 12`), never a line inside a generated buffer.

---

### Task 1: Token trees and macro declarations

**Files:**
- Create: `crates/yashy-expand/{Cargo.toml,src/lib.rs,src/tt.rs}`
- Modify: `Cargo.toml` (member), `crates/yashy-driver/Cargo.toml`
- Modify: `crates/yashy-lexer/src/token.rs` (`macro` keyword; `KEYWORDS` 33 -> 34)
- Modify: `crates/yashy-parser/src/{ast.rs,items.rs}` (`Item::Macro { name, arms: Vec<MacroArm>, span }`)
- Modify: `crates/yashy-fmt/src/lib.rs`

**Interfaces:**
- Produces:
  ```rust
  pub enum TokenTree { Leaf(Token), Delimited { open: Delim, trees: Vec<TokenTree>, close: Span, span: Span } }
  pub struct MacroArm { pub pattern: Vec<TokenTree>, pub body: Vec<TokenTree>, pub span: Span }
  ```
- Produces: `macro unless(cond, body) { ... }` parses to an `Item::Macro` whose arms are token trees, not AST.

- [ ] **Step 1: Write the failing tests** — a balanced `(`, `[`, `{` group becomes one `Delimited`; an unbalanced group is a parse error naming the opener's line; two arms with different patterns both parse; the formatter prints the macro back unchanged (macros are formatted as written, not reformatted inside).
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — the lexer already produces spans; the token-tree builder is a bracket stack with a depth cap (256) so hostile input cannot recurse without bound. `yashy-expand` depends on `yashy-lexer` and `yashy-parser` only.
- [ ] **Step 4: Run** `cargo test -p yashy-expand -p yashy-parser -p yashy-fmt` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(expand): token trees and macro declarations"`

---

### Task 2: Metavariables and matching

**Files:**
- Create: `crates/yashy-expand/src/{match.rs,fragment.rs}`
- Modify: `crates/yashy-common/src/codes.rs` (`MACRO_NO_MATCH = "E1600"`, `MACRO_FRAGMENT = "E1602"`)
- Create: `docs/errors/{E1600,E1602}.md`, modify `docs/errors.md`

**Interfaces:**
- Produces: `$name:expr`, `$name:stmt`, `$name:pat`, `$name:ty`, `$name:ident`, `$name:block`, `$name:item`; `$( ... ),*` repetition is **not** in v1 (recorded in the cap).
- Produces: `pub fn matches(pattern: &[TokenTree], input: &[TokenTree]) -> Option<Bindings>`; a fragment kind is validated by re-parsing the matched tokens with the parser's entry point for that kind.
- Produces: `E1600` no arm matched (invocation tokens printed); `E1602` a metavariable used where its kind cannot go.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn an_expr_metavariable_consumes_one_expression() {
    // unless(x > 0, { a(); }) binds $cond = `x > 0` and $body = `{ a(); }`
}

#[test]
fn a_mismatched_fragment_kind_is_rejected() {
    // $x:ty given `1 + 2` -> E1602 at the invocation span
}

#[test]
fn no_matching_arm_prints_the_invocation() {
    // E1600 whose label contains the invocation's source text
}

#[test]
fn an_identifier_metavariable_matches_only_an_identifier() {
    // $n:ident given `1` -> E1600 (no arm matched), not a type error later
}
```

- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — matching is left to right, greedy only within a fragment kind (an `expr` fragment parses the longest prefix that is one expression, using the parser's own expression entry point, so the grammar stays single-sourced). Bindings are a `BTreeMap<String, Vec<TokenTree>>` for deterministic iteration.
- [ ] **Step 4: Run** `cargo test -p yashy-expand` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(expand): metavariables and fragment matching (E1600, E1602)"`

---

### Task 3: Hygiene

**Files:**
- Create: `crates/yashy-expand/src/hygiene.rs`
- Modify: `crates/yashy-common/src/span.rs` (`Span` gains `expansion: Option<ExpansionId>`)
- Modify: `crates/yashy-sema/src/scope.rs` (scope sets: an identifier resolves against the set of scopes it carries)

**Interfaces:**
- Produces: `pub struct ScopeSet(u64)`; every identifier token carries one. Template identifiers get the macro definition's scope set plus a fresh gensym mark; substituted tokens keep the caller's.
- Produces: `fn fresh_ident(name: &str, mark: u64) -> String` — the printed form stays `name` for diagnostics; the resolved form is `name#mark`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_template_local_does_not_capture_a_caller_local() {
    // macro swap(a, b) { let tmp = $a; $a = $b; $b = tmp; }
    // with a caller `tmp` already in scope: the caller's tmp is untouched
}

#[test]
fn a_caller_expression_keeps_resolving_in_the_caller() {
    // macro call_it(f) { $f(); } invoked as `call_it(g)` inside a scope where
    // `g` is a local, while the macro is defined in a scope where `g` is a
    // top-level function: the local is called
}

#[test]
fn two_expansions_of_one_macro_get_different_marks() {
    // two `let tmp` from two invocations are two bindings, both invisible to the caller
}
```

- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — the mark is a counter on the expander, not a hash, so expansion stays deterministic. Scope sets are compared as sets at resolution: a use resolves only against a binding whose set is a subset of the use's set. This is the standard scope-set formulation and it is the only one that gets both directions of hygiene right.
- [ ] **Step 4: Run** `cargo test -p yashy-expand -p yashy-sema` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(expand): hygienic expansion with scope sets"`

---

### Task 4: The expander

**Files:**
- Create: `crates/yashy-expand/src/expand.rs`
- Modify: `codes.rs` (`MACRO_DEPTH = "E1601"`, `MACRO_DUPLICATE = "E1603"`)
- Create: `docs/errors/{E1601,E1603}.md`

**Interfaces:**
- Produces: `pub fn expand_program(items: Vec<Item>) -> (Vec<Item>, ExpansionMap)`; the expander walks items, expands invocations, re-parses fragments, and recurses on the result.
- Produces: `E1601` depth > 64 or output > 1 MiB; `E1603` two macros with one name, or a macro whose name collides with a function in the same module.

- [ ] **Step 1: Write the failing tests** — a macro that expands to its own invocation is `E1601` (not a stack overflow); 64 nested distinct macros pass, 65 fails; a duplicate macro name is `E1603`; expanding the same program twice produces equal token streams.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — an explicit work list, not recursion, so the depth cap is a number in the data rather than the Rust stack. Output size is counted as tokens are appended. The `ExpansionMap` records `ExpansionId → { macro name, arm span, invocation span }` for Task 6.
- [ ] **Step 4: Run** `cargo test -p yashy-expand` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(expand): the expander with depth and size caps (E1601, E1603)"`

---

### Task 5: `yashy expand`

**Files:**
- Modify: `crates/yashy-driver/src/main.rs` (`expand` subcommand), `crates/yashy-driver/src/pipeline.rs` (expansion is a pipeline stage before parse-of-bodies)
- Modify: `docs/cli.md`
- Modify: `crates/yashy-driver/tests/expand.rs` (new)

**Interfaces:**
- Produces: `yashy expand file.ya` prints the fully expanded program, formatted canonically; `yashy expand --json file.ya` emits `{"schema_version":1,"tool":"yashy","expansions":[{"macro","arm_span","invocation_span","output_tokens"}]}`.
- Produces: `yashy check`, `build`, `run`, `test` and `verify` all expand before anything else; `--emit-ir` shows IR from expanded code, as it must.

- [ ] **Step 1: Write the failing tests** — `expand` output contains no `macro` item and no `$`; `--json` has one entry per invocation in source order; `check --json` on a file with a macro error has `code: "E1600"` and a `span` inside the user's file.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — one `expand_then_parse` helper in the pipeline, called by every entry point; the `expand` command stops after it. Nothing else in the driver learns about macros.
- [ ] **Step 4: Run** `cargo test -p yashy-driver --test expand --test diagnostics_json` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(driver): yashy expand, and expansion in every pipeline"`

---

### Task 6: Diagnostics through expansion

**Files:**
- Modify: `crates/yashy-common/src/diagnostic.rs` (labels may carry an expansion note)
- Modify: `crates/yashy-common/src/json.rs` (a `notes` entry with `"kind": "expansion"`)
- Modify: `crates/yashy-driver/src/main.rs` (human rendering)
- Modify: `crates/yashy-driver/tests/diagnostics_json.rs`

**Interfaces:**
- Produces: a diagnostic in expanded code has its primary span remapped to the invocation, and a note chain: `in macro `unless`, arm at line 3` / `expanded from line 12`.
- Produces: the JSON carries the same chain in `notes`, so an agent gets the same information as a human.

- [ ] **Step 1: Write the failing test** — a type error inside a macro body reports the invocation's line as primary, and the note names the macro and arm; `--json` has the note with `kind: "expansion"`.
- [ ] **Step 2: Watch it fail.**
- [ ] **Step 3: Implement** — a span with `expansion: Some(id)` resolves through `ExpansionMap` to the invocation span for display; the original span is kept in the note so the information is not lost.
- [ ] **Step 4: Run** `cargo test -p yashy-driver --test diagnostics_json --test expand` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(diagnostics): report through macro expansion"`

---

### Task 7: The verifier runs on expanded code

**Files:**
- Create: `crates/yashy-driver/tests/verify_corpus/{macro_hidden_io.ya,macro_double_free.ya,macro_leak.ya}`
- Modify: `crates/yashy-driver/tests/verify.rs` (expected-code table)

**Interfaces:**
- Produces: no verifier change at all. The phase's verifier work is proving that statement: expansion happens first, so every existing rule applies to the generated code, and the corpus proves it with one seeded bug per rule class.

- [ ] **Step 1: Write the three seeded bugs** — a macro expanding to `println` inside a `pure fn` (`E1300`), a macro expanding to a second `free` (`E1400`), a macro whose expansion drops a linear value (`E1000`).
- [ ] **Step 2: Run the corpus** — `cargo test -p yashy-driver --test verify` → every expected code set matches, each diagnostic's primary span inside the user's file.
- [ ] **Step 3: Run the zero-false-positive loop** from phase 1b Task 7 → no new line.
- [ ] **Step 4: Commit** — `git commit -m "test(verify): prove the verifier covers expanded code"`

---

### Task 8: Fuzz the expander

**Files:**
- Modify: `crates/yashy-driver/tests/never_panics.rs` (macro corpus)
- Create: `crates/yashy-driver/tests/never_panics_corpus/macro_*.ya` (20 hand-written hostile inputs: unbalanced brackets, deep nesting, `$` at end of input, empty arm, fragment kind on a delimiter)

**Interfaces:**
- Produces: no panic, no hang, no stack overflow on any corpus input; every input either expands or produces a coded diagnostic.

- [ ] **Step 1: Add the corpus** and the test that each file is `check`ed in-process and must return a `Result`, not panic.
- [ ] **Step 2: Run** `cargo test -p yashy-driver --test never_panics` → PASS. Any panic is fixed in the expander, and the input stays in the corpus.
- [ ] **Step 3: Commit** — `git commit -m "test(expand): a hostile-input corpus for the expander"`

---

### Task 9: Tooling and docs

**Files:** `docs/tutorial/15-macros.md`, `docs/cli.md` (`expand`), `AGENTS.md` (identity: declarative macros only; the cap), `crates/yashy-lsp/src/server.rs` (hover on a macro shows its arms; no completion inside macro patterns in v1).

- [ ] **Step 1: Write the chapter** with runnable blocks, including the two hygiene examples from Task 3.
- [ ] **Step 2: Run the gate table on both OSes.**
- [ ] **Step 3: Commit and record** — `git commit -m "docs: macros, and close phase 9"`.

---

## Gate summary

| Gate | Command | Passes when |
|---|---|---|
| Verifier runs on expanded code | `cargo test -p yashy-driver --test verify` | three macro seeded bugs caught, spans in user code |
| Deterministic, inspectable expansion | `cargo test -p yashy-driver --test expand` | two runs equal; `--json` lists every expansion in source order |
| Hygiene both directions | `cargo test -p yashy-expand` | template capture and caller capture both correct |
| Caps enforced | `cargo test -p yashy-expand` | depth 65 is `E1601`, not a crash |
| No panic | `cargo test -p yashy-driver --test never_panics` | hostile corpus produces diagnostics only |
| No false positives | the phase 1b loop | no `FALSE POSITIVE` line |
| No regression | `cargo test --workspace` | green on Windows and Linux |
