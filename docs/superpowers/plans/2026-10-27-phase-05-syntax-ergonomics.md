# Phase 5 — Syntax and Ergonomics: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the boilerplate that makes Yashy read like a language from 2010 — without adding a concept the verifier, the formatter or a reader has to learn. The first task moves every declaration to type-first order; the rest add eight forms, every one of which desugars into HIR that already exists.

**Architecture:** Every item is a **parse-and-lower** change, never a new HIR construct and never a new runtime entry point. Type-first declarations are a surface change: the AST keeps `name` and `ty` and only the parse order moves. `if let`/`while let` reuse the `switch` pattern tests but carry two real blocks, so they lower to one small statement node (`HirStmt::IfLet`) — a `switch` arm is a single expression and cannot hold `return`, `break` or `continue`; `while let` is one `IfLet` inside the existing `for` skeleton. `?` on `Option` already reuses the existing `Try` lowering and needs only its diagnostics and tests finished; trailing blocks are ordinary lambdas with their parameter types unified from the callee; spread becomes field copies and concatenation; a chained comparison becomes nested `switch` expressions that bind each operand once, in source order, then `&&`; `for i, x` becomes a `for` over the index range; a field default is inserted where the literal is built; `arena { }` is a block whose `HirBlock::arena` bit moves its allocations into the block's existing arena. The test that keeps this honest is behaviour: each new form must run as the hand-written form it replaces, and where the plan promises HIR equality the HIR is the same.

**Tech Stack:** `yashy-lexer` (`TokenKind`, no new keywords at all), `yashy-parser/src/{exprs.rs,stmts.rs,items.rs,ast.rs}`, `yashy-sema/src/{check_expr.rs,check_pat.rs,scope.rs}` (lowering, typing and the partial-move rules), `yashy-hir/src/{hir.rs,escape.rs}` (the arena boundary; `escape.rs` moves here in Task 8 so the verifier and codegen share one analysis), `yashy-codegen/src/{expr_gen.rs,stmt_gen.rs,memory.rs,arena.rs}` (only for `?` on `Option`'s existing tag lookup and arena placement), `yashy-verify/src/pass/arena.rs`, `yashy-fmt`, `yashy-lsp`.

**Spec:** `docs/superpowers/specs/2026-10-27-yashy-master-design.md` §8 row 5. Index: `docs/superpowers/plans/2026-10-27-yashy-master-plan.md`. Predecessor: `2026-10-27-phase-04-optimization.md` (its CI-owned rows are recorded under the dated exception in that plan).

## Revision note (2026-11-04)

This revision was written against today's code, not the code the plan was first drafted from. Four findings changed task detail:

- **Type-first declarations are a new Task 0.** The user asked for the type to come before the name everywhere a variable or parameter is declared, with the old spelling rejected. That is a surface migration, not sugar, and it must land first so every later task's examples are written in the new spelling.
- **`?` on `Option` already ships.** It landed in phase 3b (`87c0e5b`): `check_expr.rs` already accepts `TypeKind::Option`, and `gen_try` already chooses `success_tag` from the subject type. Task 2 is therefore *finishing* the feature — tests, `E0301`, and docs — not writing the lowering. The old review-focus warning about an inverted tag is kept as a regression test, not as a bug to fix.
- **The formatter never reorders tokens.** `yashy-fmt` is a token/gap printer and deliberately never reorders, adds or removes a token. Task 0's migration therefore cannot be "run `yashy fmt`"; it is a span-based one-off rewriter, and the plan says so.
- **Several paths had drifted.** Ownership lives in `crates/yashy-sema/src/{scope.rs,check_expr.rs}`, not `yashy-verify/src/pass/ownership.rs`. The tutorial's memory chapter is `07-memory.md`, not `13-memory.md`. `fill_struct_fields` does not exist yet and is created by Task 4. `KEYWORDS` is already 34. The label lowering must use mutable `let`, not `const`, for its flags.

## What already ships (do not re-add)

Three items in the approved design were already implemented before this plan was written. They are listed here so a future reader does not schedule them again:

- **String interpolation** — `"n = ${n}"` is implemented end to end: the lexer splits `${ ... }` (`yashy_lexer::split_interpolation`), the parser builds `InterpPart`s, `:?` is `InterpFormat::Debug`, and the checker lowers each part to a conversion plus `+`. `crates/yashy-driver/tests/programs/interp.ya` asserts that the interpolated form and the written-out form are the same program. The spelling is `${...}`, and it stays that spelling.
- **`const fn`** — `Item::ConstFn` exists in the parser (`items.rs`) and `yashy-sema/src/const_eval.rs` runs it. A `const fn` has no run-time existence; it is evaluated where a `const` initialiser needs it.
- **`?` on `Option`** — `check_expr.rs` accepts it and `gen_try` reads the success tag from the subject type. There is no `option_try.ya` test and no `E0301`, which is what Task 2 adds.

The items that replace the first two in the curated list are **default field values** and the **`arena { }` block** — both from the same approved categories (boilerplate removal, and performance syntax that pairs with phase 4's promotion), and neither exists today.

## Global Constraints

See the master index. The ones that bite here:

- **No new keywords in this phase.** `if let`, `while let`, `for i, x`, `arena` and `..` are all spelled with tokens that already exist. Phase 2's `pure`/`io` and phase 3b's `repr` added three keywords, so `KEYWORDS` is 34 entering this phase (28 at the start, plus phase 1a's three contract keywords, plus phase 2's `pure`/`io`, plus phase 3b's `repr`) and stays 34.
- **Type-first is the only declaration order after Task 0.** A binding or parameter writes `type name`; the old `name: type` spelling is `E0109`, not a warning and not a second accepted form. Fields, enum payloads and function types keep `name: type` because their grammar is a field list or a type, not a variable declaration.
- **No HIR growth for sugar.** If an item needs a new `HirExpr`/`HirStmt` variant, it is not sugar and does not belong in this phase. Two exceptions are recorded: `HirBlock::arena` is one boolean on an existing struct marking an arena boundary, because the boundary is a placement fact, not a statement; and `HirStmt::IfLet` carries two blocks (`then_block`, `else_block`) plus the pattern and subject, because `HirSwitchArm::body` is a single `HirExpr` and cannot represent a block, `return`, `break` or `continue` — lowering `if let` to `switch` would silently lose those statements. `while let` still lowers to the existing `HirStmt::ForC` whose body is one `HirStmt::IfLet` with `else { break; }`.
- **No keyword renames, no unification, no attributes.** `switch` stays `switch`; `let` stays mutable and `const` immutable; `var` stays reserved and keeps its "use `let`" error; there is no `@inline`.
- **One rule, one implementation.** Each desugaring lives in one function in `sema`, called from every place the form can appear. The arena escape question is answered once in `yashy-hir/src/escape.rs` and shared by the verifier and codegen.
- **Both operating systems or it is not done.**
- **Format touched files only.**

## Written cap (this phase)

- **No `?.`**, no optional-chaining operator, no `??`.
- **No operator overloading**, so a chained comparison is a comparison chain, never a call to a user `PartialOrd`.
- **No format specifiers beyond `:?`** — interpolation keeps exactly the two forms it has.
- **No `for i, x` over iterators in v1.** Arrays and slices only; the diagnostic says when the general form arrives (phase 8).
- **No nested `arena` inference across function calls.** An allocation made by a callee is placed by the callee; the block governs the code written inside it.
- **No field-order flip, no `var` reintroduction, no expression-bodied functions.** Type-first reaches bindings and parameters only.

## Review Focus

1. **`?` on `Option` is already correct, and a regression test keeps it that way.** `variant_tag` in `codegen/ir.rs` says `Option`: `None` = 0, `Some` = 1; `Result`: `Ok` = 0, `Err` = 1. `gen_try` already branches on the subject's own success tag (`success_tag` at `expr_gen.rs:2433`). Task 2 must not "fix" it back to a hard-coded `0`; it adds `option_try.ya` and `E0301` for mixing the two error types.
2. **`if let` with an irrefutable pattern is an error, not a warning** (`E0603`). `if let x = 1 { }` and `if let Point { x, y } = p { }` are always true; the confusion with `let` is exactly what the diagnostic exists to catch, and a warning would be ignored. The question is type-aware: a struct pattern covering every field is irrefutable, a variant pattern on a two-variant `Option` is not.
3. **A chained comparison evaluates every operand once, in source order.** `0 <= f() < 10` calls `f()` exactly once, after `0` and before `10`. The lowering uses nested `switch` expressions with binding arms — one subject per operand — because HIR has no block expression and statement injection would be wrong under short-circuiting and in loop conditions. The HIR-equality test compares the sugar to the written nested `switch`.
4. **Spread moves, and reusing the operand is `E1002`.** `P { x: 1, ..base }` takes the remaining fields out of `base`. Version 1 moves the operand as a whole and rejects any later use, even of an overridden field; a path-sensitive partial move is a later design. Copying the fields instead would double-free a linear field, so the move is the only sound rule.
5. **`arena { }` is a promise the block releases everything in it.** A value that escapes the block is `E1003`; an individual `.free()` of an arena-placed value is `E1004` (the block owns it). `yashy-codegen` already gives every `HirBlock` an arena scope and releases it on fall-through, `break`, `continue` and `return` (`stmt_gen.rs:176-209`); Task 8 reuses that machinery and adds the boundary bit, not a second cleanup path.
6. **Labels are flags, and the flags are mutable.** `const` is immutable and assigning to it is `E0400`, so the lowering must emit `let __brk_L = false;`. A `break L` sets the flag and executes a real `break`; each intervening loop's body ends with `if __brk_L { break; }`, and `continue L` is propagated the same way. A check at the *top* of the body would miss a flag set later in the same iteration.

---

### Task 0: Type-first declarations and parameters (`E0109`)

**Files:**
- Modify: `crates/yashy-parser/src/stmts.rs` (`parse_const`, `parse_let`)
- Modify: `crates/yashy-parser/src/items.rs` (`parse_param`, extern declarations, `impl` methods)
- Modify: `crates/yashy-parser/src/parser.rs` (`peek_nth`)
- Modify: `crates/yashy-parser/src/exprs.rs` (a lambda's parameter list reuses `parse_param`, so only its doc contract changes)
- Modify: `crates/yashy-common/src/codes.rs` (`TYPE_AFTER_NAME = "E0109"`)
- Create: `docs/errors/E0109.md`; modify `docs/errors.md`
- Create: `crates/yashy-parser/src/bin/migrate.rs` (the one-off migrator; it lives
  beside the token stream it needs, and is **deleted in the cutover commit**)
- Create: `crates/yashy-parser/tests/declarations.rs` (the new grammar and `E0109`)
- Modify: `crates/yashy-driver/tests/e2e.rs` (one test that builds and runs a type-first program), `crates/yashy-fmt/tests/format.rs` (round-trip)
- Migrate: every tracked `.ya` file, every fenced `yashy` block under `docs/`, `examples/` and `benchmarks/`, and the Yashy fixtures inside Rust string literals

**Interfaces:**
- Produces the grammar:
  ```text
  binding := (`let` | `const`) [type] name `=` expr `;`
  param   := type name | `self` | `self` `:` type
  lambda  := `|` [lambda_param (`,` lambda_param)*] `|` [`->` type] block
  ```
  The untyped binding is still legal: after `let`/`const`, an identifier followed immediately by `=` means "no written type". A written type is any shape `parse_type_expr` accepts: `int`, `Point`, `io.Error`, `[]T`, `[T; N]`, `[T]`, `*mut T`, `*const T`, tuples, `Option[T]`, `Result[T, E]`, and `fn(T) -> U`. The AST does not change: `Stmt::Let/Const` keep `{ name, ty: Option<TypeExpr>, init, span }`, and `Param` keeps `{ name, ty, span }`.
- Produces `E0109` at the colon for `let x: int`, `const x: int`, `fn f(a: int)`, and `|x: int|`, with the message "the type is written before the name: write `int x`, not `x: int`" and a `docs:` line. A parsed `self: *mut T` receiver is untouched: `self` is the receiver, not a named parameter.
- Produces a token-based migration command that lexes a region and rewrites each `name : type` binding or parameter into `type name`, applying the edits backwards so comments and layout outside the edit survive. It reads three kinds of file: a `.ya` file whole; a `.md` file's fenced ` ```yashy ` blocks, leaving every other byte alone; and the body of every Rust string literal in a `.rs` file, since the compiler's tests hold their fixtures there. In the `.rs` mode each escape sequence is replaced by spaces of the same byte length, so a token offset is a byte offset in the file and no edit can land inside an escape; Rust comments and char literals are skipped. A region that does not lex is left exactly as it is. A struct field or a struct literal's `field: value` is never preceded by `let`, `const`, `fn name(` or a lambda's `|`, so neither is touched.
- Does **not** flip struct/object fields (`struct Point { x: int }` stays), enum payloads, or function type syntax (`fn(int) -> int` stays). Does **not** reintroduce `var`.

- [x] **Step 1: Write the failing tests** — `let int x = 5;`, `const string name = "x";`, `let *mut int p = x;`, `let []int xs = ys;`, `let Option[int] o = none;`, `let (int, string) pair = p;`, `let fn(int) -> int f = g;`, and an untyped `let x = 5;` all parse; `fn add(int a, int b) -> int` parses; `|int x| -> int` parses; `self: *mut T` still parses; `let x: int = 5;`, `fn f(a: int)`, and `|x: int|` each produce `E0109` at the colon. The migrator is a throwaway verified on one sample of each file kind (a `.ya` binding, a `.md` fence, a `.rs` fixture string) rather than a unit-tested component: it is deleted at the cutover, so a test for it would outlive the code it tests. The `crates/yashy-fmt` round-trip is covered by the migrated tree compiling and formatting unchanged.
- [x] **Step 2: Run and watch them fail** — `cargo test -p yashy-parser --test declarations` → FAIL.
- [x] **Step 3: Implement the dual-accept grammar and the migration command** — the parser accepts both orders for this commit only; the migration command is the span editor described above. Commit this separately: `feat(parser): accept type-first declarations and add the one-off migrator`. Landed as `f10831e`.
- [x] **Step 4: Migrate the tree** — run the command over every tracked `.ya` file and over `docs/**/*.md`, `examples/**/*.md`, `benchmarks/**/*.md` and the Yashy fixtures in `crates/**/*.rs`; inspect the diff for anything outside a declaration. 1466 edits in 199 files; `cargo check --workspace --all-targets` clean. Landed with the cutover in `fd380f1`.
- [x] **Step 5: Cut over** - the old-order branches are gone (`parse_binding_head` and `parse_param` both return `type_after_name` at the colon), `migrate.rs` and its `bin` directory are deleted, and `docs/errors/E0109.md`, its `codes.rs` entry and its `docs/errors.md` row are added. The gate ran as `cargo test --workspace` through the padded launcher: every one of the 120 test binaries passed (the runner also picked up the `yashy` CLI itself, which exits 2 with no arguments and is not a test target), including `e2e` 153/153, the tutorial 2/2 and the examples 3/3. Commit: `fd380f1` `feat!: type-first declarations and parameters (E0109)`.
- [x] **Step 6: Fix the prose** - the master spec's `link_name` example, `AGENTS.md`'s declaration rule, `README.md` (generic signature and lambda), `docs/tutorial/02-values.md` (the typed binding the tutorial never showed), `docs/tutorial/11-functions.md` generics, the E0002/E0100/E0105/E0300/E0900/E0901/E0902/E2003 transcripts (code and recomputed header column), and the future phase 07/08/10/11 snippets. `cargo test -p yashy-driver --test docs` (3/3) and `--test agents` (2/2) pass.

---

### Task 1: `if let` and `while let`

**Files:**
- Modify: `crates/yashy-parser/src/ast.rs` (`Stmt::IfLet { pattern, value, then_block, else_block, span }`, `Stmt::WhileLet { pattern, value, body, span }`)
- Modify: `crates/yashy-parser/src/stmts.rs` (`parse_if_let`, `parse_while_let`)
- Modify: `crates/yashy-hir/src/hir.rs` (`HirStmt::IfLet { pattern, subject, then_block, else_block, span }`)
- Modify: `crates/yashy-sema/src/check_expr.rs` (`check_if_let`, `check_while_let`, `check_let_head`)
- Modify: `crates/yashy-sema/src/check_pat.rs` (`is_irrefutable(pattern, ty, variants, resolve) -> bool`)
- Modify: `crates/yashy-codegen/src/patterns.rs` (`test_for_pattern`, `emit_test`, `pattern_names`, `gen_if_let`)
- Modify: `crates/yashy-codegen/src/stmt_gen.rs` (the `HirStmt::IfLet` arm), `crates/yashy-codegen/src/escape.rs` (the `IfLet` walk)
- Modify: `crates/yashy-verify/src/lib.rs` and `crates/yashy-verify/src/pass/{contracts,exhaustive,inventory,leak,params,typestate,warnings}.rs` (each `IfLet` arm)
- Modify: `crates/yashy-common/src/codes.rs` (`IRREFUTABLE_IF_LET = "E0603"`), create `docs/errors/E0603.md`, modify `docs/errors.md`
- Modify: `crates/yashy-fmt/tests/fmt.rs` (round-trip; the printer is token-based, so no formatter change was needed)
- Create: `crates/yashy-parser/tests/if_let.rs`, `crates/yashy-sema/tests/if_let.rs`, `crates/yashy-driver/tests/programs/if_let.ya`; modify `crates/yashy-driver/tests/e2e.rs` (one test that builds and runs it)

**Interfaces:**
- Produces: `if let Some(x) = opt { } else { }` and `while let Some(x) = it.next() { }`, with the existing `Pattern` grammar — variants, struct patterns and tuple patterns, exactly what `switch` accepts today. Or-patterns and ranges arrive with phase 7 and are accepted here for free, because the pattern is the same node.
- Produces: `HirStmt::IfLet` with the pattern, the subject expression and two real blocks. This is the recorded deviation from "lower to existing HIR": `HirSwitchArm::body` is a single `HirExpr`, so a `switch` lowering cannot carry a block, a `return`, a `break` or a `continue` in either branch. The pattern test is still shared with `switch` (`test_for_pattern` in `patterns.rs`), so the two forms ask the same question and only the shape of the answer differs.
- Produces: `while let` as `HirStmt::ForC { init: None, cond: None, update: None }` whose body is one `IfLet` with `else_block = Some(HirBlock { stmts: vec![HirStmt::Break] })`. Owning subjects are released at the merge point, so a subject built for the test (`next()`) is not leaked; the e2e program keeps to `Option[int]` subjects, which own nothing.
- Produces: `E0603` on a pattern that matches every value of its type, with the fix "use `let`". The message names the form: ``this pattern matches every `{ty}`: `if let` needs a pattern that can fail`` or ``...`while let` would never end``. `is_irrefutable` is type-aware and lives in `check_pat.rs`, where the checker's enum table is in scope: a wildcard or binding is irrefutable; a struct pattern is irrefutable when it names every field of its variant; a tuple pattern is irrefutable when every element is; a variant pattern is irrefutable only for a single-variant enum whose payload pattern is irrefutable; `Option` and `Result` patterns are never irrefutable; a literal is never irrefutable.

- [x] **Step 1: Write the failing tests** — parser (`tests/if_let.rs`): both forms parse, the else block is kept, a struct-variant pattern parses, an ordinary `if` is unchanged, `invariant` still heads a `while let` body, a missing pattern or value is a parse error. Sema (`tests/if_let.rs`): the payload is bound for the then block, the binding is not visible after the statement or in the else block, ownership merges, `while let` lowers to the loop-and-break shape, a literal pattern type-checks, and `if let x`, `if let _`, a single-variant payload and a single-variant struct pattern are `E0603` (with the `while let` wording pinned). `codes.rs` pins the `E0603` span, `fmt.rs` round-trips both forms byte-identically.
- [x] **Step 2: Run and watch them fail** — the new suites failed against the old parser and checker before the implementation landed.
- [x] **Step 3: Implement** — parse `let` after the `if`/`while` keyword into a pattern plus `= expr`; `let` is already a keyword and an `if` condition cannot start with it. `while let` uses `parse_loop_body`, so `invariant` works there. Lowering happens in `check_expr` next to `Stmt::Switch`; the irrefutability question is asked before the blocks are checked, because the error is about the form, not the HIR. The pattern's scope is popped before the `else` branch, so the else path does not see the names it did not bind.
- [x] **Step 4: Run** `cargo test -p yashy-parser -p yashy-sema -p yashy-fmt` and `cargo test -p yashy-driver --test e2e if_let` → PASS. The `E0603` transcript in `docs/errors/E0603.md` was re-run through the CLI and matches (`irrefutable.ya:3:12`, the same caret and `docs:` line).
- [x] **Step 5: Commit** — `git commit -m "feat: if let and while let (E0603)"`. Landed as `379c3b0`.

---

### Task 2: `?` on `Option` — finish the diagnostics and tests

**Files:**
- Modify: `crates/yashy-sema/src/check_expr.rs` (`Expr::Try`: code the three mixing/return-shape errors with `E0301`; the `Option` lowering itself already exists from `87c0e5b`)
- Modify: `crates/yashy-common/src/codes.rs` (`MIXED_TRY = "E0301"`), create `docs/errors/E0301.md`, modify `docs/errors.md`
- Modify: `crates/yashy-codegen/src/expr_gen.rs` (comment only: name the tag table `gen_try` already uses; no behavioural change)
- Create: `crates/yashy-driver/tests/programs/option_try.ya`, modify `crates/yashy-driver/tests/e2e.rs` (one test that builds and runs it)
- Create: `crates/yashy-codegen/tests/option_try.rs` (the success tag is `1` for `Option` and `0` for `Result`; the regression that stops a future copy-paste from inverting it)

**Interfaces:**
- Produces: `fn lookup(string k) -> Option[i64]` and `let v = lookup(k)?;` inside a function returning `Option[...]`, lowering to "on `None`, return `None`; otherwise continue with the payload". This already works; the task proves it.
- Produces: `gen_try` keeps reading the success tag from `variant_tag(&subject_ty, "Some"|"Ok", None)`. The codegen test asserts the emitted IR uses `icmp eq i8 %tag, 1` for an `Option` subject and `icmp eq i8 %tag, 0` for a `Result` subject.
- Produces: `E0301` when `?` mixes the two: an `Option` inside a function returning `Result`, a `Result` inside a function returning `Option`, and either inside a function returning anything else. The note names both types and the two legal fixes (`switch`, or change the return type).

- [x] **Step 1: Write the failing tests** — `option_try.ya` returns `None` through two levels and asserts the early return; a `Result` `?` in an `Option` function is `E0301`; an `Option` `?` in a `Result` function is `E0301`; `?` on an `Option` in a function returning `int` is `E0301`; the existing `Result` `?` tests are unchanged apart from the message text; the IR test pins the two success tags. The span of `Expr::Try` covers the subject and the `?`, not the `?` alone, so the three `codes.rs` tests expect `o?` and `g()?`. Semantics tests in `check.rs` add the `Option` success path, both mixings, the plain-function rejection, and a note check; `codes.rs` pins the span for each rejection path.
- [x] **Step 2: Watch them fail** — the first run of the new tests failed on the span (they expected `?`, the checker reports `o?`/`g()?`) and on the older `errors.rs` test that still pinned `` `?` needs a function returning `Result` ``. Both were corrected; after that the suites went green.
- [x] **Step 3: Implement** — the checker's `Expr::Try` arm already computes `TypeKind::Option(inner)` and yields `HirExpr::Try` with `ty = inner`; the three rejection paths now report `.with_code(MIXED_TRY)` and a note naming the enclosing return type and the two fixes. The codegen change is a comment naming the `variant_tag` table; the IR test is the guard against inverting `Some` = 1 and `Ok` = 0.
- [x] **Step 4: Run** — `cargo test -p yashy-sema` green (16 targets; `methods` ran under the padded launcher at 15/15 after the host blocked the fresh binary), `cargo test -p yashy-codegen --test option_try` green (2/2), `cargo test -p yashy-driver --test e2e question_mark_on_an_option` green (1/1), `cargo test -p yashy-driver --test docs --test explain` green (3/3, 12/12). The `E0301` transcript in `docs/errors/E0301.md` was re-run through the CLI and matches (`mixed.ya:2:19`, the `^^` under `o?`, and the note).
- [x] **Step 5: Commit** — `git commit -m "feat: finish ? on Option with E0301 and the tag regression test"`. Landed as `972d937`.

---

### Task 3: Trailing block argument and closure parameter inference

**Files:**
- Modify: `crates/yashy-parser/src/ast.rs` (`LambdaParam { name, ty: Option<TypeExpr>, span }`; `Expr::Lambda` uses it)
- Modify: `crates/yashy-parser/src/exprs.rs` (trailing `Block` becomes the last argument of `Call`/`MethodCall`; a `no_block_arg` flag suppresses it while parsing `if`/`while`/`for`/`switch` heads)
- Modify: `crates/yashy-parser/src/parser.rs` (the flag, if the parser field lives there)
- Modify: `crates/yashy-sema/src/check_expr.rs` (unify the lambda's parameter and return types from the callee's signature)
- Modify: `crates/yashy-fmt/src/lib.rs` (put the trailing block on its own line; the printer keeps the tokens in order)
- Modify: `crates/yashy-lsp/src/server.rs` (signature help and hover for a parameter list in the new order)
- Modify: `crates/yashy-driver/tests/programs/closures.ya` (add a trailing-block call beside the existing closures), `crates/yashy-driver/tests/e2e.rs` (the existing `closures` test now covers the block form too)

**Interfaces:**
- Produces: `xs.each { |x| println("${x}") }` and `with_file(path) { |f| read(f) }` — the block is the last argument, and `|x|` takes its type from the callee's parameter type. `LambdaParam.ty` is `None` for an inferred parameter; function and extern parameters still require a type.
- Produces: the parser refuses a trailing block in a condition position (`if f(x) { }` keeps meaning "if the call, then this block"), with a `no_block_arg` flag set while parsing `if`, `while`, `for` and `switch` heads. The current struct-literal decision is a local `looks_like_struct_lit` boolean in `exprs.rs:487`; the trailing-block decision reuses the same "not in a head" flag, so the two ambiguities have one cause.
- Produces: `E0100` naming both signatures when inference fails (a callee that takes `fn(i64) -> i64` called with `|x| "text"` reports the expected and the found return type).

- [x] **Step 1: Write the failing tests** — a trailing block passes as the last argument; `|x|` infers from the callee; an explicit `|int x|` still works and must agree; `if f(x) { }` parses as an `if`; a lambda passed to a generic parameter that does not fix its type is `E0100` with both signatures; the formatter puts the block on its own line and round-trips. Parser tests in `crates/yashy-parser/tests/lambdas.rs` (inferred param, trailing call, paren-less method block, head-`{` regression for `if`/`while`/`for`); sema tests in the new `crates/yashy-sema/tests/lambdas.rs` (9: callee inference, return inference, explicit types, both disagreements, map/filter/fold, generic `fn(T) -> T`); one fmt test pinning the block layout and idempotence; `closures.ya` gains a `twice(20) { |x| return x + 1; }` call printing `22`.
- [x] **Step 2: Watch them fail.** The parser tests failed to compile against `Vec<Param>` (the RED that drove `LambdaParam { ty: Option }`), and the sema trailing-block test failed in the parser until `parse_trailing_lambda` landed.
- [x] **Step 3: Implement** — one `no_block_arg` field on `Parser`, set with `without_block_arg()` around the `if`/`while`/`for`/`switch` heads, switch-arm guards, and the `switch`-expression subject; when a `{` followed by `|` or `||` follows a call's `)` (or a bare callee path, or a dotted method with no parentheses), it is parsed as a lambda and appended to `args`. `check_expr_with_expected` checks a lambda against an expected `Func` signature; `check_callback` does the same for `map`/`filter`/`fold`, with `map`'s return inferred from the body's first `return` via a `lambda_ret_infer` stack. The generic-argument pass refuses inferred parameters with `E0100` naming the declared `fn(T) -> T`. Rulings: the formatter needed no code change (its token printer already lays the block out as a block, pinned by the new test); the LSP needed no change (there is no signature-help request, and hover's `user_signature` reads unchanged `fn` declarations).
- [x] **Step 4: Run** `cargo test -p yashy-parser -p yashy-sema -p yashy-fmt -p yashy-lsp` → PASS, plus `cargo test -p yashy-driver --test e2e a_closure_outlives` and `cargo check --workspace --all-targets` clean.
- [x] **Step 5: Commit** — `git commit -m "feat: trailing block arguments with inferred closure parameters"`. Landed as `150579c`.

---

### Task 4: Struct and array spread

**Files:**
- Modify: `crates/yashy-parser/src/ast.rs` (`StructLit.spread: Option<Box<Expr>>`, `SliceLit.elements: Vec<SliceElement>` with `SliceElement::{Value, Spread}`)
- Modify: `crates/yashy-parser/src/exprs.rs`
- Modify: `crates/yashy-sema/src/check_expr.rs` (create `fill_struct_fields`; field fill and concatenation)
- Modify: `crates/yashy-sema/src/scope.rs` (a spread moves its operand; record the reason so a later use is `E1002`)
- Modify: `crates/yashy-common/src/codes.rs` (`DUPLICATE_FIELD = "E0202"`, `PARTIAL_MOVE = "E1002"`), create `docs/errors/{E0202,E1002}.md`, modify `docs/errors.md`
- Modify: `crates/yashy-fmt/src/lib.rs`
- Create: `crates/yashy-driver/tests/programs/spread.ya`, modify `crates/yashy-driver/tests/e2e.rs` (one test that builds and runs it)

**Interfaces:**
- Produces: `P { x: 1, ..base }` — explicit fields win, the rest are moved out of `base`; `P { ..base }` alone is legal. The spread must be last (`..base, x: 1` is a parse error with "the spread is written last").
- Produces: `[0, ..xs, 4]` — a new `[]T` whose elements are the prefix, every element of `xs`, then the suffix. The element type comes from the non-spread elements, or from the spread when there are none.
- Produces: `fill_struct_fields(fields, spread, defaults) -> Result<Vec<...>>` in `check_expr.rs`. Task 7 calls the same function, so precedence has exactly one implementation: explicit field, then spread, then default, then `E0201`.
- Produces: `E0202` when a field is given twice (`P { x: 1, x: 2 }`), naming both spans.
- Produces: `E1002` when the spread operand is used after the literal, and when a linear field would be copied rather than moved. Version 1 treats the operand as moved as a whole; `scope.rs` records a `MoveReason::Spread` so the later-use diagnostic is `E1002` rather than the generic `E0500`.

- [ ] **Step 1: Write the failing tests** — struct spread fills the unlisted fields; an explicit field overrides the spread; a field given twice is `E0202`; using `base` after `P { ..base }` is `E1002`; a spread of the wrong type is `E0100`; `[0, ..xs, 4]` produces the concatenation in order; `[..xs]` alone is legal; spreading a slice of a linear type moves it (`E1002` if used after); the formatter round-trips `..`.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — the checker builds the field list through `fill_struct_fields` and lowers to the same `HirExpr::StructLit` it already produces. Array spread lowers to a sequence of element stores into a freshly allocated slice; the length is known at check time for fixed arrays and computed once for slices, so there is no growth loop. The move rule is in `scope.rs` beside the existing `move_out`, because that is the one place a binding becomes moved.
- [ ] **Step 4: Run** `cargo test -p yashy-parser -p yashy-sema -p yashy-driver --test e2e` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: struct and array spread, with partial-move rules (E0202, E1002)"`

---

### Task 5: Chained comparison

**Files:**
- Modify: `crates/yashy-parser/src/ast.rs` (`Expr::CompareChain { operands: Vec<Expr>, ops: Vec<BinOp>, span }`)
- Modify: `crates/yashy-parser/src/exprs.rs` (merge the comparison levels so a chain is one node)
- Modify: `crates/yashy-sema/src/check_expr.rs` (lower to nested `switch` expressions and `&&`)
- Modify: `crates/yashy-common/src/codes.rs` (`CHAINED_EQUALITY = "E0101"`), create `docs/errors/E0101.md`, modify `docs/errors.md`
- Modify: `crates/yashy-fmt/src/lib.rs`
- Create: `crates/yashy-driver/tests/programs/chained_cmp.ya`, modify `crates/yashy-driver/tests/e2e.rs` (one test that builds and runs it, including the once-only evaluation counter)

**Interfaces:**
- Produces: `0 <= x < 10` lowered to nested binding `switch` expressions:
  ```text
  switch 0 { case __c0 { switch x { case __c1 { switch 10 { case __c2 {
      __c0 <= __c1 && __c1 < __c2
  } } } } }
  ```
  Each subject is evaluated once, in source order, and the `&&` keeps the short-circuit. The synthetic names are mangled so they cannot collide with a user binding. This uses only `HirExpr::SwitchExpr`, `HirPattern::Binding`, `HirExpr::Binary` — no new HIR and no statement injection.
- Produces: chains of any length (`a < b <= c < d`) and any mix of ordering operators.
- Produces: `E0101` when `==` or `!=` appears anywhere in a chain (`a == b == c`), with the fix "write the two comparisons separately with `&&`". A single `a == b` is not a chain and stays legal.
- Produces: operands must all have the same type and it must be `i64`, `f64` or `char`; anything else is the existing `E0100`.

- [ ] **Step 1: Write the failing tests** — `0 <= x < 10` is true only in range; `f()` in the middle is called exactly once (asserted with a counter); `a < b > c` parses as `a < b && b > c`; `a == b == c` is `E0101`; `1 < "x"` is `E0100`; the formatter prints the chain unparenthesised and round-trips; the HIR-equality test compares the sugar to the written nested `switch`.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — `parse_cmp` collects consecutive comparison operands (a comparison operator followed by another comparison operator continues the chain rather than nesting); `parse_eq` folds into the same collector so `==` is seen in the chain. The checker binds each operand through a `SwitchExpr` arm, then builds `HirExpr::Binary { op: And, .. }` over adjacent comparisons. Because HIR already short-circuits `&&`, the chain inherits it for free and a chain inside a false short-circuit is never evaluated.
- [ ] **Step 4: Run** `cargo test -p yashy-parser -p yashy-sema -p yashy-codegen -p yashy-driver --test e2e` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: chained comparisons with one evaluation of each operand (E0101)"`

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
- Modify: `crates/yashy-parser/src/{ast.rs,items.rs}` (`FieldDef.default: Option<Expr>`; field syntax stays `name: type`)
- Modify: `crates/yashy-sema/src/check_expr.rs` (`fill_struct_fields` from Task 4 gains the default leg; `E0201` message updated)
- Modify: `crates/yashy-sema/src/const_eval.rs` (evaluate a default that is a literal or a `const` name)
- Modify: `crates/yashy-common/src/codes.rs` (`FIELD_DEFAULT_NOT_CONST = "E0102"`), create `docs/errors/E0102.md`, modify `docs/errors.md`
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
- [ ] **Step 3: Implement** — Task 4 created `fill_struct_fields`; this task adds its default leg, so precedence has exactly one implementation. The default expression is checked against the field type when the struct is declared; the literal insertion clones the *checked* HIR expression, so a default cannot be type-checked differently at two call sites.
- [ ] **Step 4: Run** `cargo test -p yashy-parser -p yashy-sema -p yashy-driver --test e2e` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: default struct field values (E0102)"`

---

### Task 8: `arena { }`

**Files:**
- Modify: `crates/yashy-parser/src/ast.rs` (`Stmt::Arena { body: Block, span }`)
- Modify: `crates/yashy-parser/src/stmts.rs`, `crates/yashy-fmt/src/lib.rs`
- Modify: `crates/yashy-hir/src/hir.rs` (`HirBlock { stmts, span, arena: bool }` — one bit, not a new statement kind)
- Create: `crates/yashy-hir/src/escape.rs` (move the pure analysis out of `yashy-codegen/src/escape.rs`)
- Modify: `crates/yashy-codegen/src/escape.rs` (becomes a re-export of `yashy_hir::escape`), `{fn_gen.rs,memory.rs,arena.rs,expr_gen.rs,stmt_gen.rs}` (the arena bit forces placement; `gen_block` already releases the block's arena on every exit)
- Create: `crates/yashy-verify/src/pass/arena.rs` (`E1003` escape, `E1004` individual free), modify `crates/yashy-verify/src/pass/mod.rs`
- Modify: `crates/yashy-common/src/codes.rs` (`ARENA_ESCAPE = "E1003"`, `ARENA_FREE = "E1004"`), create `docs/errors/{E1003,E1004}.md`
- Modify: `crates/yashy-driver/tests/programs/arena.ya` (add the block form beside the existing builtin form), `crates/yashy-driver/tests/e2e.rs` (the existing `arena` test asserts both forms print the same sum)

**Interfaces:**
- Produces: `arena { ... }` — one arena for the block, released at block exit on every path, including `break`, `continue` and an early `return`. The lowering is `HirStmt::If { cond: true, then_block: HirBlock { arena: true, .. }, else_block: None }`: the existing `gen_block` already pushes an `arena_scopes` entry, releases it on fall-through and on every terminating flow (`stmt_gen.rs:176-209`), so there is no second cleanup path.
- Produces: every allocation inside an arena block is placed in that block's arena, regardless of the stack-size heuristic; the placement reuses `ArenaId::of_block`, so the id is the block's span and codegen can map it back. The analysis lives in `yashy-hir/src/escape.rs` so the verifier and codegen call the same function; `yashy-codegen/src/escape.rs` keeps its public names as a re-export so phase 4's imports and tests do not churn.
- Produces: no individual release runs for an arena-placed value inside the block — no `free`, no refcount decrement — because the block releases the whole arena. The `E1004` check is the verifier's, at the `.free()`/dealloc call.
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
- [ ] **Step 3: Implement** — move `classify_sites` and its types to `yashy-hir/src/escape.rs`, leaving `yashy-codegen/src/escape.rs` as `pub use yashy_hir::escape::*;`. Add `arena: bool` to `HirBlock` (the seven construction sites are in `check_expr.rs` and `check_expr/debug.rs`). In `escape.rs`, a site inside an arena block is `Placement::Arena(ArenaId::of_block(block))`, and an escape that crosses the arena block is recorded for the verifier. The verifier pass walks the same HIR, reports `E1003` for a recorded crossing and `E1004` for a free of a binding whose placement is `Arena`. The checker's `Stmt::Arena` lowering is the `if true` block above; it also inserts nothing else, because the block already owns its release.
- [ ] **Step 4: Run** `cargo test -p yashy-parser -p yashy-sema -p yashy-codegen -p yashy-verify -p yashy-driver --test e2e arena` and the leak suite → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: arena blocks with escape and free proofs (E1003, E1004)"`

---

### Task 9: Labeled `break` and `continue`

**Files:**
- Modify: `crates/yashy-parser/src/ast.rs` (`Stmt::ForIn`/`ForRange`/`ForC`/`WhileLet` gain `label: Option<String>`; `Stmt::Break(Option<String>, Span)`, `Stmt::Continue(Option<String>, Span)`)
- Modify: `crates/yashy-parser/src/stmts.rs` (`ident : for` and `ident : while` in statement position; `break ident`, `continue ident`)
- Modify: `crates/yashy-sema/src/check_expr.rs` (label resolution and the flag lowering)
- Modify: `crates/yashy-common/src/codes.rs` (`UNKNOWN_LABEL = "E0604"`, `DUPLICATE_LABEL = "E0605"`), create `docs/errors/{E0604,E0605}.md`, modify `docs/errors.md`
- Modify: `crates/yashy-fmt/src/lib.rs`
- Create: `crates/yashy-driver/tests/programs/labeled_loops.ya`, modify `crates/yashy-driver/tests/e2e.rs`

**Interfaces:**
- Produces: `outer: for i in 0..3 { for j in 0..3 { if j == 1 { break outer; } } }` and `continue outer`.
- Produces: a label is an identifier and a colon in **statement position only**, immediately before `for` or `while`. No new keyword, and `x: int` in a `const`/`let` or a struct literal is unaffected because those positions are not statement starts (and after Task 0 a type precedes a name; a statement-start identifier followed by `:` is unambiguous).
- Produces: `E0604` for `break nope` with no enclosing label of that name, listing the labels in scope; `E0605` for two loops with the same label nested inside each other.
- Produces: the lowering is **flags, not new HIR**, so the HIR-equality gate stays true. For a label `L`:
  - each loop strictly between the `break L`/`continue L` site and the labelled loop, and the labelled loop itself, gets `let __brk_L = false;` / `let __cont_L = false;` as needed. These are mutable `let` bindings, not `const` — assigning to a `const` is `E0400`;
  - `break L;` becomes `__brk_L = true; break;` — the existing `break`, which already exits the innermost loop through a `switch`;
  - `continue L;` becomes `__cont_L = true; break;`;
  - **at the end of** each intervening loop's body, emit `if __brk_L { break; }` and `if __cont_L { break; }`; the labelled loop's body ends with `if __brk_L { break; }` and `if __cont_L { __cont_L = false; continue; }`. The check is at the end, not the top, because the flag is set later in the same iteration.

  The cost is one boolean test per iteration of each intervening loop, and the plan accepts it: the alternative — a label on `HirStmt::Break` — is a new HIR concept, and phase 5's whole claim is that it adds none. The optimizer in phase 4 removes the test when the flag is provably false at that point.
- Produces: `defer` still runs on every path, because the lowering only ever emits a real `break`/`continue` — the arena and cleanup tests are the regression proof.
- Produces: an unlabelled `break` keeps its meaning exactly (innermost loop, through a `switch`), and the existing `loop_exit.ya` test is unchanged.

- [ ] **Step 1: Write the failing tests** — `break outer` from two loops deep exits both; `continue outer` skips the rest of the inner loop and re-tests the outer; a label on a `while` works; `break nope` is `E0604` with the in-scope labels; nested `L:` is `E0605`; a `defer` inside the inner loop runs when `break outer` fires; an unlabelled `break` in the same program still exits only the inner loop; the HIR of the sugar equals the HIR of the hand-written flags.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — the parser accepts `Ident Colon` only when the next token is `For` or `While`, which is decided by one token of lookahead and cannot misfire in a declaration; resolution walks the loop stack, which already exists for `break`/`continue` checking; the lowering runs in `check_expr` beside the existing `break` handling so there is one place that knows what "innermost loop" means.
- [ ] **Step 4: Run** `cargo test -p yashy-parser -p yashy-sema -p yashy-fmt -p yashy-driver --test e2e` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: labeled break and continue, lowered to flags (E0604, E0605)"`

---

### Task 10: Doc comments

**Files:**
- Modify: `crates/yashy-lexer/src/token.rs` (`TokenKind::DocComment(StringLit)`; `///` is lexed as one token carrying its text with the leading `/// ` stripped)
- Modify: `crates/yashy-lexer/src/lexer.rs` (`///` is checked before `//`; `////` is an ordinary comment)
- Modify: `crates/yashy-parser/src/ast.rs` (`Item::Documented { docs: Vec<StringLit>, item: Box<Item>, span }` — the same wrapper pattern as `Item::Public`, so no other item variant gains a field)
- Modify: `crates/yashy-parser/src/items.rs` (a run of `DocComment` tokens becomes one `Documented` wrapper; `Public` and `Documented` nest in either order)
- Modify: `crates/yashy-sema/src/lib.rs` (unwraps both wrappers when building the item table)
- Modify: `crates/yashy-common/src/codes.rs` (`DOC_COMMENT_MISPLACED = "E2100"`), create `docs/errors/E2100.md`, modify `docs/errors.md`
- Modify: `crates/yashy-fmt/src/lib.rs` (re-emits the doc block immediately above the item, with one space after `///`)
- Create: `crates/yashy-driver/tests/programs/doc_comments.ya`, modify `crates/yashy-driver/tests/e2e.rs`

**Interfaces:**
- Produces: `/// Adds two numbers.` above a `fn`, `struct`, `enum`, `object`, `const`, or a method inside `impl`; consecutive `///` lines form one block; the block attaches to the next item.
- Produces: `///` before a statement, an expression, or the end of a file is `E2100`, naming the item kinds a doc comment may precede.
- Produces: the text is **stored, not interpreted** in this phase — no Markdown rendering, no `#[doc]`, no doc tests. Phase 11's `yashy doc` reads it, and `yashy doc` is the only consumer.
- Produces: the formatter preserves the block byte-for-byte except for normalising `///` + no space to `/// `; `////` stays an ordinary comment and never attaches.
- Produces: the LSP hovers the doc block (a one-line change to the hover handler, which already has the item's span).

- [ ] **Step 1: Write the failing tests** — a doc block above `pub fn` is stored and round-trips through the formatter; two blocks separated by a blank line attach only the second; `///` before a `let` is `E2100`; `////` is not a doc comment; `///` at end of file is `E2100`; the LSP hover for a documented function returns the text; a doc comment does not change the program's output.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — the lexer's comment branch checks the third `/` before falling through to `//`; the parser's item loop collects a leading run into one wrapper and unwraps it when it dispatches on the item kind; the formatter prints `docs` before the item's own text. The AST change is one variant, so every `match` on `Item` gains one arm — the compiler finds them all.
- [ ] **Step 4: Run** `cargo test -p yashy-lexer -p yashy-parser -p yashy-sema -p yashy-fmt -p yashy-lsp -p yashy-driver --test e2e` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: doc comments, stored for yashy doc (E2100)"`

---

### Task 11: The sugar proof, docs and gate

**Files:**
- Create: `crates/yashy-sema/tests/desugar.rs` (each new form's HIR equals the hand-written form's HIR)
- Modify: `docs/tutorial/` (new sections in `03-control-flow.md` for labels and doc comments, `04-structs.md`, `05-patterns.md`, `07-memory.md`, `09-arrays.md`, `11-functions.md`)
- Modify: `docs/errors/{E0101,E0102,E0109,E0202,E0301,E0603,E0604,E0605,E1002,E1003,E1004,E2100}.md`, `docs/errors.md`
- Modify: `AGENTS.md` (identity: sugar desugars, it does not add HIR; `///` is documentation, not an attribute; declarations are type-first), `llms.txt`
- Create: `crates/yashy-driver/tests/verify_corpus/{spread_partial_move.ya,arena_escape.ya,arena_free.ya}`

**Interfaces:**
- Produces: a test per item asserting **HIR equality** between the sugar and its expansion — the only way the "no new concept" claim stays true after this phase. The helper compares with `==` (`HirProgram` already derives `PartialEq`) and prints both with `{:#?}` only on mismatch, so a failure names the differing node rather than dumping two whole programs. The chained comparison's expansion is the nested `switch` from Task 5; the arena block's expansion is the `if true` block with `arena: true`; labels expand to the flag statements.
- Produces: the three seeded bugs join the phase 1b corpus with their expected code sets (`E1002`, `E1003`, `E1004`).

- [ ] **Step 1: Write the HIR-equality tests** — ten tests, one per item (except `arena`, whose test asserts the inserted `if true`, the `arena` bit and placement instead, and `///`, which is stored rather than lowered).
- [ ] **Step 2: Run** `cargo test -p yashy-sema --test desugar` → PASS; any failure is a real desugaring difference and is fixed in the lowering, not in the test.
- [ ] **Step 3: Write the tutorial sections** with runnable blocks; the tutorial harness compiles every block. The memory chapter is `07-memory.md`.
- [ ] **Step 4: Run the gate table on both OSes**, then the zero-false-positive loop from phase 1b Task 7 → no new `FALSE POSITIVE` line.
- [ ] **Step 5: Commit and record** — `git commit -m "docs: the syntax changes, their proofs, and close phase 5"`; update the index status and the ledger.

---

## Gate summary

| Gate | Command | Passes when |
|---|---|---|
| Type-first, no second spelling | `cargo test -p yashy-parser --test declarations` + `yashy check` | every type shape parses before its name; `let x: int`, `fn f(a: int)` and `|x: int|` are `E0109`; no tracked source uses name-first |
| Migration complete | `cargo test --workspace` + tutorial + examples | every `.ya` file and every ` ```yashy ` block is type-first and compiles |
| Sugar adds no concept | `cargo test -p yashy-sema --test desugar` | every form's HIR equals its hand-written expansion |
| No new keywords | `cargo test -p yashy-lexer` | `KEYWORDS.len() == 34` (28 at the start, plus `requires`/`ensures`/`invariant` from phase 1a, plus `pure`/`io` from phase 2, plus `repr` from phase 3b) |
| `?` is not inverted | `cargo test -p yashy-driver --test e2e` + `-p yashy-codegen --test option_try` | `option_try.ya`: `None` returns early; `Some` continues with the payload; the IR uses success tag 1 for `Option` and 0 for `Result`; mixing is `E0301` |
| Middle operand evaluated once | `cargo test -p yashy-driver --test e2e` | `chained_cmp.ya`: the counter in the middle is 1, and every operand is evaluated in source order |
| Spread moves | `cargo test -p yashy-driver --test e2e` + corpus | partial move `E1002`; duplicate field `E0202` |
| Arena sound | `cargo test -p yashy-driver --test e2e arena` + leak suite | escape `E1003`, individual free `E1004`, no leak on `break` |
| Labels exit the right loop | `cargo test -p yashy-driver --test e2e` | `labeled_loops.ya`: `break outer` exits both; unlabelled `break` still exits one |
| Doc comments are inert | `cargo test -p yashy-driver --test e2e` | `doc_comments.ya` output is identical with and without the blocks; `E2100` on a misplaced one |
| Formatter | `cargo test -p yashy-fmt` | every new form round-trips byte-identically |
| No false positives | the phase 1b loop | no `FALSE POSITIVE` line |
| No regression | `cargo test --workspace` | green on Windows and Linux |
