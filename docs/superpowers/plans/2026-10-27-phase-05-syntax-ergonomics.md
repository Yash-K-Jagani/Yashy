# Phase 5 — Syntax and Ergonomics: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the boilerplate that makes Yashy read like a language from 2010 — without adding a concept the verifier, the formatter or a reader has to learn. Eight additive forms, every one of which desugars into HIR that already exists.

**Architecture:** Every item is a **parse-and-lower** change, never a new HIR construct and never a new runtime entry point. `if let`/`while let` become the `switch` the language already has; `?` on `Option` reuses the existing `Try` lowering with the tag convention corrected; trailing blocks are ordinary lambdas with their parameter types unified from the callee; spread becomes field copies and concatenation; a chained comparison becomes `&&` with the middle operand bound once; `for i, x` becomes a `for` over the index range; a field default is inserted where the literal is built; `arena { }` is a scope that changes where allocations inside it are placed. The single test that keeps this honest is HIR equality: each new form must produce the same HIR as the hand-written form it replaces.

**Tech Stack:** `yashy-lexer` (`TokenKind`, no new keywords at all), `yashy-parser/src/{exprs.rs,stmts.rs,items.rs,ast.rs}`, `yashy-sema/src/check_expr.rs` (lowering and typing), `yashy-codegen/src/{expr_gen.rs,stmt_gen.rs,memory.rs,arena.rs}` (only for `?` on `Option` and arena placement), `yashy-verify` (partial move, arena escape), `yashy-fmt`, `yashy-lsp`.

**Spec:** `docs/superpowers/specs/2026-10-27-yashy-master-design.md` §8 row 5. Index: `docs/superpowers/plans/2026-10-27-yashy-master-plan.md`. Predecessor: `2026-10-27-phase-04-optimization.md`.

## What already ships (do not re-add)

Two items in the approved design were already implemented before this plan was written. They are listed here so a future reader does not schedule them again:

- **String interpolation** — `"n = ${n}"` is implemented end to end: the lexer splits `${ ... }` (`yashy_lexer::split_interpolation`), the parser builds `InterpPart`s, `:?` is `InterpFormat::Debug`, and the checker lowers each part to a conversion plus `+`. `crates/yashy-driver/tests/programs/interp.ya` asserts that the interpolated form and the written-out form are the same program. The spelling is `${...}`, and it stays that spelling.
- **`const fn`** — `Item::ConstFn` exists in the parser (`items.rs`) and `yashy-sema/src/const_eval.rs` runs it. A `const fn` has no run-time existence; it is evaluated where a `const` initialiser needs it.

The two items that replace them in this phase are **default field values** and the **`arena { }` block** — both from the same approved categories (boilerplate removal, and performance syntax that pairs with phase 4's promotion), and neither exists today.

## Global Constraints

See the master index. The ones that bite here:

- **No new keywords.** `if let`, `while let`, `for i, x`, `arena` and `..` are all spelled with tokens that already exist. `KEYWORDS` stays at 28.
- **No HIR growth for sugar.** If an item needs a new `HirExpr`/`HirStmt` variant, it is not sugar and does not belong in this phase. The exception is `arena { }`, whose *placement* is a codegen concept, not a new statement kind.
- **No keyword renames, no unification, no attributes.** `switch` stays `switch`; `let`/`const`/`var` keep their meanings; there is no `@inline`.
- **One rule, one implementation.** Each desugaring lives in one function in `sema`, called from every place the form can appear.
- **Both operating systems or it is not done.**
- **Format touched files only.**

## Written cap (this phase)

- **No `?.`**, no optional-chaining operator, no `??`.
- **No operator overloading**, so a chained comparison is a comparison chain, never a call to a user `PartialOrd`.
- **No format specifiers beyond `:?`** — interpolation keeps exactly the two forms it has.
- **No `for i, x` over iterators in v1.** Arrays and slices only; the diagnostic says when the general form arrives (phase 8).
- **No nested `arena` inference across function calls.** An allocation made by a callee is placed by the callee; the block governs the code written inside it.

## Review Focus

1. **`?` on `Option` has the opposite tag convention to `?` on `Result`.** `variant_tag` in `codegen/ir.rs` says `Option`: `None` = 0, `Some` = 1; `Result`: `Ok` = 0, `Err` = 1. `gen_try` currently branches on `icmp eq i8 tag, 0` and calls that the success path, which is correct for `Result` and **inverted for `Option`**. A copy-paste that reuses `gen_try` unchanged compiles `?` on `Option` into "return early on the value you wanted", and every test that only checks the happy path still passes.
2. **`if let` with an irrefutable pattern is an error, not a warning** (`E0603`). `if let x = 1 { }` and `if let Point { x, y } = p { }` are always true; the confusion with `let` is exactly what the diagnostic exists to catch, and a warning would be ignored.
3. **A chained comparison evaluates its middle operand once.** `0 <= f() < 10` calls `f()` exactly once, in the order written. The desugaring binds the middle operand to a synthetic `let`; anything that re-evaluates it changes a program's side effects while looking like sugar.
4. **Spread moves, and a partial move is `E1002`.** `P { x: 1, ..base }` takes the remaining fields out of `base`; using `base` afterwards is rejected. Copying the fields instead would double-free a linear field, so the move is the only sound rule.
5. **`arena { }` is a promise the block releases everything in it.** A value that escapes the block is `E1003`; an individual `.free()` of an arena-placed value is `E1004` (the block owns it). The runtime's arenas are single-threaded, so capturing an arena-placed value into a spawned task is `E1003` too — the same rule, one implementation.

---

### Task 1: `if let` and `while let`

**Files:**
- Modify: `crates/yashy-parser/src/ast.rs` (`Stmt::IfLet { pattern, value, then_block, else_block, span }`, `Stmt::WhileLet { pattern, value, body, span }`)
- Modify: `crates/yashy-parser/src/stmts.rs`
- Modify: `crates/yashy-sema/src/check_expr.rs` (lower both to existing HIR)
- Modify: `crates/yashy-hir/src/exhaustive.rs` (`is_irrefutable(pattern, ty) -> bool`)
- Modify: `crates/yashy-common/src/codes.rs` (`IRREFUTABLE_IF_LET = "E0603"`), create `docs/errors/E0603.md`, modify `docs/errors.md`
- Modify: `crates/yashy-fmt/src/lib.rs`
- Create: `crates/yashy-driver/tests/programs/if_let.ya`, modify `crates/yashy-driver/tests/e2e.rs` (one test that builds and runs it)

**Interfaces:**
- Produces: `if let Some(x) = opt { } else { }` and `while let Some(x) = it.next() { }`, with the existing `Pattern` grammar — variants, struct patterns and tuple patterns, exactly what `switch` accepts today. Or-patterns and ranges arrive with phase 7 and are accepted here for free, because the pattern is the same node.
- Produces: `HirStmt::Switch` with two arms (`pattern => then`, `_ => else`) for `if let`; `HirStmt::ForC { init: None, cond: None, update: None }` whose body is a `Switch` with `pattern => body` and `_ => break` for `while let`. `break` inside a `switch` inside a loop already leaves the loop (`ctx.loops` is pushed only by loops, not by `Switch`), which is what makes this lowering correct rather than clever.
- Produces: `E0603` on a pattern that matches every value of its type, with the fix "use `let`".

- [ ] **Step 1: Write the failing tests** — both forms parse and run; `if let` with an `else` runs the else branch on `None`; a binding is scoped to its block (using `x` after the `if` is `E0001`); `while let` over a slice drains it; `break` inside the `while let` body leaves the loop; `if let x = 1` is `E0603`; `if let Point { x, y } = p` is `E0603`; the formatter round-trips both forms byte-identically.
- [ ] **Step 2: Run and watch them fail** — `cargo test -p yashy-parser if_let` → FAIL.
- [ ] **Step 3: Implement** — parse `let` after the `if`/`while` keyword into a pattern plus `= expr`; there is no ambiguity because `let` is already a keyword and an `if` condition cannot start with it. Lowering happens in `check_expr` next to `Stmt::Switch`, so the two share the arm-checking path. `is_irrefutable` asks the exhaustiveness module whether the pattern alone covers the type, which is the same question `missing_cases` answers with an empty result.
- [ ] **Step 4: Run** `cargo test -p yashy-parser -p yashy-sema -p yashy-fmt -p yashy-driver --test e2e` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: if let and while let as switch lowerings (E0603)"`

---

### Task 2: `?` on `Option`

**Files:**
- Modify: `crates/yashy-sema/src/check_expr.rs` (`Expr::Try`: accept `Option[T]` when the function returns `Option[U]`)
- Modify: `crates/yashy-codegen/src/expr_gen.rs` (`gen_try` branches on the type's own success tag)
- Modify: `crates/yashy-common/src/codes.rs` (`MIXED_TRY = "E0301"`), create `docs/errors/E0301.md`
- Create: `crates/yashy-driver/tests/programs/option_try.ya`, modify `crates/yashy-driver/tests/e2e.rs` (one test that builds and runs it)

**Interfaces:**
- Produces: `fn lookup(k: string) -> Option[i64]` and `let v = lookup(k)?;` inside a function returning `Option[...]`, lowering to "on `None`, return `None`; otherwise continue with the payload".
- Produces: `gen_try` reads the success tag from `variant_tag(&subject_ty, "Ok"|"Some", None)` instead of hard-coding `0` — one table, already written, already tested.
- Produces: `E0301` when `?` mixes the two: an `Option` inside a function returning `Result`, or a `Result` inside a function returning `Option`. The note names both types and the two legal fixes (`switch`, or change the return type).

- [ ] **Step 1: Write the failing tests** — `option_try.ya` returns `None` through two levels and asserts the early return; a `Result` `?` in an `Option` function is `E0301`; an `Option` `?` in a `Result` function is `E0301`; `?` on an `Option` in a function returning `i64` is `E0301`; the existing `Result` `?` tests are unchanged.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — the checker's `Expr::Try` arm gains a second shape: `TypeKind::Option(inner)` with `fn_ret == Option(U)`, yielding `HirExpr::Try` with `ty = inner`. The codegen change is two lines plus a comment naming the tag table, and it must be a table lookup rather than an `if is_option` — the convention belongs in `ir.rs`, where it is already written down.
- [ ] **Step 4: Run** `cargo test -p yashy-sema -p yashy-codegen -p yashy-driver --test e2e` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: ? on Option, and E0301 when the two error types are mixed"`

---

### Task 3: Trailing block argument and closure parameter inference

**Files:**
- Modify: `crates/yashy-parser/src/{ast.rs,exprs.rs}` (`Expr::Lambda` params may omit types; `CallExpr`/`MethodCall` gain a trailing `Block` argument)
- Modify: `crates/yashy-sema/src/check_expr.rs` (unify the lambda's parameter and return types from the callee's signature)
- Modify: `crates/yashy-fmt/src/lib.rs`
- Modify: `crates/yashy-lsp/src/server.rs` (signature help inside a trailing block)
- Modify: `crates/yashy-driver/tests/programs/closures.ya` (add a trailing-block call beside the existing closures), `crates/yashy-driver/tests/e2e.rs` (the existing `closures` test now covers the block form too)

**Interfaces:**
- Produces: `xs.each { |x| println("${x}") }` and `with_file(path) { |f| read(f) }` — the block is the last argument, and `|x|` takes its type from the callee's parameter type.
- Produces: the parser refuses a trailing block in a condition position (`if f(x) { }` keeps meaning "if the call, then this block"), with a `no_block_arg` flag set while parsing `if`, `while`, `for` and `switch` heads — the same restriction Rust has, for the same reason.
- Produces: `E0100` naming both signatures when inference fails (a callee that takes `fn(i64) -> i64` called with `|x| "text"` reports the expected and the found return type).

- [ ] **Step 1: Write the failing tests** — a trailing block passes as the last argument; `|x|` infers from the callee; an explicit type still works and must agree; `if f(x) { }` parses as an `if`; a lambda passed to a generic parameter that does not fix its type is `E0100` with both signatures; the formatter puts the block on its own line and round-trips.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — the parser already has a flag that suppresses struct literals in condition position (struct literals and blocks are the same ambiguity); reuse it. Inference is a new step in argument checking: when the parameter's declared type is a function type and the argument is a lambda with omitted parameter types, the lambda is checked *against* that type instead of on its own.
- [ ] **Step 4: Run** `cargo test -p yashy-parser -p yashy-sema -p yashy-fmt -p yashy-lsp` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: trailing block arguments with inferred closure parameters"`

---

### Task 4: Struct and array spread

**Files:**
- Modify: `crates/yashy-parser/src/ast.rs` (`StructLit.spread: Option<Box<Expr>>`, `SliceLit.elements: Vec<SliceElement>` with `SliceElement::{Value, Spread}`)
- Modify: `crates/yashy-parser/src/exprs.rs`
- Modify: `crates/yashy-sema/src/check_expr.rs` (field fill and concatenation)
- Modify: `crates/yashy-common/src/codes.rs` (`DUPLICATE_FIELD = "E0202"`), create `docs/errors/E0202.md`
- Modify: `crates/yashy-verify/src/pass/ownership.rs` (spread is a move; partial move is `E1002`)
- Modify: `crates/yashy-fmt/src/lib.rs`
- Create: `crates/yashy-driver/tests/programs/spread.ya`, modify `crates/yashy-driver/tests/e2e.rs` (one test that builds and runs it)

**Interfaces:**
- Produces: `P { x: 1, ..base }` — explicit fields win, the rest are moved out of `base`; `P { ..base }` alone is legal. The spread must be last (`..base, x: 1` is a parse error with "the spread is written last").
- Produces: `[0, ..xs, 4]` — a new `[]T` whose elements are the prefix, every element of `xs`, then the suffix. The element type comes from the non-spread elements, or from the spread when there are none.
- Produces: `E0202` when a field is given twice (`P { x: 1, x: 2 }`), naming both spans.
- Produces: `E1002` when the spread operand is used after the literal, and when a linear field would be copied rather than moved.

- [ ] **Step 1: Write the failing tests** — struct spread fills the unlisted fields; an explicit field overrides the spread; a field given twice is `E0202`; using `base` after `P { ..base }` is `E1002`; a spread of the wrong type is `E0100`; `[0, ..xs, 4]` produces the concatenation in order; `[..xs]` alone is legal; spreading a slice of a linear type moves it (`E1002` if used after); the formatter round-trips `..`.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — the checker builds the field list (explicit, then spread, then default from Task 7) and the literal is lowered to the same `HirExpr::StructLit` it already produces. Array spread lowers to a sequence of element stores into a freshly allocated slice; the length is known at check time for fixed arrays and computed once for slices, so there is no growth loop.
- [ ] **Step 4: Run** `cargo test -p yashy-parser -p yashy-sema -p yashy-verify -p yashy-driver --test e2e` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: struct and array spread, with partial-move rules (E0202, E1002)"`

---

### Task 5: Chained comparison

**Files:**
- Modify: `crates/yashy-parser/src/{ast.rs,exprs.rs}` (`Expr::CompareChain { operands, ops, span }`)
- Modify: `crates/yashy-sema/src/check_expr.rs` (lower to `&&` with one binding for the middle)
- Modify: `crates/yashy-common/src/codes.rs` (`CHAINED_EQUALITY = "E0101"`), create `docs/errors/E0101.md`
- Modify: `crates/yashy-fmt/src/lib.rs`
- Create: `crates/yashy-driver/tests/programs/chained_cmp.ya`, modify `crates/yashy-driver/tests/e2e.rs` (one test that builds and runs it, including the once-only evaluation counter)

**Interfaces:**
- Produces: `0 <= x < 10` → `let __cmp0 = x; 0 <= __cmp0 && __cmp0 < 10`, with `x` evaluated once and in source order. The synthetic name is mangled so it cannot collide with a user binding.
- Produces: chains of any length (`a < b <= c < d`) and any mix of ordering operators.
- Produces: `E0101` when `==` or `!=` appears anywhere in a chain (`a == b == c`), with the fix "write the two comparisons separately with `&&`". Equality chaining is where the operator stops meaning what a reader thinks.
- Produces: operands must all have the same type and it must be `i64`, `f64` or `char`; anything else is the existing `E0100`.

- [ ] **Step 1: Write the failing tests** — `0 <= x < 10` is true only in range; `f()` in the middle is called exactly once (asserted with a counter); `a < b > c` parses as `a < b && b > c`; `a == b == c` is `E0101`; `1 < "x"` is `E0100`; the formatter prints the chain unparenthesised and round-trips.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — the parser collects consecutive comparison operands into one node (a comparison operator followed by another comparison operator continues the chain rather than nesting). The checker emits `HirExpr::Block`-equivalent bindings plus `HirExpr::Binary(And)`; because the HIR already has `&&` with short-circuit lowering, the chain inherits short-circuiting for free.
- [ ] **Step 4: Run** `cargo test -p yashy-parser -p yashy-sema -p yashy-codegen -p yashy-driver --test e2e` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: chained comparisons with one evaluation of the middle (E0101)"`

---

### Task 6: `for i, x in xs`

**Files:**
- Modify: `crates/yashy-parser/src/ast.rs` (`Stmt::ForIn.index: Option<String>`)
- Modify: `crates/yashy-parser/src/stmts.rs`
- Modify: `crates/yashy-sema/src/check_expr.rs` (lower to `ForRange` + one element read)
- Modify: `crates/yashy-fmt/src/lib.rs`
- Create: `crates/yashy-driver/tests/programs/indexed_for.ya`, modify `crates/yashy-driver/tests/e2e.rs` (one test that builds and runs it)

**Interfaces:**
- Produces: `for i, x in xs { }` over `[]T` and views, lowering to `ForRange { name: i, start: 0, end: xs.len() }` whose body reads element `i` once into `x`.
- Produces: `xs` must be a **binding** (the same rule `ForIn` already has) and must not be moved or resized inside the body; the length is read once at loop entry, which the tutorial states explicitly because it differs from `for x in xs` (which re-reads the header).
- Produces: the element read is the existing bounds-checked getter; the verifier proves `i < len` from the range, so phase 4's bounds elision removes the check — the two phases meet here, and a test asserts the check is gone from the IR.
- Produces: `E0100` for an iterable that is not an array or a view, with the note "indexed iteration over an iterator arrives with the iterator protocol".

- [ ] **Step 1: Write the failing tests** — indices and values match for a non-empty slice, an empty slice, and a view; a `push` inside the body does not extend the loop (length read once) and the tutorial says so; `for i, x in "abc"` is `E0100`; the IR for a `for i, x` over a slice contains no bounds-check call; the formatter round-trips.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — the parser accepts `for name, name in` by looking for the comma after the first name; the checker lowers in one function next to `ForIn`, so both loops share the "iterable must be a binding" rule. The element read reuses `slices.rs`'s getter rather than adding a second way to read an element.
- [ ] **Step 4: Run** `cargo test -p yashy-parser -p yashy-sema -p yashy-codegen -p yashy-driver --test e2e` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: for i, x over arrays and views"`

---

### Task 7: Default field values

**Files:**
- Modify: `crates/yashy-parser/src/{ast.rs,items.rs}` (`FieldDecl.default: Option<Expr>`)
- Modify: `crates/yashy-sema/src/check_expr.rs` (fill at literal construction; `E0201` message updated)
- Modify: `crates/yashy-common/src/codes.rs` (`FIELD_DEFAULT_NOT_CONST = "E0102"`), create `docs/errors/E0102.md`
- Modify: `crates/yashy-fmt/src/lib.rs`
- Modify: `crates/yashy-lsp/src/server.rs` (completion lists a field with its default)
- Create: `crates/yashy-driver/tests/programs/field_defaults.ya`, modify `crates/yashy-driver/tests/e2e.rs` (one test that builds and runs it)

**Interfaces:**
- Produces: `struct Config { retries: i64 = 3, name: string }` and `Config { name: "x" }` — the missing field is filled at construction, so the HIR literal has every field and no later pass learns about defaults.
- Produces: the default must be a literal or a `const` name, evaluated at check time (`E0102` otherwise). It may not call a function, read a parameter or depend on the enclosing scope, because it is inserted at a *call site's* literal, not run there.
- Produces: precedence — explicit field, then spread (Task 4), then default. A field with neither is still `E0201`, whose message now says "has no default".
- Produces: a default is checked once against the field's declared type; a mismatched default is `E0100` at the default's span, not at every construction.

- [ ] **Step 1: Write the failing tests** — a literal omitting a defaulted field compiles and has the default; an explicit value overrides it; a spread's field overrides a default; a literal omitting a non-defaulted field is still `E0201` with the new wording; `= f()` is `E0102`; `= 1` on a `string` field is `E0100`; the formatter round-trips the declaration.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — the field list is built in one place (`fill_struct_fields`) that Task 4 also calls, so precedence has exactly one implementation. The default expression is checked against the field type when the struct is declared; the literal insertion clones the *checked* HIR expression, so a default cannot be type-checked differently at two call sites.
- [ ] **Step 4: Run** `cargo test -p yashy-parser -p yashy-sema -p yashy-driver --test e2e` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: default struct field values (E0102)"`

---

### Task 8: `arena { }`

**Files:**
- Modify: `crates/yashy-parser/src/ast.rs` (`Stmt::Arena { body: Block, span }`)
- Modify: `crates/yashy-parser/src/stmts.rs`, `crates/yashy-fmt/src/lib.rs`
- Modify: `crates/yashy-hir/src/hir.rs` (`HirStmt::Arena { arena: String, body: HirBlock, span }` — placement, not a new statement kind)
- Modify: `crates/yashy-codegen/src/{fn_gen.rs,memory.rs,arena.rs,expr_gen.rs}` (an arena stack in `FnCtx`; allocations inside target it; releases inside are suppressed)
- Modify: `crates/yashy-verify/src/pass/{ownership.rs,typestate.rs}` (`E1003` escape, `E1004` individual free)
- Modify: `crates/yashy-common/src/codes.rs` (`ARENA_ESCAPE = "E1003"`, `ARENA_FREE = "E1004"`), create `docs/errors/{E1003,E1004}.md`
- Modify: `crates/yashy-driver/tests/programs/arena.ya` (add the block form beside the existing builtin form), `crates/yashy-driver/tests/e2e.rs` (the existing `arena` test asserts both forms print the same sum)

**Interfaces:**
- Produces: `arena { ... }` — one arena for the block, released at block exit on every path, including `break`, `continue` and an early `return` (through the existing `defer` machinery, not a second cleanup path).
- Produces: every allocation whose value does not escape the block is placed in that arena; the placement reuses phase 4's `Placement` analysis with the block as the escape boundary, so there is one analysis, not two.
- Produces: no individual release runs for an arena-placed value inside the block — no `free`, no refcount decrement — because the block releases the whole arena.
- Produces: `E1003` when an arena-placed value escapes (returned, assigned to a binding declared outside, stored in a struct that outlives the block, or captured by `thread_spawn`), naming the value and the boundary; `E1004` when `.free()` or an explicit refcount release is applied to one, with the fix "the block releases it".
- Produces: nesting is legal; the inner block uses its own arena and the outer one is untouched.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_string_built_in_an_arena_is_freed_by_the_block() {
    // arena { let s = "a" + "b"; println(s); } leaks nothing (the leak suite)
}

#[test]
fn returning_an_arena_value_is_rejected() {
    // arena { let s = "a" + "b"; } s   -> E1003
}

#[test]
fn freeing_an_arena_value_inside_the_block_is_rejected() {
    // arena { let s = "a" + "b"; s.free(); } -> E1004
}

#[test]
fn a_break_out_of_an_arena_still_releases_it() {
    // for i in 0..3 { arena { if i == 1 { break; } } } -> no leak
}
```

- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — the arena binding is a synthetic `const` holding `arena_new()`, and the release is a `defer` of `arena_free_all`; both are inserted in the HIR, so `break`/`continue`/`return` already run it. Codegen keeps an arena stack in `FnCtx`; `memory.rs` and `arena.rs` consult the top of the stack when choosing the allocator, and the release paths consult the value's placement before emitting a decrement. The verifier's escape check is the phase 4 placement question asked with a different boundary, so it calls the same function.
- [ ] **Step 4: Run** `cargo test -p yashy-parser -p yashy-sema -p yashy-codegen -p yashy-verify -p yashy-driver --test e2e arena` and the leak suite → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: arena blocks with escape and free proofs (E1003, E1004)"`

---

### Task 9: The sugar proof, docs and gate

**Files:**
- Create: `crates/yashy-sema/tests/desugar.rs` (each new form's HIR equals the hand-written form's HIR)
- Modify: `docs/tutorial/` (new sections in `04-structs.md`, `05-patterns.md`, `09-arrays.md`, `11-functions.md`, `13-memory.md`)
- Modify: `docs/errors/{E0101,E0102,E0202,E0301,E0603,E1003,E1004}.md`, `docs/errors.md`
- Modify: `AGENTS.md` (identity: sugar desugars, it does not add HIR), `llms.txt`
- Create: `crates/yashy-driver/tests/verify_corpus/{spread_partial_move.ya,arena_escape.ya,arena_free.ya}`

**Interfaces:**
- Produces: a test per item asserting **HIR equality** between the sugar and its expansion — the only way the "no new concept" claim stays true after this phase. The helper compares with `==` (`HirProgram` already derives `PartialEq`) and prints both with `{:#?}` only on mismatch, so a failure names the differing node rather than dumping two whole programs.
- Produces: the three seeded bugs join the phase 1b corpus with their expected code sets (`E1002`, `E1003`, `E1004`).

- [ ] **Step 1: Write the HIR-equality tests** — eight tests, one per item (except `arena`, whose test asserts the inserted `defer` and placement instead).
- [ ] **Step 2: Run** `cargo test -p yashy-sema --test desugar` → PASS; any failure is a real desugaring difference and is fixed in the lowering, not in the test.
- [ ] **Step 3: Write the tutorial sections** with runnable blocks; the tutorial harness compiles every block.
- [ ] **Step 4: Run the gate table on both OSes**, then the zero-false-positive loop from phase 1b Task 7 → no new `FALSE POSITIVE` line.
- [ ] **Step 5: Commit and record** — `git commit -m "docs: the eight forms, their proofs, and close phase 5"`; update the index status and the ledger.

---

## Gate summary

| Gate | Command | Passes when |
|---|---|---|
| Sugar adds no concept | `cargo test -p yashy-sema --test desugar` | every form's HIR equals its hand-written expansion |
| No new keywords | `cargo test -p yashy-lexer` | `KEYWORDS.len() == 28` |
| `?` is not inverted | `cargo test -p yashy-driver --test e2e` | `option_try.ya`: `None` returns early; `Some` continues with the payload |
| Middle operand evaluated once | `cargo test -p yashy-driver --test e2e` | `chained_cmp.ya`: the counter in the middle is 1 |
| Spread moves | `cargo test -p yashy-verify` + corpus | partial move `E1002`; duplicate field `E0202` |
| Arena sound | `cargo test -p yashy-driver --test e2e arena` + leak suite | escape `E1003`, individual free `E1004`, no leak on `break` |
| Formatter | `cargo test -p yashy-fmt` | every new form round-trips byte-identically |
| No false positives | the phase 1b loop | no `FALSE POSITIVE` line |
| No regression | `cargo test --workspace` | green on Windows and Linux |
