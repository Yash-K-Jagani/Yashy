# M3–M4 Implementation Plan — Memory and Faults

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give Otter real manual memory — explicit allocation, arenas, linear ownership, refcounted strings — and a defined response to every runtime fault: `panic` with unwinding that runs pending `defer` handlers.

**Architecture:** `otter-runtime` gains the memory and string primitives from the master ledger L1. `otter-sema` gains pointer types and a linearity checker that rejects double-moves. `otter-codegen` gains allocation calls, retain/release balancing, string lowering, and — in M4 — landing pads and inline overflow checks that replace the interim bare `sdiv`/`add` contract pinned in M0–M2 Task 11.

**Tech Stack:** Rust 1.99, LLVM 17+ opaque-pointer IR as text, C runtime shim, `cargo test`.

**Spec:** `docs/superpowers/specs/2026-10-16-otter-language-design.md` §5 (memory), §6 (faults). **Master plan:** `docs/superpowers/plans/2026-10-16-otter-master-plan.md` — read its Interface Ledger and Global Constraints first; this plan extends that ledger and must not contradict it.

## Prerequisites

- M0–M2 complete and reviewed: `cargo test --workspace` green, four end-to-end programs producing correct output.
- `StringLit` from `otter_lexer` and `Type`/`HirExpr` from `otter_sema` exist as specified in the M0–M2 plan.

## Scope

**In:** `alloc`/`dealloc`, arenas, `move` linearity, refcounted strings and their operations, string interpolation, `panic` with unwinding, `defer` on unwind, checked arithmetic, `assert`, exit codes.

**Out, and why:** slices and maps (M6) — but `Move` linearity in M3 must already be written so that `Slice` becomes a linear type in M6 by adding one match arm, not by rewriting the checker. Closures (out of scope entirely). Data-race guarantees (M7).

---

### Task 1: Memory primitives in the runtime

**Files:**
- Create: `crates/otter-runtime/runtime.c` (extend), `crates/otter-runtime/include/otter_runtime.h` (extend)
- Modify: `crates/otter-runtime/build.rs`
- Create: `crates/otter-runtime/tests/memory.rs` (Rust tests calling the linked C symbols)

**Interfaces:**
- Consumes: M0–M2 runtime build
- Produces: the M3 half of ledger L1 — `otter_alloc`, `otter_free`, `otter_arena_new`, `otter_arena_alloc`, `otter_arena_free_all`

- [x] **Step 1: Write the failing tests**

`crates/otter-runtime/tests/memory.rs`: allocate 1 MiB, write a byte at both ends, read them back, free. Assert every allocation respects its requested alignment (and that a non-power-of-two request falls back instead of handing back a misaligned block). Allocate 1000 blocks of mixed sizes in an arena, write a distinct pattern into each, verify all patterns survive (proving the blocks are independent and writable), then `otter_arena_free_all`. Assert `otter_free(null)` and `otter_arena_free_all(null)` are no-ops, and that an arena is reusable after `free_all`.

> **Deviation from the plan as written:** the plan called for a compiled C `main` shelled out to from a `#[test]`. The tests are ordinary Rust tests that reach the same C symbols through `otter-runtime`'s `extern` block — the same approach `tests/print.rs` already uses. They prove the static library from `build.rs` is linked while needing no extra compiler invocation and no child process.


- [x] **Step 2: Run it to verify it fails**

Run: `cargo test -p otter-runtime`
Expected: FAIL — the symbols do not exist. (Observed as a compile error in the test
crate, which is the same signal: the declarations have nothing to bind to.)

- [x] **Step 3: Implement the primitives**

`otter_alloc(size, align)` over-allocates by one pointer plus `align - 1` bytes
and stashes `malloc`'s address in the word before the aligned block, so there is
one allocator and one free path on every platform — no `aligned_alloc` /
`_aligned_malloc` pair to mix up. It does not zero-fill: Otter has no zero-fill
guarantee, and `defer` exists. `otter_free` walks back to the stash and tolerates
NULL. `otter_arena_new` mallocs a header; `otter_arena_alloc` bumps within the
current 64 KiB block and links a new — possibly larger — block when full;
`otter_arena_free_all` frees every block and empties the header. Individual arena
allocations are never freed.

- [x] **Step 4: Run it to verify it passes**

Run: `cargo test -p otter-runtime` → PASS (9 memory tests, plus the existing print
tests).

- [x] **Step 5: Commit**

```bash
git add crates/otter-runtime/
git commit -m "feat(runtime): heap allocation and arena primitives"
```

---

### Task 2: Pointer types and linear ownership in sema

**Files:**
- Modify: `crates/otter-sema/src/ty.rs`
- Create: `crates/otter-sema/src/ownership.rs`
- Modify: `crates/otter-sema/src/scope.rs` (track moved bindings)
- Modify: `crates/otter-sema/src/check_expr.rs` (`move`, assignment to linear bindings)
- Modify: `crates/otter-sema/tests/check.rs`

**Interfaces:**
- Consumes: `Type`/`TypeKind`/`Scope` from M0–M2
- Produces:
  - `TypeKind::Ptr(Box<Type>)` added to `ty.rs` (ledger L2, M3 row). The master
    plan spells this `Ptr(Type)`; this phase plan's original `Box<TypeKind>` was
    a slip — only `Type` carries a span, and `Display` needs it.
  - `pub enum Linearity { Linear, Copy }`
  - `pub fn linearity_of(ty: &TypeKind) -> Linearity` — `Linear` for `Ptr` and, from M6, `Slice`; `Copy` for everything else
  - `pub enum Ownership { Live, Moved }`
  - `Binding { ty, mutability, span, ownership: Ownership }` — one new field
  - `Scope::move_out(&mut self, name: &str, span: Span) -> Result<(), TypeError>` — sets the binding to `Moved`
  - `Scope::check_live(&self, name: &str, span: Span) -> Result<(), TypeError>` — errors on a read of a `Moved` binding
  - `Scope::rebind(&mut self, name: &str)` — an assignment installs a fresh owner
  - `Scope::moved_bindings(&self) -> BTreeSet<(usize, String)>` and
    `Scope::apply_moved(&mut self, ...)` — snapshot/restore of ownership state,
    used to check the two branches of an `if` from the same entry state and then
    merge them by union
  - `Checker::forbid_linear_copy(&mut self, source: &Expr, checked: &HirExpr)` —
    the one rule "a linear value may leave its binding only through `move`",
    applied at `const`/`var` initialisers and assignment right-hand sides
  - `alloc(i64) -> *mut i64` and `dealloc(*mut i64)` builtins. **Divergence from
    ledger L3**, which says `alloc(i64) -> *mut u8`: Otter has no `u8`, and
    without indexing or pointer casts a `*mut u8` could not be used at all. The
    master plan's L3 row is updated to `*mut i64` in the same change.
  - Errors: "value `buf` has already been moved" naming the binding, and "cannot assign linear value `buf` without `move`"

- [x] **Step 1: Write the failing tests**

`crates/otter-sema/tests/check.rs`, 15 tests: `a_pointer_type_prints_as_a_mutable_pointer`,
`alloc_returns_a_pointer_to_i64`, `a_pointer_annotation_and_alloc_agree`,
`a_pointer_cannot_be_initialised_from_an_integer`,
`assigning_a_linear_binding_without_move_is_an_error`,
`moving_a_linear_binding_is_legal_and_releases_it`,
`reading_a_moved_binding_is_an_error`,
`a_second_dealloc_of_the_same_pointer_is_an_error`,
`rebinding_a_var_releases_the_previous_linear_value`,
`rebinding_a_var_from_another_linear_binding_needs_move`,
`dealloc_wants_a_pointer_not_an_integer`,
`freeing_in_both_branches_of_an_if_is_legal`,
`a_move_in_only_one_branch_is_conservatively_moved`,
`moving_a_copy_value_is_a_no_op`,
`a_linear_argument_to_a_user_function_is_moved_out`, and
`a_linear_argument_does_not_have_to_be_a_name`.

Two are additions beyond this plan's sketch, and both exist because the sketch
had a hole:

- `freeing_in_both_branches_of_an_if_is_legal` — checking `then` and `else`
  sequentially from one running state would leave `then`'s moves visible to
  `else`, rejecting `if c { dealloc(buf); } else { dealloc(buf); }`. Branch
  checking snapshots the entry state, checks each branch from it, and merges by
  union.
- `a_linear_argument_to_a_user_function_is_moved_out` — a parameter of linear
  type takes ownership, so calling such a function moves the argument. Without
  this rule `dealloc` would need a special case; with it, `dealloc` is just a
  function whose parameter is `*mut i64`.

- [x] **Step 2: Run to verify they fail**

Run: `cargo test -p otter-sema` → FAIL — `ownership` module missing.

- [x] **Step 3: Add `Ptr` and the ownership state**

Add `TypeKind::Ptr(Box<Type>)` with `Display` as `*mut T`. The parser gains
`*mut T` in `parse_type_expr` (`TypeExpr::Ptr`), rejecting anything but `mut`
after the `*`. `linearity_of` is one match; the `Slice` arm is a commented
placeholder so M6 has an obvious slot. `Scope::declare` records `ownership:
Live`. `Scope::move_out` flips to `Moved` and errors on a second move.
`check_live` is called at every identifier read. `alloc`/`dealloc` are
predeclared builtins (see the divergence note above).

- [x] **Step 4: Wire `move` and assignment checking**

`move` was already parsed in M0–M2 as a unary operator producing a
`HirExpr::Move`; what changed is that it now calls `Scope::move_out` when its
operand is a bare name of linear type. `move` of a copy value is a no-op rather
than an error — being explicit about intent is not a mistake. On assignment of a
linear value to a new binding, `forbid_linear_copy` requires `move`. On rebinding
an existing `var`, `rebind` installs a fresh owner, so
`buf = alloc(32); dealloc(buf);` is legal. Reading a `Moved` binding always
errors — including in a `defer`.

- [x] **Step 5: Run to verify they pass**

Run: `cargo test -p otter-sema` → PASS (56 tests, up from 41).

- [x] **Step 6: Commit**

```bash
git add crates/otter-sema/ crates/otter-parser/ crates/otter-codegen/src/ir.rs
git commit -m "feat(sema): pointer types and linear ownership checking"
```

---

### Task 3: Allocation and move codegen

**Files:**
- Create: `crates/otter-codegen/src/memory.rs`
- Modify: `crates/otter-codegen/src/expr_gen.rs` (`alloc`/`dealloc` call sites, `move`)
- Modify: `crates/otter-codegen/src/stmt_gen.rs` (release-on-overwrite), `module.rs` (declarations)
- Modify: `crates/otter-sema/src/hir.rs`, `check_expr.rs` (`release_previous`)
- Create: `crates/otter-codegen/tests/memory.rs`, `tests/golden/memory.ot`, `tests/golden/memory.expected.ll`

**Interfaces:**
- Consumes: `gen_expr`, `FnCtx` from M0–M2; ledger L1 M3 symbols
- Produces:
  - `pub fn gen_alloc(ctx: &mut FnCtx, args: &[HirExpr], ret: &TypeKind) -> GenValue` — scales the element count by the element size and emits `%t = call i8* @otter_alloc(i64 <bytes>, i64 8)`. `alloc(n)` grows in *elements*: the result is a `*mut i64`, so `alloc(16)` is room for sixteen `i64`s.
  - `pub fn gen_dealloc(ctx: &mut FnCtx, args: &[HirExpr]) -> GenValue` — emits `call void @otter_free(i8* <ptr>)` and produces `()`.
  - `pub fn gen_move(ctx: &mut FnCtx, inner: &HirExpr, ty: &TypeKind) -> GenValue` — evaluates the operand as a read would, then **poisons the source slot** with a null `i8*` when the operand is a linear binding, so a backend bug cannot hand out a second copy of a pointer the program already gave away.
  - `HirStmt::Assign { release_previous: bool, .. }` — set by the checker to true only when the overwritten binding still owned a value, so the backend releases the old pointer and never double-frees.

  The plan's original sketch had `gen_addr_of_slot(name) -> Option<String>` "the slot address for a binding, `None` if moved". It is not implemented, and it is not needed: poisoning already makes a stale slot load a null, and `AddressOf` does not exist in the language, so the function would have had no caller.

- [x] **Step 1: Write the failing tests**

`crates/otter-codegen/tests/memory.rs` — a body-lowering harness through `gen_block`, then
`alloc_counts_elements_and_asks_for_i64_alignment`, `dealloc_is_one_call_and_no_value`,
`move_poisons_the_source_slot`, `move_of_a_copy_value_does_not_poison`,
`rebinding_a_live_linear_var_releases_the_old_pointer`,
`rebinding_after_a_dealloc_does_not_free_the_same_pointer_twice`,
`a_deallocated_slot_is_not_released_by_a_later_rebind`, and the `memory` golden.

- [x] **Step 2: Run to verify they fail**

Run: `cargo test -p otter-codegen --test memory` → FAIL — `memory` module missing.

- [x] **Step 3: Implement `memory.rs`**

Alignment is the `i64` layout (8) because `*mut i64` is M3's only pointer type. `gen_move` stores the value, then writes a null into the source slot: deliberate belt and braces, since the linearity checker has already made a second use an error.

- [x] **Step 4: Run to verify they pass**

Run: `cargo test -p otter-codegen` → PASS (8 new tests).

- [x] **Step 5: Add and read the golden file**

`memory.ot` exercises alloc, move, dealloc, and overwriting a live binding. Generated `memory.expected.ll` with `UPDATE_GOLDEN=1` and read it back: the null-poisoning store is present, and the overwrite of `second` loads and frees the old pointer before storing the new one — three frees for three allocations, and no free for the binding that was already released.

  The plan's `defer_dealloc_captures_the_pointer_at_registration` test was not written here, because at this step the codegen could not yet lower a `defer` body that a linear binding had been moved into — see Task 5c, which fixed that. The release-on-overwrite tests cover the same "which pointer is freed" question with a construct the language already lowers.

- [x] **Step 6: Commit**

```bash
git add crates/otter-codegen/ crates/otter-sema/ crates/otter-driver/
git commit -m "feat(codegen): allocation, deallocation, and move lowering"
```

---

### Task 4: Arena codegen

**Files:**
- Create: `crates/otter-codegen/src/arena.rs`
- Create: `crates/otter-codegen/tests/golden/arena.ot`, `tests/golden/arena.expected.ll`

**Interfaces:**
- Consumes: `memory.rs` from Task 3, ledger L1 arena symbols
- Produces: `pub fn gen_arena_new`, `gen_arena_alloc`, `gen_arena_free_all` mirroring the alloc trio, plus `Arena` as a pointer type in `otter-sema`'s builtin table with methods `alloc(i64) -> *mut u8` and `free()`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn arena_new_emits_a_call_to_otter_arena_new() {}
#[test]
fn arena_alloc_calls_otter_arena_alloc_with_the_arena_pointer() {}
#[test]
fn arena_free_all_is_not_called_automatically() {
    // A program that creates an arena and never frees it emits exactly one
    // arena_new and zero arena_free_all — the language does not insert a
    // release the programmer did not write.
}
```

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement**

Arena values are ordinary pointer-typed bindings. `arena.free()` calls `otter_arena_free_all` once; the sema marks the arena binding as moved afterwards so a second `free` is an error rather than a double free.

- [ ] **Step 4: Run to verify they pass** → PASS.

- [ ] **Step 5: Golden file** — generate, read, confirm one `arena_new` per `Arena.new()`.

- [ ] **Step 6: Commit**

```bash
git add crates/otter-codegen/
git commit -m "feat(codegen): arena allocation"
```

---

### Task 5a: String operations without refcounting (delivered as the M3 interim)

The tasks below give strings the design's length-prefixed, refcounted
representation, which is what provides the release points. What ships first is
the *operations*, over the representation the backend already has — an `i8*` to
NUL-terminated bytes — so a program can build and compare text while the memory
work finishes:

- Runtime: `otter_str_len`, `otter_str_byte_at`, `otter_str_concat`,
  `otter_str_from_i64`, `otter_str_from_f64`, `otter_str_from_bool`, `otter_str_eq`
  (`crates/otter-runtime/runtime.c`, declared in the header and in
  `otter-runtime/src/lib.rs`). Results are built in one internal arena that is
  released at process exit; nothing is individually freed. `from_bool` takes an
  `i64` where the design says `i1`, because C has no one-bit type and a mismatched
  declaration is worse than a widened argument.
- Sema: `+` on two `string`s is concatenation; `==`/`!=` already type-checked and
  now compare contents; `len(string) -> i64` is a builtin.
- Codegen: `crates/otter-codegen/src/strings.rs` emits one runtime call per
  operator, dispatched from `gen_binary` (before the numeric table, so that table
  stays about numbers) and from `gen_call`.
- Tests: 11 in `crates/otter-runtime/tests/strings.rs`, 6 in
  `crates/otter-codegen/tests/string_ops.rs`, and an end-to-end
  `tests/programs/strings.ot` that concatenates across a function boundary.

The leak is the documented interim (master plan ledger L1's M3 note, ledger L4,
and the spec's §5.4 note): a built string has no owner to release, because the
value carries no refcount and the language exposes no `release`; a refcount
without a release is worse than no refcount. Nothing here is undone by Tasks 5–7:
they replace the *representation* underneath these same operators, which is why
the tests for `+`, `==` and `len` are written against the operations rather than
the layout.

### Task 5b: `main` returns `i32` (a defect found while testing strings)

Writing the end-to-end string test exposed a latent bug. `fn main()` was emitted
as `define void @main()`, and a `void main` leaves the process exit status as
whatever the last call happened to leave in the return register. The string
program's last statement is a comparison, which returns 1, so it printed exactly
the right output and exited 1.

- `emit_program` now routes `main` through `gen_main`
  (`crates/otter-codegen/src/fn_gen.rs`): `define i32 @main()` for both entry
  signatures, `ret i32 0` on every `return;` and on falling off the end, and a
  `trunc` for an `i64`-returning `main` whose value is wider than an exit code.
- `check_main_exists` also rejects a `main` that declares parameters: the C
  runtime calls the entry point with its own argument convention and the symbol
  this backend emits takes none, so such a `main` is a symbol the loader cannot
  call rather than a program.
- Deterministic guard: `a_unit_main_exits_zero_even_when_its_last_call_returns_nonzero`
  in `crates/otter-driver/tests/e2e.rs` and four IR tests in
  `crates/otter-codegen/tests/codegen.rs`.

### Task 5c: `defer` captures the values it will read (a double free, fixed)

Writing a deferred `dealloc` test exposed the real defect in M0–M2 Task 12's
`defer`. The checker validates a deferred body when the `defer` statement is
**reached**; the codegen runs it at **scope exit**. Those are different moments,
and this program makes them disagree:

```otter
var buf = alloc(16);
defer { dealloc(buf); }   // the checker's `buf` is live here (the move inside
                          // the body is applied at registration) ...
buf = alloc(32);          // ... so this rebind emits no free of the old block
dealloc(buf);             // and this releases the second block
```

At exit the deferred body loaded `buf`'s *slot* — the second block — and handed
it to `otter_free` again. The process aborted with `STATUS_HEAP_CORRUPTION`
(`0xC0000374`). Ownership checking was doing its job; the lowering was not.

The fix makes the two moments agree, and its rule is the spec's §5.5 "`defer`
captures at registration time":

- `crates/otter-codegen/src/stmt_gen.rs`: `defer` keeps a `Deferred { body,
  captured }` rather than a bare body. At registration, every **linear** binding
  in scope is copied into a snapshot slot of its own (sorted by name, so
  `HashMap` order never reaches the IR); `run_defers` installs those slots into
  `FnCtx::locals` while the body runs and restores the previous entries
  afterwards. So the deferred `dealloc` releases the block that was live at
  registration, and the body's own `dealloc` releases the rebind: one free per
  allocation.
- Copy-typed bindings are deliberately not captured. They carry no ownership, so
  a stale read cannot free anything twice, and snapshotting every integer in
  scope at every registration would cost a load and a store per binding — much
  more than the rule is worth inside a loop. A deferred body that reads an `i64`
  still sees its exit-time value; recorded in ledger L5.
- Tests: `a_deferred_free_targets_the_value_live_at_registration`,
  `a_defer_captures_only_linear_bindings`, and
  `a_defer_inside_a_loop_snapshots_once_per_iteration` in
  `crates/otter-codegen/tests/memory.rs`; `defer_runs_at_scope_exit_and_sees_registration_time_values`
  and `a_deferred_free_after_a_rebind_does_not_double_free` in
  `crates/otter-driver/tests/e2e.rs`, the second being the program above that
  used to corrupt the heap.

M4's Task 10 replaces the static exit sequence with a `cleanup` landing pad, so
the capture moves with it — a landing pad reads the same snapshot slots.

### Task 5d: reading input (`read_line`, `parse_i64`, `parse_f64`)

Not in the original task list, and not strictly memory or faults — but every
program the milestone sets out to make possible (the string builder, the buffer
sum) needs input to be *about* something, and until this landed a program could
print and nothing else. Between them they make the compiler usable for a program
that takes input, which is the bar "people can use it" actually sets.

- Runtime: `otter_read_line`, `otter_parse_i64`, `otter_parse_f64`
  (`crates/otter-runtime/runtime.c`). The line is built in the same never-freed
  string arena as every other produced string, so it needs no release point and
  no ownership rule of its own.
- Sema: three more entries in the builtins table (nine now). `read_line` takes
  nothing and yields a `string`; the parsers take a `string`.
- Codegen: only the symbol differs — the Otter name is `read_line` and the
  runtime's is `otter_read_line` — so `gen_call`'s common path gained a rename
  (`gen_plain_call`, factored out of the default arm rather than duplicated).
- Tests: 6 in `crates/otter-runtime/tests/input.rs` for the parse contract, one
  IR test in `crates/otter-codegen/tests/codegen.rs`, and
  `a_program_can_read_its_input` in `crates/otter-driver/tests/e2e.rs`, which
  pipes lines to `tests/programs/adder.ot` through a real pipe.

Two contracts, stated once and repeated in the spec §8.2 and ledger L1: the line
at end of input is the empty string, and a string that is not *wholly* a number
parses to zero (`parse_i64("12x")` is 0, not 12 — trailing text is rejected,
overflow folds into the same answer). Both are losses that a checked form
replaces in M4; neither is a wrong answer presented as a right one.

### Task 5e: reading a struct field (`p.x`)

The M0–M2 Task 12 plan says "`StructLit` allocs an aggregate slot and stores each
field by index; `FieldAccess` loads from the aggregate's declared field index".
The first half shipped differently (a struct value is built with `insertvalue`
into `undef`, not an alloca) and the second half did not ship at all: `FieldAccess`
lowered to a comment plus a zero of its type, so a program could construct a
struct and never read it. Field access is the thing that makes a struct useful,
so it lands here rather than waiting.

Three pieces, and only the middle one was missing:

- **The parser already produced the node.** A dotted name is one
  `Ident(Vec<String>)` for the whole path (`p.x` → `["p", "x"]`), which is why
  `move p.x` needed a rewrite in `parse_move` to stop the path collapsing into one
  name. Field access and a dotted module path are the same shape, and only the
  types can tell them apart, so the resolution belongs in the checker.
- **Sema** resolves a dotted name into a `FieldAccess` chain
  (`check_field_path`): the first segment goes through the ordinary name path — so
  a moved or unbound base gets the diagnostic it would get alone — and every later
  segment is a field of the struct the previous value has. `a.b.c` is that step
  repeated. The single-level `Expr::FieldAccess` arm (produced by `move p.x`)
  resolves through the same rule. This is also where the field's **index** is
  resolved, against `HirStruct.fields`, and carried on `HirExpr::FieldAccess`.
- **Codegen** reads the field with `extractvalue`, one instruction, no address:
  the index is already the declaration-order position, and the aggregate type
  spells as `%struct.Name` (ledger L1's type table).

One correctness fix came with it. A struct literal's fields used to reach the HIR
in the order the *literal* wrote them, and `define_named_type` keeps only the
first shape it sees — so `P { y: 4, x: 3 }` would have defined `%struct.P` with
`y` first and an index that means a different field than the same struct built
`P { x: 3, y: 4 }`. Sema now orders a literal's fields by declaration, which makes
the index mean one thing for every literal of one struct. Recorded in ledger L2.

Tests: `a_struct_field_is_read_by_name_whatever_order_it_was_written` and
`tests/programs/structs.ot` (out-of-order literal, a field of a field, and a
struct crossing a call boundary by value) end to end; six sema tests pinning the
field's type, its declaration index, a nested chain, an unknown field, a
non-struct base, and the literal's declaration order; one codegen test pinning
`extractvalue %struct.P ..., 0` and `..., 1` with the aggregate spelled
`{ i64, double }` from a literal written `y, x`.

Not here: a struct behind a pointer (`*mut P`), `p.x = 1` on a live binding, and
nested struct *mutation*. Reading a field of a value is the whole of this task.

### Task 5f: the string arena is per thread (a data race, fixed)

Found by the suite, not by reading: after Task 5d added tests that built strings,
`cargo test -p otter-runtime` failed about one run in five, always as one string
test reading another test's bytes (`left: "012"`, `right: "xyz"`). Single-threaded
it never failed, which is the signature of a race rather than a logic error.

The cause was the string arena. `otter_string_arena` published **one** arena
through a plain `static` pointer, and `otter_arena_alloc` is an unsynchronized
bump: it writes `arena->head` and `block->used` with no lock, deliberately, so
that allocation is a pointer increment. Two threads allocating at once could take
the same block, and then one string's bytes were the other's.

The fix is per-thread state rather than a lock — `OTTER_THREAD_LOCAL` (a macro for
`__declspec(thread)` on MSVC and `__thread` elsewhere) on the arena slot. This
removes the sharing instead of guarding it, so the path to a string gains no lock;
and because a thread's arena is never freed, a string built on one thread and read
on another is still valid.

The public `otter_arena_*` API keeps its contract and now states it: **single
threaded** — a program that shares one arena across threads must serialize the
calls (header, `include/otter_runtime.h`). The runtime's other `static`s were
audited at the same time: the stdout-mode flag became thread-local too, because an
unsynchronized `static` written by two threads is a data race even when the work it
guards (`_setmode`, idempotent) is harmless to repeat. `otter_alloc`/`otter_free`
are `malloc`/`free`, which are already thread-safe.

Test: `concurrent_builders_never_share_a_string_block` — sixteen threads each
build two hundred strings and check every result. Against the shared arena it
fails almost every run; it is what keeps the flakiness from returning.

Not here: making the *public* arena safe to share (it stays a caller's problem,
documented), and `%otter_str`'s refcount, which M3's remaining string task
replaces wholesale anyway.

### Task 5g: reaching the value-to-string conversions

The runtime has had `otter_str_from_i64`, `otter_str_from_f64` and
`otter_str_from_bool` since the M3 string work (Task 5a), all three declared in
the emitted IR from the start — and nothing in the language could call them. So a
program could print a number (`print_i64`) and could not put one *inside* a
string: `"count = " + n` did not type-check and there was no other spelling. It
left a hole in exactly the thing strings are for.

Three more entries in the builtins table close it: `str_from_i64`,
`str_from_f64`, `str_from_bool`, all `-> string`. The names are the runtime's
minus the prefix, so the codegen rename path (`read_line` → `otter_read_line`)
covers two of them with no new code; the third needs one arm, because the runtime
takes an `i64` where the language has a `bool` (C has no one-bit type, argued in
the header) and the `i1` is zero-extended to meet the declaration. `str_from_bool`
is the only conversion whose IR signature is not the language signature.

Three names rather than one `str` is forced, not chosen: a builtin declares its
name once in the root scope and a duplicate declaration is an error, so there is
no overloading to hang three argument types on. **Task 7's interpolation is the
ergonomic form of this** — `"n = ${n}"` — and it lowers to exactly these three
conversions, which is why they are the right thing to build first: interpolation
becomes sugar over a working operation rather than the only way to get one.

Tests: `the_value_to_string_conversions_resolve_and_check_their_argument`
(each takes exactly one type; a wrong argument is a diagnostic), one sema test
pinning that a built string type-checks at all, one codegen test pinning the two
direct calls and the `zext` on the bool, and `greet.ot` end to end — read a name
and an age, print one sentence built from both, which is the whole reason the
pair is needed.

### Task 5h: `while`, and the empty-body misparse it exposed

`for ;; { ... if done { break; } }` is how the language spelled an unbounded loop,
and it reads badly: nothing about the loop is a `for`, and the condition sits at
the bottom instead of the top. `while cond { }` is the missing spelling. It is a
lexer keyword plus a parser rule that builds the **existing** `Stmt::ForC` with no
init and no update, so the checker and the backend never learn the keyword exists;
the pin is
`while_is_the_condition_only_for_loop`, which asserts the node and that the two
optional parts are empty.

Writing the test with an empty body (`while flag { }`) failed, and the failure was
the interesting part. The parser decides a `{` is a struct literal when it is
`{}` or `{ field: ... }` — a heuristic, not a grammar rule, because `p.x` and a
module path are one AST shape and the arms of a `switch` whose subject is a bare
identifier must not be eaten. With `{}` counted as a literal, `while flag { }`
parsed as `flag {}` and then reported "expected `{`, found `}`" at the closing
brace of the enclosing function. `if flag { }`, `for i in 0..n { }` and an empty
`switch` arm body were all broken the same way, and had been since the heuristic
was written — they were invisible because every test used a non-empty body.

The fix is to drop the empty form from the heuristic. **Every struct declares at
least one field** (pinned by `struct_requires_at_least_one_named_field`), so `X {}`
is never a program; `{}` after a bare name is always a block. The cost is that an
empty struct literal is now a parse error rather than a "missing field" checker
error, and it is not a cost: there is nothing a program could have meant.
`an_empty_body_after_a_bare_name_is_a_block_not_a_struct_literal` pins all three
shapes plus the span, and a sema test pins that the checker then accepts them.

This is a syntax change beyond the plan, recorded in the spec's section 3.1 and
3.3 rather than here.

---

### Task 5: Refcounted strings in the runtime

**Files:**
- Modify: `crates/otter-runtime/runtime.c`, `include/otter_runtime.h`
- Modify: `crates/otter-runtime/tests/memory.c`
- Create: `crates/otter-runtime/tests/strings.c`

**Interfaces:**
- Consumes: Task 1 runtime
- Produces: the string half of ledger L1, plus
  - `typedef struct otter_str { int refcount; int64_t len; char* data; } otter_str;` in the header, matching ledger L2's `{ i32, i64, i8* }`
  - `i64 otter_str_count(otter_str*)` — test-only introspection so a refcount-balance test can assert exact counts rather than guessing

- [ ] **Step 1: Write the failing C tests**

`strings.c`: create from bytes, assert `len` and contents; retain twice then release twice and assert `otter_str_count` returns 1 in between; release to zero and assert the allocation was actually freed (via a malloc interposer or by running under a leak check); concat two strings and assert contents; `otter_str_from_i64(-42)` yields `"-42"`; `otter_str_from_bool(1)` yields `"true"`.

- [ ] **Step 2: Run to verify they fail** → FAIL — symbols missing.

- [ ] **Step 3: Implement**

`otter_str_new` copies the bytes and sets `refcount = 1`. `otter_str_release` decrements and frees when it reaches zero; it must tolerate a null pointer. `otter_str_concat` allocates a fresh string, so the result is a new owner and the operands are untouched. The three `from_*` converters format into a bounded stack buffer then allocate.

- [ ] **Step 4: Run to verify they pass** → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-runtime/
git commit -m "feat(runtime): refcounted string primitives"
```

---

### Task 6: String types and operations in sema and codegen

**Files:**
- Modify: `crates/otter-sema/src/ty.rs` (string methods table)
- Modify: `crates/otter-sema/src/check_expr.rs`
- Create: `crates/otter-codegen/src/strings.rs`
- Modify: `crates/otter-codegen/src/module.rs` (emit the new `%otter_str` definition)
- Modify: `crates/otter-codegen/tests/golden/ops.expected.ll`
- Create: `crates/otter-codegen/tests/golden/strings.ot`, `tests/golden/strings.expected.ll`

**Interfaces:**
- Consumes: ledger L1 string symbols, ledger L2's widened `%otter_str`
- Produces:
  - `pub struct StringMethods` in `otter-sema`, predeclared in the root scope:
    `len() -> i64`, `byte_at(i64) -> i64`, `concat(string) -> string`, `to_i64() -> Option[i64]`, `retain()`, `release()` (explicit for symmetry with the compiler's own analysis)
  - `pub fn gen_string_literal(ctx: &mut FnCtx, bytes: &[u8]) -> String` — interns the global, allocates via `otter_str_new`, returns the `%otter_str*` value
  - `pub fn gen_str_concat(ctx: &mut FnCtx, lhs: &str, rhs: &str) -> String`
  - `pub fn gen_str_release(ctx: &mut FnCtx, str_value: &str) -> ()`
  - `Module::finish` emits `%otter_str = type { i32, i64, i8* }` — **the layout change from ledger L2, made exactly here**

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn string_literal_emits_a_global_a_copy_and_a_refcount_of_one() {
    // IR must contain a private constant, a call to otter_str_new, and the
    // resulting %otter_str* value.
}

#[test]
fn assigning_a_string_retains_and_dropping_releases() {
    // "const a = \"x\"; const b = a;" -> a retain for b; end of scope -> releases
    // for both. Exact balance asserted by counting calls in the golden IR.
}

#[test]
fn concat_emits_one_call_and_does_not_touch_its_operands() {
    // IR contains one otter_str_concat and zero releases of the operands.
}

#[test]
fn scope_exit_releases_every_string_binding_in_reverse_declaration_order() {}

#[test]
fn byte_at_out_of_range_returns_zero_not_a_trap() {
    // Documented behaviour, ledger L1 otter_str_byte_at.
}

#[test]
fn the_otter_str_layout_changed_exactly_once() {
    // Assert the module emits { i32, i64, i8* } and not the M0-M2
    // { i32, i32, i8* }. This test is the guard for ledger L2.
}
```

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement sema method dispatch**

Add the methods table to the root scope. `len`, `byte_at`, `concat`, `to_i64` are ordinary pure methods; `retain` and `release` are builtin verbs. A `string` binding is a *copy* type at the language level but every assignment emits a retain and every scope exit a release — this is the compiler's job, and the linearity checker deliberately does **not** apply to strings (ledger L5 lists only `Ptr` and, later, `Slice`).

- [ ] **Step 4: Implement codegen**

Change the `%otter_str` type. Emit a retain after every assignment or argument copy and a release at every scope exit, in reverse declaration order. Interprocedurally, a function returning a `string` transfers ownership to the caller and must not release it.

- [ ] **Step 5: Regenerate `ops.expected.ll` and read it**

The M0–M2 `ops.ot` golden file now compiles differently because the `Str` layout changed. Regenerate, then **read the whole diff** and confirm the only changes are the layout line and the new global. Ledger L4 says this is a milestone boundary — the diff must look exactly like that.

- [ ] **Step 6: Add and read `strings.expected.ll`**

- [ ] **Step 7: Commit**

```bash
git add crates/otter-sema/ crates/otter-codegen/
git commit -m "feat: refcounted strings, retype %otter_str, retain/release balancing"
```

---

### Task 7: String interpolation

**Files:**
- Modify: `crates/otter-lexer/src/lexer.rs` (recognise `${` inside string literals)
- Modify: `crates/otter-parser/src/exprs.rs` (build an interpolation node)
- Modify: `crates/otter-sema/src/check_expr.rs`
- Create: `crates/otter-codegen/src/interp.rs`
- Create: `crates/otter-codegen/tests/golden/interp.ot`, `tests/golden/interp.expected.ll`

**Interfaces:**
- Consumes: `gen_str_concat` from Task 6
- Produces: `HirExpr::Interp { segments: Vec<InterpSegment>, span }` where `InterpSegment` is `Literal(Vec<u8>)` or `Expr(HirExpr)`. `pub fn gen_interp(ctx: &mut FnCtx, segs: &[InterpSegment]) -> String` lowers left to right, converting each non-string operand with the matching `otter_str_from_*`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_literal_only_interpolation_is_a_plain_string() { /* "${}" is not valid */ }

#[test]
fn interpolation_with_one_i64_operand_converts_then_concatenates() {
    // "\"far: ${d}\"" -> from_i64 then concat, in that order.
}

#[test]
fn mixed_operands_convert_by_their_own_type() {
    // A string, an i64, and a bool in one literal each use their converter.
}

#[test]
fn an_unterminated_interpolation_is_a_diagnostic_not_a_panic() {
    // "\"a ${x" -> Err.
}

#[test]
fn an_escaped_dollar_is_literal() {
    // "\"\\${x}\"" -> literal text "${x}", no interpolation.
}
```

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement lexing**

Extend `StringLit` to carry segments rather than a flat `String`, or add a parallel `InterpLit`. `\\` escapes and `\${` produce literal text.

- [ ] **Step 4: Implement lowering**

Convert then concat, left-associated. A `string` operand needs no conversion — it is already one.

- [ ] **Step 5: Run to verify they pass** → PASS. Add and read the golden file.

- [ ] **Step 6: Commit**

```bash
git add crates/
git commit -m "feat: string interpolation with typed conversion"
```

---

### Task 8: End-to-end memory programs

**Files:**
- Create: `crates/otter-driver/tests/programs/{buffer,builder}.ot`
- Modify: `crates/otter-driver/tests/e2e.rs`
- Create: `crates/otter-driver/tests/leaks.rs`

**Interfaces:**
- Consumes: everything above plus the M0–M2 driver
- Produces: two end-to-end programs and a leak-detection harness

- [ ] **Step 1: Write the failing tests**

`buffer.ot` fills a manual `*mut i64` of 1024 elements in a loop and sums it, releasing with `defer`. `builder.ot` builds a string 100 times in a loop by concatenation into a fresh string each time. Both print their result.

`tests/leaks.rs` runs each program under a leak check: on Linux via `valgrind --leak-check=full --error-exitcode=9`, on Windows via the CRT debug allocator in a debug build. The test asserts exit code 0 and no leak report. **Skip with a printed notice, not a silent pass, if no checker is installed.**

- [ ] **Step 2: Run to verify they fail** → FAIL — programs do not compile yet.

- [ ] **Step 3: Write the programs and run them**

Run: `cargo test -p otter-driver` → PASS, with both programs printing their expected values.

- [ ] **Step 4: Commit**

```bash
git add crates/otter-driver/
git commit -m "test: end-to-end memory programs with leak detection"
```

---

### Task 9: `panic` and the diagnostic path

**Files:**
- Modify: `crates/otter-runtime/runtime.c`, `include/otter_runtime.h`
- Create: `crates/otter-codegen/src/panic.rs`
- Modify: `crates/otter-codegen/src/expr_gen.rs`
- Modify: `crates/otter-driver/src/pipeline.rs`

**Interfaces:**
- Consumes: ledger L1 `otter_panic`
- Produces: `pub fn gen_panic_block(ctx: &mut FnCtx, message: &str, span: Span) -> String` returning a label name for a block that calls `otter_panic` and is `unreachable` after; `HirStmt::Panic`; `builtin panic(string)` as a never-returning function whose `Type` is `Never`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn panic_emits_a_call_with_message_and_source_location() {
    // IR contains otter_panic with the message bytes and the span's
    // line and column from the SourceMap.
}

#[test]
fn a_call_to_panic_is_followed_by_unreachable() {}

#[test]
fn panic_from_within_a_function_propagates_to_the_panic_main() {
    // "fn f() { panic(\"x\"); } fn main() { f(); }" -> both functions route to
    // one panic function.
}

#[test]
fn the_panic_message_is_rendered_before_the_stack_trace() {}
```

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement**

One module-level `@otter_panic_entry` function shared by every panic site, taking the message and a source location. Every `fn` that can panic gets `nounwind` on nothing and a call to it.

- [ ] **Step 4: Run to verify they pass** → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/
git commit -m "feat(codegen): panic with source location"
```

---

### Task 10: Unwinding, and `defer` on the unwind path

**Files:**
- Create: `crates/otter-codegen/src/unwind.rs`
- Modify: `crates/otter-codegen/src/fn_gen.rs` (landing pads)
- Modify: `crates/otter-codegen/src/stmt_gen.rs` (static defer becomes a cleanup landing pad)
- Modify: `crates/otter-runtime/runtime.c` (personality routine, landing pad helpers)
- Create: `crates/otter-codegen/tests/golden/unwind.ot`, `tests/golden/unwind.expected.ll`

**Interfaces:**
- Consumes: Task 9's panic entry; ledger L4's defer row
- Produces:
  - `pub fn emit_personality() -> String` — the `otter` personality routine, emitted once per module
  - `pub fn emit_landing_pad_cleanup(ctx: &mut FnCtx, defers: &[String]) -> String`
  - The personality routine is registered with `uwtable` and referenced by a module-level `!llvm.module.flags` `uwtable` entry

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn every_function_that_can_panic_emits_a_landing_pad() {}

#[test]
fn defer_lowering_becomes_a_cleanup_landing_pad_not_a_pre_return_sequence() {
    // This is ledger L4's M4 replacement, and Review Focus item 1. The golden
    // IR must show `landingpad` and the deferred calls inside it.
}

#[test]
fn a_return_path_no_longer_duplicates_the_deferred_calls() {
    // After M4 the deferred calls appear exactly once, in the landing pad.
}

#[test]
fn the_personality_routine_is_emitted_exactly_once() {}
```

- [ ] **Step 2: Run to verify they fail** → FAIL — unwinding is not implemented, so a panicking program currently aborts.

- [ ] **Step 3: Implement the runtime side**

Port the two-phase personality routine from `rustc`'s `libunwind`: phase 1 returns the landing pad for an exception, phase 2 resumes after cleanup. `cleanup` landing pads call the deferred functions and then `resume`.

- [ ] **Step 4: Implement codegen**

Every `call` that can panic becomes an `invoke`. Every scope with pending defers gets a `cleanup` landing pad. Blocks that cannot unwind are marked `nounwind` so LLVM elides the machinery — this is where the performance comes from, and getting it wrong makes every function slow.

- [ ] **Step 5: Run to verify they pass** → PASS.

- [ ] **Step 6: Read `unwind.expected.ll` carefully**

Confirm `landingpad`, `cleanup`, and the personality declaration are all present and the ordering of the deferred calls is reversed relative to registration.

- [ ] **Step 7: Commit**

```bash
git add crates/
git commit -m "feat(codegen): unwind landing pads and defer cleanup on unwind"
```

---

### Task 11: Checked arithmetic

**Files:**
- Modify: `crates/otter-codegen/src/expr_gen.rs`
- Create: `crates/otter-sema/src/builtins.rs` (wrapping/checked/saturating declarations)
- Regenerate: `crates/otter-codegen/tests/golden/ops.expected.ll`
- Create: `crates/otter-codegen/tests/golden/checked.ot`, `tests/golden/checked.expected.ll`

**Interfaces:**
- Consumes: `gen_panic_block` from Task 9; ledger L3 M4 builtins; ledger L4's arithmetic row
- Produces: `pub fn gen_checked_binary(ctx: &mut FnCtx, op: BinOp, lhs: &GenValue, rhs: &GenValue, span: Span) -> String` and `pub fn gen_wrapping_binary(...)`, `gen_saturating_binary(...)`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn addition_emits_an_overflow_check_before_the_add() {
    // IR contains an icmp against the add's own result and a conditional
    // branch to a panic block, then the plain `add` with no nsw.
}

#[test]
fn division_emits_a_zero_check_before_the_div() {
    // This replaces the M0-M2 bare `sdiv`. Ledger L4, Review Focus item 1.
}

#[test]
fn wrapping_add_omits_the_check() {
    // "wrapping_add(a, b)" -> plain add, no icmp, no branch.
}

#[test]
fn checked_add_yields_an_option_not_a_panic() {
    // "checked_add(a, b)" -> Option[i64], None on overflow, no panic block.
}

#[test]
fn saturating_add_clamps_instead_of_branching() {}

#[test]
fn boolean_and_floating_arithmetic_are_never_checked() {
    // No overflow check on `and i1` or `fadd double`.
}

#[test]
fn comparison_and_logical_operators_are_unchanged() {
    // Guards against the M4 rewrite accidentally touching non-arithmetic ops.
}
```

- [ ] **Step 2: Run to verify they fail** → FAIL — `sdiv` is still bare.

- [ ] **Step 3: Implement**

Emit the operation into a temp, then `icmp` the temp against the operand to detect wraparound, then `br i1` to the panic block. `Div` checks the divisor for zero first. `checked_*` returns a real `Option` rather than a branch. Modulo follows the same rule as division.

- [ ] **Step 4: Regenerate and read `ops.expected.ll`**

This is the ledger L4 arithmetic replacement. The diff must be exactly the added checks; if the bare `add` opcodes changed shape, something else moved.

- [ ] **Step 5: Run to verify they pass** → PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/
git commit -m "feat: checked integer arithmetic replacing the M0-M2 wrapping contract"
```

---

### Task 12: `assert`, exit codes, and `os.exit`

**Files:**
- Modify: `crates/otter-sema/src/builtins.rs`
- Modify: `crates/otter-codegen/src/expr_gen.rs`
- Modify: `crates/otter-codegen/src/fn_gen.rs` (main's exit code)
- Create: `crates/otter-codegen/tests/golden/assert.ot`, `tests/golden/assert.expected.ll`

**Interfaces:**
- Consumes: ledger L1 `otter_assert_failed`, ledger L3 M4 builtins
- Produces: `assert(bool)`, `debug_assert(bool)`, `assert_eq`/`assert_ne` as builtin forms specialised per type pair, `os.exit(i32)` emitting a call to `exit`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn assert_emits_a_branch_to_assert_failed_carrying_the_expression_source() {}

#[test]
fn debug_assert_disappears_under_the_release_flag() {
    // Compiling with --release removes the branch and the string.
}

#[test]
fn assert_eq_compares_with_the_right_instruction_for_the_type() {
    // i64 -> icmp eq; f64 -> fcmp oeq; bool -> icmp eq on i1.
}

#[test]
fn main_without_a_return_type_exits_zero() {}

#[test]
fn main_returning_i32_sets_the_process_exit_status() {}

#[test]
fn os_exit_is_never_returning() {}
```

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement**

`assert` carries the original expression source text, recovered from the span against the source map, so the runtime message reads `assertion failed: x > 0`. `main` becomes a real `i32`-returning C `main` that calls the user's body and returns its value, or `0`.

- [ ] **Step 4: Run to verify they pass** → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/
git commit -m "feat: assert, debug_assert, exit codes"
```

---

### Task 13: End-to-end fault programs

**Files:**
- Create: `crates/otter-driver/tests/programs/{divzero,overflow,deeprecurse,cleanup}.ot`
- Modify: `crates/otter-driver/tests/e2e.rs`

**Interfaces:**
- Consumes: everything above
- Produces: four fault programs with asserted stderr and exit codes

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn division_by_zero_panics_with_a_caret_and_exits_1() {
    // stderr contains "error:", the message, and a caret line. Not a segfault.
}

#[test]
fn integer_overflow_panics_rather_than_wrapping() {
    // Explicitly asserts the value did NOT wrap, pinning ledger L4's change.
}

#[test]
fn every_pending_defer_runs_before_a_panic_message_is_printed() {
    // This is Review Focus item 1 at its hardest. cleanup.ot registers four
    // defers across three nesting levels and panics from the deepest; assert
    // all four ran, in reverse registration order, before the panic text.
}

#[test]
fn a_fault_deep_in_a_recursion_unwinds_the_whole_stack() {
    // deeprecurse.ot recurses 5000 frames then panics; assert the process exits
    // cleanly rather than overflowing the C stack.
}

#[test]
fn a_successful_program_still_exits_zero_and_leaks_nothing() {}
```

- [ ] **Step 2: Run to verify they fail** → FAIL — programs abort instead of unwinding.

- [ ] **Step 3: Write the programs and run them**

Run: `cargo test -p otter-driver` → PASS. Then run each compiled binary by hand and **read the actual stderr** — a passing assertion on a substring can still hide a garbled caret.

- [ ] **Step 4: Commit**

```bash
git add crates/otter-driver/
git commit -m "test: end-to-end fault, unwind, and cleanup programs"
```

---

## Definition of Done for M3–M4

- `buffer.ot` and `builder.ot` run correctly with no leaks under an installed checker.
- `divzero.ot`, `overflow.ot`, `deeprecurse.ot`, and `cleanup.ot` each exit with a caret diagnostic, never a segfault, and every pending `defer` runs first.
- Ledger L1, L2, L3, L4, and L5 in the master plan match what the code implements. Any divergence is written back into the master plan in the same commit as the code change.
- `ops.expected.ll`'s M0–M2 diff has been reviewed as a whole and contains only the `%otter_str` layout change and the added arithmetic checks.
- The M0–M2 end-to-end programs still compile and produce identical output.
- `cargo test --workspace` passes with no warnings; 120 seconds of fuzzing per target finds no panic.