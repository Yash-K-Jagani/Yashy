# Phase 8 — Traits, OOP and Iterators: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a function state what it needs from a type (`T: Show`), let a type state what it can do (`impl Show for Point`), let a type offer constructors and namespaced functions (`Point::origin()`), and make `for x in xs` a protocol rather than a special case — without adding vtables, runtime type information, inheritance, or a second dispatch mechanism the verifier cannot see through.

**Architecture:** A trait is a named set of method signatures; an impl is a checked bundle of bodies; a bound is a constraint on an existing type parameter. Dispatch is **static only** in v1: a call to a trait method resolves to one concrete `impl` at type-check time and is monomorphised exactly like today's generic functions (`mangled_generic_name` in `check_expr.rs` already does this). That choice is what makes "dispatch proven safe" a fact rather than an aspiration — there is no dynamic target, so there is nothing the verifier has to trust. The iterator protocol is one trait, `Iterator[T]`, and `for x in xs` lowers to it in the HIR, so every existing `for` over an array becomes a protocol use that the verifier sees.

This phase also completes the **object model written in spec §3.1** — `object` and `impl` already parse and run (`objects.ya`, `methods.ya`), so what remains is the parts the model promises and the language does not yet have: associated functions called through a type path, visibility applied to fields and methods rather than only to items, and the negative rules (no inheritance, no `Drop`, no operator overloading) enforced by tests rather than by prose. The phase is not done when the model is documented; it is done when a conformance program exercises every rule and a corpus proves the excluded forms do not compile.

**Tech Stack:** `yashy-parser` (`Item::Fn.type_params`, `Item::Struct`/`Item::Enum` type parameters, `Item::Impl`), `yashy-sema/src/check_expr.rs` (`GenericFn`, `mangled_generic_name`, `TypeScheme` for generic types, method resolution), `yashy-hir` (`HirStmt::ForIn`), `yashy-codegen` (`stmt_gen.rs`), `yashy-verify`, `yashy-lsp`.

**Spec:** `docs/superpowers/specs/2026-10-27-yashy-master-design.md` §3.1 (the object model), §8 row 8. Index: `docs/superpowers/plans/2026-10-27-yashy-master-plan.md`. Predecessor: `2026-10-27-phase-07-patterns-named-args.md`.

## Revision note (2026-11-07)

Written against today's code, not the code this plan was drafted from. Findings
that changed task detail:

- **No trait syntax exists.** `trait` is not a keyword (`KEYWORDS` is 34) and
  `Item` has no `Trait` variant. Task 1 is greenfield lexer, parser and
  formatter work.
- **`impl` exists, but has no trait name.** `Item::Impl` is `{ type_name,
  methods, span }`; methods are stored in one `"Type.method"` table and emitted
  as `HirFn` names of the same spelling (`check_impl_methods`). A trait impl
  therefore needs its own table and symbol spelling, or it will collide with an
  inherent method of the same name.
- **Associated functions already exist with dot spelling.** `Counter.new(3)`
  resolves through `check_static_method`, and `p.get()` on a static function is
  already rejected. Task 6 is a second, spec spelling (`Type::name`) over the
  same resolution path, plus coding the two existing uncoded diagnostics as
  `E1506`/`E1507`.
- **Type parameters are function-only and unbounded.** `Item::Fn.type_params`
  is `Vec<(String, Span)>`; there is no `bounds`. Task 3 changes the shape to
  `TypeParam { name, bounds }` and follows every construction site.
- **`TypeExpr::Generic` does not exist.** `parse_type_expr` recognises
  `Option`, `Result`, `Channel`, `Mutex`, slices and views, and deliberately
  rejects `Name[` for every other name because "Yashy has no generics"
  (`crates/yashy-parser/src/types.rs`). Task 4 is parser work as well as sema
  and codegen work; the `TypeExpr` variant is the first step.
- **Field visibility already holds; method visibility does not.**
  `FieldDef.public` is parsed, `StructSig.field_public` records it, and
  `check_field_visibility` enforces it across modules. `pub fn` inside an
  `impl`/`object` is parsed as `Item::Public`, but `unwrap_public` discards the
  bit before `method_signature`, so Task 7 is methods plus the missing
  struct-literal check.
- **`for` is array-only.** `Stmt::ForIn` requires a binding and lowers to
  `HirStmt::ForIn` (arrays/slices/views) or `ForRange` for the indexed form.
  `while let` already lowers to `ForC` + `IfLet`, which is the shape the
  iterator protocol will reuse. There is no `iter.ya`.
- **Both documented spellings of a loop body must stay identical.** The plan's
  "every existing `for` becomes a protocol use" is narrowed: built-in
  iterables keep their proven lowering, and the protocol is how a user type
  becomes iterable. This is recorded under Task 8.
- **The tutorial ends at `14-modules.md`.** The traits chapter is
  `15-traits.md`; the chapter list is discovered from the directory, so the
  number is documentation only.
- **Windows linker flake, not a code failure.** `link.exe` intermittently
  fails with `LNK1104` when cargo links many test binaries; `-j1` and a retry
  clear it. Use `-j1` for workspace runs and do not treat that error as a test
  failure.

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
| Associated functions `Type::name(..)` | The behaviour exists with dot spelling (`Counter.new(..)`); no `::` token | Task 6 |
| Visibility applies to fields and methods, not only items | Items and fields hold (phase 6); methods do not | Task 7 |

## Written cap (this phase)

**Static dispatch only. No `dyn Trait`, no trait objects, no vtable, no runtime type information.** Dynamic dispatch is not cut for time; it is excluded because it would put a call target the verifier cannot name behind every trait call, and the verifier's soundness argument is the whole product. A future dynamic-dispatch design needs its own spec section, its own verifier story and its own phase. Recording it here is the exception record the governance section requires.

**Generic functions, structs and enums — but not generic traits.** `trait From[T]` is out of scope: a generic trait multiplies the coherence question (one impl per argument tuple, and two impls can overlap without either being a duplicate) and coherence is the thing this phase exists to prove. `Vec[T]` and `Map[K, V]` remain built-in types with monomorphic runtime representations.

The **one** exception is the standard library's `Iterator[T]`. It is
compiler-known, declared in `crates/yashy-stdlib/yashy/iter.ya`, and its
argument is read from the `impl`'s `next` return type; it is not a general
feature. A trait declaration with type parameters outside the stdlib is
rejected as `E1511` and the message says that user generic traits are not in
1.0. This keeps the iterator protocol expressible without reopening the
coherence question the cap closes.

## Review Focus

1. **An impl must implement exactly the declared methods.** A missing method is `E1502`; an extra method is `E1502` too (it would be dead code the trait cannot call). A method with the wrong receiver kind (`self` vs `self: *mut T`) is `E1502` with the expected signature printed.
2. **Coherence is per (trait, type) pair, checked before bodies.** Two impls of the same trait for the same type is `E1501`, and it must be detected before any body is checked so the diagnostic is not a cascade.
3. **A bound is checked at the call site, not inside the generic body.** Inside `fn f[T: Show](x: T)`, only `Show`'s methods are available on `x`; calling anything else is `E1503` naming the missing bound. This is the rule that keeps generic bodies analysable without knowing their instantiations.
4. **A trait call's effect is the effect of the one impl it resolves to.** Static dispatch plus per-instantiation re-checking means the emitted call names `Type.Trait.method`, so the existing effect pass sees that impl's body and nothing else. The original draft required a join over every impl because it assumed the generic body was checked once; it is not — `instantiate_generics` re-checks each substitution. A `pure` caller of an `io` impl is `E1300`; a `pure` trait (every impl must be pure) is enforced at each impl body, not as a join at the call site.
5. **`for x in xs` must produce identical results to today for arrays and slices.** The lowering changes; the observable behaviour may not. The existing `for` tests are the regression proof.
6. **The excluded parts of the object model fail to compile, and a test says so.** `object A : B` (inheritance), `impl Drop for T` (a release hook), `trait T { x: i64 }` (state in a trait) and `a + b` on a user type that has an `add` method (operator overloading) must each be rejected with a diagnostic that names the excluded feature and the replacement (`defer`/`.free()`, a named method, an explicit function). Prose in the spec is not enforcement; the corpus is.
7. **An associated function is not a method.** `Point::origin()` has no receiver and cannot touch `self`; `Point::new(x)` returns a value and does not mutate an existing one. Allowing `self` in an associated function would make the type path a second way to spell a method call, and the two would drift in resolution order. A body that names `self` in a function with no receiver is `E0001` at that `self` — an ordinary unresolved name, not a special diagnostic — and the fix note says to add a receiver if a method was meant.
8. **A generic type is a scheme that is instantiated, not a template that is copied.** `Pair[i64]` and `Pair[string]` are two types, interned once each, and `Pair[i64]` written at ten sites is still one type with one layout and one emitted `type` definition. The failure mode this guards against is a naive substitution that emits a fresh struct per *use site*, which inflates the binary and defeats the monomorphisation budget in Task 10. The test asserts one `type` per argument tuple, not one per use.
9. **A generic type's bound is checked where it is instantiated, which includes inside another generic body.** `fn first[T](p: Pair[T]) -> T` is only legal if `Pair`'s own bound is satisfiable for every `T` the function can be called with, so the check is on the function's bound set, not deferred to the call. Deferring it would let a monomorphisation fail at codegen time, where there is no diagnostic span for it.

---

### Task 1: Trait declarations

**Files:**
- Modify: `crates/yashy-lexer/src/token.rs` (`trait` keyword; `KEYWORDS` grows from 34 to 35)
- Modify: `crates/yashy-parser/src/{ast.rs,items.rs}` (`Item::Trait { name, type_params, methods: Vec<TraitMethod>, span }`)
- Modify: `crates/yashy-fmt/src/lib.rs`
- Modify: `editors/vim/yashy.vim`, `editors/emacs/yashy.el`, `editors/vscode/syntaxes/yashy.tmLanguage.json` (the LSP grammar test requires every keyword in `KEYWORDS` to be listed in all three)
- Create: `crates/yashy-parser/tests/traits.rs`

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
- Produces: `trait Show { fn show(self) -> string; }` parses; a method with a body inside a trait is a parse error ("a trait method has no body"); a `pub fn` inside a trait is a parse error ("a trait method takes the trait's visibility").
- Produces: `trait Iterator[T] { fn next(self: *mut Self) -> Option[T]; }` parses. Type parameters on traits are syntax-only in this task; Task 2 rejects a user declaration as `E1511` and Task 8 reserves the form for the stdlib.

- [x] **Step 1: Write the failing tests** — a trait with one method parses; a trait method with a body is rejected; `pub trait` works through the existing `Item::Public` wrapper; a `pub` method inside a trait is rejected; the formatter round-trips a multi-method trait.
- [x] **Step 2: Watch them fail.**
- [x] **Step 3: Implement** — the declaration grammar is `Item::Fn`'s without a body; `type_params` uses the existing `Vec<(String, Span)>` shape (Task 3 gives it bounds). Receiver detection stays in sema: a first parameter written `self` is the receiver. The `trait` keyword is added to `KEYWORDS`, `TokenKind`, the keyword table and `Naming`.
- [x] **Step 4: Run** `cargo test -p yashy-parser -p yashy-fmt` → PASS.
- [x] **Step 5: Commit** — `git commit -m "feat: parse trait declarations"`

---

### Task 2: Impls for traits, coherence and completeness

**Files:**
- Modify: `crates/yashy-parser/src/{ast.rs,items.rs}` (`Item::Impl.trait_name: Option<String>`; `impl Trait for Type { .. }`)
- Modify: `crates/yashy-sema/src/check_expr.rs` (impl collection and checking)
- Modify: `crates/yashy-common/src/codes.rs` (`UNKNOWN_TRAIT = "E1500"`, `CONFLICTING_IMPL = "E1501"`, `INCOMPLETE_IMPL = "E1502"`, `GENERIC_TRAIT = "E1511"`)
- Create: `docs/errors/{E1500,E1501,E1502,E1511}.md`, modify `docs/errors.md`

**Interfaces:**
- Produces: `impl Show for Point { fn show(self) -> string { .. } }`; today's `impl Point { .. }` stays valid and means an inherent impl.
- Produces: a `TraitTable` in sema: `BTreeMap<(String /*trait*/, String /*type*/), TraitImpl>`, built before any body is checked. `TraitImpl` holds the declared trait's method signatures and the impl's `HirFn` symbol per method.
- Produces: one symbol spelling. An inherent method stays `Type.method`; a trait impl method is `Type.Trait.method`. Both are one `HirFn` and one call target, so there is no second dispatch table at codegen and the existing ownership/effect passes see a concrete callee.
- Produces: a user trait declaration with `type_params` is `E1511`; the stdlib's `Iterator` is allowed only when the declaration's file is `iter.ya` in the stdlib (the driver marks stdlib files on the `FileId`).
- Produces: a method declaration inside a trait with no receiver is an associated function; the trait table records `has_self` and `receiver_is_pointer` exactly as `method_signature` does for an inherent method.

- [x] **Step 1: Write the failing tests**

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

#[test]
fn a_user_declared_generic_trait_is_e1511() {
    // `trait Pair[T] { .. }` outside the stdlib
}
```

- [x] **Step 2: Watch them fail.**
- [x] **Step 3: Implement** — collect impls first, reject duplicates (`E1501`), then for each impl compare the method set and receiver kinds against the trait (`E1502`), then check bodies. An impl of a trait that does not exist is `E1500` with the nearest trait names suggested. Trait methods are registered in a table keyed by `(trait, type, method)`; the call path (Task 5) resolves a value receiver to `Type.method` first and to `Type.Trait.method` second.
- [x] **Step 4: Run** `cargo test -p yashy-sema` → PASS.
- [x] **Step 5: Commit** — `git commit -m "feat: check trait impls for coherence and completeness (E1500-E1502, E1511)"`

---

### Task 3: Bounds on type parameters

**Files:**
- Modify: `crates/yashy-parser/src/ast.rs` (`pub struct TypeParam { pub name: String, pub bounds: Vec<String>, pub span: Span }`; `Item::Fn.type_params: Vec<TypeParam>`, and the same on `Item::Trait` and, from Task 4, on `Item::Struct`/`Item::Enum`)
- Modify: `crates/yashy-parser/src/items.rs` (`fn f[T: Show, U](..)`, `trait Iterator[T]`)
- Modify: `crates/yashy-sema/src/check_expr.rs` (`GenericFn`, `mangled_generic_name`; each monomorphised body resolves a trait call to the concrete `Type.Trait.method`)
- Modify: `codes.rs` (`UNSATISFIED_BOUND = "E1503"`), create `docs/errors/E1503.md`

**Interfaces:**
- Produces: `fn show_all[T: Show]([]T xs) -> string`; inside the body, `x.show()` resolves through the bound, not through a concrete type.
- Produces: `E1503` at the call site when the argument type has no impl for a bound, naming the type, the trait and the file that would need the impl.
- Produces: each instantiation is re-checked with the substitution in force (that is what `instantiate_generics` does today), so a trait call inside a generic body is lowered to the concrete impl symbol before codegen. The effect and ownership passes therefore see one call target per instantiation, not a symbolic trait call.

- [x] **Step 1: Write the failing tests** — a bound is enforced at the call site; a generic body may call only bound methods; an unbounded `T` cannot be printed; two instantiations produce two symbols, and a trait call in each names that type's impl; a bound on an unknown trait is `E1500`.
- [x] **Step 2: Watch them fail.**
- [x] **Step 3: Implement** — `TypeParam` replaces the tuple everywhere; `GenericFn` gains `bounds: Vec<String>` per parameter; instantiation checks each argument type against each bound's `TraitTable` entry; inside the body, method lookup on a type parameter consults the bound set only. When `instantiate_generics` re-checks a body, `type_subst` turns the bound method's receiver into the concrete type inside that bound, and the call is emitted as that impl's symbol.
- [x] **Step 4: Run** `cargo test -p yashy-sema` → PASS.
- [x] **Step 5: Commit** — `git commit -m "feat: generic bounds checked at the call site (E1503)"`

---

### Task 4: Generic structs and enums

**Files:**
- Modify: `crates/yashy-parser/src/ast.rs` (`Item::Struct` and `Item::Enum` gain `type_params: Vec<TypeParam>`; new `TypeExpr::Generic { name, args, span }`)
- Modify: `crates/yashy-parser/src/types.rs` (`Name [ type (, type)* ]` becomes `TypeExpr::Generic`; the built-in `Option`/`Result`/`Channel`/`Mutex` spellings keep their dedicated variants)
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

- [x] **Step 1: Write the failing tests** — `Pair[i64]` and `Entry[string, i64]` construct, read their fields and print; `Maybe[T]` matches exhaustively for each instantiation and a missing arm is still `E0600`; two instantiations of one scheme emit two types and one instantiation emits one; `Pair[i64, i64]` is `E1509` with both arities; a bare `Pair` is `E1509`; `struct Wrapper[T] { n: i64 }` is `E1510`; `Pair[T: Show]` with an unbounded argument type is `E1503`; `Pair[i64]` and `Pair[string]` do not unify; a generic type used inside a generic function (`fn first[T](p: Pair[T]) -> T`) monomorphises once per `T`; the formatter round-trips the declaration and the use.
- [x] **Step 2: Watch them fail.**
- [x] **Step 3: Implement** — `parse_type_expr` grows the `Name[type, ..]` case and emits `TypeExpr::Generic` for every name except the built-ins that already have a dedicated variant; `Struct`/`Enum` gain `type_params`; the item pass records a `TypeScheme` instead of a concrete type when the list is non-empty; `resolve_type` on `TypeExpr::Generic` looks the scheme up, checks arity (`E1509`), substitutes the arguments into the fields, and interns the result under `mangled_generic_name`; a non-generic name with type arguments is `E1509` too, and a generic name with none is `E1509`. The unused-parameter check (`E1510`) is one walk over the field types collecting the parameters actually mentioned. Codegen emits from the interned table, so a type used twice is declared once.
- [x] **Step 4: Run** `cargo test -p yashy-parser -p yashy-sema -p yashy-codegen -p yashy-fmt -p yashy-driver --test e2e --test verify` → PASS.
- [x] **Step 5: Commit** — `git commit -m "feat: generic structs and enums, one monomorphisation per argument tuple (E1509, E1510)"`

---

### Task 5: Method resolution order

**Files:**
- Modify: `crates/yashy-sema/src/check_expr.rs` (method call path)
- Modify: `codes.rs` (`AMBIGUOUS_METHOD = "E1504"`), create `docs/errors/E1504.md`

**Interfaces:**
- Produces: resolution order — inherent impl, then the single trait impl in scope, then a bound on the receiver's type parameter. Two candidates at the same level is `E1504` with both traits named. The note names the qualified spelling `Trait::method(x)`, which becomes resolvable in Task 6; Task 5 itself only reports the ambiguity, so it does not depend on `::` existing yet.
- Keeps: inherent dispatch as today's `"Type.method"` lookup; a trait candidate comes from `TraitTable[(trait, type)]`, and its call symbol is `Type.Trait.method`. There is one method table per kind, not a merged one, so a trait impl cannot silently replace an inherent method.
- Produces: `Show::show(x)` as the qualified spelling in Task 6: an associated-call form whose resolution is restricted to the named trait, not a new dispatch path.

- [x] **Step 1: Write the failing tests** — an inherent method shadows a trait method silently; two traits in scope both providing `show` is `E1504`; a trait method on a value whose type has no impl is `E1503`. The qualified call is tested in Task 6.
- [x] **Step 2: Watch them fail.**
- [x] **Step 3: Implement** — collect candidates, filter by arity and receiver, then apply the order above. Ambiguity is reported, never resolved by import order.
- [x] **Step 4: Run** `cargo test -p yashy-sema -p yashy-driver --test e2e` → PASS.
- [x] **Step 5: Commit** — `git commit -m "feat: resolve methods inherent, then trait, then bound (E1504)"`

---

### Task 6: Associated functions and type paths

**Files:**
- Modify: `crates/yashy-lexer/src/token.rs` (`ColonColon`; `KEYWORDS` stays at 35)
- Modify: `crates/yashy-parser/src/{ast.rs,exprs.rs}` (`Expr::TypePath { type_name, name, span }`)
- Modify: `crates/yashy-sema/src/check_expr.rs` (resolution through the impl table)
- Modify: `crates/yashy-codegen/src/{expr_gen.rs,module.rs}` (mangled symbol, no receiver)
- Modify: `crates/yashy-common/src/codes.rs` (`METHOD_THROUGH_TYPE = "E1506"`, `ASSOC_THROUGH_VALUE = "E1507"`, `UNKNOWN_ASSOCIATED_FN = "E1508"`), create `docs/errors/{E1506,E1507,E1508}.md`, modify `docs/errors.md`
- Modify: `crates/yashy-fmt/src/lib.rs`
- Create: `crates/yashy-driver/tests/programs/associated_fns.ya`

**Interfaces:**
- Produces: `impl Point { fn origin() -> Point { .. } fn new(i64 x, i64 y) -> Point { .. } }`, called as `Point::origin()` and `Point::new(1, 2)`. A function in an `impl` or `object` block whose first parameter is not `self` is an **associated function**, not a method.
- Keeps: the existing `Point.new(1, 2)` spelling, which already resolves through `check_static_method`. Both spellings lower to the same `HirExpr::Call` and the same symbol; there is one implementation and two spellings, not two dispatch paths.
- Produces: the symbol is mangled from `(type, name)` — `Point::new` and a free `fn new` coexist, and so do `Point::new` and `Circle::new`.
- Produces: `E1506` for `Point::get()` where `get` takes a receiver ("`get` is a method; call it on a value: `p.get()`"), `E1507` for `p.origin()` where `origin` has no receiver ("`origin` is an associated function; call it as `Point::origin()`"), `E1508` for a name the type does not declare, with the nearest names suggested. The first two replace the existing uncoded "is a method and needs a receiver" / "is a static method" diagnostics; the third replaces "no method `m` on `T`".
- Produces: `self` inside an associated function is an ordinary unresolved name (`E0001`), because there is no receiver to bind — not a panic and not a special diagnostic.
- Produces: a trait may declare an associated function (no receiver); its call is still `Type::name(..)`, resolved from the impl, so there is no `Trait::name` path in v1.

- [x] **Step 1: Write the failing tests** — `Point::new(1, 2)` parses, type-checks and runs; the dot spelling still runs and produces the same output; `Point::new` and `fn new` in the same module both resolve; `Point::get()` is `E1506`; `p.origin()` is `E1507`; `Point::missing()` is `E1508` with suggestions; `Show::show(x)` resolves the Task 5 ambiguity; `self` inside an associated function is `E0001`; a three-segment path (`a::b::c`) is a parse error (qualified names are not a module path in 1.0); the formatter round-trips `::`.
- [x] **Step 2: Watch them fail.**
- [x] **Step 3: Implement** — the lexer adds `::` as one token (not two `:`); the parser, in expression position, turns `Ident :: Ident` into `TypePath` and refuses a third segment. Resolution looks the type up in the same table method calls use, then asks the impl table for a function with that name and no receiver. Codegen emits the same mangled name the impl block already produces for a receiver-less function, so there is one symbol per `(type, name)` and no second naming scheme.
- [x] **Step 4: Run** `cargo test -p yashy-parser -p yashy-sema -p yashy-codegen -p yashy-fmt -p yashy-driver --test e2e` → PASS.
- [x] **Step 5: Commit** — `git commit -m "feat: associated functions called through a type path (E1506-E1508)"`

---

### Task 7: Visibility on fields and methods

**Files:**
- Modify: `crates/yashy-sema/src/check_expr.rs` (`MethodSig.public`; record the `Item::Public` bit that `unwrap_public` currently discards; method lookup enforces it)
- Modify: `crates/yashy-common/src/codes.rs` (no new code — reuse `PRIVATE_ITEM = "E0801"`), modify `docs/errors/E0801.md` (extend from items and fields to methods)
- Create: `crates/yashy-driver/tests/programs/visibility.ya`
- Already done: `FieldDef.public` parses, `check_field_visibility` enforces reads and struct-literal writes across modules, and `pub fn` inside `impl`/`object` parses as `Item::Public`. This task adds no visibility syntax; it stops throwing the method bit away.

**Interfaces:**
- Produces: `object Counter { pub value: i64, secret: i64, pub fn get(self) -> i64 { .. } fn bump(self: *mut Counter) { .. } }` — `pub` on a field or a method is legal, and the absence of it means private, exactly as for items.
- Produces: `E0801` when another module calls a private method, with the message naming the method, the type and the module that hides it. Field reads and struct literals already produce it.
- Keeps: field visibility enforced by `check_field_visibility`; the method case calls the same predicate rather than growing a second implementation.
- Produces: inside the declaring module, private members are reachable. Visibility is **module-scoped, not type-scoped**, so a sibling type's methods can use `secret` — the same rule phase 6 applies to items, not a second one.
- Produces: `pub` on a local `const`/`var` is a parse error — visibility applies to items and members, never to locals.

- [x] **Step 1: Write the failing tests** — a public field and a public method are reachable across modules; a private field read across modules is `E0801` naming field, type and module (already passing); a private method called across modules is `E0801`; a sibling type in the same module may read the private field; a struct literal naming a private field from another module is `E0801` (already passing); `pub const x = 1;` inside a function is a parse error; a `pub` method round-trips through the formatter.
- [x] **Step 2: Watch them fail.**
- [x] **Step 3: Implement** — `method_signature` gains a `public` bit from the `Item::Public` wrapper (the impl/object collection passes it), and the method-call paths (`check_static_method`, `check_user_method_on`) check it against `origin.file`. The struct-literal path is already covered by `check_field_visibility` at the literal site, so no change is needed there.
- [x] **Step 4: Run** `cargo test -p yashy-parser -p yashy-sema -p yashy-driver --test e2e` → PASS.
- [x] **Step 5: Commit** — `git commit -m "feat: pub on fields and methods, private members behind E0801"`

---

### Task 8: The iterator protocol

**Files:**
- Create: `crates/yashy-stdlib/yashy/iter.ya`: `trait Iterator[T] { fn next(self: *mut Self) -> Option[T]; }` (compiler-known; Task 2's `E1511` exempts this stdlib file)
- Modify: `crates/yashy-sema/src/check_expr.rs` (`Stmt::ForIn` resolves a user type to its `Iterator` impl and lowers to `ForC` + `IfLet`)
- Modify: `crates/yashy-sema/src/check_expr.rs` and `crates/yashy-codegen/src/stmt_gen.rs` only where a built-in iterable's existing lowering needs extending
- Create: `crates/yashy-driver/tests/programs/iterators.ya`, register it in `crates/yashy-driver/tests/e2e.rs`

**Interfaces:**
- Produces: `for x in xs` over an **array, slice or view with a binding** keeps today's `HirStmt::ForIn`, and `for i, x` keeps today's `ForRange`. The output must be byte-identical; the existing `for` tests are the regression proof.
- Produces: ranges (`0..n`), maps and strings gain the explicit loop lowering their current built-ins need. They are compiler-known iterables, not trait impls.
- Produces: any **other** type is iterable when it has an `Iterator[T]` impl. `for x in it` moves the value into a mutable binding and lowers to exactly `let mut it = it; while let Some(x) = it.next() { body }`, which the parser already expresses as `ForC` + `IfLet`. There is no `iter()` method in v1: the iterable is the iterator. One call shape, verified like any other loop.
- Produces: a type with no `Iterator` impl is `E1503` at the `for`, naming the type and the missing `Iterator` impl.
- Produces: because `next` takes `self: *mut Self`, the iterator binding is mutable and the ownership pass sees the receiver move. The loop body may not use the iterator again after the loop; that is ordinary ownership, not a new rule.

- [x] **Step 1: Write the failing test** — `iterators.ya` collects from an array, a range, a map and a string and asserts the exact sequence (built-in path); a user type with an `Iterator` impl is consumed by `for` and yields its sequence (protocol path); a type with no `Iterator` impl is `E1503` at the `for`.
- [x] **Step 2: Watch it fail.**
- [x] **Step 3: Implement** — resolve the iterable's type: built-ins enter the existing lowering; a user type looks up `TraitTable[(Iterator, Type)]`, requires `next` to return `Option[T]`, and synthesizes the `ForC` + `IfLet` shape with a fresh `it` binding and the concrete `Type.Iterator.next` symbol. The loop is checked through the same path as a hand-written `while let`, including label wrap and scope handling, so the verifier and codegen need no new loop node.
- [x] **Step 4: Run** `cargo test -p yashy-driver --test e2e` → PASS.
- [x] **Step 5: Commit** — `git commit -m "feat: the iterator protocol and a protocol-based for-in"`

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

- [x] **Step 1: Write the failing tests** — a filtered comprehension matches the equivalent loop's output; a comprehension over a map yields insertion order; nesting two comprehensions works; a `break` inside a comprehension is a parse error (the construct has no body to break out of).
- [x] **Step 2: Watch them fail.**
- [x] **Step 3: Implement** — parse to a dedicated AST node, lower in sema (not in codegen) so exactly one implementation exists. The `out` binding is linear and the lowered loop releases it on every path, which the verifier checks.
- [x] **Step 4: Run** → PASS.
- [x] **Step 5: Commit** — `git commit -m "feat: array and map comprehensions"`

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

- [x] **Step 1: Write the failing tests** — 64 distinct instantiations of one generic function pass, 65 is `E1505`; the same substitution from 100 call sites is one instantiation; `--timings` reports the count.
- [x] **Step 2: Watch them fail.**
- [x] **Step 3: Implement** — the cache key is `(fn name, Vec<Type>)`; the cap is checked on insert, so the diagnostic fires at the call site that exceeded it. The budget is a *correctness* device as much as a speed one: unbounded monomorphisation is how compile times become unbounded, and the language would rather reject than hang.
- [x] **Step 4: Run** `cargo test -p yashy-sema -p yashy-bench` → PASS.
- [x] **Step 5: Commit** — `git commit -m "feat: cap and count monomorphisation (E1505)"`

---

### Task 11: The verifier learns traits

**Files:**
- Modify: `crates/yashy-verify/src/pass/effects.rs` (a trait impl body's effect is the function's effect; a `pure` trait requires every impl body to be pure)
- Modify: `crates/yashy-verify/src/pass/ownership.rs` (a trait call moves its receiver exactly when the resolved impl's receiver is by value — already true once the callee is `Type.Trait.method`)
- Modify: `crates/yashy-verify/src/pass/typestate.rs` (an `Iterator` is `Open` until a `next` that returned `None`, `Closed` after it on that path)
- Create: `crates/yashy-driver/tests/verify_corpus/{trait_hidden_io.ya,iterator_after_done.ya}`

**Interfaces:**
- Produces: `E1300` when a `pure fn` calls a trait method whose resolved impl is `io`; declaring a trait `pure` and writing an `io` impl is `E1300` at the impl.
- Produces: `E1401` when an iterator is used after `next` returned `None` on the same path (a terminal state, because the protocol does not promise resurrection). The check is path-sensitive and keyed on the concrete `Type.Iterator.next` callee; a call not preceded by a `None` observation is not diagnosed.
- Produces: trait-call ownership is the *impl's* rule, looked up from the concrete callee's signature, not a default.

- [ ] **Step 1: Write the failing tests** — a `pure` caller of a trait with an `io` impl is `E1300`; declaring the trait `pure` and writing an `io` impl is `E1300` at the impl; `it.next()` after `None` is `E1401`; a `next()` after `None` on a different branch is not diagnosed (no false positive); the two seeded bugs join the corpus.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — because Task 3 emits concrete trait-call symbols per instantiation, the effect and ownership passes need no join: they already walk each `HirExpr::Call`'s callee and its `HirFn` signature. The iterator terminal state is one rule in the typestate table keyed on the callee, reusing the file/mutex state machinery. Phase 2's fixpoint runs over the whole program, so traits add edges, not a second pass.
- [ ] **Step 4: Run the zero-false-positive loop** from phase 1b Task 7 → no new line.
- [ ] **Step 5: Commit** — `git commit -m "feat(verify): effects, ownership and typestate through traits"`

---

### Task 12: The OOP conformance program

**Files:**
- Create: `crates/yashy-driver/tests/programs/oop_conformance.ya`
- Create: `crates/yashy-driver/tests/verify_corpus/{inherit.ya,drop_hook.ya,trait_state.ya,operator_overload.ya}`
- Modify: `crates/yashy-driver/tests/verify.rs` (the expected-code table gains the four entries)
- Modify: `crates/yashy-common/src/codes.rs` (`INHERITANCE = "E1512"`, `DROP_TRAIT = "E1513"`, `TRAIT_STATE = "E1514"`, `OPERATOR_OVERLOAD = "E1515"`), create `docs/errors/{E1512,E1513,E1514,E1515}.md`, modify `docs/errors.md`
- Modify: `docs/tutorial/15-traits.md`

**Interfaces:**
- Produces: one program that exercises every rule of spec §3.1 and prints a stable transcript: an `object` with a mix of public and private members, an inherent impl, a trait impl, a bound, an associated constructor, a receiver-taking mutator, a `defer` release, and an iterator consumed by `for`. The transcript is the proof that the rules compose, not just that each compiles.
- Produces: four negative programs in the phase 1b corpus, each rejected with its named code:
  - `inherit.ya` (`object A : B { .. }`) → `E1512` from the parser, naming composition or an explicit field as the replacement.
  - `drop_hook.ya` (`impl Drop for T { .. }`) → `E1513` from the trait-impl checker, naming `defer` and `.free()` as the release path.
  - `trait_state.ya` (`trait T { x: i64 }`) → `E1514` from the parser, naming a struct/object field as the replacement.
  - `operator_overload.ya` (`a + b` on a type with an `add` method) → `E1515` from the checker, naming the method call `a.add(b)` as the replacement.
- Produces: the excluded programs are part of the never-panics corpus too — they must produce a diagnostic, not a Rust panic.

- [ ] **Step 1: Write the conformance program** with the transcript in a comment, then run it and paste the actual output into the e2e test's expectation.
- [ ] **Step 2: Write the four excluded programs** and add their expected code sets to `verify.rs`; run them and confirm the codes are the ones named above.
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

**Files:** `docs/tutorial/15-traits.md` (new, "Traits and objects", covering §3.1 rule by rule and the iterator protocol), `docs/tutorial/04-structs.md` (generic structs and enums), `docs/tutorial/09-arrays.md` (comprehensions), `docs/tutorial/14-modules.md` (link to the new chapter), `docs/errors/{E0801,E1500..E1515}.md`, `AGENTS.md` (identity: static dispatch only; no inheritance; no `Drop`; associated functions are not methods; a generic type is a scheme, not a template; the written cap), `llms.txt`.

- [ ] **Step 1: Write the chapters** with runnable blocks, including the conformance program from Task 12 as the worked example.
- [ ] **Step 2: Run the gate table on both OSes.**
- [ ] **Step 3: Commit and record** — `git commit -m "docs: traits, the object model, iterators and comprehensions, and close phase 8"`.

---

## Gate summary

| Gate | Command | Passes when |
|---|---|---|
| Dispatch proven safe | `cargo test -p yashy-sema` | no dynamic target exists; every trait call names one impl |
| Coherence | `cargo test -p yashy-sema` | duplicate impl `E1501`, incomplete impl `E1502`, user generic trait `E1511` |
| Bounds | `cargo test -p yashy-sema` | unsatisfied bound `E1503`, ambiguity `E1504` |
| Generic types instantiate once | `cargo test -p yashy-codegen -p yashy-driver --test e2e` | `generic_types.ya`: `Pair[i64]` written twice emits one `type`; `Pair[i64]` and `Pair[string]` emit two |
| Generic type arity and parameters | `cargo test -p yashy-driver --test verify` | wrong arity `E1509`; unused type parameter `E1510`; a generic type inside a generic function monomorphises per argument |
| Associated functions | `cargo test -p yashy-driver --test e2e` | `associated_fns.ya`: `Type::name(..)` and `Type.name(..)` resolve; `E1506`/`E1507`/`E1508` fire on the three wrong forms |
| Member visibility | `cargo test -p yashy-driver --test e2e` | `visibility.ya`: private field/method across modules is `E0801`; sibling types may read it |
| Object model conformance | `cargo test -p yashy-driver --test verify --test e2e --test never_panics` | the conformance transcript matches; `E1512`/`E1513`/`E1514`/`E1515` reject the four excluded forms |
| Monomorphisation within budget | `cargo test -p yashy-bench` + `--timings` | count ≤ caps on the corpus; `E1505` fires at 65 |
| Iterators unchanged | `cargo test -p yashy-driver --test e2e` | every existing `for` produces identical output |
| Verifier covers traits | `cargo test -p yashy-verify` + corpus loop | trait effect/ownership/typestate rules hold; no false positive |
| No regression | `cargo test --workspace` | green on Windows and Linux |

## Outcome

- The demo is `yashy run crates/yashy-driver/tests/programs/oop_conformance.ya`;
  its output is the transcript committed with Task 12.
- A trait call resolves to one named impl, a generic type is interned once per
  argument tuple, and `for` over a user type lowers to the `while let` shape
  the verifier already checks.
- The excluded forms are diagnostics with the codes in Task 12, not prose in
  the spec.
- The written cap still holds: static dispatch only, no `dyn`/vtable/RTTI, no
  user generic traits, no inheritance, no `Drop`, no operator overloading.
