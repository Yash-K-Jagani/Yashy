# Phase 6 — Traits and Iterators: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a function state what it needs from a type (`T: Show`), let a type state what it can do (`impl Show for Point`), and make `for x in xs` a protocol rather than a special case — without adding vtables, runtime type information, or a second dispatch mechanism the verifier cannot see through.

**Architecture:** A trait is a named set of method signatures; an impl is a checked bundle of bodies; a bound is a constraint on an existing type parameter. Dispatch is **static only** in v1: a call to a trait method resolves to one concrete `impl` at type-check time and is monomorphised exactly like today's generic functions (`mangled_generic_name` in `check_expr.rs` already does this). That choice is what makes "dispatch proven safe" a fact rather than an aspiration — there is no dynamic target, so there is nothing the verifier has to trust. The iterator protocol is one trait, `Iterator[T]`, and `for x in xs` lowers to it in the HIR, so every existing `for` over an array becomes a protocol use that the verifier sees.

**Tech Stack:** `yashy-parser` (`Item::Fn.type_params`, `Item::Impl`), `yashy-sema/src/check_expr.rs` (`GenericFn`, `mangled_generic_name`, method resolution), `yashy-hir` (`HirStmt::ForIn`), `yashy-codegen` (`stmt_gen.rs`), `yashy-verify`, `yashy-lsp`.

**Spec:** `docs/superpowers/specs/2026-10-27-yashy-master-design.md` §8 row 6. Index: `docs/superpowers/plans/2026-10-27-yashy-master-plan.md`. Predecessor: `2026-10-27-phase-05-patterns-named-args.md`.

## Global Constraints

See the master index. The ones that bite here:

- No class inheritance. A trait is a set of requirements, never a base class with state.
- No operator overloading. A trait method is called by name; `+` stays `+`.
- One rule, one implementation: a trait method has one dispatch path, static.
- Every construct ships with its verifier rules in the same phase.
- Both operating systems or it is not done.

## Written cap (this phase)

**Static dispatch only. No `dyn Trait`, no trait objects, no vtable, no runtime type information.** Dynamic dispatch is not cut for time; it is excluded because it would put a call target the verifier cannot name behind every trait call, and the verifier's soundness argument is the whole product. A future dynamic-dispatch design needs its own spec section, its own verifier story and its own phase. Recording it here is the exception record the governance section requires.

**Generic functions only.** Generic structs, enums and traits-with-generic-parameters are out of scope; `Vec[T]` and `Map[K, V]` remain built-in types with monomorphic runtime representations.

## Review Focus

1. **An impl must implement exactly the declared methods.** A missing method is `E1502`; an extra method is `E1502` too (it would be dead code the trait cannot call). A method with the wrong receiver kind (`self` vs `self: *mut T`) is `E1502` with the expected signature printed.
2. **Coherence is per (trait, type) pair, checked before bodies.** Two impls of the same trait for the same type is `E1501`, and it must be detected before any body is checked so the diagnostic is not a cascade.
3. **A bound is checked at the call site, not inside the generic body.** Inside `fn f[T: Show](x: T)`, only `Show`'s methods are available on `x`; calling anything else is `E1503` naming the missing bound. This is the rule that keeps generic bodies analysable without knowing their instantiations.
4. **The effect of a trait method is the join of every impl's effect.** A `pure` caller may not call a trait method whose impls include an `io` body, because the call site does not know which impl it will get — even under static dispatch, the *generic body* was checked once. The remedy is a `pure` trait, and every impl is then required to be pure.
5. **`for x in xs` must produce identical results to today for arrays and slices.** The lowering changes; the observable behaviour may not. The existing `for` tests are the regression proof.

---

### Task 1: Trait declarations

**Files:**
- Modify: `crates/yashy-lexer/src/token.rs` (`trait` keyword; `KEYWORDS` grows from 28 to 29)
- Modify: `crates/yashy-parser/src/{ast.rs,items.rs}` (`Item::Trait { name, methods: Vec<TraitMethod>, span }`)
- Modify: `crates/yashy-fmt/src/lib.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct TraitMethod {
      pub name: String,
      pub params: Vec<Param>,      // first is the receiver when written `self`
      pub ret: Option<TypeExpr>,
      pub span: Span,
  }
  ```
- Produces: `trait Show { fn show(self) -> string; }` parses; a method with a body inside a trait is a parse error (`E1502`-adjacent, but reported by the parser as "a trait method has no body").

- [ ] **Step 1: Write the failing tests** — a trait with one method parses; a trait method with a body is rejected; `pub trait` works through the existing `Item::Public` wrapper; the formatter round-trips a multi-method trait.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — the declaration grammar is `Item::Fn`'s without a body and without `type_params` (generic traits are capped out). Receiver detection reuses the existing rule: a first parameter written `self` is the receiver.
- [ ] **Step 4: Run** `cargo test -p yashy-parser -p yashy-fmt` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: parse trait declarations"`

---

### Task 2: Impls for traits, coherence and completeness

**Files:**
- Modify: `crates/yashy-parser/src/{ast.rs,items.rs}` (`Item::Impl.trait_name: Option<String>`)
- Modify: `crates/yashy-sema/src/check_expr.rs` (impl collection and checking)
- Modify: `crates/yashy-common/src/codes.rs` (`UNKNOWN_TRAIT = "E1500"`, `CONFLICTING_IMPL = "E1501"`, `INCOMPLETE_IMPL = "E1502"`)
- Create: `docs/errors/{E1500,E1501,E1502}.md`, modify `docs/errors.md`

**Interfaces:**
- Produces: `impl Show for Point { fn show(self) -> string { .. } }`; today's `impl Point { .. }` stays valid and means an inherent impl.
- Produces: a `TraitTable` in sema: `BTreeMap<(String /*trait*/, String /*type*/), ImplId>`, built before any body is checked.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn two_impls_of_one_trait_for_one_type_conflict() {
    // E1501 at the second impl's header, not at its first method
}

#[test]
fn an_impl_missing_a_method_is_rejected() {
    // E1502 naming the missing method and its signature
}

#[test]
fn an_impl_with_a_method_the_trait_does_not_declare_is_rejected() {
    // E1502 naming the extra method
}

#[test]
fn a_method_with_the_wrong_receiver_kind_is_rejected() {
    // trait says `self`, impl says `self: *mut T` -> E1502 with both signatures
}
```

- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — collect impls first, reject duplicates (`E1501`), then for each impl compare the method set and receiver kinds against the trait (`E1502`), then check bodies. An impl of a trait that does not exist is `E1500` with the nearest trait names suggested.
- [ ] **Step 4: Run** `cargo test -p yashy-sema` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: check trait impls for coherence and completeness (E1500-E1502)"`

---

### Task 3: Bounds on type parameters

**Files:**
- Modify: `crates/yashy-parser/src/ast.rs` (`type_params: Vec<TypeParam>` with `bounds: Vec<String>`)
- Modify: `crates/yashy-parser/src/items.rs`, `crates/yashy-sema/src/check_expr.rs` (`GenericFn`, `mangled_generic_name` gains the impl's identity so `f[Point]` and `f[Circle]` are distinct symbols even when the trait method is the same)
- Modify: `codes.rs` (`UNSATISFIED_BOUND = "E1503"`), create `docs/errors/E1503.md`

**Interfaces:**
- Produces: `fn show_all[T: Show](xs: []T) -> string`; inside the body, `x.show()` resolves through the bound, not through a concrete type.
- Produces: `E1503` at the call site when the argument type has no impl for a bound, naming the type, the trait and the file that would need the impl.

- [ ] **Step 1: Write the failing tests** — a bound is enforced at the call site; a generic body may call only bound methods; an unbounded `T` cannot be printed; two instantiations produce two symbols; a bound on an unknown trait is `E1500`.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — extend `GenericFn` with `bounds: Vec<String>`; instantiation checks each argument type against each bound's `TraitTable` entry; inside the body, method lookup on a type parameter consults the bound set only.
- [ ] **Step 4: Run** `cargo test -p yashy-sema` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: generic bounds checked at the call site (E1503)"`

---

### Task 4: Method resolution order

**Files:**
- Modify: `crates/yashy-sema/src/check_expr.rs` (method call path)
- Modify: `codes.rs` (`AMBIGUOUS_METHOD = "E1504"`), create `docs/errors/E1504.md`

**Interfaces:**
- Produces: resolution order — inherent impl, then the single trait impl in scope, then a bound on the receiver's type parameter. Two candidates at the same level is `E1504` with both traits named and the qualified call (`Show::show(x)`) suggested.

- [ ] **Step 1: Write the failing tests** — an inherent method shadows a trait method silently; two traits in scope both providing `show` is `E1504`; the qualified form resolves it; a trait method on a value whose type has no impl is `E1503`.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — collect candidates, filter by arity and receiver, then apply the order above. Ambiguity is reported, never resolved by import order.
- [ ] **Step 4: Run** `cargo test -p yashy-sema -p yashy-driver --test e2e` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: resolve methods inherent, then trait, then bound (E1504)"`

---

### Task 5: The iterator protocol

**Files:**
- Modify: `crates/yashy-stdlib/yashy/iter.ya` (new): `trait Iterator[T] { fn next(self: *mut Self) -> Option[T]; }`
- Modify: `crates/yashy-sema/src/check_expr.rs` (`Stmt::ForIn` lowers to a `next` loop in the HIR)
- Modify: `crates/yashy-codegen/src/stmt_gen.rs`
- Modify: `crates/yashy-driver/tests/programs/iterators.ya`

**Interfaces:**
- Produces: `for x in xs` accepts any type with an `Iterator` impl; arrays, slices, ranges (`0..n`), maps and strings gain one.
- Produces: the HIR for `for x in xs` is `let mut it = xs.iter(); while let Some(x) = it.next() { .. }` — one shape, verified like any other loop.
- Keeps: `for` over an array with an index-free body produces identical output; `iterators.ya` asserts element order for every iterable.

- [ ] **Step 1: Write the failing test** — `iterators.ya` collects from an array, a range, a map and a string and asserts the exact sequence; a type with no `Iterator` impl is `E1503` at the `for`.
- [ ] **Step 2: Watch it fail.**
- [ ] **Step 3: Implement** — `Iterator[T]`'s `next` takes `*mut Self` because it mutates its cursor; that makes the iterator a linear value the verifier tracks, so using it after the loop is `E1002`-class and the loop itself releases nothing the caller did not ask it to. `Option[T]` already exists.
- [ ] **Step 4: Run** `cargo test -p yashy-driver --test e2e iterators` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: the iterator protocol and a protocol-based for-in"`

---

### Task 6: Comprehensions

**Files:**
- Modify: `crates/yashy-parser/src/exprs.rs` (`[expr for x in xs if cond]`, `{k: v for x in xs}`)
- Modify: `crates/yashy-sema/src/check_expr.rs` (lower to a builder loop)
- Modify: `crates/yashy-fmt/src/lib.rs`
- Modify: `crates/yashy-driver/tests/programs/iterators.ya`

**Interfaces:**
- Produces: an array comprehension lowers to `let mut out = []; for x in xs { if cond { out.push(expr); } } out`; a map comprehension to the same with `out.insert(k, v)`.
- Produces: the lowered form is the HIR, so the verifier proves the loop and the push like any other.

- [ ] **Step 1: Write the failing tests** — a filtered comprehension matches the equivalent loop's output; a comprehension over a map yields insertion order; nesting two comprehensions works; a `break` inside a comprehension is a parse error (the construct has no body to break out of).
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — parse to a dedicated AST node, lower in sema (not in codegen) so exactly one implementation exists. The `out` binding is linear and the lowered loop releases it on every path, which the verifier checks.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: array and map comprehensions"`

---

### Task 7: Monomorphisation budget

**Files:**
- Modify: `crates/yashy-sema/src/check_expr.rs` (instantiation cache and cap)
- Modify: `crates/yashy-common/src/codes.rs` (`INSTANTIATION_BUDGET = "E1505"`), create `docs/errors/E1505.md`
- Modify: `crates/yashy-driver/src/main.rs` (`--timings` row `monomorphise_ms`, counts)
- Modify: `crates/yashy-bench/src/lib.rs` (row `instantiation_count`)

**Interfaces:**
- Produces: `MAX_INSTANTIATIONS_PER_FN = 64`, `MAX_INSTANTIATIONS_TOTAL = 4096`, both named constants with the reason in a comment.
- Produces: identical substitutions reuse one symbol (already true); `--timings` reports `instantiations=N`; exceeding a cap is `E1505` naming the generic function and the two call sites that pushed it over.

- [ ] **Step 1: Write the failing tests** — 64 distinct instantiations of one generic function pass, 65 is `E1505`; the same substitution from 100 call sites is one instantiation; `--timings` reports the count.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — the cache key is `(fn name, Vec<Type>)`; the cap is checked on insert, so the diagnostic fires at the call site that exceeded it. The budget is a *correctness* device as much as a speed one: unbounded monomorphisation is how compile times become unbounded, and the language would rather reject than hang.
- [ ] **Step 4: Run** `cargo test -p yashy-sema -p yashy-bench` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: cap and count monomorphisation (E1505)"`

---

### Task 8: The verifier learns traits

**Files:**
- Modify: `crates/yashy-verify/src/pass/effects.rs` (trait method effect = join over impls; `pure` trait ⇒ every impl must be pure)
- Modify: `crates/yashy-verify/src/pass/ownership.rs` (a trait call moves its receiver exactly when the impl's receiver is by value)
- Modify: `crates/yashy-verify/src/pass/typestate.rs` (an `Iterator` is `Open` until exhausted, `Closed` after `None`)
- Create: `crates/yashy-driver/tests/verify_corpus/{trait_hidden_io.ya,iterator_after_done.ya}`

**Interfaces:**
- Produces: `E1300` when a `pure fn` calls a trait method with a non-pure impl; `E1401` when an iterator is used after `next` returned `None` (a terminal state, because the protocol does not promise resurrection).
- Produces: trait-call ownership is the *impl's* rule, looked up from `TraitTable`, not a default.

- [ ] **Step 1: Write the failing tests** — a `pure` caller of a trait with an `io` impl is `E1300`; declaring the trait `pure` and writing an `io` impl is `E1300` at the impl; `it.next()` after `None` is `E1401`; the two seeded bugs join the corpus.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — effect joins are computed after impl bodies are verified (phase 2's fixpoint already runs over the whole program, so traits add edges, not a second pass). The iterator terminal state is one rule in the typestate table, keyed on the `Iterator::next` callee.
- [ ] **Step 4: Run the zero-false-positive loop** from phase 1b Task 7 → no new line.
- [ ] **Step 5: Commit** — `git commit -m "feat(verify): effects, ownership and typestate through traits"`

---

### Task 9: Tooling

**Files:**
- Modify: `crates/yashy-lsp/src/server.rs` (completion lists trait methods with their signatures; go-to-definition on `x.show()` lands on the impl, and on the trait declaration from a bound)
- Modify: `crates/yashy-fmt/src/lib.rs` (impl and trait formatting)
- Modify: `docs/tutorial/04-structs.md`, create `docs/tutorial/14-traits.md`

- [ ] **Step 1: Write the LSP test** — completion inside an impl body offers only the trait's unimplemented methods; definition on a trait-bound call lands on the trait method, not an impl (there is no single impl to land on).
- [ ] **Step 2: Run** `cargo test -p yashy-lsp -p yashy-fmt` → PASS.
- [ ] **Step 3: Commit** — `git commit -m "feat(lsp): trait-aware completion and definition"`

---

### Task 10: Docs and gate

**Files:** `docs/tutorial/14-traits.md`, `docs/tutorial/09-arrays.md` (comprehensions), `docs/errors/{E1500..E1505}.md`, `AGENTS.md` (identity: static dispatch only; the written cap), `llms.txt`.

- [ ] **Step 1: Write the chapters** with runnable blocks.
- [ ] **Step 2: Run the gate table on both OSes.**
- [ ] **Step 3: Commit and record** — `git commit -m "docs: traits, iterators and comprehensions, and close phase 6"`.

---

## Gate summary

| Gate | Command | Passes when |
|---|---|---|
| Dispatch proven safe | `cargo test -p yashy-sema` | no dynamic target exists; every trait call names one impl |
| Coherence | `cargo test -p yashy-sema` | duplicate impl `E1501`, incomplete impl `E1502` |
| Bounds | `cargo test -p yashy-sema` | unsatisfied bound `E1503`, ambiguity `E1504` |
| Monomorphisation within budget | `cargo test -p yashy-bench` + `--timings` | count ≤ caps on the corpus; `E1505` fires at 65 |
| Iterators unchanged | `cargo test -p yashy-driver --test e2e iterators` | every existing `for` produces identical output |
| Verifier covers traits | `cargo test -p yashy-verify` + corpus loop | trait effect/ownership/typestate rules hold; no false positive |
| No regression | `cargo test --workspace` | green on Windows and Linux |
