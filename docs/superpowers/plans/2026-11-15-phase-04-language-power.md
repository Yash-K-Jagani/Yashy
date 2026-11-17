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
BPE measurement. `E1511` changes from "user generic traits are refused" to the
new trait surface. `alloc` zeroes; `alloc_uninit` is `unsafe`. `const fn`
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
- [ ] **Step 2: Keep dispatch static.** A trait method call resolves to one `HirFn` per `(impl, substitution)`; the call graph names it. There is no trait object, no vtable, no runtime lookup. A trait used only through generics is monomorphised; a trait method called on a concrete type is a direct call.
- [ ] **Step 3: Bound the expansion.** The existing cap (64 instantiations per function, 4096 per program, `E1505`) covers trait methods and associated-type projections. Exceeding it is a diagnostic naming the function and the substitution that grew, never a compile that runs out of memory.
- [ ] **Step 4: Coherence.** An impl must be for a local type or a local trait; overlapping impls are refused with both spans; an associated type without a binding and without a default is refused; a supertrait obligation is checked at the impl and at every use.
- [ ] **Step 5: `Iterator[T]` stays special.** It remains the compiler-known `for` protocol. A user trait named `Iterator` is a diagnostic; an adapter trait (`DoubleEnded`, `ExactSize`) is an ordinary generic trait with supertrait `Iterator[T]`.
- [ ] **Step 6: Commit.** `git commit -m "feat(sema): static generic traits with associated types and defaults"`.

### Task 2: Type aliases, visibility and re-exports

**Files:**
- Modify: `crates/yashy-parser/src/items.rs` (`type`, visibility forms)
- Modify: `crates/yashy-sema/src/scope.rs`, `crates/yashy-driver/src/modules.rs`, `crates/yashy-driver/src/index.rs`
- Modify: `crates/yashy-common/src/codes.rs`
- Test: `crates/yashy-driver/tests/modules.rs`, `crates/yashy-sema/tests/visibility.rs`

- [ ] **Step 1: Write the failing test.** `type Pair[T] = (T, T);`, `pub(crate) fn`, `pub(super) fn`, `pub(in crate::db) fn`, and `pub use db::Row;`. Assert an out-of-scope use is refused with the span of the declaration and the use; assert `index --json` shows the re-export once, with the original span.
- [ ] **Step 2: Aliases are transparent.** An alias expands at resolution and never reaches HIR as a distinct type. A recursive alias is `E15xx`; an alias that shadows a type is refused.
- [ ] **Step 3: Visibility is one lattice.** `private < pub(crate) < pub(super) < pub(in path) < pub` is not a total order; the checker compares the declaring module and the use site, not a rank. `pub use` re-exports the item once; two paths to one item are one symbol in `index`.
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
- [ ] **Step 3: `let else`.** `let Some(x) = opt else { return; };` binds on the matching path and requires the else block to diverge (`return`, `break`, `continue`, `panic`, or a call returning `!`-like divergence). A non-diverging else is a diagnostic; a refutable `let` without `else` stays `E0100`.
- [ ] **Step 4: Slice/rest and `@`.** `[first, .., last]`, `[head, ..rest]`, `x @ 0..10`, and nested combinations bind names with element types; a rest binding is a view with the source's lifetime rules; every alternative of an or-pattern binds the same names (`E1002`).
- [ ] **Step 5: `matches` and if-let chains.** `x matches Some(_)` is an expression returning `bool` (desugars to a two-arm `switch`); `if let Some(x) = a && let Some(y) = b { }` desugars to nested `if let` with short-circuit order, and the bindings are visible in the body only.
- [ ] **Step 6: Commit.**

### Task 4: Built-in derives and explicit closure captures

**Files:**
- Modify: `crates/yashy-parser/src/items.rs` (`derive(...)` clause), `crates/yashy-sema/src/lib.rs` (derive expansion)
- Modify: `crates/yashy-hir/src/hir.rs` (closure capture mode), `crates/yashy-verify/src/ownership.rs`
- Modify: `crates/yashy-codegen/src/higher_order.rs`
- Test: `crates/yashy-sema/tests/derive.rs`, `crates/yashy-sema/tests/closures.rs`

- [ ] **Step 1: Write the failing test.** `struct Point derive(Debug, Eq, Hash) { x: int, y: int }` and an enum with `Encode`/`Decode`. Assert the generated impls are deterministic (same bytes across runs), visible in `index --json`, and refused when a field does not implement the requirement (naming the field).
- [ ] **Step 2: Derives are compiler-known, not macros.** Each derive is a built-in expansion over HIR with a stable order: fields in declaration order, variants in declaration order, no hash-map iteration. `derive(Debug)` renders a stable, readable form; `Encode`/`Decode` use the Phase 7 codec interface.
- [ ] **Step 3: Capture modes are in the type.** A closure writes `move` / value / by-pointer captures; the closure's type records each captured binding and its mode, the verifier sees every capture, and a `move` closure cannot outlive a value it moved. A capture mode that cannot be honoured is a diagnostic, not a silent copy.
- [ ] **Step 4: `defer expr;` and scoped release.** The expression form is
  sugar for `defer { expr; }`; a comma list registers each release in order; an
  arena's exit hook is a `defer` over the arena, not a new construct. Release
  order and double-free rules are unchanged. `defer` across an `.await` is
  checked by the Phase 10 rule; this phase adds the parse and the desugar test.
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
- [ ] **Step 2: Write the rules down before the code.** `docs/architecture.md` §5.3 becomes normative: owning pointers are unique by linearity; FFI pointers may alias; `noalias` is legal only on `extern` declarations and `unsafe fn` parameters; disjoint views are proved, not assumed; no assumption crosses a safe call. Every rule gets an id and a corpus file.
- [ ] **Step 3: `unsafe` is a block, not a mode.** Dereferencing an FFI pointer, pointer arithmetic beyond a proved bound, calling an `unsafe fn`/`extern` function, and `alloc_uninit` require `unsafe { }`. A safe function cannot contain one. `verify --inventory` lists every unsafe site with its span and the reason; the count is a release number.
- [ ] **Step 4: Null has one spelling.** `Option[*const T]` and `Option[*mut T]` are guaranteed to use the null pointer for `None` at the ABI boundary; `Some(p)` is non-null. There is no `null` literal and no nullable pointer type. A C function returning null round-trips to `None`; a null passed where `Some` is expected is refused at the call site.
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
- [ ] **Step 2: Zero by default.** `alloc` writes zeros; `alloc_uninit` is the explicit unsafe spelling and is inventoried. The runtime never hands out uninitialised memory through a safe builtin. This is the memory half of the determinism contract.
- [ ] **Step 3: Every allocation is visible.** The allocator interface is static-dispatch only; collections take an allocator or use the global one; `--alloc-report` (Phase 11A) reads the same hooks. No runtime path allocates behind the language's back.
- [ ] **Step 4: `repr` stays a keyword form.** `repr(C)`, `repr(packed)`, `repr(align(N))`, `repr(transparent)` and an integer enum repr are clauses, not attributes; `repr(C)` is never reordered; a packed field that would be misaligned is a diagnostic at the field, not a silent unaligned load.
- [ ] **Step 5: Layout is a compiler fact with a surface.** `size_of`, `align_of` and `offset_of` are const-evaluable and agree with the IR; a mismatch is a compiler bug caught by the layout test. `inspect --layout` (Phase 11A) renders the same table.
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

- [ ] **Step 1: Write the failing test.** A `Buffer[16]` and a `Buffer[64]`
  are distinct monomorphic types with distinct sizes; a generic function over
  `N` instantiates per value; `size_of[Buffer[16]]()` is 16 plus the length;
  a non-constant length is a diagnostic naming the expression; a const argument
  that cannot be evaluated is a diagnostic, not a compiler panic.
- [ ] **Step 2: Const evaluation is total.** The evaluator has a step and depth
  cap, an overflow rule that matches runtime arithmetic, and no division by zero;
  every failure is a diagnostic at the expression. A const generic whose value
  depends on a runtime value is refused.
- [ ] **Step 3: Instantiation is capped.** A const parameter is part of the
  monomorphisation key and counts against the existing 64/4096 caps (`E1505`);
  a substitution explosion names the function and the values that grew it.
- [ ] **Step 4: Layout reads the instantiated value.** `size_of`, `align_of`
  and the IR agree for every `N`; `repr(C)` fixed buffers keep their ABI shape;
  a zero-length array is legal and has size 0.
- [ ] **Step 5: Commit.**

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
- [ ] **Step 2: `Range[T]` is a value, not new control flow.** `..` and `..=`
  construct the built-in `Range[T]` (and `InclusiveRange[T]`), over `int` and
  `char` only; `float` is refused with `E1518` because a half-open float range
  has no total `len`. A `for` over a range lowers to the existing counter loop,
  so the HIR has no range node. `start`, `end`, `len`, `is_empty`, `contains`
  and `step_by` are ordinary methods; `step_by` with a non-positive step is a
  diagnostic, not a hang.
- [ ] **Step 3: Membership is compiler-known, never operator overloading.**
  `needle in container` is defined for `[T]`, `[T; N]`, `[]T`, `string`
  (substring, byte-exact), `map[K]V` (key), `Set[T]` and `Range[T]`. It lowers
  to the method the type already documents (`contains`, `contains_key`), so a
  user type gets no new operator and `E1515` stays true. A container without a
  membership fact is `E1516` naming the method to call instead. Membership is a
  comparison-precedence operator and does not chain.
- [ ] **Step 4: `is` is identity and variant test, never value equality.** For
  raw pointers and owning pointers, `p is q` compares addresses and
  `p is not q` is its negation; for `Option[*T]` a null pointer `is None`.
  For `Option`, `Result` and an enum, `x is Variant` is a variant test that
  desugars to the `matches` form from Task 3 and binds nothing; `x is Some(v)`
  is written `x matches Some(v)` instead. `is` on integers, strings, structs or
  floats is `E1517`: `==` is the only value comparison and there is no `is`
  overloading.
- [ ] **Step 5: Word forms are spelling, not semantics.** `and`, `or` and `not`
  are reserved keywords in edition 2027 and lex to the same tokens as `&&`,
  `||` and `!`; precedence and short-circuiting are unchanged, and the
  formatter preserves the spelling the source used. A 1.0 program that used one
  of them as a name gets a migration diagnostic naming the replacement. The BPE
  audit in Task 10 measures all three before they ship.
- [ ] **Step 6: Commit.**
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
- [ ] **Step 2: Const evaluation is ordinary code, not a macro.** A `const fn`
  is parsed and checked like any function, then evaluated by the same typed
  interpreter that evaluates `size_of`, array lengths and const generic
  arguments. It may call other `const fn`s, loop, branch, index fixed arrays and
  construct structs/enums. It may not perform I/O, allocate, use `unsafe`, call
  `extern`, await, or read a capability; each refusal is a diagnostic at the
  call.
- [ ] **Step 3: Evaluation is total and bounded.** A step/fuel cap, a recursion
  depth cap and the same overflow/division rules as run time make every failure
  a diagnostic with the expression span. A `const fn` that does not terminate
  is an error, never a compiler hang. Evaluation is deterministic and produces
  the same bytes on both OSes.
- [ ] **Step 4: Const and run time agree.** For every `const fn`, a differential
  test evaluates it at compile time and calls it at run time on the same inputs
  and compares the values. A mismatch is a compiler bug, not a language
  difference. A `const fn` that is only used at compile time is not emitted; one
  that is also called at run time is emitted once.
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
- [ ] **Step 2: Reject a spelling before it ships.** If a candidate keyword splits, choose a single-token synonym and record the measurement in `docs/bpe.md`. No aesthetic rewrites; only measured ones.
- [ ] **Step 3: Rule ids.** Every new diagnostic names a rule (`TRAIT-1`, `PAT-2`, `ALIAS-3`, `LAYOUT-1`, `CONST-1`, `DET-1`). `yashyc trace` (Phase 9) will consume the registry; this phase fills it.
- [ ] **Step 4: Update the architecture document.** §4 (program model), §5 (memory, aliasing, layout, allocation, determinism) and §12 (phase map) are updated in the same commit as the code.
- [ ] **Step 5: Commit.**

## Gates

| Gate | Command | Passes when |
|---|---|---|
| Traits | `cargo test -p yashy-sema --test traits` | generic traits, associated types, defaults and supertraits monomorphise; no vtable in the IR; the cap holds |
| Patterns | `cargo test -p yashy-sema --test pats --test desugar` | `let else`, slice/rest, `@`, `matches`, if-let chains and `else if` have desugar proofs |
| Expressions | `cargo test -p yashy-sema --test expressions --test desugar` | `Range[T]`, `in`, `is`/`is not`, `and`/`or`/`not` parse, check and desugar; `float` ranges, membership on a user type and `is` on a value are diagnosed |
| Derives | `cargo test -p yashy-sema --test derive` | deterministic bytes; `index --json` shows each impl; a missing field bound names the field |
| Unsafe and aliasing | `cargo test -p yashy-driver --test alias_corpus` | every rule has a positive and a negative case; no attribute lacks a justification |
| Null niche | `cargo test -p yashy-verify --test unsafe` | `Option[*const T]` round-trips null; a null `Some` is refused |
| Allocation | `cargo test -p yashy-driver --test programs` | `alloc` is zeroed; `alloc_uninit` needs `unsafe`; a custom allocator sees every call |
| Layout | `cargo test -p yashy-driver --test layout` | `size_of`/`align_of`/`offset_of` agree with the IR; `repr(C)` is stable; packed misalignment is diagnosed |
| Const generics | `cargo test -p yashy-sema --test const_generics` | `Buffer[16]` and `Buffer[64]` are distinct; evaluation is total; the cap holds |
| Const evaluation | `cargo test -p yashy-sema --test const_fn --test const_eval` | compile-time and run-time values agree; fuel/depth caps terminate; I/O, allocation, `unsafe` and `extern` are refused; no panic |
| BPE | `cargo test -p yashy-driver --test bpe` | 100% of keywords/operators single-token; corpus ≥ 97% |
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
- Compile-time code generation, reflection or macros. `const fn` evaluates
  typed HIR; it does not print source, reflect on names or run at run time.
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
| `const fn` becomes a second execution model | It evaluates typed HIR with the same overflow and layout rules; a compile-time/run-time differential test; no I/O, allocation, `unsafe`, `extern` or capability access |
| A word-form keyword breaks a 1.0 name | Edition-gated (`2027`); a migration diagnostic names the replacement; the BPE audit runs before the spelling ships |
| `in` or `is` becomes a second dispatch path | Both are compiler-known over built-in types and lower to existing methods or `matches`; no user type can define either |
| Const evaluation hangs the compiler | Fuel, depth and size caps; every failure is a diagnostic at the expression; the `never_panics` suite covers the evaluator |
