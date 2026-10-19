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

### Task 5i: compound assignment (`+=`, `-=`, `*=`, `/=`, `%=`)

The lexer has had all five spelling tokens since it was written and nothing
consumed them — the same wart as `@`. They are now the statement forms they were
meant to be, as pure sugar: the parser builds `Stmt::Assign` whose target is the
expression the source wrote and whose value is that same expression as the left
operand of the operator. There is no compound-assignment node, so the checker and
the backend never learn the forms exist, and the desugar cannot disagree with the
expanded spelling because the expanded spelling *is* what everything downstream
sees.

That is also the whole correctness argument: `n += 1.5` is rejected with the
message for `n = n + 1.5` ("cannot apply `+` to `i64` and `f64`"), and
`c += 1` on a `const` is rejected with "cannot assign to `c`, which is
immutable" — for the same reason, since the target rules are the assignment's.
The target being an expression rather than a name means `+=` will keep working
when M6 makes `p.x` and `v[i]` assignable, without reopening the parser.

Tests: `compound_assignment_is_sugar_for_the_expanded_assignment` (the node, the
shared target, the operand order),
`every_compound_operator_maps_to_its_own_binary` (each token reaches its own
operator), a sema test for all five on a `var` plus the two rejections, and
`collatz.ot` now writes `n /= 2`, `steps += 1` and `i += 1` — its expected output
is unchanged, so the end-to-end test covers the sugar without knowing about it.

### Task 5j: `${` is a diagnostic until interpolation exists

> **Superseded by Task 7.** Interpolation is implemented, so the `${`
> *diagnostic* below is gone: the lexer now scans `${ ... }` and the checker
> lowers it. What Task 7 kept from this task is `\$` — the escape for a literal
> `$` — and the reason it exists. The task is kept as written for the record; the
> diagnostic text no longer appears in the compiler.

The design has advertised `"far: ${d}"` since it was written (spec §2 goal 2 and
the §3 example), the compiler had no idea what `${` was, and the result was the
worst kind of failure: `"n = ${n}"` compiled, ran, and printed `n = ${n}`. A
wrong answer presented as a right one, and easy to mistake for success because the
program does not crash and the output looks like the program's own text.

Interpolation is Task 7 and is real work (a lexer that can produce an inner token
stream with correct spans, a node the checker can type per operand, and a lowering
that concatenates conversions). Doing it half-way inside this task would leave the
language in a worse place than it is now. So this task does the part that makes
the current state *honest*: the marker is reserved and reported, at the two bytes
of the `${` itself, with the workaround named —

    string interpolation is not implemented yet: build the string with `+` and a
    conversion such as `str_from_i64`, or write `\$` for a literal `$`

`\$` was added to the escape set in the same task, and not as an afterthought:
once `${` means something, a literal `$` before `{` has to stay expressible, so the
escape is the escape hatch interpolation needs anyway — and Task 7 found exactly
that, so `\$` never had to change meaning.

The check lived in the lexer's literal scan, next to the escape handling, which is
also why it could not misfire on `\$`: the backslash arm consumes two bytes, so an
escaped `$` never reaches the `$` arm. `decode_escape` stays the single definition
of what an escape *means*, and `${` is not an escape, so it is not defined there.

Tests: `interpolation_is_reported_rather_than_printed_through` pinned the
diagnostic, the message naming the workaround, and the two-byte caret; Task 7
replaced it with tests for the feature. `\$` joins the escape table test, which
enumerates the whole set, and still does.

Not here: interpolation itself (Task 7, now delivered).

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

**As built (the representation half, in `547e8e1`).** Task 5 landed in two
halves because the representation change is what made the release half
possible, and the representation is the part that touches every layer at once.

*The layout.* `%otter_str` is `{ i64, i64, i64, i8* }` — refcount, length,
capacity, data — one width for every count and a capacity beside the length.
That is a deliberate step away from ledger L2's sketched `{ i32, i64, i8* }`:
the `int` refcount would make overflow unreachable-by-construction only by
making the struct's size depend on a second type rule, `long long` matches
what the C side already uses for every other count, and the capacity is what
a future `push`/slice will need to avoid re-measuring. One struct, one width,
described once in `module.rs`, once in the header, once in `runtime.c`, and
once in the Rust `OtterStr` the tests build — the tests then fail if any of
the four drift.

*Literals.* `intern_string` emits two globals per literal: the NUL-terminated
bytes (`@.str.N.data`) and the `%otter_str` that points at them with refcount
`-1`. `intern_bytes_ptr` returns the struct global itself, because a `string`
value is a pointer to the struct and the global already is that pointer — the
`getelementptr` the interim needed is gone. A static string is never freed, so
`otter_str_retain` and `otter_str_release` are both no-ops on a literal, which
is what makes the release half uniform.

*No lowered program changed shape.* The runtime's entry points still take and
return `i8*` in the IR (a pointer is a pointer), so `strings.rs`, `expr_gen.rs`
and `faults.rs` emit the same calls; only `module.rs`'s literal emission and
the four C/Rust sides of the layout moved. The print, panic and parser
functions read bytes through `otter_str_data`, which is the one new place a
`const char*` is demanded.

*Tests.* `memory.c`/`strings.c` from the plan's Step 1 were not written: the
runtime's C half is driven from Rust through the crate's `extern`
declarations, which is the same contract the IR calls and the place a
signature that drifts from the header already failed (`crates/otter-runtime/
tests/strings.rs`). Those tests now build real `OtterStr` values, so they test
the refcount and not just the bytes — a built string comes back with count 1,
`retain` counts up and returns its argument, `release` walks to zero, and a
static literal is not counted. The two tests that only made sense against the
arena (an allocation-per-call smoke test, and the thread-local reasoning that
motivated Task 5f) became tests of ownership: a 2,000-string build/release
loop, and sixteen threads building and releasing without reading each other's
bytes.

*As built.* The release half landed in two parts, in the two commits that follow
this one: the temporary release first, then the scope-exit release a binding
needs.

**As built (the temporary half, in the commit that follows).** The release half
landed in the one part of it that is provably safe without scope tracking: a
**temporary the runtime built is released the moment an operation has consumed
it**. The consumers are the calls the runtime implements, and each one copies
what it is given and keeps no pointer — `otter_str_concat`, `otter_str_eq`,
`otter_str_len`, the two parsers, and `otter_print_str` — plus an expression
statement whose value is discarded and the passing path of an
`assert_eq`/`assert_ne`. That set is exactly the operations the checker already
admits on a string, and it excludes the one call that could keep what it was
handed: a *user* function, whose parameter is borrowed and whose body might bind
it. So no `release` is emitted for an argument to a user call, for a field of a
struct literal, or for the value a `var`/`const` stores — those are leaks, and a
leak is not a corruption.

The mechanism is a per-function set of SSA names that hold an owned reference
(`FnCtx::owned_strings`), not a field on `GenValue`. The fact being tracked is
"this exact value is owned", it belongs to the value rather than to the
expression that built it, and a name set states it once without threading a flag
through every construction in the backend. A producer marks its result
(`str_from_*`, `read_line`, a concatenation); a consumer calls
`release_owned`, which emits `call void @otter_str_release` only for a name in
the set. A literal — a static string, refcount -1 — and a name bound to a local
are never in the set, which is what keeps the release from dropping a reference
the program still holds.

What it buys is the common case: a `print_str` of an interpolated number in a
loop builds a conversion and two concatenations per iteration and releases all
three, where before they accumulated for the life of the process. The IR is three
`release` calls inside the loop body, one after each consumer.

**As built (the scope-exit half, in the commit that follows).** The second part is
the one a *binding* needs, and it is where `retain` earns its place. A `string` is
a **copy type**: `var b = a;` has to give the slot `b` a reference of its own,
because overwriting `a` afterwards must not free what `b` still holds. That is the
case that made `retain`-on-bind the safe design rather than a `move` that would
have to prove no other binding aliases the value. So every `string` binding takes
a reference and every binding gives it back:

- A `const`/`var` whose value is a *temporary* takes the reference over — the name
  leaves `owned_strings`, and no consumer will release it. Any other value (a
  literal, whose retain is a no-op; a local; a user call's result) gets
  `otter_str_retain`.
- An assignment `s = e` builds `e` first, secures its reference, and *then*
  releases what `s` held. That order is what makes `s = s + "..."` safe: dropping
  the old value first would free the bytes the concatenation was about to read.
- A `return` of a `string` is taken *before* the scope's locals are released, and
  kept with `otter_str_retain` when it was one of them, so a helper that returns a
  string built from a local hands the caller a live reference.
- A scope releases its own locals on fall-through, and a
  `return`/`break`/`continue` releases the innermost scope's before it branches —
  always *after* that scope's `defer` bodies have run, because
  `defer { print_str(s); }` reads `s` at exit and a `defer` takes no snapshot of a
  copy type the way it does of a linear binding.

The bookkeeping is a stack of slot lists, `FnCtx::scope_strings`, pushed and
popped next to `FnCtx::defers`, so "the scope" means the same thing to both
features — and the one place a deferred body and a release can disagree (which
runs first) is settled by running the defers first.

What it buys is the balance: `var acc = ""; while ... { acc = acc + "z"; }`
releases the previous accumulator on every iteration instead of growing by one
string per iteration for the life of the process. What it still does not buy is
stated rather than hidden, and each item is a **leak, not a corruption**:

- a `string` a *user call* returns is treated as borrowed, because a function may
  return one of its parameters, whose reference the caller still owns; releasing
  the result would be a use-after-free, so it leaks instead;
- a `string` stored in a struct field, or read on the panic path, is not tracked;
- a `break` out of a *doubly* nested block releases only the innermost scope, so
  the scopes between it and the loop leak — the same reading of the same
  limitation the `defer` list already has, where a `break` runs only the current
  scope's defers.

The rule the whole feature follows is that a reference is released only where the
consumer provably does not keep it, and a leak is the failure mode chosen
wherever that cannot be proved.

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

**Status:** delivered. The original sketch below is kept for the record; the
"what was built" note says what changed and why.

**What was built (the deviation, and why):**

- **No `crates/otter-codegen/src/interp.rs`, and no `HirExpr::Interp`.** Choosing
  `str_from_i64` over `str_from_f64` needs the *type* of the interpolated
  expression, which only the checker knows, so the plan's own interfaces were
  already pointing at the checker doing the work. Doing the conversion there —
  desugaring an `Expr::Interp` (AST) into the `str_from_*` call and the `+` the
  language already has — means the typed tree gains no node, code generation
  learns nothing new, and `"n = ${n}"` is provably the same program as
  `"n = " + str_from_i64(n)`. A backend module would have been a second lowering
  of `+` to keep in step with the first.
- **The lexer scans `${ ... }` as a unit** (brace-depth counted, a nested string
  skipped whole), so `"${f("x")}"` does not end the literal at the inner quote,
  and **the split into text and expression pieces is one function**
  (`otter_lexer::split_interpolation`) that both the lexer's validation and the
  parser's tree building call, so the escape rules and the `${` shape are
  defined once.
- **The parser sub-lexes each `${ ... }` expression** over the slice of the
  original file, shifts the sub-token spans back to file offsets, and parses it
  with a sub-parser that borrows the same `SourceMap` and `FileId` — so a name
  inside the interpolation is read out of the file exactly as one outside it is,
  and every diagnostic points at the real text.
- **No golden file.** The HIR shape is pinned in `otter-sema` and the runtime
  shape in `otter-driver`; a golden `.ll` would have pinned the same thing twice.

**Files (as built):**
- Modify: `crates/otter-lexer/src/{lexer,token,error}.rs` (`split_interpolation`, `skip_interpolation`, `StringPiece`, `invalid_interpolation`)
- Modify: `crates/otter-parser/src/{ast,exprs,stmts}.rs` (`Expr::Interp`, `InterpPart`, the sub-parse, a `case` pattern rejects interpolation)
- Modify: `crates/otter-sema/src/check_expr.rs` (desugar to the conversion + `+`; reject types with no text form)
- Modify: `crates/otter-codegen/tests/string_ops.rs`, `crates/otter-sema/tests/check.rs`, `crates/otter-parser/tests/{items,parse}.rs`, `crates/otter-lexer/tests/lex.rs`
- Create: `crates/otter-driver/tests/programs/interp.ot` + a test in `tests/e2e.rs`

**Steps (all done):**

- [x] Lexer: scan `${ ... }` as a unit; split a literal into `StringPiece`s in one place; `\$` stays a literal `$`.
- [x] Parser: `Expr::Interp { parts }`; sub-parse each `${ expr }` with true spans; empty `${}` and `${a b}` are errors; a `case` pattern rejects `${`.
- [x] Checker: `${x}` → `str_from_i64(x)`/`str_from_f64(x)`/`str_from_bool(x)` or `x` for a `string`, joined with `+`; a struct/enum/pointer is "cannot interpolate".
- [x] Tests at every level, including the end-to-end `interp.ot` whose last line is the written-out form the interpolation must equal.
- [x] Commit (`feat: string interpolation`).

**Interfaces (as built):**
- Consumes: `gen_string_binary` (`+`) and the three `str_from_*` builtins, both already present.
- Produces: nothing new downstream. `Expr::Interp` is AST-only; the HIR is a `Binary`/`Call` the backend already lowered.

---

### Task 7 (original sketch, kept for the record)

**Files:**
- Modify: `crates/otter-lexer/src/lexer.rs` (recognise `${` inside string literals)
- Modify: `crates/otter-parser/src/exprs.rs` (build an interpolation node)
- Modify: `crates/otter-sema/src/check_expr.rs`
- Create: `crates/otter-codegen/src/interp.rs`
- Create: `crates/otter-codegen/tests/golden/interp.ot`, `tests/golden/interp.expected.ll`

**Interfaces (original):**
- Consumes: `gen_str_concat` from Task 6
- Produces: `HirExpr::Interp { segments: Vec<InterpSegment>, span }` where `InterpSegment` is `Literal(Vec<u8>)` or `Expr(HirExpr)`. `pub fn gen_interp(ctx: &mut FnCtx, segs: &[InterpSegment]) -> String` lowers left to right, converting each non-string operand with the matching `otter_str_from_*`.

- [x] **Step 1: Write the failing tests** (a literal-only interpolation; one i64 operand converts then concatenates; mixed operands convert by their own type; an unterminated interpolation is a diagnostic, not a panic; an escaped `$` is literal).

- [x] **Step 2: Run to verify they fail** → FAIL (before the feature).

- [x] **Step 3: Implement lexing** — done, but as a shared `split_interpolation` rather than a second literal shape.

- [x] **Step 4: Implement lowering** — done, but in the checker rather than a codegen module.

- [x] **Step 5: Run to verify they pass** → PASS.

- [x] **Step 6: Commit**

```bash
git add crates/
git commit -m "feat: string interpolation"
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

**Status:** delivered (the diagnostic half). Unwinding is Task 10, and
`otter_panic`'s body is the seam where it attaches.

**What was built (the deviation, and why):**

- **No `crates/otter-codegen/src/panic.rs`, and no `HirStmt::Panic`.** `panic` is
  an ordinary builtin call whose result type is `Never`, so it works in every
  position a call works in — an expression statement, a `return`, a `switch` arm
  — without a statement form to go with it, and without the parser, the AST or
  the HIR gaining a variant. A statement form would have been a second way to
  write a call.
- **The checker resolves the source position, not the backend.** Handing
  `emit_program` the `SourceMap` would have changed a signature every test uses,
  to put into the backend a fact the checker already has. Instead `panic(msg)`
  is desugared — after its argument is checked — into the *same* call with four
  arguments: the message, then the file path, line and column as ordinary
  literals. The runtime needs no global state, the IR shows the position as
  operands, and the backend keeps lowering calls it already knows how to lower.
  See `Checker::panic_call`.
- **The call is lowered into a block of its own.** `panic` never returns, so the
  call cannot be followed by more instructions in the same block — but they exist:
  the enclosing expression may be an operand of a larger one, a `switch` arm that
  still has to branch to the merge, or an argument list. Emitting `call` +
  `unreachable` in place would leave the next instruction after a terminator,
  which LLVM rejects. So the current block branches to a fresh block holding the
  call and its `unreachable`, and a second fresh block follows for the rest. That
  continuation is dead, but it is where the rest of the expression belongs, and
  no later stage has to know a panic happened.
- **`!` fits wherever a value is wanted** (`fits` in `check_expr.rs`). Without
  this rule the useful shapes did not work: `return switch o { case None {
  panic("none") } case Some(v) { v } }` needed a diverging arm to stand in for an
  `i64`, and `return panic("...")` needed a diverging expression to stand in for
  any return type. Two sites were widened — a call argument and a `return` — plus
  the switch arms' agreement rule (`merge_arm_type`). Nothing else changed:
  `return true` from an `i64` function is still an error, which a test pins.
- **A defect found on the way:** `return <never-valued expression>` emitted
  `ret i64 ` with a missing operand, because the `Return` arm takes its repr from
  the *function's* return type. The block is unreachable, so `unreachable` is the
  honest terminator; the arm checks the expression's type first now.
- **It does not unwind, and a `defer` does not run before a panic.** This is the
  half Task 10 replaces, not an accident: `otter_panic` flushes `stdout`, writes
  `<file>:<line>:<column>: panic: <message>` to `stderr`, flushes that, and calls
  `exit(1)`. `exit` rather than `abort` so the program's earlier output is not
  lost to a buffer that never flushes, and so the caller sees a failure rather
  than a signal. There is **no stack trace** yet — the plan's fourth test named
  one, and it belongs to the unwinding work; the end-to-end test written instead
  pins the message, the position and the exit status.
- **A golden file, after all.** Task 7 could skip one because interpolation leaves
  no trace in the IR; a panic is the one place lowering emits a terminator of its
  own inside an expression, so `tests/golden/panic.ot` and its `.expected.ll` pin
  the block structure directly. The three existing goldens gained exactly one
  line (the `declare`).

**Files (as built):**
- Modify: `crates/otter-runtime/runtime.c`, `crates/otter-runtime/include/otter_runtime.h` (`otter_panic`)
- Modify: `crates/otter-codegen/src/{module,expr_gen,stmt_gen}.rs` (the `declare`, the panic block, the `unreachable` return)
- Modify: `crates/otter-sema/src/check_expr.rs` (the builtin, `fits`, `merge_arm_type`, `panic_call`, `str_lit`; `Checker` holds the `SourceMap`)
- Create: `crates/otter-codegen/tests/golden/panic.ot`, `panic.expected.ll`; `crates/otter-driver/tests/programs/panic.ot`
- Modify: the goldens, and the tests in `otter-sema`, `otter-codegen` (`codegen.rs`, `fn_gen.rs`), `otter-driver` (`e2e.rs`, `never_panics.rs`)

**Steps (all done):**

- [x] Runtime: `otter_panic(message, file, line, column)` — flush, one `stderr` line, flush, `exit(1)`; declared in the header and in every module.
- [x] Checker: the `panic` builtin (`string` in, `!` out); `panic(msg)` → the four-argument call with the position resolved from the source map.
- [x] Checker: `!` fits where a value is wanted, at call arguments, `return`, and the switch arms' agreement rule.
- [x] Backend: the panic block (branch in, call, `unreachable`, fresh block after) and `unreachable` for a diverging `return`.
- [x] Tests at every level: HIR shape and the position operands in `otter-sema`; block structure, routing and the goldens in `otter-codegen`; the message, the position, the failure status and "nothing after it runs" end to end; nine malformed panics in `never_panics.rs`.
- [x] Commit (`feat: panic with a source location`).

**Interfaces (as built):**
- Consumes: nothing new. The message is an ordinary `string` expression; the position is resolved by the checker; the backend lowers a `call`.
- Produces: `otter_panic` in the runtime (the seam Task 10 turns into a throw), the `panic` builtin, and `Checker::panic_call`. No new HIR node, so `Task 11`'s `gen_panic_block` becomes a call to the same lowering rather than a second one.

---

### Task 9 (original sketch, kept for the record)

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

> **As built (first half).** `defer` bodies registered in the panicking
> function's scopes now run, innermost first, before `otter_panic` is called:
> the panic/exit/fault lowering emits `run_all_scopes_defers` into the fault
> block, in reverse registration order, so the capture-at-registration snapshot
> is honoured. The panic's `undef`/`!` value and the fresh continuation block
> are unchanged, so no golden file grew a new shape beyond the existing one.
>
> What is **not** done: the call stack is not unwound, so defers registered in
> *calling* functions do not run, and there is no stack trace. Doing that needs
> the full landing-pad machinery (`invoke` for every panicking call, a
> personality routine in the runtime, cleanup landing pads carrying the deferred
> bodies, and `nounwind` elision so non-panicking code stays fast) — the
> remaining half of this task. The `defer` stack now lives in `FnCtx` (a
> `Vec<Vec<Deferred>>`, one level per active scope) instead of being threaded
> through `gen_block`/`gen_stmt` by hand, which is what made running it from the
> fault block possible at all.

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

> **As built (both halves).** The checked default and the
> `wrapping_*` / `checked_*` / `saturating_*` opt-outs are in. What differed from
> the sketch, and why:
>
> - **No `span` reaches the backend.** The sketch had `gen_checked_binary(...,
>   span: Span)`, but the backend is handed IR text and a path and has no source
>   map, so a span is meaningless to it. The checker resolves the position once
>   into a new `otter_sema::FaultSite { file, line, column }` and hangs it on
>   `HirExpr::Binary`/`HirExpr::Unary`. That is the same division of labour
>   `panic(msg)` uses (Task 9): the tree records *that* an operation can fault
>   and *where*; codegen decides *how*.
> - **`add`/`sub`/`mul` go through `llvm.s{add,sub,mul}.with.overflow.i64`**
>   rather than "emit the operation, then `icmp` the result against an operand".
>   The intrinsic produces the value and the flag together — one `add` and one
>   `seto` on x86 — where the comparison form needs more instructions and a
>   predicate that is easy to get wrong for `mul`. The sketch's `icmp` wording
>   dates from before the intrinsic was on the table.
> - **`div`/`rem` check both of their faults in one branch.** A zero divisor, or
>   `i64::MIN / -1` (whose answer is one past `i64::MAX`, and where `srem` is
>   undefined too). The *division itself* is emitted in the fall-through block,
>   after the branch: a `sdiv` that is going to fault does not return a wrong
>   answer, it raises `#DE`.
> - **Negation is checked too.** `-i64::MIN` wraps to itself; it reports as an
>   overflow of `0 - i64::MIN` through the same runtime function.
> - **The panic arm and the fault path are one shape.** `crate::faults::
>   close_fault` emits the block Task 9 built by hand, and Task 9's panic block
>   label changed from `bbN_panic` to `bbN_fault` because it is the same thing.
> - **Two runtime functions, not a message per fault**: `otter_panic_overflow`
>   and `otter_panic_division` take the operator's symbol and both operands, so
>   the message names the numbers:
>   `main.ot:7:13: panic: integer overflow: 9223372036854775807 + 1`.
> - **Measured cost**, in interleaved passes so the ratios are trustworthy
>   (the machine's clock moves every row together between windows): `fib`
>   79 → 158 ms, `loop` at the portable tier 253 → 261 ms, `float` 529 → 535 ms.
>   The call-dominated benchmark pays for three checks per call. The portable
>   loop and `float` are within noise, because LLVM proves the arithmetic safe
>   and deletes the checks. **With the opt-out in, `fib` is 84 ms and the
>   portable `loop` 262 ms — the escape hatch recovers all of it**, which is why
>   it is part of this task rather than a later one.
> - **The first measurement of `loop` was wrong, and the way it was wrong is
>   worth keeping.** It was taken at the portable tier only, where the checks are
>   proved away, which produced the comfortable conclusion "the loop is
>   unaffected". Measured at `--cpu=x86-64-v3`, the checked loop is **171 ms
>   against 69 ms opted out** — 2.5× — because a check is a potentially-faulting
>   branch in the body and LLVM's loop vectoriser will not speculate it. The
>   baseline loop is scalar so the checks vanish; the `v3` loop would vectorise,
>   so they do not. The opt-out lands on clang's number exactly (69 vs 68 ms).
>   See `docs/performance-improvements.md` item 9 for the table and item 10 for
>   making the checked form vectorisable.
>
> **The second half — the opt-outs.** Nine builtins, declared in the checker's
> builtin table like any other function:
>
> | Family | Type | Lowering |
> |---|---|---|
> | `wrapping_add`, `_sub`, `_mul` | `(i64, i64) -> i64` | the bare `add`/`sub`/`mul`: no flag, no branch, no cold block |
> | `saturating_add`, `_sub` | `(i64, i64) -> i64` | `llvm.sadd.sat` / `llvm.ssub.sat` |
> | `saturating_mul` | `(i64, i64) -> i64` | by hand — see below |
> | `checked_add`, `_sub`, `_mul` | `(i64, i64) -> Option[i64]` | the overflow intrinsic, then a `select` between `Some(value)` and `None` |
>
> - **`llvm.smul.sat.i64` does not exist.** It is in LLVM's language reference
>   and clang 23 emits a call to an undefined symbol for it, which fails at link
>   time. So `saturating_mul` is built from `smul.with.overflow` plus the sign of
>   the true product — negative exactly when the operands' signs differ, which is
>   never ambiguous at the point of overflow because neither operand is zero
>   then. Both ends and the "did not overflow" case are pinned by tests, because
>   a hand-rolled clamp is exactly the kind of predicate that is wrong in one
>   corner.
> - **`checked_*` is a `select`, not a branch**, which is what makes it usable
>   for code that can recover: no runtime call, no cold block, no fault. The
>   result is an ordinary `Option[i64]` that `switch` matches on.
> - **No `wrapping_div`/`checked_div`/`saturating_div`.** A wrapped quotient is
>   not the quotient of anything, and both ways a division has no answer are
>   reported either way; `/` and `%` keep their check with no opt-out.
> - Named functions rather than an overloaded name, because the table declares a
>   name once and a redeclaration is an error. `acc = wrapping_add(acc, x);`
>   reads as what it does.

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

> **As built (the assertion and exit-code halves).** `assert`, `assert_eq`,
> `assert_ne` and `debug_assert` are in, with the `--release` build mode that
> compiles the last one away; `exit(i64)` (the `!`-typed `os.exit`) is in,
> flushing both streams before ending the process; and `main` returning `i64`
> is the exit status. The `i32` spelling in the specification is Otter's `i64`
> -- Otter has no `i32` type, and the runtime narrows it to the C `int`.
>
> Four things differ from the sketch above, and each is worth keeping:
>
> - **No new runtime function.** The sketch assumed an `otter_assert_failed`
>   symbol. The existing `otter_panic(message, file, line, column)` already
>   takes a message and a position, and an assertion failure *is* a panic, so
>   the message is built before the call and nothing new is declared. The whole
>   feature is therefore checker plus codegen: no runtime change, no golden
>   regeneration, and a failed assertion reads exactly like every other fault.
> - **The message is an argument, not a string the backend builds.** For
>   `assert` the checker writes the whole message (`assertion failed: n > 0`)
>   as a literal -- it has the source text, the backend does not. For
>   `assert_eq`/`assert_ne` the two *operands* travel down as expressions with
>   the two halves of the message around them, because only the backend can
>   print the values, and lowering each operand once and using the register
>   twice is what stops `assert_eq(f(), 0)` from running `f` again on the
>   failure path and reporting a number that is not the one that failed.
> - **`assert_eq` is one name over four types, built in the checker.** The
>   builtin table has no generics and declares a name once, so the comparison
>   forms are constructed in `Checker::assert_cmp_call` before the declared
>   parameter types are consulted; the table entry exists to reserve the name,
>   so a program that defines `assert_eq` of its own is a duplicate definition.
>   The comparison is `icmp` for `i64`/`bool`, `fcmp oeq`/`une`
>   for `f64` and `otter_str_eq` for `string`.
> - **The result type is `()`.** `assert` does not diverge when it holds, and
>   typing it `!` would let its non-existent value into anything.
> - **`--release` is not tied to `-O`.** `-O2` is already the default, so a flag
>   meaning "optimize harder" would be a no-op; this one answers whether the
>   build is one a user runs. The flag reaches the checker
>   (`CheckOptions { release }`, with `check_program_with` beside
>   `check_program`) rather than the backend, because dropping a
>   `debug_assert` is dropping the *tree*: the condition is still checked --
>   a release build that accepted source a debug build rejects would be the
>   worst of both -- but the message is never interned and an argument with a
>   side effect never runs. `debug_assert` in a debug build desugars to exactly
>   the `assert` call; in a release build to a call with no arguments, which
>   the backend lowers to nothing.
>
> Tests: 13 in `crates/otter-sema/tests/check.rs`, 9 in
> `crates/otter-codegen/tests/expr_gen.rs`, 7 in
> `crates/otter-driver/tests/e2e.rs` -- the last of which asserts that an
> operand with a side effect runs exactly once, passing *and* failing, and that
> a release build drops a `debug_assert` while keeping an `assert`.

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