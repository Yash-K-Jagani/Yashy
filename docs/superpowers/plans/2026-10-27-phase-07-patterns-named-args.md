# Phase 7 — Patterns and Named Arguments: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `switch` express what people actually mean (guards, alternatives, ranges, nesting anywhere a binding is allowed) and make calls readable at the call site (named and default arguments), without losing the exhaustiveness proof or the ownership proof.

**Architecture:** `Pattern` grows four variants (or, range, guard-on-arm, and a `let`-position pattern replacing today's tuple-only `Stmt::Destructure`). Exhaustiveness moves from "which variant names are missing" to a usefulness matrix over pattern columns, which is the only algorithm that stays correct once or-patterns and ranges exist. Named arguments are a call-site record — `CallArg { name: Option<String>, value, span }` — resolved to a parameter index before the HIR is built, so the HIR and every later pass see positional arguments and need no new concept.

**Tech Stack:** `yashy-parser` (`ast::Pattern`, `Stmt::Destructure`), `yashy-hir::exhaustive` (the module phase 1b moved out of sema), `yashy-sema/src/check_expr.rs` (pattern typing, call resolution), `yashy-verify` (partial-move rule), `yashy-fmt`.

**Spec:** `docs/superpowers/specs/2026-10-27-yashy-master-design.md` §8 row 7. Index: `docs/superpowers/plans/2026-10-27-yashy-master-plan.md`. Predecessor: `2026-10-27-phase-06-modules-unicode.md`.

## Global Constraints

See the master index. The ones that bite here:

- No implicit coercion, no operator overloading: a range pattern is a pattern, never an expression that compares.
- One rule, one implementation: exhaustiveness has exactly one algorithm, and it lives in `yashy-hir::exhaustive`.
- Ownership is explicit: a pattern that binds a linear value binds it on every path or the program is rejected.
- Both operating systems or it is not done.
- Format touched files only.

## Review Focus

1. **A guard does not make an arm exhaustive.** `case Color::Red if x > 0` covers `Red` only when the guard holds, so `Red` is still missing. The single most likely bug in this phase is treating a guarded arm as covering its pattern.
2. **Or-patterns bind the same names with the same types on every alternative.** `case Red | Green(n)` is an error because `n` exists on one side only; `case Red | Green` is fine.
3. **A range pattern over floats is rejected at parse time.** Ranges are for `int` and `char`; `1.0..2.0` has no decidable membership story here and is not a silent comparison.
4. **Named arguments do not change evaluation order.** Arguments evaluate left to right in source order, whatever order the parameters are declared in. Reordering evaluation is a semantic change hidden inside sugar.
5. **A default argument must be evaluable without effects or the caller's scope.** It may name a `const` or a literal; it may not call an `io` function or reference a parameter.

---

### Task 1: Pattern syntax: or, range, and a guard on the arm

**Files:**
- Modify: `crates/yashy-parser/src/ast.rs` (`Pattern::Or`, `Pattern::Range`, `SwitchArm.guard: Option<Expr>`)
- Modify: `crates/yashy-parser/src/{exprs.rs,stmts.rs}`
- Modify: `crates/yashy-lexer/src/token.rs` (`..` range token; confirm `|` already lexes as a single token outside `||`)
- Modify: `crates/yashy-fmt/src/lib.rs`

**Interfaces:**
- Produces:
  ```rust
  pub enum Pattern {
      // ... the six existing variants ...
      Or { alternatives: Vec<Pattern>, span: Span },
      Range { lo: Literal, hi: Literal, inclusive: bool, span: Span },
  }
  pub struct SwitchArm { pub pattern: Pattern, pub guard: Option<Expr>, pub body: Block, pub span: Span }
  ```
- Produces: `E0602` — a range pattern whose bounds are not both `int` or both `char`, or whose `lo > hi`.

- [ ] **Step 1: Write the failing tests** — `case 1 | 2 | 3 => {}` parses to one `Or` with three alternatives; `case 0..10 => {}` and `case 'a'..='z' => {}` parse; `case x if x > 0 => {}` puts the guard on the arm and not inside the pattern; `case 1.0..2.0` is `E0602`; the formatter round-trips every form byte-identically.
- [ ] **Step 2: Run and watch them fail** — `cargo test -p yashy-parser patterns` → FAIL.
- [ ] **Step 3: Implement** — or-patterns are right-associative and flatten into one `Or` node; ranges bind nothing; the guard is parsed by `parse_expr` after the `if` keyword that already exists in the grammar for `if` statements, so no new keyword is added. `|` must be checked for conflict with the bitwise-or operator the expression grammar already has: in pattern position the parser expects a pattern, so the ambiguity is positional and needs a test, not a new token.
- [ ] **Step 4: Run** `cargo test -p yashy-parser -p yashy-fmt` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(parser): or-patterns, range patterns and arm guards"`

---

### Task 2: A pattern in `let`, `const` and `for`

**Files:**
- Modify: `crates/yashy-parser/src/ast.rs` (`Stmt::Destructure` → `Stmt::LetPattern { kind, pattern: Pattern, init: Expr, span }`, with `Destructure` kept as a deprecated alias during migration or removed in one commit with every use updated)
- Modify: `crates/yashy-parser/src/stmts.rs`, `crates/yashy-sema/src/check_expr.rs`, `crates/yashy-codegen/src/stmt_gen.rs`
- Modify: `crates/yashy-fmt/src/lib.rs`

**Interfaces:**
- Produces: `let Point { x, y } = p;`, `let (a, _) = pair;`, `for Point { x, .. } in points { }` — one `Pattern` in binding position, everywhere a binding is introduced.
- Keeps: the tuple form `let (a, b) = pair;` parses to the same node as before, so no program changes.

- [ ] **Step 1: Write the failing tests** — struct destructuring in `let`, nested `let Pair { left: Point { x, .. }, .. }`, wildcard in a tuple, `for` over a slice with a struct pattern, and an irrefutable-pattern check: `let Some(x) = opt;` is `E0100` (a refutable pattern in an irrefutable position) with the fix "use `switch`".
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — the checker walks the pattern against the initialiser's type, binding each name in the enclosing scope with the same rules `Stmt::Let` uses today. The codegen path is the existing `Destructure` lowering generalised: a struct pattern becomes field extracts, a nested pattern recurses.
- [ ] **Step 4: Run** `cargo test -p yashy-sema -p yashy-codegen -p yashy-driver --test e2e` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: destructure structs and nested patterns in let, const and for"`

---

### Task 3: Exhaustiveness as a usefulness matrix

**Files:**
- Modify: `crates/yashy-hir/src/exhaustive.rs` (replace `missing_cases` internals; keep the signature, add `usefulness` for Task 4)
- Modify: `crates/yashy-verify/src/pass/exhaustive.rs`

**Interfaces:**
- Produces: `pub fn missing_cases(enum_name, variants, arms, has_default) -> Vec<String>` still returning variant names (so existing diagnostics and tests keep working), implemented over a matrix of pattern columns.
- Produces: `pub fn reachable_arms(enum_name, variants, arms, has_default) -> Vec<bool>` for Task 4.
- Produces: `E0600` now also names the *witness* — the shape no arm covers, e.g. `Point { x: _, y: _ }` — not only the variant name.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn an_or_pattern_covers_every_alternative() {
    // switch c { Color::Red | Color::Green => {}, Color::Blue => {} } is exhaustive
}

#[test]
fn a_range_pattern_covers_only_its_interval() {
    // switch n { 0..=9 => {}, _ => {} } is exhaustive; without the default, 10 is missing
}

#[test]
fn a_guarded_arm_does_not_cover_its_pattern() {
    // switch c { Color::Red if x > 0 => {}, Color::Green => {}, Color::Blue => {} } reports Red
}

#[test]
fn nested_payloads_are_checked_column_by_column() {
    // enum E { A(Option[int]), B } with arms A(Some(1)) and B reports A(None) and A(Some(_))
}
```

- [ ] **Step 2: Run and watch them fail** — `cargo test -p yashy-hir exhaustive` → FAIL.
- [ ] **Step 3: Implement** — the standard usefulness algorithm: `U(matrix, vector)` returns whether the vector is useful; a switch is exhaustive when the wildcard row is not useful against the arm matrix. Guards are handled by treating a guarded arm as *not contributing* to the matrix for the exhaustiveness question (it contributes to reachability, which is Task 4's job, with a comment saying so). Range patterns are expanded per-variant only for enums whose payload is `int`/`char` and only for the ranges present, so the matrix stays finite; a range over a non-enumerable type is rejected earlier by `E0602`.
- [ ] **Step 4: Run** `cargo test -p yashy-hir -p yashy-verify -p yashy-driver --test e2e` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(verify): exhaustiveness over or-, range- and nested patterns"`

---

### Task 4: Unreachable arms

**Files:**
- Modify: `crates/yashy-hir/src/exhaustive.rs`, `crates/yashy-verify/src/pass/exhaustive.rs`
- Modify: `crates/yashy-common/src/codes.rs` (`UNREACHABLE_ARM = "E0601"`), create `docs/errors/E0601.md`, modify `docs/errors.md`

**Interfaces:**
- Produces: `E0601` on an arm no value can reach, with the earlier arm that covers it named in the label.

- [ ] **Step 1: Write the failing tests** — `case _ => {}` followed by `case Red => {}` reports the second; `case 0..=10` followed by `case 5` reports `5`; a guarded arm is reachable even when an earlier arm covers its pattern (the guard may fail, so the arm is not dead).
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — walk arms in order, asking `usefulness` against the matrix of the arms before it; a `false` answer is `E0601`. This is a warning-grade mistake in every other language and an error here: dead code in a `switch` is almost always a copy-paste bug in the pattern, and the verifier's job is to say so.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(verify): reject unreachable switch arms (E0601)"`

---

### Task 5: Ownership through patterns

**Files:**
- Modify: `crates/yashy-verify/src/pass/ownership.rs` (pattern binding sites)
- Modify: `crates/yashy-verify/src/domain.rs` (per-alternative state join)
- Modify: `codes.rs` (`PARTIAL_MOVE = "E1002"`), create `docs/errors/E1002.md`

**Interfaces:**
- Produces: `E1002` when an or-pattern's alternatives do not bind the same linear names, and when one alternative moves a value the other does not.
- Produces: `E1001` (defer) unchanged; a `defer` inside a pattern arm still releases at scope exit.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn an_or_pattern_that_binds_a_linear_name_on_one_side_is_rejected() {
    // case Red | Green(s) => {} with s: *mut i64 is E1002
}

#[test]
fn an_or_pattern_that_binds_the_same_name_on_both_sides_is_accepted() {
    // case Green(s) | Blue(s) => { free(s); }
}

#[test]
fn a_move_in_one_alternative_and_not_the_other_is_rejected() {
    // case Some(s) => { free(s); } | None => {} is E1002 at the merge
}
```

- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — bind each alternative into a scratch scope, then require identical linear binding sets and identical post-state per name; disagreement is `E1002` naming the name and the two alternatives' states. Non-linear bindings may differ freely, because they own nothing.
- [ ] **Step 4: Run** `cargo test -p yashy-verify` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(verify): ownership through or- and nested patterns (E1002)"`

---

### Task 6: Named and default arguments

**Files:**
- Modify: `crates/yashy-parser/src/ast.rs` (`CallArg { name: Option<String>, value: Expr, span }`; `CallExpr.args: Vec<CallArg>`; `Param.default: Option<Expr>`)
- Modify: `crates/yashy-parser/src/{exprs.rs,items.rs}`, `crates/yashy-sema/src/check_expr.rs` (resolution to positional), `crates/yashy-fmt/src/lib.rs`
- Modify: `codes.rs` (`UNKNOWN_NAMED_ARGUMENT = "E0701"`, `DUPLICATE_ARGUMENT = "E0702"`), create the two pages

**Interfaces:**
- Produces: `f(1, name: "x")`, `f(name: "x", 1)` (positional after named is `E0701`), and `fn f(a: int, b: int = 2) -> int`.
- Produces: the HIR call is **positional**; defaults are inserted at resolution, so no later pass learns about named arguments.
- Produces: `E0701` unknown name or positional-after-named; `E0702` a parameter given twice; **`E0102`** (the code phase 5 introduced for field defaults) for a default that is not a literal or a `const` name. One rule, one code: `is_const_default` is a single function that both a field declaration and a parameter declaration call, so the two default sites cannot drift apart.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn named_arguments_resolve_to_parameter_positions() {
    // f(b: 2, a: 1) produces the same HIR call as f(1, 2)
}

#[test]
fn a_default_fills_a_missing_argument() {
    // fn f(a: int, b: int = 2); f(1) calls f(1, 2) at the HIR level
}

#[test]
fn a_named_argument_that_names_no_parameter_is_rejected() {
    // f(c: 1) is E0701 with the parameter list in the note
}

#[test]
fn a_positional_argument_after_a_named_one_is_rejected() {
    // f(b: 2, 1) is E0701
}

#[test]
fn a_default_that_calls_a_function_is_rejected() {
    // fn f(a: int = g()) is E0102, the same code as a field default that calls one
}
```

- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — resolution builds a `Vec<Option<Expr>>` of parameter slots, fills from named and positional in source order (evaluating each expression once, left to right, into a temporary in the HIR so a default and an explicit argument keep their relative order), then checks every slot is filled or has a default. `E0702` is checked by slot, not by name, so `f(a: 1, a: 2)` and `f(1, a: 2)` both report it.
- [ ] **Step 4: Run** `cargo test -p yashy-parser -p yashy-sema -p yashy-fmt -p yashy-driver --test e2e` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: named and default arguments resolved at the call site"`

---

### Task 7: Named arguments meet contracts, effects and traits

**Files:**
- Modify: `crates/yashy-verify/src/pass/contracts.rs` (parameter-name lookup)
- Modify: `crates/yashy-verify/src/pass/effects.rs` (a default is evaluated at the callee's effect level)
- Modify: `crates/yashy-driver/tests/verify_corpus/{pattern_partial_move.ya,unreachable_arm.ya,named_arg_unknown.ya}`

**Interfaces:**
- Produces: a contract may name any parameter, including one the caller passed by name; the diagnostic names the parameter, never the position.
- Produces: `E1300` when a default expression is `io` even though the function is `pure` (the default runs inside the callee, so its effect is the callee's).

- [ ] **Step 1: Write the failing tests** — a contract referring to `b` when the caller wrote `f(b: 1, a: 2)` still proves; a `pure fn` with a default calling `println` is `E1300`; the three seeded bugs land in the corpus with their expected code sets.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — because resolution already produced positional HIR, contract checking needs no change beyond confirming parameter names survive on `HirFn.params`; the effect change is one line in the default-insertion path.
- [ ] **Step 4: Run the zero-false-positive loop** from phase 1b Task 7 → no new line.
- [ ] **Step 5: Commit** — `git commit -m "feat(verify): contracts and effects across named and default arguments"`

---

### Task 8: Docs, tutorial and gate

**Files:** `docs/tutorial/05-patterns.md` (rewritten with guards, or, ranges, nested), `docs/tutorial/11-functions.md` (named/default section), `docs/errors/{E0102,E0601,E0602,E0701,E0702,E1002}.md`, `AGENTS.md` (identity section: patterns are irrefutable in binding position).

- [ ] **Step 1: Write the chapters** with runnable blocks; the tutorial harness compiles every block.
- [ ] **Step 2: Run the gate table on both OSes.**
- [ ] **Step 3: Commit and record** — `git commit -m "docs: patterns and named arguments, and close phase 7"`; update the index status and the ledger.

---

## Gate summary

| Gate | Command | Passes when |
|---|---|---|
| Exhaustiveness still proven | `cargo test -p yashy-hir -p yashy-verify exhaustive` | or/range/guard/nested cases pass; a guarded arm never counts as covering |
| Every pattern form has ownership rules | `cargo test -p yashy-verify` | partial-move corpus 100% caught |
| Named arguments are positional in the HIR | `cargo test -p yashy-sema` | two spellings produce equal HIR |
| No evaluation reordering | `cargo test -p yashy-driver --test e2e` | side-effect order is source order |
| Formatter | `cargo test -p yashy-fmt` | every new form round-trips |
| No false positives | the phase 1b loop | no `FALSE POSITIVE` line |
| No regression | `cargo test --workspace` | green on Windows and Linux |
