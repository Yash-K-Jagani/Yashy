# Phase 4 — Language Power

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the language the pieces a library needs — generic traits,
associated types, default methods, complete patterns, built-in derives, explicit
captures, const generics for fixed buffers, compile-time evaluation — and the
pieces a systems programmer needs — typed pointers with provenance, `unsafe`
blocks, a written aliasing model, `repr` completeness, and allocation that is
visible and deterministic.

**Outcome:** Every form in §6.1 of the 2.0 design has a parser rule, a HIR
lowering (or a desugar proof), a verifier rule, a diagnostic, a test and a
BPE measurement -- except the forms recorded under [Exceptions](#exceptions).
User generic traits stay refused: `E1511` is the design's conclusion in §4.2 of
the 2.0 design record, not a 1.0 gap this phase closes. `alloc` zeroes;
`alloc_uninit` is `unsafe`. `const fn`
evaluates pure code at compile time with a fuel cap and no I/O, so tables,
fixed buffers and validated constants cost nothing at run time. The aliasing
rules in `docs/architecture.md` §5.3 become normative, and an alias corpus keeps
them honest.

**Time:** ~14 weeks.
**Depends on:** Phase 3 gate.
**Spec:** [`../specs/2026-11-15-yashy-2.0-design.md`](../specs/2026-11-15-yashy-2.0-design.md) §6.1, §6.6, §6.8, §14.

## Why this phase

The stdlib is the first consumer of every form here. `Vec[T]` needs generic
traits and an allocator interface; `Iterator` adapters need associated types and
default methods; serialization needs `Encode`/`Decode` derives; the web
framework needs `pub(crate)` and `cfg(feature)`; a database driver needs
`unsafe` and a pointer model it can explain. Building those libraries first and
retrofitting the language is the second stdlib rewrite 1.0's ordering avoided.

The pointer work is also the phase's hardest safety question. Yashy has no borrow
checker, so it cannot assume the aliasing model a borrow checker would give it.
The plan therefore writes the model down, makes `noalias` an `unsafe`/`extern`
promise only, and requires every emitted attribute to be justified by a fact the
verifier proved. That is what lets Phase 11B optimise without guessing.

## Starting point

| Fact | 1.0 value |
|---|---|
| Traits | static dispatch; no user generic traits (`E1511`); no associated types, default methods or supertraits |
| Patterns | `if let`, `while let`, guards, or/range/nested; no `let else`, slice/rest, `@`, `matches`, if-let chains |
| Control flow | `if` / `else { }` only; no `else if` |
| Derives | none; `Debug` is a library function, `Eq`/`Hash` are hand-written |
| Closures | inferred capture; no capture mode in the type |
| Visibility | `pub` only |
| Pointers | owning `*mut T`; FFI `*const T`/`*T`; no provenance model, no `unsafe` |
| Allocation | `alloc(n)` is uninitialised; no allocator interface |
| Layout | `repr(C)` only; no `size_of`/`align_of`/`offset_of` |
| Const generics | fixed arrays with literal lengths; no const parameters |
| Compile-time evaluation | `const` values and layout facts; no `const fn`, no compile-time tables, loops or validation |
| `cfg` keys | platform/debug, closed set; no features |
| `defer` | block form only |
| Operators | arithmetic, comparison, bitwise, compound assignment and `&&`/`\|\|`/`!`; no `in`, `is`, or word forms |
| Ranges | `..`/`..=` as a `for` head and a pattern; no range value |

## Deliverables

### Task 1: Generic traits, associated types, default methods, supertraits

**Files:**
- Modify: `crates/yashy-parser/src/items.rs`, `crates/yashy-parser/src/types.rs`, `crates/yashy-parser/src/ast.rs`
- Modify: `crates/yashy-sema/src/lib.rs`, `crates/yashy-sema/src/scope.rs`, `crates/yashy-sema/src/check_expr.rs`
- Modify: `crates/yashy-hir/src/ty.rs`, `crates/yashy-hir/src/hir.rs`
- Modify: `crates/yashy-verify/src/lib.rs`, `crates/yashy-codegen/src/fn_gen.rs`
- Modify: `crates/yashy-common/src/codes.rs` (`E1511` text, new codes)
- Test: `crates/yashy-sema/tests/traits.rs`, `crates/yashy-driver/tests/programs/traits/`

**Interfaces:**
- `TraitDef { name, generics, supertraits: Vec<TraitRef>, assoc_types: Vec<AssocType>, methods: Vec<TraitMethod> }`
- `AssocType { name, bounds: Vec<TraitRef>, default: Option<Type> }`
- `TraitMethod { name, has_default: bool, body: Option<HirBlock> }`
- `ImplDef { trait_ref: Option<TraitRef>, self_ty: Type, assoc_bindings: Vec<(String, Type)>, methods: Vec<HirFn> }`

- [ ] **Step 1: Write the failing test.** `trait Container[T] { type Item; fn get(self, int i) -> T; fn len(self) -> int { 0 } }`, two impls, a supertrait `trait Ordered[T]: Eq`, and a default method overridden in one impl. Assert each call is monomorphic in the IR (no vtable, no `dyn`), the default body is instantiated per impl, and an associated type is inferred at the call site.
  *Current:* the gate target `crates/yashy-sema/tests/traits.rs` exists and pins
  the trait *impl* surface (completeness, coherence, `E1511`), but the generic
  form this step writes has no parser rule: a trait takes no type parameters, no
  `type Item`, no default body and no supertrait list. The step stays open and
  the exclusion is recorded under [Exceptions](#exceptions).
- [ ] **Step 2: Keep dispatch static.** A trait method call resolves to one `HirFn` per `(impl, substitution)`; the call graph names it. There is no trait object, no vtable, no runtime lookup. A trait used only through generics is monomorphised; a trait method called on a concrete type is a direct call.
  *Current:* there is no trait object, no vtable and no runtime lookup in the
  language at all -- `impl Trait for Type` is static dispatch and every method
  is a direct `Type.Trait.method` symbol. Monomorphisation of a trait method per
  substitution cannot exist while generic traits are `E1511`, so the first
  sentence describes a form the language does not have.
- [ ] **Step 3: Bound the expansion.** The existing cap (64 instantiations per function, 4096 per program, `E1505`) covers trait methods and associated-type projections. Exceeding it is a diagnostic naming the function and the substitution that grew, never a compile that runs out of memory. The E1505 message now names the substitution (crates/yashy-sema/src/check_expr.rs, pinned by tests/instantiation_budget.rs); trait methods and associated-type projections are still not routed through this budget because generic traits are not implemented.
- [ ] **Step 4: Coherence.** An impl must be for a local type or a local trait; overlapping impls are refused with both spans; an associated type without a binding and without a default is refused; a supertrait obligation is checked at the impl and at every use.
  *Current:* coherence and completeness are pinned (`E1500`-`E1502`):
  one impl per `(trait, type)`, an impl must provide exactly the trait's
  methods with their signatures, and a trait has no associated items or
  supertraits to check today. The associated-type and supertrait halves follow
  Step 1 and are excluded with it.
- [x] **Step 5: `Iterator[T]` stays special.** It remains the compiler-known
  `for` protocol; `crates/yashy-sema/src/check_expr.rs` refuses a user trait
  that declares the name `Iterator` outside `iter.ya` with `E1511` and a message
  naming the rule, and `crates/yashy-sema/tests/traits.rs` pins the diagnostic.
  Adapter traits (`DoubleEnded`, `ExactSize`) are not added by this slice.
- [ ] **Step 6: Commit.** `git commit -m "feat(sema): static generic traits with associated types and defaults"`.
  *Current:* nothing to commit. The trait surface this task asks for is
  excluded (`E1511`), and the impl checks it does have landed with 1.0.

### Task 2: Type aliases, visibility and re-exports

**Files:**
- Modify: `crates/yashy-parser/src/items.rs` (`type`, visibility forms)
- Modify: `crates/yashy-sema/src/scope.rs`, `crates/yashy-driver/src/modules.rs`, `crates/yashy-driver/src/index.rs`
- Modify: `crates/yashy-common/src/codes.rs`
- Test: `crates/yashy-driver/tests/modules.rs`, `crates/yashy-sema/tests/visibility.rs`

- [ ] **Step 1: Write the failing test.** `type Pair[T] = (T, T);`, `pub(crate) fn`, `pub(super) fn`, `pub(in crate::db) fn`, and `pub use db::Row;`. Assert an out-of-scope use is refused with the span of the declaration and the use; assert `index --json` shows the re-export once, with the original span.
  *Current:* the `type Pair[T]` half is in place and pinned by parser and sema
  tests (`crates/yashy-parser/tests/items.rs`, `crates/yashy-sema/tests/check.rs`).
  The visibility and `pub use` half is still absent: the lexer has no
  `crate`/`super` keywords, `Item::Public` carries no level, and the index has
  no re-export dedup. The step stays open for that reason.
- [x] **Step 2: Aliases are transparent.** An alias expands at resolution and never reaches HIR as a distinct type. A recursive alias is `E15xx`; an alias that shadows a type is refused.
  *Current:* `type Name = Target;` and `type Pair[T] = (T, T)` both parse;
  plain aliases resolve their target once at declaration, generic aliases
  substitute the written arguments at each use with the checker's existing
  type-expression substitution helper, and neither emits an HIR item. Shadowing
  an existing type is refused, and a target that mentions its own alias name is
  E1519 with a catalog page and type rule (TYPE-11).
- [ ] **Step 3: Visibility is one lattice.** `private < pub(crate) < pub(super) < pub(in path) < pub` is not a total order; the checker compares the declaring module and the use site, not a rank. `pub use` re-exports the item once; two paths to one item are one symbol in `index`.
  *Current:* `pub(crate)` parses as a scoped form on `Item::Public` and, because
  this compiler's crate is its whole program, has the same reach as plain `pub`.
  `pub(super)` and `pub(in path)` are rejected with a diagnostic that names
  the limitation. The full declaring-module-vs-use-site lattice, `pub use`
  re-exports and index dedup are still owed.
- [ ] **Step 4: Commit.**

### Task 3: Patterns and control flow

**Files:**
- Modify: `crates/yashy-parser/src/stmts.rs`, `crates/yashy-parser/src/exprs.rs`
- Modify: `crates/yashy-sema/src/check_pat.rs`, `crates/yashy-hir/src/exhaustive.rs`, `crates/yashy-verify/src/ownership.rs`
- Modify: `crates/yashy-codegen/src/patterns.rs`
- Modify: `crates/yashy-fmt/src/lib.rs`
- Test: `crates/yashy-sema/tests/pats.rs`, `crates/yashy-sema/tests/desugar.rs`, `crates/yashy-driver/tests/programs/patterns/`

- [x] **Step 1: Write the failing test for `else if`.** `if a { } else if b { } else if let Some(x) = c { } else { }` parses, runs the first true branch, and formats back to the same `else if` spelling. Assert the HIR is exactly the nested `if` it desugars to.
  *Current:* `crates/yashy-parser/tests/if_let.rs` pins the parse (`if`, `if let`, nested chains and spans) and `crates/yashy-sema/tests/check.rs` pins the HIR: the outer `else` block holds one nested `If`. The token-preserving formatter prints the source spelling unchanged; it does not canonicalise a user-written `else { if ... }` to `else if`, so that half of the step is not claimed.
- [x] **Step 2: `else if` is parser sugar, not HIR.** On `else` followed by `if`, `parse_else_block` parses the nested `if`/`if let` and wraps it in a one-statement `Block` as the `else_block`. There is no new AST, HIR or verifier node. A user-written `else { if ... }` produces the same tree; semantics never change.
- [x] **Step 3: `let else`.** `let Some(x) = opt else { return; };` binds on the matching path and requires the else block to diverge (`return`, `break`, `continue`, `panic`, or a call returning `!`-like divergence). A non-diverging else is a diagnostic; a refutable `let` without `else` stays `E0100`.
  *Current:* the parser records an optional else block on `Stmt::LetPattern` and
  the checker lowers it to a new `HirStmt::LetElse`. The else block is checked
  before the pattern declares names and must always return (reusing the
  existing divergence analysis). Codegen tests the pattern, binds its names in
  the taken block, emits the else branch on its own and continues from a merge
  block, so bindings stay visible after the statement. Parser and sema tests pin
  both the accepted form and the non-diverging refusal.
- [x] **Step 4: Slice/rest and `@`.** `[first, .., last]`, `[head, ..rest]`, `x @ 0..10`, and nested combinations bind names with element types; a rest binding is a view with the source's lifetime rules; every alternative of an or-pattern binds the same names (`E1002`).
  *Current:* `Pattern::FixedArray` parses `[a, b]`, `[first, .., last]` and
  `[head, ..rest]`; the checker resolves written elements to indices in a
  `[T; N]`, binds a `..name` run as a `[]T` view, and rejects a wrong element
  count or a second `..` with E0606 (`PAT-7`). The HIR carries the resolved
  indices and the rest's `start..end`, so codegen reads bytes with a typed GEP
  per element and builds the view with the same two `insertvalue`s a range uses.
  Exhaustiveness models a fixed array as one constructor with one column per
  element, so a refutable element in a nested payload is not claimed covered.
  `@` and the or-pattern name rule were already in place. Parser, sema,
  exhaustive and end-to-end tests pin the forms; the end-to-end program and
  tutorial block could not be executed locally (see the ledger).
- [x] **Step 5: `matches` and if-let chains.** `x matches Some(_)` is an
  expression returning `bool`: the parser desugars it to a two-arm `switch`
  value (`case PAT => true`, `default => false`), so no HIR or verifier node is
  added and the pattern's bindings stay arm-local. `if let A = a && let B = b {
  } else { }` desugars to nested `Stmt::IfLet`s in source order with the same
  else block on every level, preserving short-circuit order and binding scope.
  Parser tests pin both trees and the `&& let` requirement, and a sema test
  checks a two-test chain. The keyword is in all eleven generated editor
  grammars.
- [x] **Step 6: Commit.** Landed as `0fc2c1c` (else if), `e4318fc` (if-let
  chains), `5aa6f0c` (let else), `eff0f10` (matches), `4638704` + `43ed1e3`
  (@ and fixed-array rest), with the editor grammars regenerated in the same
  series.

### Task 4: Built-in derives and explicit closure captures

**Files:**
- Modify: `crates/yashy-parser/src/items.rs` (`derive(...)` clause), `crates/yashy-sema/src/lib.rs` (derive expansion)
- Modify: `crates/yashy-hir/src/hir.rs` (closure capture mode), `crates/yashy-verify/src/ownership.rs`
- Modify: `crates/yashy-codegen/src/higher_order.rs`
- Test: `crates/yashy-sema/tests/derive.rs`, `crates/yashy-sema/tests/closures.rs`

- [ ] **Step 1: Write the failing test.** `struct Point derive(Debug, Eq, Hash) { x: int, y: int }` and an enum with `Encode`/`Decode`. Assert the generated impls are deterministic (same bytes across runs), visible in `index --json`, and refused when a field does not implement the requirement (naming the field).
  *Current:* `derive(Debug, Eq, Hash)` now synthesizes inherent `debug()`,
  `eq()` and `hash()` methods, checked by the same two passes as hand-written
  ones; the names, order and receiver shape are pinned by
  `crates/yashy-sema/tests/derive.rs`, the generated methods are named in
  `crates/yashy-driver/tests/index.rs`, and an e2e program runs all three. A
  second expansion of the same declaration produces the same typed program.
  `Encode` and `Decode` are still owed (they need Phase 7's codec interface),
  as is an enum with a derived impl.
- [ ] **Step 2: Derives are compiler-known, not macros.** Each derive is a built-in expansion over HIR with a stable order: fields in declaration order, variants in declaration order, no hash-map iteration. `derive(Debug)` renders a stable, readable form; `Encode`/`Decode` use the Phase 7 codec interface.
  *Current:* expansion synthesizes AST methods in declaration order, with no
  hash-map iteration, so `Debug` renders fields in source order, `Eq` compares
  them pairwise and `Hash` folds them with a fixed integer polynomial.
  `Encode`/`Decode`, enum derives and trait-based dispatch are still owed, so
  the step stays open.
- [ ] **Step 3: Capture modes are in the type.** A closure writes `move` / value / by-pointer captures; the closure's type records each captured binding and its mode, the verifier sees every capture, and a `move` closure cannot outlive a value it moved. A capture mode that cannot be honoured is a diagnostic, not a silent copy.
  *Current:* `move |x| ...` parses as a lambda with `CaptureMode::Move`; the
  checker marks each captured binding moved at the point the environment is
  made and records the mode on every `HirCapture`, so the verifier and code
  generator read the same fact. Plain lambdas keep `Value`. Capturing a value
  held by an enclosing closure, or a linear value without `move`, is a
  diagnostic. By-pointer captures are still not a spelling, and neither the
  outliving rule for a moved value nor the full capture-mode type is proved
  beyond the recorded mode, so the step stays open.
- [x] **Step 4: `defer expr;` and scoped release.** `defer expr;` and
  `defer a(), b();` parse into the same `Stmt::Defer` body a braced block
  produces, one `Stmt::Expr` per release in source order, so the verifier's
  release order and double-free rules are unchanged. Parser tests in
  `crates/yashy-parser/tests/defer.rs` pin the tree and a sema test pins both
  spellings. An arena's exit hook is still a `defer` over the arena; no new
  construct was added.
- [ ] **Step 5: Commit.**

### Task 5: Typed pointers, `unsafe`, and the aliasing model

**Files:**
- Modify: `crates/yashy-parser/src/types.rs`, `crates/yashy-parser/src/exprs.rs` (`unsafe` blocks, `noalias` params)
- Modify: `crates/yashy-sema/src/check_expr.rs`, `crates/yashy-hir/src/ty.rs` (`Option[*const T]` niche)
- Modify: `crates/yashy-verify/src/ownership.rs`, `crates/yashy-verify/src/lib.rs` (unsafe inventory)
- Modify: `crates/yashy-codegen/src/ffi.rs`, `crates/yashy-codegen/src/memory.rs` (attributes)
- Modify: `docs/architecture.md` §5.3 (normative), `docs/errors/`
- Create: `crates/yashy-driver/tests/alias_corpus.rs`, `crates/yashy-driver/tests/programs/unsafe/`
- Test: `crates/yashy-verify/tests/unsafe.rs`

- [ ] **Step 1: Write the failing test.** A safe function that dereferences a `*const T`; a `noalias` on a safe parameter; an `Option[*const T]` passed to and from C with null; a pointer offset past its allocation; an `unsafe` block that calls an `extern` function. Assert each refusal names the exact rule (`ALIAS-1`…`ALIAS-5`) and that the FFI round-trip preserves null as `None`.
  *Current:* `crates/yashy-driver/tests/alias_corpus.rs` now holds positive and
  negative cases for ALIAS-1, ALIAS-2, ALIAS-3 and ALIAS-5, and names ALIAS-4
  as unenforced. The `*const T` deref is pinned by E1406; the null round-trip
  and the `noalias`-on-a-safe-parameter case are still owed.
- [ ] **Step 2: Write the rules down before the code.** `docs/architecture.md` §5.3 becomes normative: owning pointers are unique by linearity; FFI pointers may alias; `noalias` is legal only on `extern` declarations and `unsafe fn` parameters; disjoint views are proved, not assumed; no assumption crosses a safe call. The five rules now carry ids ALIAS-1 through ALIAS-5 (docs/architecture.md 5.3), so later diagnostics and corpus cases can name the rule they test. Still owed: a corpus file per rule and codes that emit those ids.
  *Current:* §5.3 is normative and the five IDs are written there. ALIAS-1,
  ALIAS-2, ALIAS-3 and ALIAS-5 now have enforcing diagnostics (`E1005`, `E1406`,
  `E1403` and `E1404`); the two FFI/alias gates are pinned by
  `crates/yashy-sema/tests/ffi_deref.rs` and
  `crates/yashy-driver/tests/ffi_deref.rs`. ALIAS-4 and a corpus file per rule
  are still owed.
- [ ] **Step 3: `unsafe` is a block, not a mode.** Dereferencing an FFI pointer, pointer arithmetic beyond a proved bound, calling an `unsafe fn`/`extern` function, and `alloc_uninit` require `unsafe { }`. A safe function cannot contain one. `verify --inventory` lists every unsafe site with its span and the reason; the count is a release number.
  *Current:* `unsafe { ... }` parses as an expression, checks its body and is
  lowered as a block by codegen; parser and sema tests pin the form. It is now
  a real gate for `alloc_uninit` and for dereferencing a C pointer: the builtins
  are refused outside a block with `E1405` / `E1406` and accepted inside one,
  with nested blocks tracked by depth. Pointer arithmetic is still not gated,
  so the rest of the gating half stays open. The inventory records an `unsafe_block` assumption for every explicit block
  with its span and a reason, alongside extern declarations and unsafe calls,
  in both the human table and `--json`; the driver test pins the new row.
- [ ] **Step 4: Null has one spelling.** `Option[*const T]` and `Option[*mut T]` are guaranteed to use the null pointer for `None` at the ABI boundary; `Some(p)` is non-null. There is no `null` literal and no nullable pointer type. A C function returning null round-trips to `None`; a null passed where `Some` is expected is refused at the call site.
  *Current:* an `Option` whose payload is a pointer now crosses an `extern "C"`
  boundary as one C pointer: `None` is null and `Some(p)` is p. Both directions
  are pinned by `crates/yashy-driver/tests/programs/null_ffi.ya`, which reads
  `getenv` (null for an unset name) and hands `None` to `free`. The aggregate
  remains `{ i8, ptr }` inside Yashy; the niche is an ABI guarantee, not a new
  source-level nullable type. There is still no source `null` literal, so a
  caller cannot pass a null where `Some` is expected, but the checker has no
  dedicated diagnostic for that case. Pointer arithmetic gating and the
  attribute-justification gate remain owed, so the step stays open.
- [ ] **Step 5: Spend only proved facts.** `noalias`, `nonnull` and `dereferenceable` are emitted only where Phase 4's facts license them; `--opt-report` names the fact. A pointer of unknown provenance gets no attribute. The alias corpus runs on both OSes and fails on any attribute without a justification.
- [ ] **Step 6: Commit.**

### Task 6: Zeroed allocation, the allocator interface, and layout

**Files:**
- Modify: `crates/yashy-codegen/src/memory.rs`, `crates/yashy-runtime/` (allocator hooks)
- Modify: `crates/yashy-stdlib/yashy/alloc.ya` (documented interface)
- Modify: `crates/yashy-parser/src/items.rs` (`repr(packed|align|transparent)`, integer enums)
- Modify: `crates/yashy-hir/src/layout.rs`, `crates/yashy-sema/src/const_eval.rs` (`size_of`/`align_of`/`offset_of`)
- Test: `crates/yashy-driver/tests/programs/memory/`, `crates/yashy-driver/tests/layout.rs`

**Interfaces:**
- `trait Allocator { fn alloc(self: *mut Self, size: int, align: int) -> Option[*mut u8]; fn free(self: *mut Self, p: *mut u8, size: int, align: int); }`
- `alloc(n) -> *mut int` (zeroed); `alloc_uninit(n) -> *mut int` (unsafe)
- `size_of[T]() -> int`, `align_of[T]() -> int`, `offset_of(T, field) -> int` (const-evaluable)

- [ ] **Step 1: Write the failing test.** `alloc(4)[0]` is `0`; `alloc_uninit(4)` in safe code is refused; a custom allocator records every call; `size_of[Point]()`, `align_of[Point]()` and `offset_of(Point, y)` equal the values the layout table reports; a `repr(packed)` struct has alignment 1 and a `repr(align(16))` struct has alignment 16.
- [ ] **Step 2: Zero by default.** Safe `alloc` zeroes every byte it hands
  out, so uninitialised memory cannot make a program's output differ between
  runs; `crates/yashy-runtime/tests/memory.rs` writes a stale pattern, frees and
  reallocates, and asserts the new block is all zeroes. The explicit
  `alloc_uninit(n)` is the explicit unsafe opt-out and E1405 refuses it outside
  an `unsafe` block. The verifier inventory counts only the sites that exist today.
  *Current:* `yashy_alloc_with` in `yashy_runtime.h` memsets every byte when its
  `zero` bit is set, and both the normal and small allocation paths go through
  it. The stale-pattern/leak test needs binary execution and is CI-only.
  `yashy_alloc_uninit` clears that bit, the checker admits it only inside
  `unsafe { }`, codegen calls a distinct runtime symbol and never promotes it
  to a stack slot; `crates/yashy-sema/tests/unsafe_alloc.rs` and
  `crates/yashy-driver/tests/unsafe_alloc.rs` pin both halves.
- [ ] **Step 3: Every allocation is visible.** The allocator interface is static-dispatch only; collections take an allocator or use the global one; `--alloc-report` (Phase 11A) reads the same hooks. No runtime path allocates behind the language's back.
  *Current:* `crates/yashy-stdlib/yashy/alloc.ya` now exposes
  `global_allocator()`, `global_alloc()` and `global_free()` as the documented
  process-wide path; `crates/yashy-driver/tests/programs/stdlib/global_alloc.ya`
  uses it and is covered by the stdlib determinism and leak tests. A collection
  *parameter* and `--alloc-report` are still owed, so the step stays open.
- [x] **Step 4: `repr` stays a keyword form.** `repr(C)`, `repr(packed)`, `repr(align(N))`, `repr(transparent)` and an integer enum repr are clauses, not attributes; `repr(C)` is never reordered; a packed field that would be misaligned is a diagnostic at the field, not a silent unaligned load.
  *Current:* all five clauses parse; a non-power-of-two align is refused. The
  layout walk uses packed (alignment 1, summed sizes) and align(N) (rounded
  size, raised alignment), and a sema test folds `alignof`/`sizeof` for both.
  A field of a `repr(packed)` struct or object that would land at an offset its
  type cannot accept is now refused at that field with E0607 (`LAYOUT-1`),
  pinned by sema tests; a packed struct whose fields happen to stay aligned
  still checks. `repr(C)` fields are never reordered.
- [x] **Step 5: Layout is a compiler fact with a surface.** `size_of`, `align_of` and `offset_of` are const-evaluable and agree with the IR; a mismatch is a compiler bug caught by the layout test. `inspect --layout` (Phase 11A) renders the same table.
  *Current:* the checker folds `sizeof(T)`, `alignof(T)` and `offsetof(T, f)`
  from the shared HIR layout walk, and `inspect --layout` reads that table. The
  underscore spellings are accepted, and the bracket spelling
  `size_of[Buffer[16]]()` now names a const-generic instantiation.
  `crates/yashy-driver/tests/layout.rs` folds the queries, reads the same layout
  table and compares the emitted `@main` body with one written with those
  literal numbers, so the checker, the layout table and the IR agree.
- [ ] **Step 6: Commit.**

### Task 7: Const generics for array lengths and fixed buffers

**Files:**
- Modify: `crates/yashy-parser/src/items.rs`, `crates/yashy-parser/src/types.rs` (const parameters)
- Modify: `crates/yashy-sema/src/lib.rs` (const arguments), `crates/yashy-sema/src/const_eval.rs` (total evaluation)
- Modify: `crates/yashy-hir/src/ty.rs`, `crates/yashy-hir/src/layout.rs`
- Modify: `crates/yashy-codegen/src/fixed.rs`
- Test: `crates/yashy-sema/tests/const_generics.rs`, `crates/yashy-driver/tests/layout.rs`

**Interfaces:**
- `struct Buffer[const N: int] { data: [u8; N], len: int }`
- `fn zeroed[const N: int]() -> [u8; N]`
- `N` is an `int` const-evaluable expression; an array length is `N` or a literal

- [x] **Step 1: Write the failing test.** A `Buffer[16]` and a `Buffer[64]`
  are distinct monomorphic types with distinct sizes; a generic function over
  `N` instantiates per value; `size_of[Buffer[16]]()` is 16 plus the length;
  a non-constant length is a diagnostic naming the expression; a const argument
  that cannot be evaluated is a diagnostic, not a compiler panic.
- [x] **Step 2: Const evaluation is total.** The evaluator has a step and depth
  cap, an overflow rule that matches runtime arithmetic, and no division by zero;
  every failure is a diagnostic at the expression. A const generic whose value
  depends on a runtime value is refused.
  *Current:* the shared evaluator keeps its 1,000,000 step/call caps, 32-level
  depth cap and checked arithmetic; a generic argument is read from a decimal
  literal or a symbolic name, and a name that is not a const in scope is
  `E1524` rather than a panic. A full const expression such as `Buffer[1 + 2]`
  is not a const-argument spelling yet: it is refused at parse time.
- [x] **Step 3: Instantiation is capped.** A const parameter is part of the
  monomorphisation key and counts against the existing 64/4096 caps (`E1505`);
  a substitution explosion names the function and the values that grew it.
  *Current:* each const value is mangled into the key as a `cintN` placeholder
  and goes through `queue_instantiation`, so the existing per-function and
  program-wide budgets apply unchanged. A test instantiates one const-generic
  function 65 times and expects `E1505`.
- [x] **Step 4: Layout reads the instantiated value.** `size_of`, `align_of`
  and the IR agree for every `N`; `repr(C)` fixed buffers keep their ABI shape;
  a zero-length array is legal and has size 0.
  *Current:* both `sizeof(Buffer[n])` and `size_of[Buffer[n]]()` fold through
  the shared layout walk, and the driver layout test compares the emitted
  `main` body with one written using the literal sizes. A declared `[T; 0]`
  field is legal and folds to size 0.
- [x] **Step 5: Commit.** Landed as `29d2925`.

### Task 8: Ranges, membership, identity, and word-form logic

**Files:**
- Modify: `crates/yashy-lexer/src/token.rs` (`in`, `is`, `and`, `or`, `not`)
- Modify: `crates/yashy-parser/src/exprs.rs` (range, membership, identity, word forms)
- Modify: `crates/yashy-sema/src/check_expr.rs`, `crates/yashy-hir/src/hir.rs` (desugar)
- Modify: `crates/yashy-sema/tests/desugar.rs`, `crates/yashy-verify/src/lib.rs` (range bounds)
- Modify: `crates/yashy-common/src/codes.rs` (`E1516` membership, `E1517` identity, `E1518` float range)
- Modify: `crates/yashy-fmt/src/lib.rs`, `docs/tutorial/03-control-flow.md` and `05-patterns.md`
- Test: `crates/yashy-sema/tests/expressions.rs`, `crates/yashy-parser/tests/expressions.rs`,
  `crates/yashy-driver/tests/programs/expressions/`

**Interfaces:**
- `Range[T] { fn start(self) -> T; fn end(self) -> T; fn len(self) -> int; fn is_empty(self) -> bool; fn contains(self, T) -> bool; fn step_by(self, int) -> RangeIter[T] }`
- `x in y` desugars to `y.contains(x)` for a sequence, `y.contains_key(x)` for a map
- `p is q`, `p is not q` compare addresses; `x is Some`, `x is Ok`, `x is Err`, `x is Color.Red` test a variant
- `and`, `or`, `not` lex to `&&`, `||`, `!`; precedence does not change

- [ ] **Step 1: Write the failing test.** `const r = 0..10;`, `r.contains(3)`,
  `for n in (0..10).step_by(2)`, `'a'..='z'`, `"ell" in "hello"`,
  `3 in xs`, `"k" in m`, `3 in Set[int] { 1, 3 }`, `p is q`,
  `p is not q`, `opt is Some`, `Color.Red is red`, and
  `a and b or not c`. Assert each parses, checks and produces the same HIR or
  IR as its documented desugaring; assert `1.0..2.0` is `E1518`.
- [x] **Step 2: `Range[T]` is a value, not new control flow.** `..` and `..=`
  construct the built-in `Range[T]` (and `InclusiveRange[T]`), over `int` and
  `char` only; `float` is refused with `E1518` because a half-open float range
  has no total `len`. A `for` over a range lowers to the existing counter loop,
  so the HIR has no range node. `start`, `end`, `len`, `is_empty`, `contains`
  and `step_by` are ordinary methods; `step_by` with a non-positive step is a
  diagnostic, not a hang.
  *Current:* `..` and `..=` build a two-field `Range` value from `int` or
  `char` bounds (char bounds convert to their integer codes); an inclusive end
  is stored as the first excluded value. `start`, `end`, `len`, `is_empty` and
  `contains` lower to field reads and comparisons, and `3 in r` reaches the
  same `contains`. A `for` over either spelling of a range, including
  `(0..10).step_by(2)`, lowers to `HirStmt::ForRange` with an optional stride;
  a non-positive step is refused. A float bound is `E1518`. The value is
  monomorphic over `int` rather than a generic `Range[T]`, which is enough for
  the two bound types the language allows.
- [x] **Step 3: Membership is compiler-known, never operator overloading.**
  `needle in container` parses at comparison precedence and desugars in the
  parser to `container.contains(needle)` — an existing method call — so no new
  operator, HIR node or verifier rule is introduced. The checker already maps
  `contains` to `str_contains` and `slice_contains`; one alias on `Map`
  (`contains` → `map_contains_key`) makes map keys work too. Parser tests in
  `crates/yashy-parser/tests/membership.rs` pin the desugaring, precedence and
  non-chaining, and a sema test checks string, slice and map membership. A
  container without `contains` reports E1516 with a catalog page and rule ID
  (IN-1); `Range[T]` membership is still owed with Step 2.
- [x] **Step 4: `is` is identity and variant test, never value equality.** For
  raw pointers and owning pointers, `p is q` compares addresses and
  `p is not q` is its negation; for `Option[*T]` a null pointer `is None`.
  For `Option`, `Result` and an enum, `x is Variant` is a variant test that
  desugars to the `matches` form from Task 3 and binds nothing; `x is Some(v)`
  is written `x matches Some(v)` instead. `is` on integers, strings, structs or
  floats is `E1517`: `==` is the only value comparison and there is no `is`
  overloading.
  *Current:* `is`/`is not` are reserved keywords; the parser records one `Expr::Is`
  node and the checker resolves it from the subject type: pointer subjects become
  address `==`/`!=`, enum/Option/Result subjects become the two-arm `matches`
  switch, and value types get E1517 with a catalog page and rule ID (ID-1).
  Parser and sema tests pin all three cases; the keywords are in all eleven
  generated editor grammars.
- [x] **Step 5: Word forms are spelling, not semantics.** `and`, `or` and
  `not` are reserved keywords with their own token kinds; the parser accepts
  them where `&&`, `||` and `!` are expected and builds the identical
  `BinOp`/`UnOp` tree, so precedence and short-circuiting are unchanged.
  `crates/yashy-parser/tests/word_forms.rs` proves tree equality and
  precedence. All eleven generated editor files were regenerated from the
  lexer vocabulary. The formatter prints the keyword it read because it renders
  tokens; it does not rewrite one spelling to the other, and no 1.0 migration
  diagnostic is emitted yet. The BPE audit in Task 10 remains owed.
- [x] **Step 6: Commit.**
  `git commit -m "feat(lang): ranges, membership, identity and word-form logic"`.

### Task 9: `const fn` and compile-time evaluation

**Files:**
- Modify: `crates/yashy-parser/src/items.rs` (`const fn`, const methods)
- Modify: `crates/yashy-sema/src/const_eval.rs`, `crates/yashy-sema/src/check_expr.rs`, `crates/yashy-sema/src/lib.rs`
- Modify: `crates/yashy-hir/src/hir.rs`, `crates/yashy-hir/src/const_value.rs`
- Modify: `crates/yashy-verify/src/lib.rs`, `crates/yashy-codegen/src/expr_gen.rs`
- Modify: `crates/yashy-common/src/codes.rs`
- Create: `docs/const-eval.md`
- Test: `crates/yashy-sema/tests/const_fn.rs`, `crates/yashy-driver/tests/programs/const_eval/`

**Interfaces:**
- `const fn table[const N: int]() -> [int; N] { ... }`
- `ConstValue = Int | Float | Bool | String | Array | Struct | Enum`
- `eval_const(fn_id, args, fuel) -> Result<ConstValue, ConstEvalError>`

- [ ] **Step 1: Write the failing test.** A `const fn` builds a lookup table, a
  `const` item calls it, a fixed buffer size is computed from it, and a
  compile-time assertion rejects a bad constant. Assert the table is a literal in
  the IR, the run-time program and a hand-written table produce the same values,
  and the compiler does not emit the function body when it is only called at
  compile time.
  *Current:* the named gate target now exists as
  `crates/yashy-sema/tests/const_fn.rs`, with the folding, non-emission,
  call-site diagnostic, nested-call and termination tests. A `const fn` that
  only folds arithmetic is proved; building a lookup table now works too:
  `ConstValue::Array` folds a fixed array into `HirExpr::FixedArrayLit`, and a
  checked constant index reads one entry. `const_fn_table.ya` and sema tests pin
  the form. A compile-time assertion now exists as `const_assert(c)`: the same
  interpreter folds it and a false condition is `E1525`, pinned by
  `crates/yashy-sema/tests/const_assert.rs` and
  `crates/yashy-driver/tests/const_assert.rs`. The table's IR literal assertion
  is pinned by `crates/yashy-driver/tests/const_fn_ir.rs`. The differential
  comparison against a hand-written table and a fixed-buffer size computed from
  a const fn are still owed, so the step stays open.
- [ ] **Step 2: Const evaluation is ordinary code, not a macro.** A `const fn`
  is parsed and checked like any function, then evaluated by the same typed
  interpreter that evaluates `size_of`, array lengths and const generic
  arguments. It may call other `const fn`s, loop, branch, index fixed arrays and
  construct structs/enums. It may not perform I/O, allocate, use `unsafe`, call
  `extern`, await, or read a capability; each refusal is a diagnostic at the
  call.
  *Current:* branch (`if`), `let` with a scalar type, assignment and compound
  assignment to a `let` binding, and `while c { .. }` / `for ; c; u { .. }` with
  unlabelled `break`/`continue` are now evaluated, so a body can accumulate a
  table in a loop instead of writing one expression per entry; a runaway loop is
  stopped by the same step budget as a wide expression. Assigning a parameter is
  refused with `E0400`, exactly as in a run-time body. Still refused, and why
  the step stays open: method calls and `switch` have no compile-time value.
  Struct and enum constructors, struct field reads and unit variants are now
  folded, with their types checked against the declarations through
  `ConstTypes`. The evaluator
  folds `const` initializers, array
  lengths and defaults through `const_eval`, while `size_of`/`align_of`/
  `offset_of` fold from the shared layout table in the checker; `docs/const-eval.md`
  writes both halves down.
- [ ] **Step 3: Evaluation is total and bounded.** A step/fuel cap, a recursion
  depth cap and the same overflow/division rules as run time make every failure
  a diagnostic with the expression span. A `const fn` that does not terminate
  is an error, never a compiler hang. Evaluation is deterministic and produces
  the same bytes on both OSes.
  *Current:* the depth (160) and call (1M) caps existed; a 1M expression-step
  fuel was added and the failure is reported at the expression that exceeded
  it. A test drives `Work` to the cap directly. The evaluator still consumes
  the parser AST rather than typed HIR, but loops, indexing and accumulation
  are now covered by both the checker and the evaluator. A loop spends one step
  per iteration, so it is stopped by the same fuel as a wide expression.
  Overflow, division by zero, fuel, recursion depth and non-constant
  names now carry stable codes E1520-E1524 (CONST-1..5) with catalog pages and
  tests.
- [ ] **Step 4: Const and run time agree.** For every `const fn`, a differential
  test evaluates it at compile time and calls it at run time on the same inputs
  and compares the values. A mismatch is a compiler bug, not a language
  difference. A `const fn` that is only used at compile time is not emitted; one
  that is also called at run time is emitted once.
  *Current:* `crates/yashy-driver/tests/programs/const_fn_differential.ya`
  runs a loop-based sum through the constant evaluator and through a paired
  ordinary function with the same body, and the e2e test asserts the two lines
  are equal. The language cannot call a `const fn` at run time (E1524), so a
  paired body is the available differential; the general "for every `const fn`"
  claim and the two-emission rule are still owed.
- [ ] **Step 5: The evaluator is not a second compiler.** It consumes HIR and
  the layout table, reports through the same diagnostics, and is covered by the
  `never_panics` suite. `--emit-ir` shows the folded constant and the source
  span that produced it.
- [ ] **Step 6: Commit.**

### Task 10: BPE audit, rule ids, and the architecture update

**Files:**
- Modify: `crates/yashy-driver/src/tokens.rs` (Phase 2 owns the measurement)
- Create: `docs/bpe.md` (generated report), `docs/rules.md` (generated ids)
- Modify: `docs/architecture.md`, `docs/tutorial/*` for every new form
- Test: `crates/yashy-driver/tests/bpe.rs`, `crates/yashy-driver/tests/rules.rs`

- [ ] **Step 1: Write the failing test.** Every keyword and operator introduced by this phase (`trait`, `type`, `derive`, `unsafe`, `noalias`, `packed`, `align`, `transparent`, `const fn`, `let else`, `matches`, `@`) is one token in `o200k`, `cl100k` and a Llama-family tokenizer. The corpus reports ≥ 97% single-token lexical tokens.
  *Current:* `crates/yashy-driver/tests/bpe.rs` pins the Yashy-lexer half and
  `docs/bpe.md` records it. The three external vocabularies are not vendored,
  so the o200k/cl100k/Llama measurement and the 97% corpus row cannot be
  claimed here.
- [ ] **Step 2: Reject a spelling before it ships.** If a candidate keyword splits, choose a single-token synonym and record the measurement in `docs/bpe.md`. No aesthetic rewrites; only measured ones.
  *Current:* no external measurement exists, so no spelling has been rewritten.
  Once the vocabularies are vendored, this step decides whether any of the
  Phase 4 spellings need a measured synonym.
- [x] **Step 3: Rule ids.** Every new diagnostic names a rule (`TRAIT-1`,
  `PAT-2`, `ALIAS-3`, `LAYOUT-1`, `CONST-1`, `DET-1`). `yashyc trace` (Phase 9) will consume the registry; this phase fills it.
  *Current:* every catalog code has a stable rule through `codes::rule_for`,
  and `crates/yashy-driver/tests/rules.rs` pins both the generated
  `docs/rules.md` page and the four design rules that still have no code
  (`ALIAS-4`, `DET-2`, `DET-3`, `DET-5`); the page names them as orphans
  instead of hiding them. `docs/traceability.md` pairs the same registry with
  the tests that mention each code.
- [x] **Step 4: Update the architecture document.** §4 (program model), §5 (memory, aliasing, layout, allocation, determinism) and §12 (phase map) are updated in the same commit as the code.
  *Current:* §4.1 records `else if` sugar; §4.2 records that user traits stay
  `E1511`; §4.3 records `defer` sugar; §4.5 distinguishes the landed
  `matches`/`is`/`in`/if-let sugar from the owed pattern forms; §4.6 records
  word-form tokens; §5.3 carries ALIAS-1..5; §5.5 records zeroed allocation.
  The phase map in §12 already names Phase 4.
- [ ] **Step 5: Commit.**

## Gates

| Gate | Command | Passes when |
|---|---|---|
| Traits | `cargo test -p yashy-sema --test traits` | a generic trait, an associated type, a default body or a supertrait is `E1511` (the design's exclusion); impl completeness and coherence are enforced; dispatch is static, with no vtable in the IR |
| Patterns | `cargo test -p yashy-sema --test pats --test desugar` | `let else`, slice/rest, `@`, `matches`, if-let chains and `else if` have desugar proofs |
| Expressions | `cargo test -p yashy-sema --test expressions --test desugar` | `Range[T]`, `in`, `is`/`is not`, `and`/`or`/`not` parse, check and desugar; `float` ranges, membership on a user type and `is` on a value are diagnosed |
| Derives | `cargo test -p yashy-sema --test derive` | deterministic bytes; `index --json` shows each impl; a missing field bound names the field |
| Unsafe and aliasing | `cargo test -p yashy-driver --test alias_corpus` plus `-p yashy-sema --test ffi_deref -p yashy-driver --test ffi_deref` | ALIAS-1, ALIAS-2, ALIAS-3 and ALIAS-5 have positive/negative cases in the corpus; the inventory lists every explicit `unsafe` block. ALIAS-4 is named as unenforced; pointer arithmetic and a corpus-based attribute-justification gate are still owed |
| Unsafe inventory | `cargo test -p yashy-verify --test unsafe` | every `extern` declaration and explicit `unsafe` block is listed with a kind and a reason; safe code has none. Pointer `Option`s cross C as nullable pointers (`null_ffi.ya`); the internal tagged aggregate, a `null` literal and the call-site null-to-`Some` diagnostic are still owed (Task 5 Step 4) |
| Allocation | `cargo test -p yashy-runtime --test memory` (binary execution is CI-only) plus `cargo test -p yashy-sema --test unsafe_alloc -p yashy-driver --test unsafe_alloc` | `alloc` is zeroed on both paths; `alloc_uninit` needs `unsafe` (E1405) and calls a distinct symbol; a custom allocator's call log is owed (Task 6 Step 3) |
| Layout | `cargo test -p yashy-driver --test layout` | `size_of`/`align_of`/`offset_of` agree with the IR; `repr(C)` is stable; packed misalignment is diagnosed |
| Const generics | `cargo test -p yashy-sema --test const_generics` (file owed) | `const N` parameters do not exist (Task 7), so there is no gate to run; the row stays until the task lands |
| Const evaluation | `cargo test -p yashy-sema --test const_fn -p yashy-driver --test const_fn_ir --test const_assert` | folding produces the value the compiler computed (arithmetic, `if`, loops, arrays, nested calls); a const table is a literal in the IR with no emitted function; fuel/depth caps terminate; I/O, allocation, `unsafe` and `extern` are refused; no panic. The compile-time/run-time differential is owed (Task 9 Step 4) |
| BPE | `cargo test -p yashy-driver --test bpe --test tokens` | every Phase 4 single-word spelling is one Yashy lexer token; `docs/bpe.md` records the measurement. The o200k/cl100k/Llama counts and 97% corpus row are owed (Task 10 Step 1) and need those vocabularies vendored |
| No false positives | `yashyc verify --json examples/ crates/yashy-driver/tests/programs/` | zero |
| No regression | `cargo test --workspace` | green on Windows and Linux |
| No P0 | `docs/triage.md` | no P0 |

## Demo

```console
$ yashyc run examples/traits/main.ya
trait Container[T] { type Item; fn len(self) -> int { 0 } }
Point(3, 4).len() = 2   // default method, monomorphised
Stack[string].get(0) = "a"  // associated type inferred

$ yashyc run examples/patterns/main.ya
head=1 rest=3 last=5     // [head, ..rest, last]
inside                     // x @ 0..10

$ yashyc run examples/expressions/main.ya
3 in 0..10 = true          // membership, desugared to Range.contains
"ell" in "hello" = true    // substring membership
0, 2, 4, 6, 8              // for n in (0..10).step_by(2)
same = false               // p is q is address identity
pos = true                 // x is Some is a variant test
```

```console
$ yashyc inspect --layout Point --json
{ "type": "Point", "size": 16, "align": 8,
  "fields": [ {"name":"x","offset":0}, {"name":"y","offset":8} ] }

$ yashyc verify --inventory examples/unsafe/main.ya
unsafe sites: 2
  main.ya:14  call extern "C" puts      ALIAS-4
  main.ya:21  deref *const u8           ALIAS-2
```

```console
$ yashyc run examples/const_eval/main.ya
table[0] = 1  table[15] = 987          // a const fn table, folded to a literal
buffer[32] = 32 bytes                   // length computed at compile time
checked  = ok                           // const assertion rejected the bad case
$ yashyc build examples/const_eval/main.ya --emit-ir | grep -c 'call.*build_table'
0                                       // compile-time-only fn is not emitted
```

## Out of scope

- `dyn`, vtables, trait objects, async traits (`E1703`), trait fields (`E1514`).
- Operator overloading (`E1515`) and chained comparisons. `in` and `is` are
  compiler-known forms over built-in types only; a user type cannot define
  either.
- Specialisation, negative impls and coherence beyond local-type/local-trait.
- Named read-only views; views stay unnamed.
- Generators/`yield`.
- Compile-time code generation, reflection or macros. `const fn` evaluates a
  restricted expression language at compile time; it does not print source,
  reflect on names or run at run time.
- `const` trait methods and const dispatch across a generic bound; the first
  release covers free functions, inherent methods and pure operators.
- JIT or run-time compilation.
- `noalias` on safe parameters; that stays illegal for the life of the edition.
- The layout optimiser and stack promotion (Phase 11A); this phase makes the
  facts available, not the optimisation.

## Risks

| Risk | Mitigation |
|---|---|
| Generic traits explode monomorphisation | The existing 64/4096 cap covers trait methods; the diagnostic names the substitution; a benchmark row tracks code size |
| Trait coherence rejects a valid stdlib impl | Coherence is local-type/local-trait only; the stdlib is in the same workspace and tested first |
| `else if` changes a span or a `defer` order | It desugars to a one-statement block; the desugar test asserts the HIR shape and the formatter round-trip |
| Pattern rules reject valid code | Every refutable form has a positive corpus; the no-false-positive gate includes the new patterns |
| `noalias` miscompiles | Only proved facts emit it; safe code cannot declare it; the alias corpus fails any unjustified attribute |
| `alloc` zeroing costs performance | Measured in Phase 11A; the zeroing is one `memset` the optimiser can remove when the program writes first |
| `Option[*const T]` niche leaks into layout | A layout test and an FFI round-trip pin the representation; the niche is documented, not incidental |
| `unsafe` erodes the safety claim | Every site is inventoried and counted; the count is a release note number, not a footnote |
| Const generics explode code size or const evaluation loops | A step/depth cap, the existing 64/4096 instantiation cap, and a diagnostic naming the substitution |
| `const fn` becomes a second execution model | It evaluates a restricted expression language with the same overflow rules; a compile-time/run-time differential test is owed; no I/O, allocation, `unsafe`, `extern` or capability access |
| A word-form keyword breaks a 1.0 name | Edition-gated (`2027`); a migration diagnostic names the replacement; the BPE audit runs before the spelling ships |
| `in` or `is` becomes a second dispatch path | Both are compiler-known over built-in types and lower to existing methods or `matches`; no user type can define either |
| Const evaluation hangs the compiler | Fuel, depth and size caps; every failure is a diagnostic at the expression; the `never_panics` suite covers the evaluator |

## Exceptions

These are the places where the shipped code is smaller than the task text, or
where the task text contradicts the design record this repository treats as
authority. They are recorded here rather than silently ticked so a later phase
does not build on a claim that is not true.

| Task | Claimed | Shipped | Follow-up |
|---|---|---|---|
| 1.1-1.4, 1.6 | User generic traits, associated types, default methods, supertraits | `E1511`: a trait has no type parameters, no `type` item, no default body and no supertrait list; the impl surface (completeness, coherence, one impl per trait-and-type, static dispatch, no vtable) is what exists | Not a Phase 4 gap. §4.2 of the 2.0 design makes `Iterator[T]` the one compiler-known generic trait and keeps user generic traits excluded; reopening it needs a design-record change first |
| 2.1, 2.3, 2.4 | `pub(crate)` / `pub(super)` / `pub(in path)` lattice, `pub use` re-exports | `pub(crate)` parses; `pub(super)` and `pub(in path)` are refused by the parser with a diagnostic naming the limitation; there is no `pub use`; visibility is the binary `pub`-vs-module-private model with the declaration's file as the module | §4.2 records the binary model: this compiler's crate is its whole program, so there is no `super` to compare a use site against. Reopen only with a module-graph design |
| 5.1-5.6 | `unsafe` gates deref and pointer arithmetic; `Option[*const T]` null niche; alias corpus per rule | `unsafe { }` parses, checks, is inventoried and gates both `alloc_uninit` (`E1405`) and C-pointer dereference (`E1406`, ALIAS-2). ALIAS-1, ALIAS-3 and ALIAS-5 have codes. Pointer arithmetic is still ungated. A pointer `Option` crosses C as a nullable pointer (`null_ffi.ya`); inside Yashy it keeps the tagged aggregate, and there is no `null` literal. `crates/yashy-verify/tests/unsafe.rs` pins the inventory | Owed work, not excluded: pointer arithmetic, ALIAS-4, a call-site null-to-`Some` diagnostic and the attribute-justification gate |
| 4.1, 4.2 | Deterministic derive impls, visible in `index`, field-bound diagnostics | `derive(Debug, Eq, Hash)` expands to inherent methods in declaration order, visible as functions in the index; an unsupported field or unknown derive is refused with the field/name it concerns, and a second expansion of one declaration gives the same typed program. Enum derives and `Encode`/`Decode` are owed | Owed work: enum derives and Phase 7's codec interface |
| 6.5, 7.1-7.5 | `size_of[T]()` bracket spelling; `const N` parameters | `sizeof(T)` / `alignof(T)` / `offsetof(T, f)` and the underscore spellings fold from the shared layout table; the bracket spelling needs const generics, which do not exist | Owed work; the checker/layout/IR agreement is pinned by `crates/yashy-driver/tests/layout.rs` |
| 8.1, 8.2 | `Range[T]` value type with `start`/`end`/`len`/`is_empty`/`contains`/`step_by` | `..` and `..=` parse as a `for` head and as a pattern; a float bound is `E1518`; there is no range value or method surface | Owed work; `in` over a range waits on it |
| 9.1, 9.4, 9.5 | Compile-time/run-time differential test, evaluator consumes typed HIR | The evaluator consumes the parser AST and folds arithmetic, `if`, `let`/assignment, loops, fixed arrays and nested calls; `const_assert` rejects a false compile-time fact; the table's IR-literal assertion is pinned by `crates/yashy-driver/tests/const_fn_ir.rs`. `const_fn_differential.ya` compares a folded loop with a paired run-time function, but a `const fn` itself cannot be called at run time (E1524) | Owed work; the AST choice is why `switch` and method calls are still refused rather than wrong |
| 10.1, 10.2, 10.5 | BPE audit over this phase's keywords in `o200k`, `cl100k` and a Llama tokenizer | `yashyc tokens --bpe` reports one tokenizer and counts; `docs/bpe.md` records the measured Yashy-lexer half and the missing external half | Needs the three vocabularies vendored (Phase 2 Task 8 owes the same files) |
| Gates | Every acceptance row is a command that exists and passes on Windows and Linux | Rows name the test file that exists; the rows whose file is owed say so. Driver checks are green with the documented runtime build-script detach; freshly linked binaries are blocked by Windows Application Control, so the runtime, e2e and IR gates are CI-only | CI is deferred by the project owner; re-run the full gate set when CI is unblocked |

The phase is therefore **partially complete**: the pattern, range-bound,
membership, identity, word-form, `defer expr`, `repr` and const-evaluation
slices have landed; the trait, visibility, pointer, const-generic and BPE slices
are excluded or owed as named above.
