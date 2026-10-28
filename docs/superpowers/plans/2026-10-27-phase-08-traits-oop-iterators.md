# Phase 8 — Traits, OOP and Iterators: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a function state what it needs from a type (`T: Show`), let a type state what it can do (`impl Show for Point`), let a type offer constructors and namespaced functions (`Point::origin()`), and make `for x in xs` a protocol rather than a special case — without adding vtables, runtime type information, inheritance, or a second dispatch mechanism the verifier cannot see through.

**Architecture:** A trait is a named set of method signatures; an impl is a checked bundle of bodies; a bound is a constraint on an existing type parameter. Dispatch is **static only** in v1: a call to a trait method resolves to one concrete `impl` at type-check time and is monomorphised exactly like today's generic functions (`mangled_generic_name` in `check_expr.rs` already does this). That choice is what makes "dispatch proven safe" a fact rather than an aspiration — there is no dynamic target, so there is nothing the verifier has to trust. The iterator protocol is one trait, `Iterator[T]`, and `for x in xs` lowers to it in the HIR, so every existing `for` over an array becomes a protocol use that the verifier sees.

This phase also completes the **object model written in spec §3.1** — `object` and `impl` already parse and run (`objects.ya`, `methods.ya`), so what remains is the parts the model promises and the language does not yet have: associated functions called through a type path, visibility applied to fields and methods rather than only to items, and the negative rules (no inheritance, no `Drop`, no operator overloading) enforced by tests rather than by prose. The phase is not done when the model is documented; it is done when a conformance program exercises every rule and a corpus proves the excluded forms do not compile.

**Tech Stack:** `yashy-parser` (`Item::Fn.type_params`, `Item::Struct`/`Item::Enum` type parameters, `Item::Impl`), `yashy-sema/src/check_expr.rs` (`GenericFn`, `mangled_generic_name`, `TypeScheme` for generic types, method resolution), `yashy-hir` (`HirStmt::ForIn`), `yashy-codegen` (`stmt_gen.rs`), `yashy-verify`, `yashy-lsp`.

**Spec:** `docs/superpowers/specs/2026-10-27-yashy-master-design.md` §3.1 (the object model), §8 row 8. Index: `docs/superpowers/plans/2026-10-27-yashy-master-plan.md`. Predecessor: `2026-10-27-phase-07-patterns-named-args.md`.

## Global Constraints

See the master index. The ones that bite here:

- No class inheritance. A trait is a set of requirements, never a base class with state; a type has one representation and no parent.
- No operator overloading. A trait method is called by name; `+` stays `+`.
- No `Drop` and no destructors. `defer` and `.free()` are the release path, and a trait may not declare a release hook.
- One rule, one implementation: a trait method has one dispatch path, static.
- Every construct ships with its verifier rules in the same phase.
- Both operating systems or it is not done.

## The object model this phase completes (spec §3.1)

The model is nine rules. Five hold in the code today and are proved to keep holding by Task 12's conformance program; four are built here, because the language currently has `object`/`impl`/methods but not traits, trait impls, type paths or member visibility. The list is repeated so the plan can be checked against it without opening the spec:

| Rule | State entering this phase | Built here |
|---|---|---|
| One representation per type; a struct's layout is its declaration order | Holds (`object`/`struct` lowering) | — |
| A trait is a requirement set, never state or fields | — | Task 1, Task 2 |
| `impl` is inherent or `impl Trait for Type` | Inherent holds; trait impls are new | Task 2 |
| Static dispatch only: no `dyn`, no vtable, no RTTI | Holds by construction | Task 1 (cap), Task 11 |
| Receivers are `self` (by value) or `self: *mut T` | Holds (`mutating_methods.ya`) | Task 2 (receiver-kind check) |
| No `Drop`: release is `defer` and `.free()` | Holds (no hook exists) | Task 12 (proves it stays true) |
| No operator overloading | Holds (no trait-driven operators) | Task 12 (proves it stays true) |
| Associated functions `Type::name(..)` | **Does not exist** (no `::` token) | Task 6 |
| Visibility applies to fields and methods, not only items | Items hold (phase 6); members do not | Task 7 |

## Written cap (this phase)

**Static dispatch only. No `dyn Trait`, no trait objects, no vtable, no runtime type information.** Dynamic dispatch is not cut for time; it is excluded because it would put a call target the verifier cannot name behind every trait call, and the verifier's soundness argument is the whole product. A future dynamic-dispatch design needs its own spec section, its own verifier story and its own phase. Recording it here is the exception record the governance section requires.

**Generic functions, structs and enums — but not generic traits.** `trait From[T]` is out of scope: a generic trait multiplies the coherence question (one impl per argument tuple, and two impls can overlap without either being a duplicate) and coherence is the thing this phase exists to prove. `Vec[T]` and `Map[K, V]` remain built-in types with monomorphic runtime representations.

## Review Focus

1. **An impl must implement exactly the declared methods.** A missing method is `E1502`; an extra method is `E1502` too (it would be dead code the trait cannot call). A method with the wrong receiver kind (`self` vs `self: *mut T`) is `E1502` with the expected signature printed.
2. **Coherence is per (trait, type) pair, checked before bodies.** Two impls of the same trait for the same type is `E1501`, and it must be detected before any body is checked so the diagnostic is not a cascade.
3. **A bound is checked at the call site, not inside the generic body.** Inside `fn f[T: Show](x: T)`, only `Show`'s methods are available on `x`; calling anything else is `E1503` naming the missing bound. This is the rule that keeps generic bodies analysable without knowing their instantiations.
4. **The effect of a trait method is the join of every impl's effect.** A `pure` caller may not call a trait method whose impls include an `io` body, because the call site does not know which impl it will get — even under static dispatch, the *generic body* was checked once. The remedy is a `pure` trait, and every impl is then required to be pure.
5. **`for x in xs` must produce identical results to today for arrays and slices.** The lowering changes; the observable behaviour may not. The existing `for` tests are the regression proof.
6. **The excluded parts of the object model fail to compile, and a test says so.** `object A : B` (inheritance), `impl Drop for T` (a release hook), `trait T { x: i64 }` (state in a trait) and `a + b` on a user type that has an `add` method (operator overloading) must each be rejected with a diagnostic that names the excluded feature and the replacement (`defer`/`.free()`, a named method, an explicit function). Prose in the spec is not enforcement; the corpus is.
7. **An associated function is not a method.** `Point::origin()` has no receiver and cannot touch `self`; `Point::new(x)` returns a value and does not mutate an existing one. Allowing `self` in an associated function would make the type path a second way to spell a method call, and the two would drift in resolution order. A body that names `self` in a function with no receiver is `E0001` at that `self` — an ordinary unresolved name, not a special diagnostic — and the fix note says to add a receiver if a method was meant.
8. **A generic type is a scheme that is instantiated, not a template that is copied.** `Pair[i64]` and `Pair[string]` are two types, interned once each, and `Pair[i64]` written at ten sites is still one type with one layout and one emitted `type` definition. The failure mode this guards against is a naive substitution that emits a fresh struct per *use site*, which inflates the binary and defeats the monomorphisation budget in Task 10. The test asserts one `type` per argument tuple, not one per use.
9. **A generic type's bound is checked where it is instantiated, which includes inside another generic body.** `fn first[T](p: Pair[T]) -> T` is only legal if `Pair`'s own bound is satisfiable for every `T` the function can be called with, so the check is on the function's bound set, not deferred to the call. Deferring it would let a monomorphisation fail at codegen time, where there is no diagnostic span for it.

---

### Task 1: Trait declarations

**Files:**
- Modify: `crates/yashy-lexer/src/token.rs` (`trait` keyword; `KEYWORDS` grows from 32 to 33)
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

### Task 4: Generic structs and enums

**Files:**
- Modify: `crates/yashy-parser/src/ast.rs` (`Item::Struct` and `Item::Enum` gain `type_params: Vec<(String, Span)>`, the same shape `Item::Fn` already has; `TypeExpr::Generic { name, args }` already parses in a signature, so a field type needs no new syntax)
- Modify: `crates/yashy-parser/src/items.rs` (parse `struct Pair[T] { .. }`, `enum Maybe[T] { .. }`, `struct Entry[K, V] { .. }`)
- Modify: `crates/yashy-sema/src/check_expr.rs` (a generic type is a **scheme**: `TypeScheme { params: Vec<String>, body: StructBody }` in the type table; instantiating `Pair[i64]` produces one monomorphic object type per argument tuple, named by the existing `mangled_generic_name`)
- Modify: `crates/yashy-sema/src/lib.rs` (field and variant lookup instantiates the scheme before consulting the field table)
- Modify: `crates/yashy-sema/src/ty.rs` (no new `TypeKind` variant: a monomorphised generic type *is* an object type, so every existing rule — layout, ownership, pattern matching, exhaustiveness — applies unchanged)
- Modify: `crates/yashy-codegen/src/{ir.rs,module.rs}` (one emitted type per monomorphisation, memoised on the mangled name)
- Modify: `crates/yashy-common/src/codes.rs` (`WRONG_TYPE_ARG_COUNT = "E1509"`, `UNUSED_TYPE_PARAMETER = "E1510"`), create `docs/errors/{E1509,E1510}.md`, modify `docs/errors.md`
- Modify: `crates/yashy-fmt/src/lib.rs`
- Create: `crates/yashy-driver/tests/programs/generic_types.ya`, modify `crates/yashy-driver/tests/e2e.rs`
- Create: `crates/yashy-driver/tests/verify_corpus/{generic_type_arity.ya,generic_type_unused_param.ya}`

**Interfaces:**
- Produces: `struct Pair[T] { first: T, second: T }`, `enum Maybe[T] { Nothing, Just(T) }`, `struct Entry[K, V] { key: K, value: V }`, used as `Pair[i64]`, `Maybe[string]`, `Entry[string, i64]` in a type position, a field, a parameter, a return type and a `let`.
- Produces: **one monomorphisation per distinct argument tuple**, mangled by the same `mangled_generic_name` that generic functions already use (`Pair[i64]` → `Pair$i64`). A generic type and a generic function therefore share one naming scheme, one memo and one budget — Task 10's budget covers both, and there is no second instantiation cache.
- Produces: `E1509` for the wrong number of type arguments — `Pair[i64, i64]`, `Pair[]`, or a bare `Pair` where an argument is required — naming the declared arity and the arity given.
- Produces: `E1510` for a declared type parameter that no field or variant payload uses (`struct Wrapper[T] { n: i64 }`), because the type would then have two distinct instantiations with one layout and nothing to distinguish them.
- Produces: bounds from Task 3 apply to a type parameter of a struct or enum: `struct Pair[T: Show]` checks the bound at every instantiation, and a missing impl is the existing `E1503` at the instantiation site, not a new code.
- Produces: a type that names itself in a field list (`struct Node { next: Node }`, `struct Node[T] { next: Node[T] }`) is still rejected, because a type's name is not in scope inside its own fields. That behaviour is asserted by a test and **not changed here** — recursion through a pointer is the supported form and needs no new rule.
- Produces: `Pair[i64]` and `Pair[string]` are distinct types with distinct layouts and distinct symbols; `Pair[i64]` written twice is one type, and the test asserts the IR contains one `%Pair$i64 = type` definition.

- [ ] **Step 1: Write the failing tests** — `Pair[i64]` and `Entry[string, i64]` construct, read their fields and print; `Maybe[T]` matches exhaustively for each instantiation and a missing arm is still `E0600`; two instantiations of one scheme emit two types and one instantiation emits one; `Pair[i64, i64]` is `E1509` with both arities; a bare `Pair` is `E1509`; `struct Wrapper[T] { n: i64 }` is `E1510`; `Pair[T: Show]` with an unbounded argument type is `E1503`; `Pair[i64]` and `Pair[string]` do not unify; a generic type used inside a generic function (`fn first[T](p: Pair[T]) -> T`) monomorphises once per `T`; the formatter round-trips the declaration and the use.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — `Struct`/`Enum` gain `type_params`; the item pass records a `TypeScheme` instead of a concrete type when the list is non-empty; `resolve_type` on `TypeExpr::Generic` looks the scheme up, checks arity (`E1509`), substitutes the arguments into the fields, and interns the result under `mangled_generic_name`; a non-generic name with type arguments is `E1509` too, and a generic name with none is `E1509`. The unused-parameter check (`E1510`) is one walk over the field types collecting the parameters actually mentioned. Codegen emits from the interned table, so a type used twice is declared once.
- [ ] **Step 4: Run** `cargo test -p yashy-parser -p yashy-sema -p yashy-codegen -p yashy-fmt -p yashy-driver --test e2e --test verify` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: generic structs and enums, one monomorphisation per argument tuple (E1509, E1510)"`

---

### Task 5: Method resolution order

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

### Task 6: Associated functions and type paths

**Files:**
- Modify: `crates/yashy-lexer/src/token.rs` (`ColonColon`; `KEYWORDS` stays at 33)
- Modify: `crates/yashy-parser/src/{ast.rs,exprs.rs}` (`Expr::TypePath { type_name, name, span }`)
- Modify: `crates/yashy-sema/src/check_expr.rs` (resolution through the impl table)
- Modify: `crates/yashy-codegen/src/{expr_gen.rs,module.rs}` (mangled symbol, no receiver)
- Modify: `crates/yashy-common/src/codes.rs` (`METHOD_THROUGH_TYPE = "E1506"`, `ASSOC_THROUGH_VALUE = "E1507"`, `UNKNOWN_ASSOCIATED_FN = "E1508"`), create `docs/errors/{E1506,E1507,E1508}.md`, modify `docs/errors.md`
- Modify: `crates/yashy-fmt/src/lib.rs`
- Create: `crates/yashy-driver/tests/programs/associated_fns.ya`

**Interfaces:**
- Produces: `impl Point { fn origin() -> Point { .. } fn new(x: i64, y: i64) -> Point { .. } }`, called as `Point::origin()` and `Point::new(1, 2)`. A function in an `impl` or `object` block whose first parameter is not `self` is an **associated function**, not a method.
- Produces: the symbol is mangled from `(type, name)` — `Point::new` and a free `fn new` coexist, and so do `Point::new` and `Circle::new`.
- Produces: `E1506` for `Point::get()` where `get` takes a receiver ("`get` is a method; call it on a value: `p.get()`"), `E1507` for `p.origin()` where `origin` has no receiver ("`origin` is an associated function; call it as `Point::origin()`"), `E1508` for a name the type does not declare, with the nearest names suggested.
- Produces: `self` inside an associated function is an ordinary unresolved name (`E0001`), because there is no receiver to bind — not a panic and not a special diagnostic.
- Produces: a trait may declare an associated function (no receiver); its call is still `Type::name(..)`, resolved from the impl, so there is no `Trait::name` path in v1.

- [ ] **Step 1: Write the failing tests** — `Point::new(1, 2)` parses, type-checks and runs; `Point::new` and `fn new` in the same module both resolve; `Point::get()` is `E1506`; `p.origin()` is `E1507`; `Point::missing()` is `E1508` with suggestions; `self` inside an associated function is `E0001`; a three-segment path (`a::b::c`) is a parse error naming the missing feature (namespaced paths arrive with the module system, not here); the formatter round-trips `::`.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — the lexer adds `::` as one token (not two `:`); the parser, in expression position, turns `Ident :: Ident` into `TypePath` and refuses a third segment. Resolution looks the type up in the same table method calls use, then asks the impl table for a function with that name and no receiver. Codegen emits the same mangled name the impl block already produces for a receiver-less function, so there is one symbol per `(type, name)` and no second naming scheme.
- [ ] **Step 4: Run** `cargo test -p yashy-parser -p yashy-sema -p yashy-codegen -p yashy-fmt -p yashy-driver --test e2e` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: associated functions called through a type path (E1506-E1508)"`

---

### Task 7: Visibility on fields and methods

**Files:**
- Modify: `crates/yashy-parser/src/{ast.rs,items.rs}` (`FieldDef.visibility`; `pub` accepted on a method inside `impl`/`object`)
- Modify: `crates/yashy-sema/src/{check_expr.rs,scope.rs}` (member visibility table, member lookup)
- Modify: `crates/yashy-common/src/codes.rs` (no new code — reuse `PRIVATE_ITEM = "E0801"`), modify `docs/errors/E0801.md` (extend from items to members)
- Create: `crates/yashy-driver/tests/programs/visibility.ya`

**Interfaces:**
- Produces: `object Counter { pub value: i64, secret: i64, pub fn get(self) -> i64 { .. } fn bump(self: *mut Counter) { .. } }` — `pub` on a field or a method is legal, and the absence of it means private, exactly as for items.
- Produces: `E0801` when another module reads a private field, calls a private method, or names a private field in a struct literal, with the message naming the field, the type and the module that hides it.
- Produces: inside the declaring module, private members are reachable. Visibility is **module-scoped, not type-scoped**, so a sibling type's methods can use `secret` — the same rule phase 6 applies to items, not a second one.
- Produces: a type whose fields are not all public cannot be constructed by a struct literal from another module; the diagnostic names the private fields and the `pub fn new` that would be the way in.
- Produces: `pub` on a local `const`/`var` is a parse error — visibility applies to items and members, never to locals.

- [ ] **Step 1: Write the failing tests** — a public field and a public method are reachable across modules; a private field read across modules is `E0801` naming field, type and module; a private method called across modules is `E0801`; a sibling type in the same module may read the private field; a struct literal naming a private field from another module is `E0801`; `pub const x = 1;` inside a function is a parse error; a `pub` method round-trips through the formatter.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — the member table is built where the impl table is (Task 2), so visibility and dispatch cannot disagree about which functions exist. Phase 6's cross-module check gains a member case that calls the same `is_visible` predicate; there is no second visibility implementation. The struct-literal check reuses the field list the literal already builds (phase 5's `fill_struct_fields`), so defaults, spread and privacy are decided in one place.
- [ ] **Step 4: Run** `cargo test -p yashy-parser -p yashy-sema -p yashy-driver --test e2e` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: pub on fields and methods, private members behind E0801"`

---

### Task 8: The iterator protocol

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
- [ ] **Step 4: Run** `cargo test -p yashy-driver --test e2e` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: the iterator protocol and a protocol-based for-in"`

---

### Task 9: Comprehensions

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

### Task 10: Monomorphisation budget

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

### Task 11: The verifier learns traits

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

### Task 12: The OOP conformance program

**Files:**
- Create: `crates/yashy-driver/tests/programs/oop_conformance.ya`
- Create: `crates/yashy-driver/tests/verify_corpus/{inherit.ya,drop_hook.ya,trait_state.ya,operator_overload.ya}`
- Modify: `crates/yashy-driver/tests/verify.rs` (the expected-code table gains the four entries)
- Modify: `docs/tutorial/14-traits.md`

**Interfaces:**
- Produces: one program that exercises every rule of spec §3.1 and prints a stable transcript: an `object` with a mix of public and private members, an inherent impl, a trait impl, a bound, an associated constructor, a receiver-taking mutator, a `defer` release, and an iterator consumed by `for`. The transcript is the proof that the rules compose, not just that each compiles.
- Produces: four negative programs in the phase 1b corpus, each rejected with the named code: inheritance syntax, a `Drop` hook, state in a trait, and `+` on a user type that has an `add` method.
- Produces: the excluded programs are part of the never-panics corpus too — they must produce a diagnostic, not a Rust panic.

- [ ] **Step 1: Write the conformance program** with the transcript in a comment, then run it and paste the actual output into the e2e test's expectation.
- [ ] **Step 2: Write the four excluded programs** and add their expected code sets to `verify.rs`; run them and confirm the codes are the ones named in Review Focus 6.
- [ ] **Step 3: Fix what the conformance program finds.** A rule that is only true in the spec shows up here as a compile error or a wrong transcript, and the fix belongs in the phase's earlier tasks, not in the program.
- [ ] **Step 4: Run** `cargo test -p yashy-driver --test verify --test e2e --test never_panics` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "test: the OOP conformance program and the four excluded forms"`

---

### Task 13: Tooling

**Files:**
- Modify: `crates/yashy-lsp/src/server.rs` (completion lists trait methods with their signatures; go-to-definition on `x.show()` lands on the impl, and on the trait declaration from a bound; completion after `Point::` lists associated functions and `Point::` alone never lists a receiver-taking method)
- Modify: `crates/yashy-fmt/src/lib.rs` (impl, trait and object formatting; `pub` on members)
- Modify: `docs/tutorial/04-structs.md` (point at the object model chapter)

- [ ] **Step 1: Write the LSP test** — completion inside an impl body offers only the trait's unimplemented methods; definition on a trait-bound call lands on the trait method, not an impl (there is no single impl to land on); completion after `Point::` lists `new` and `origin` but not `get`.
- [ ] **Step 2: Run** `cargo test -p yashy-lsp -p yashy-fmt` → PASS.
- [ ] **Step 3: Commit** — `git commit -m "feat(lsp): trait-aware completion and definition"`

---

### Task 14: Docs and gate

**Files:** `docs/tutorial/14-traits.md` (rewritten as "Traits and objects", covering §3.1 rule by rule), `docs/tutorial/04-structs.md` (generic structs and enums), `docs/tutorial/09-arrays.md` (comprehensions), `docs/errors/{E0801,E1500..E1510}.md`, `AGENTS.md` (identity: static dispatch only; no inheritance; no `Drop`; associated functions are not methods; a generic type is a scheme, not a template; the written cap), `llms.txt`.

- [ ] **Step 1: Write the chapters** with runnable blocks, including the conformance program from Task 12 as the worked example.
- [ ] **Step 2: Run the gate table on both OSes.**
- [ ] **Step 3: Commit and record** — `git commit -m "docs: traits, the object model, iterators and comprehensions, and close phase 8"`.

---

## Gate summary

| Gate | Command | Passes when |
|---|---|---|
| Dispatch proven safe | `cargo test -p yashy-sema` | no dynamic target exists; every trait call names one impl |
| Coherence | `cargo test -p yashy-sema` | duplicate impl `E1501`, incomplete impl `E1502` |
| Bounds | `cargo test -p yashy-sema` | unsatisfied bound `E1503`, ambiguity `E1504` |
| Generic types instantiate once | `cargo test -p yashy-codegen -p yashy-driver --test e2e` | `generic_types.ya`: `Pair[i64]` written twice emits one `type`; `Pair[i64]` and `Pair[string]` emit two |
| Generic type arity and parameters | `cargo test -p yashy-driver --test verify` | wrong arity `E1509`; unused type parameter `E1510`; a generic type inside a generic function monomorphises per argument |
| Associated functions | `cargo test -p yashy-driver --test e2e` | `associated_fns.ya`: `Type::name(..)` resolves; `E1506`/`E1507`/`E1508` fire on the three wrong forms |
| Member visibility | `cargo test -p yashy-driver --test e2e` | `visibility.ya`: private field/method across modules is `E0801`; sibling types may read it |
| Object model conformance | `cargo test -p yashy-driver --test verify --test e2e --test never_panics` | the conformance transcript matches; the four excluded forms are diagnostics with their expected codes |
| Monomorphisation within budget | `cargo test -p yashy-bench` + `--timings` | count ≤ caps on the corpus; `E1505` fires at 65 |
| Iterators unchanged | `cargo test -p yashy-driver --test e2e` | every existing `for` produces identical output |
| Verifier covers traits | `cargo test -p yashy-verify` + corpus loop | trait effect/ownership/typestate rules hold; no false positive |
| No regression | `cargo test --workspace` | green on Windows and Linux |
