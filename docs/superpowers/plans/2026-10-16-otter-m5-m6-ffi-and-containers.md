# M5–M6 Implementation Plan — FFI, Containers, and Modules

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give Otter reach into C, give it slices and maps, give it a module system, and ship the core standard library — the point at which the language becomes general purpose.

**Architecture:** `extern "C" unsafe fn` declarations add C-flavoured types to `otter-sema` and link directives to the driver. Slices and maps are built on the `otter_slice` and `otter_map` runtime structures from ledger L1, with linearity extended to slices by adding one match arm to M3's `linearity_of`. Modules resolve `import` paths to files through a search path.

**Tech Stack:** Rust 1.99, LLVM IR text, C runtime shim, `cargo test`. End-to-end tests run on Windows **and** Linux from this milestone onward.

**Spec:** §7 (concurrency, out of scope here), §8 (interop and stdlib), §3.4 (imports), §4 (slices yield `Option`). **Master plan:** `2026-10-16-otter-master-plan.md` — read its Interface Ledger and Global Constraints first.

## Prerequisites

- M0–M2 and M3–M4 complete and reviewed.
- String operations, slices-as-values (`string` is `{i32,i64,i8*}` internally but is *not* a `Slice`), and `?`/`Result` all working.

## Scope

**In:** `extern` blocks and `unsafe`, C type spellings, linking, boundary copying, slice type and operations, maps, `for`-in over slices, module resolution, and the `io`, `string`, `math`, `os`, `alloc` stdlib modules.

**Out:** threads (M7), `defer` on unwinding changes (already done in M4), generics.

---

### Task 1: C types in the type grammar

**Files:**
- Modify: `crates/otter-parser/src/exprs.rs` (`parse_type_expr`)
- Modify: `crates/otter-sema/src/ty.rs`
- Modify: `crates/otter-codegen/src/ir.rs`

**Interfaces:**
- Consumes: ledger L2 M5 row
- Produces: `TypeKind::CInt(u32)`, `CUInt(u32)`, `CFloat(u32)`, `CVoid`, `CPtr(TypeKind)`, `CSize`; spellings `i8 i16 i32 i64`, `u8 u16 u32 u64`, `f32 f64`, `*T`, `*mut T`, `*const T`, `usize`, `void`

- [x] **Step 1: Write the failing tests**

```rust
#[test]
fn c_integer_spellings_parse_and_map_to_widths() {
    // i32 -> CInt(32); u8 -> CUInt(8); f64 -> CFloat(64); usize -> CSize.
}

#[test]
fn pointers_parse_in_mut_and_const_flavours() {
    // "*mut i8" and "*const u8" both parse; only mutability differs.
}

#[test]
fn c_types_are_rejected_outside_an_extern_block() {
    // A C type in ordinary Otter code -> Err naming extern. This keeps the two
    // type systems from quietly mixing.
}

#[test]
fn llvm_widths_match_the_declaration() {
    // CInt(8) -> i8, CUInt(64) -> i64, CFloat(32) -> float, CSize -> i64,
    // CPtr(_) -> i8* (opaque pointer).
}

#[test]
fn void_appears_only_as_a_return_type() {}
```

- [x] **Step 2: Run to verify they fail** → FAIL.

- [x] **Step 3: Implement**

Extend `TypeExpr` with a `CName`/`CPtr`/`CVoid` family and `TypeKind` to match. `linearity_of` returns `Linear` for `CPtr` to a non-void type — a C pointer is still linear.

- [x] **Step 4: Run to verify they pass** → PASS.

- [x] **Step 5: Commit**

```bash
git add crates/otter-parser/ crates/otter-sema/ crates/otter-codegen/
git commit -m "feat(ffi): C type spellings in the type grammar"
```

**As built (commit `4fbc8cb`).** Three decisions were made while implementing,
and each is pinned by a test rather than left to prose:

- **`i64` and `f64` are not new kinds.** `TypeKind::from_c_spelling` maps them
  to the existing `I64`/`F64`; only `i8 i16 i32 u8 u16 u32 u64 f32 usize void`
  add `CInt`/`CUInt`/`CFloat`/`CSize`/`CVoid`. Two kinds for one machine value
  would make every boundary call a cast that checks nothing. This supersedes
  the plan's `f64 -> CFloat(64)` sketch on purpose.
- **Pointer mutability is in the AST.** `TypeExpr::Ptr` now carries
  `PtrKind::{Bare, Mut, Const}`, so `*u8`, `*mut u8` and `*const u8` are three
  parse results. `parse_type_expr` reaches the bare spelling by finishing the
  named pointee it has already started, since `const` is a keyword token and
  the cursor cannot rewind. Outside an `extern` declaration only `Mut`
  resolves (to the old owning `TypeKind::Ptr`); the other two are diagnostics
  naming `extern`.
- **C spellings are rejected by the checker, not the parser.** The parser
  accepts them anywhere so a diagnostic can point at the type; `resolve_type`
  rejects FFI-only names and pointers with a message naming `extern`, while
  `resolve_type_in(.., allow_c = true)` is the path Task 2's `extern` checker
  uses. Task 5 added one more legal position: an `as` target, because that is
  the only way ordinary code can produce the `usize` that `malloc` takes.
  `void` as a *parameter* is therefore also rejected today as "C type
  outside extern"; the return-only rule is enforced where declarations are
  checked (Task 2), because that is the first point at which a `void` return
  is legal at all.

`CPtr` is linear in `linearity_of`, like `Ptr`: C code may keep the pointer or
free what it points at, and the checker cannot see across the boundary. LLVM
widths are pinned one at a time in `crates/otter-codegen/tests/c_types.rs`;
`CInt(64)` and `CFloat(64)` still lower correctly even though the checker
never constructs them.

Tests: `crates/otter-parser/tests/types.rs` (3),
`crates/otter-sema/tests/c_types.rs` (5),
`crates/otter-codegen/tests/c_types.rs` (2).

---

### Task 2: `extern` blocks

**Files:**
- Modify: `crates/otter-parser/src/items.rs`
- Modify: `crates/otter-sema/src/check_expr.rs`
- Modify: `crates/otter-sema/src/scope.rs`
- Modify: `crates/otter-codegen/src/fn_gen.rs`
- Create: `crates/otter-codegen/tests/golden/extern.ot`, `tests/golden/extern.expected.ll`

**Interfaces:**
- Consumes: Task 1
- Produces: `Item::Extern { abi: Option<String>, link: Option<String>, decls: Vec<ExternDecl>, span }` — `abi` is `"C"`, `link` is the library such as `"m"`; `ExternDecl { name, params, ret, span }`; `pub fn gen_extern_decl(module: &mut Module, d: &HirExternDecl) -> ()` emitting `declare`. `unsafe` is **required** on every declaration — an `extern fn` without it is a diagnostic, per spec §8.1 — and because it is mandatory, `ExternDecl` carries no `unsafe` flag: there is nothing to store.

- [x] **Step 1: Write the failing tests**

```rust
#[test]
fn an_extern_declaration_requires_unsafe() {
    // extern "C" fn puts(...) without unsafe -> Err naming unsafe.
}

#[test]
fn an_extern_declaration_emits_a_declare_not_a_define() {}

#[test]
fn a_call_to_an_undeclared_extern_function_is_an_error() {}

#[test]
fn extern_parameters_and_return_types_are_checked() {
    // Calling with a wrong argument type -> Err.
}

#[test]
fn an_extern_function_is_linear_if_its_return_is_a_pointer() {}

#[test]
fn a_link_directive_records_the_library_name() {
    // extern "C" link "m" { ... } -> the driver adds -lm.
}
```

- [x] **Step 2: Run to verify they fail** → FAIL.

- [x] **Step 3: Implement**

Parse `extern "C" [link "name"] [unsafe] fn name(...)`. Reject a missing `unsafe`. Emit `declare` into the module. Predeclare each in the root scope.

- [x] **Step 4: Run to verify they pass** → PASS. Add and read the golden file.

- [x] **Step 5: Commit**

```bash
git add crates/
git commit -m "feat(ffi): extern C declarations with a mandatory unsafe marker"
```

**As built.** The parser accepts the braced block *and* the single-declaration
form (`extern "C" unsafe fn puts(s: *const u8) -> i32;`); both produce the same
`Item::Extern` with a declaration list, because the AST already had one shape
for both. `const`-style keyword handling is not involved here, but `link` is an
identifier rather than a keyword and is recognised by its token text.

`unsafe` is enforced by `parse_extern_decl` before `fn` is read, so the caret is
on the declaration that omitted it. `extern_block_needs_a_declaration` rejects
a braced block with nothing in it; a single declaration is always non-empty by
construction.

The HIR gained `HirItem::Extern(HirExternBlock)` and
`HirExternDecl { name, params, ret, span }`. The block carries `abi` and `link`
through unchanged; Task 4's driver is the only consumer of `link`. Every
declaration is predeclared in the root scope and in `fns`, so calls resolve
exactly like user functions and a name collision is the existing "duplicate
definition" diagnostic.

Two boundary rules landed with the declarations:

- **`void` is legal only as a return type.** `resolve_or_report_in(.., allow_c,
  is_return)` rejects a `void` parameter (and `Option[void]`); `void*` remains a
  legal parameter, because a pointer to void is not a void value.
- **A `string` may stand where `*const u8` / `*mut u8` is wanted.** This is the
  one FFI coercion, in `ffi_string_fits`; it is what makes `strlen("hi")` type
  check. The copy itself is Task 3, so the HIR argument is still the `Str`
  value.

Codegen emits `declare <ret> @name(<params>)` through `gen_extern_decl` and
nothing else. The golden (`tests/golden/extern.expected.ll`) pins two libc
declarations and an empty `main`; the *call* IR is deliberately not pinned here
because today's `string` value is a pointer to the `%otter_str` header, which is
not a C string. Task 3's copy is what changes that, and its own golden owns the
call shape.

Known gap, deferred to Task 5: a C scalar parameter (`i32`, `usize`) cannot yet
be given a value, because `as` does not know the C numeric kinds and ordinary
Otter code cannot declare one. The libc program needs `malloc`'s `usize`, so
Task 5 extends `is_numeric` and the cast lowering rather than smuggling an
implicit conversion into this task.

---

### Task 3: Boundary copying

**Files:**
- Create: `crates/otter-codegen/src/ffi.rs`
- Modify: `crates/otter-codegen/src/expr_gen.rs`, `src/module.rs`, `src/lib.rs`
- Modify: `crates/otter-sema/src/check_expr.rs` (the `unsafe` rule on pointer arguments)
- Modify: `crates/otter-runtime/runtime.c`, `include/otter_runtime.h` (the copy and the lend)
- Create: `crates/otter-codegen/tests/ffi.rs` (replaces the sketch below)
- Create: `crates/otter-codegen/tests/golden/ffi_copy.ot`, `tests/golden/ffi_copy.expected.ll`

**Interfaces:**
- Consumes: Task 2
- Produces: `pub fn gen_ffi_call(ctx: &mut FnCtx, decl: &HirExternDecl, args: &[GenValue], unsafe_call: bool) -> String` — copies each `string` argument into temporary runtime-owned storage before the call, frees it after, and leaves raw pointers alone

- [x] **Step 1: Write the failing tests**

The four tests landed in `crates/otter-codegen/tests/ffi.rs`: the string is
copied before the call and the header pointer never crosses; the temporary is
freed after the call; an `unsafe` call lends the bytes and frees nothing; a raw
pointer is passed as itself (a hand-built lowering context, because producing a
C pointer needs `malloc`'s `usize` cast, which is Task 5). The slice test is
deferred with the slice type itself (M6): there is no `Slice` in `TypeKind` yet,
so writing the test now would pin a shape the language does not have.

- [x] **Step 2: Run to verify they fail** → FAIL.

- [x] **Step 3: Implement**

An `unsafe` marker on the *call site* is required to bypass copying. Without it, string arguments are copied and C cannot retain them. Three pieces:

1. **The runtime copy.** `otter_str_cstr` (`runtime.c`) allocates a fresh
   NUL-terminated buffer with `otter_alloc`, so the existing leak accounting and
   `otter_free` apply unchanged. `otter_str_ptr` returns the string's own data
   pointer and is the `unsafe` lend.
2. **The lowering.** `gen_ffi_call` copies each `string` argument whose
   parameter is a `CPtr`, makes the call, then frees the copies and releases any
   temporary strings it consumed. Under `unsafe` it calls `otter_str_ptr`
   instead and frees nothing: the pointer may now belong to C. Raw pointers and
   numbers cross as their machine values.
3. **The rule.** The checker records extern names and rejects a call that
   passes a linear (raw-pointer) argument without `unsafe`, at the argument's
   span, because C may keep or free what the pointer names. A `string` is not
   linear, so `strlen("hi")` stays safe.

`emit_program` now walks extern blocks in a pre-pass: all `declare`s come before
the first `define`, and each declaration is recorded in `Module` so a call site
can tell an extern callee from a user function. The two new runtime declares
appear in every golden (the header is fixed), which is the only golden churn.

- [x] **Step 4: Run to verify they pass** → PASS. Codegen 5 (`tests/ffi.rs`,
including the `ffi_copy` golden), sema 8 (`tests/externs.rs`), all other golden
files updated by exactly two `declare` lines each.

- [x] **Step 5: Commit**

```bash
git add crates/otter-codegen/ crates/otter-sema/ crates/otter-runtime/
git commit -m "feat(ffi): copy string and slice arguments at the C boundary"
```

**As built:** the produced signature gained `unsafe_call: bool`; the plan's
`gen_ffi_call` could not know which mode the call site asked for. Slices and
out-parameter copy-back are not implemented — both need the slice type (M6
Tasks 6–7); today's out-parameter mechanism is a raw pointer, which passes
unchanged and requires `unsafe`. The commit message above is kept for the
record; the actual commit says `feat(ffi): copy string arguments at the C
boundary` because "slice" would claim work not in it.

---

### Task 4: Linking and platform toolchain

**Files:**
- Modify: `crates/otter-driver/src/pipeline.rs`
- Modify: `crates/otter-driver/src/toolchain.rs`
- Create: `crates/otter-driver/tests/link.rs`

**Interfaces:**
- Consumes: M0–M2 driver, Task 2's link directives
- Produces: `pub fn link_directives(hir: &HirProgram) -> Vec<String>`; `pub fn platform_library_flags() -> Vec<&'static str>`; platform-specific library flags; a test that builds and runs the same program on both platforms

- [x] **Step 1: Write the failing tests**

`crates/otter-driver/tests/link.rs` covers four directive shapes (one
library, no directive, the same library twice, two libraries in source order)
and one execution test, `the_same_program_produces_identical_output_on_both_platforms`,
which calls `sqrt` with no `link` directive and pins the exact bytes `%.17g`
prints. Unit tests in `pipeline.rs` cover the linker-message parser and the
platform flag list.

- [x] **Step 2: Run to verify they fail** → FAIL.

- [x] **Step 3: Implement**

Collect directives, append platform defaults (`-lm -lpthread -ldl` on Linux, nothing on Windows because the CRT is linked by default), and pass them to the linker. Report a clear diagnostic naming the library when linking fails for that reason.

- [x] **Step 4: Run to verify they pass** → PASS.

- [x] **Step 5: Commit**

```bash
git add crates/otter-driver/
git commit -m "feat(driver): link directives and platform library flags"
```

**As built:** `link_directives` returns `-l<name>` flags in source order with
one flag per library; `platform_library_flags` is deduplicated against them, so
`link "m"` on Linux still passes `-lm` once. All libraries are appended after
the objects (GNU `ld` resolves rightward). A link failure whose output contains
GNU's `cannot find -lfoo` or LLD's `unable to find library -lfoo` now produces a
diagnostic that names the library and says a `link` directive or a platform
default asked for it; `missing_library` parses both spellings.

`toolchain.rs` needed no change: the library flags are a link-line concern and
live in `pipeline.rs`. On this host the execution test was blocked by
Application Control once, then passed after a clean relink (Device Guard blocks
by content hash, so a rebuilt test binary gets a fresh verdict); the test skips
its execution assertion, with a message, only for a policy block, never for a
link failure, and CI has no such policy.

---

### Task 5: Call libc

**Files:**
- Create: `crates/otter-driver/tests/programs/libc.ot`
- Modify: `crates/otter-driver/tests/e2e.rs`
- Modify: `crates/otter-sema/src/check_expr.rs` (`is_numeric`, cast targets)
- Modify: `crates/otter-codegen/src/expr_gen.rs` (`gen_cast`)
- Create: `crates/otter-codegen/tests/c_casts.rs`

**Interfaces:**
- Consumes: Tasks 1–4
- Produces: an end-to-end program calling `puts`, `strlen`, and `malloc`/`free` through `extern`; an `as` that converts between every C numeric width

- [x] **Step 1: Write the failing test**

`libc.ot` prints a line through `puts`, prints `strlen("hello")` after an
explicit `as i64`, and allocates/frees a block through `malloc(64 as usize)`.
The wrong-signature case is a sema test, not an e2e one: the point is that the
program never reaches the linker, so there is nothing to execute.

- [x] **Step 2: Run to verify they fail** → FAIL.

- [x] **Step 3: Write the program and run it** → PASS.

The gap Task 2 recorded closes here: `is_numeric` now accepts every C numeric
kind, and an `as` target is the one place outside `extern` where a C spelling
is legal. `gen_cast` was a five-entry table of exact `repr` string pairs; it is
now a width/signedness matrix covering `trunc`, `sext`/`zext`,
`sitofp`/`uitofp`, `fptosi`/`fptoui`, and `fpext`/`fptrunc`. `i64` and `usize`
share a representation, so that cast is type-level only.

- [x] **Step 4: Commit**

```bash
git add crates/otter-driver/
git commit -m "test: end-to-end libc interop"
```

**As built:** the program was verified on this host — it built, ran, and printed
`hello from C\n5\n`, which covers the Task 3 string copy, Task 4 `link "m"` on a
platform where it is redundant, and both cast directions. Cast IR is pinned
separately in `crates/otter-codegen/tests/c_casts.rs` (2 tests, signed and
unsigned widening, narrowing, int↔float). The sema suite grew by two: the
wrong-signature rejection (asserting the `i32` mismatch is named) and the
`as`-target legality/`*mut`-target rejection in `tests/c_types.rs`.

---

### Task 6: Slice type

**Files:**
- Modify: `crates/otter-parser/src/exprs.rs` (`[]T` type, `[]T { a, b }` literal)
- Modify: `crates/otter-sema/src/ty.rs`, `ownership.rs`
- Modify: `crates/otter-codegen/src/ir.rs`, `slices.rs`
- Create: `crates/otter-codegen/tests/golden/slices.ot`, `tests/golden/slices.expected.ll`

**Interfaces:**
- Consumes: ledger L1 `otter_slice`, ledger L2 M6 row
- Produces: `TypeKind::Slice(Box<TypeKind>)`; `pub fn gen_slice_literal(ctx: &mut FnCtx, elems: &[GenValue]) -> String`; `pub fn gen_slice_push(ctx: &mut FnCtx, slice_slot: &str, elem: &GenValue) -> ()`; `linearity_of` gains its `Slice` arm

- [x] **Step 1: Write the failing tests**

```rust
#[test]
fn a_slice_literal_allocates_and_stores_its_elements() {}

#[test]
fn a_slice_is_linear() {
    // "var a = [1, 2]; var b = a;" -> Err; "var b = move a;" -> Ok.
    // Proves M3's linearity_of needed exactly one new arm.
}

#[test]
fn pushing_grows_the_slice_and_preserves_earlier_elements() {}

#[test]
fn freeing_a_slice_frees_its_backing_array_once() {
    // Not per-element. A slice of structs must not double-free its contents.
}

#[test]
fn an_empty_slice_literal_is_valid() {}

#[test]
fn a_slice_of_slices_is_linear_at_both_levels() {}
```

- [x] **Step 2: Run to verify they fail** → FAIL.

- [x] **Step 3: Implement**

`{ i8*, i64, i64 }` exactly matching `otter_slice`. `otter_slice_push` reallocs with a doubling policy when `len == cap`. Indexing yields `Option[T]`, never a trap — Review Focus item 4.

- [x] **Step 4: Run to verify they pass** → PASS. Added and read the golden file.

- [x] **Step 5: Commit**

```bash
git add crates/
git commit -m "feat: slices as linear values with a doubling backing array"
```

**As built:**

- **Syntax.** `TypeExpr::Slice(Box<TypeExpr>, Span)` in `ast.rs`; the type
  grammar in `types.rs` reads `[` `]` then the element type (a non-empty pair
  is reserved for a future fixed-size array and is not read as a slice). The
  value spelling `[]T { a, b }` is parsed at primary position in `exprs.rs`;
  the element type is written, never inferred, so `[]T {}` is typed and legal.
- **Checked type.** `TypeKind::Slice(Box<Type>)`, `Display` prints `[]{T}`, and
  `linearity_of` gained exactly the promised `Slice` arm. A slice is rejected
  as a value type when its element is `()`, `!`, or `void` — such an element has
  no size, and the diagnostic fires both for a written `[]()` and for a literal
  `[]() { }`.
- **Builtins.** `push` and `free_slice` cannot live in the fixed 31-entry
  `builtins` table: their types mention the element type, and the table has no
  type parameters. Both are special-cased in the `Call` arm before name
  resolution, after the `Option`/`Result` constructors. Their receiver must be
  an identifier, because codegen passes the binding's slot address:
  `push(slice, value)` checks `value` against the element type and keeps the
  slice; `free_slice(slice)` moves the binding out, so a second call is a
  move error. This is the callable spelling the plan's "frees once" needed.
- **Lowering.** `crates/otter-codegen/src/slices.rs`:
  `gen_literal` allocates `count * elem_size` through `otter_alloc`, stores each
  element with a `getelementptr`, and builds the header with three
  `insertvalue`s; an empty literal allocates nothing and is `{ null, 0, 0 }`.
  `gen_push` stores the value in a fresh slot and calls
  `otter_slice_push(slot, &temp, elem_size)`; `gen_free` calls
  `otter_slice_free(slot)` without touching the header again (the runtime
  zeroes it). `Module::finish` always declares both symbols, as it already did
  for the string and arena sets, so all eight older goldens gained exactly
  those two lines.
- **Representation and element size.** `repr_type(TypeKind::Slice) =
  "{ i8*, i64, i64 }"` and `ir::size_of` gives exact strides for scalars,
  `string` (8), pointers (8), and slices (24). `Option`/`Result`/struct/enum
  elements fall back to one word: their real layout needs the declaration table,
  and M6's programs slice `i64`, `string`, and other slices. That limitation is
  documented at `size_of` and must be lifted before a slice of a declared type
  is supported.
- **Runtime.** `otter_slice` and the two functions are declared in the header;
  `otter_slice_push` grows 0 -> 4 -> double, copies with `memcpy`, frees the old
  block through `otter_free`, and leaves the slice unchanged on a failed
  allocation (there is no error channel in the ABI, and aborting would disguise
  an allocation failure as a language fault). `otter_slice_free` tolerates null
  and zeroes the header, so a freed slice can be reused.
- **Tests.** Parser 3 (`tests/slices.rs`), sema 9 (`tests/slices.rs`: literal
  element check, linearity at both levels, push keeps, free moves, `push` needs
  a slice, zero-sized element), codegen 5 (`tests/slices.rs`: allocation and
  stores, empty literal, push call shape, free call, golden) plus
  `golden/slices.ot`/`slices.expected.ll`; runtime 5 (`tests/slices.rs`: first
  push, doubling with preservation, free/reuse, null cases); driver e2e 1
  (`slices.ot` builds, runs, prints and exits 0) and leaks 1 (the grow's old
  block and the final block both come back: zero live allocations).
- **Deferred.** Indexing a slice still reports "cannot index `[]i64`" until
  Task 7 gives it `Option[T]`, `len`, `pop`, and `for`.

---

### Task 7: Slice operations and iteration

**Files:**
- Modify: `crates/otter-sema/src/check_expr.rs`
- Modify: `crates/otter-codegen/src/slices.rs`
- Modify: `crates/otter-parser/src/stmts.rs` (`for x in slice`)
- Modify: `crates/otter-runtime/runtime.c`, `include/otter_runtime.h`

**Interfaces:**
- Consumes: Task 6
- Produces: `len(slice) -> i64`, `pop() -> Option[T]`, `slice[i64] -> Option[T]`, and `for x in slice { }`; `slice[a..b]` and `contains` are deferred (see "As built").

- [x] **Step 1: Write the failing tests**

```rust
#[test]
fn indexing_out_of_range_yields_none() { /* review focus item 4 */ }

#[test]
fn a_negative_index_is_none_not_a_wraparound() {}

#[test]
fn for_in_iterates_a_slice_in_order() {}

#[test]
fn for_in_over_an_empty_slice_runs_zero_times() {}

#[test]
fn for_in_over_a_slice_moved_while_iterating_is_rejected() {}

#[test]
fn break_inside_for_in_exits_the_loop() {}
```

- [x] **Step 2: Run to verify they fail** → FAIL.

- [x] **Step 3: Implement**

`for x in slice` allocs a hidden index, loads the element each iteration, and re-reads `len` each time so a `push` inside the loop is visible. Moving the slice inside the body is a linearity error.

- [x] **Step 4: Run to verify they pass** → PASS.

- [x] **Step 5: Commit**

```bash
git add crates/
git commit -m "feat: slice operations and for-in iteration"
```

#### As built

- **Syntax.** `for <ident> in <expr>` decides by one token after the
  expression: `..` keeps the range form, anything else is the slice form and is
  parsed as `Stmt::ForIn`. Both share the same head, so no new keyword was
  needed. The parser has the two new tests (slice form binds one name; the range
  form still reads `start`/`end`).
- **Checking.** `len` is special-cased in the call arm rather than widened in
  the builtin table: its argument is checked once and the *type* decides whether
  it is `otter_str_len` or field 1 of a slice header. `push`, `pop`, and
  `free_slice` share `check_slice_builtin`; `pop` keeps the receiver and yields
  `Option[T]`, `free_slice` still moves it. `xs[i]` is `Option[T]` and `xs[i] =
  v` reports "cannot assign to a slice element: the slice does not expose its
  storage; use `push` to append". `for x in slice` requires an Ident iterable
  (the loop re-reads that binding's header), binds the element as `const`, and
  rejects a non-slice with "must iterate a slice".
- **Bindings only.** `push`, `pop`, `free_slice`, and `for-in` reject a dotted
  name (`s.items`) as well as a temporary. A field header has no slot to update
  in place, and lowering resolves the name through the local map: accepting a
  projection would make the call silently do nothing. The diagnostic is
  "needs a slice binding, not a field access" (or "... not a temporary or a
  field" for `for`).
- **Borrow tracking.** `Checker::iteration_borrows` records the slice binding
  while the body is checked, and `Checker::move_out` (now used by `free_slice`,
  `move`, and passing a linear value to a function) reports "cannot move `xs`
  while it is being iterated". A `push` is not a move and stays legal; the loop
  sees it because it re-reads the header.
- **Element restriction (deviation).** Slice *operations* accept numbers and
  `bool` (`i64`, `f64`, the C widths, `usize`) only. `[]string` is still a legal
  value (an empty literal, a parameter), but `push`/`pop`/indexing/`for-in` on
  one report "a slice of `string` is not supported yet: ... would need reference
  counting". A `string` element would have to be retained on extraction and
  released on replacement; a compound element needs its real layout as a stride,
  which `ir::size_of` does not have yet. Refusing at the operation, instead of
  silently copying a reference, keeps ownership sound until slices learn
  destructors.
- **Codegen.** `len` is `extractvalue { i8*, i64, i64 } %header, 1` and emits
  no call. `xs[i]` copies the header into a slot, zero-initializes an out slot,
  calls `otter_slice_get(header, index, out, size)`, and builds `{ i8, T }` from
  the 0/1 answer. `pop` calls `otter_slice_pop` with the *binding's* header so
  the decrement is in place. `for-in` lowers to `cond/body/inc/done`: the
  condition reloads `len` and the index, the body reloads `data`, GEPs to the
  element, copies it into a fresh slot, and the loop-variable binding shadows and
  restores whatever `name` meant before. No component is cached across
  iterations, which is what the push-inside-the-loop test proves.
- **Runtime.** `otter_slice_get` and `otter_slice_pop` take the stride, test the
  index/`len` as signed (negative is a miss, never a wraparound), copy only on a
  hit, and treat null slice/null out/empty/non-positive stride as the same miss.
  `Module::finish` always declares both, so the nine older goldens each gained
  exactly two lines; `golden/slices_ops.ot` joins `golden/slices.ot`.
- **Existing `defer` defect fixed.** While testing iteration, a `defer` written
  before an `if`/loop ran *inside* that nested block (once per iteration for a
  loop): `gen_if`, `gen_for_range`, `gen_for_in`, and `gen_for_c` re-ran the
  enclosing scope's defers after `gen_block` had already run and popped the
  block's own. The calls were removal-only; `defer_scope.ot` pins the order
  (nested blocks first, the deferred body once at the end).
- **Tests as built.** Parser 5 in `tests/slices.rs` (2 new); sema 21 (13 new:
  `len` on both types, `len` of a number, index in/out of range, element
  assignment, `pop` typing and receiver, `for-in` element type, non-slice and
  temporary iterables, a field slice used where a binding is required,
  move-while-iterating, `[]string` rejection); codegen 11 in `tests/slices.rs`
  (5 new plus the `slices_ops` golden) and runtime 8 (3 new: in-range copy, miss
  cases including negative/null/empty, pop order and shrinking); driver e2e 2
  (`slices_ops`, `defer_scope`) and leaks 1 (`slices_ops`: every reallocation and
  all three slices return zero live allocations).
- **Deferred.** `slice[a..b]` (Otter has no range expression yet and aliasing vs
  copying is unresolved) and `contains` are not implemented. `[]string` element
  operations landed with Task 10: the slice owns one reference per element, and
  `pop` on a `[]string` stays refused (see that task's "As built").
  `break`/`continue` nested inside a
  block still run only that block's defers (a pre-existing scope-unwinding gap,
  tracked with Task 10's unwinding limitation).

---

### Task 8: Maps

**Files:**
- Create: `crates/otter-runtime/hashmap.c`; modify `include/otter_runtime.h`,
  `runtime.c` (`otter_str_hash`), `build.rs`, and `src/lib.rs` (the embedded
  sources and the Rust-side declarations)
- Modify: `crates/otter-parser/src/ast.rs`, `types.rs` (`map[K]V`)
- Modify: `crates/otter-sema/src/ty.rs`, `check_expr.rs`, `ownership.rs`
- Create: `crates/otter-codegen/src/maps.rs`; modify `ir.rs`, `module.rs`,
  `expr_gen.rs`, `slices.rs` (share `option_from_flag`)
- Modify: `crates/otter-driver/src/pipeline.rs` (two cached runtime objects)
- Create: `crates/otter-codegen/tests/golden/maps.ot`, `tests/golden/maps.expected.ll`

**Interfaces:**
- Consumes: ledger L1 `otter_map_*`
- Produces: `TypeKind::Map(Box<Type>, Box<Type>)`; `map_new`, `map_get(K) -> Option[V]`, `map_insert(K, V)`, `map_remove(K) -> Option[V]`, `map_len(map) -> i64`, `map_free`

- [x] **Step 1: Write the failing tests**

```rust
#[test]
fn a_map_round_trips_keys_of_every_primitive_key_type() {
    // i64, bool, and string keys each insert and read back.
}

#[test]
fn a_missing_key_yields_none() { /* review focus item 4 */ }

#[test]
fn inserting_an_existing_key_overwrites() {}

#[test]
fn map_len_counts_distinct_keys() {}

#[test]
fn a_map_grows_without_losing_entries() {}

#[test]
fn keys_are_compared_by_value_not_by_pointer() {}

#[test]
fn a_map_is_linear_and_frees_once() {}
```

- [x] **Step 2: Run to verify they fail** → FAIL.

- [x] **Step 3: Implement**

An open-addressing table in C taking function pointers for hash and equality, so codegen emits small Otter-visible thunks per key type. Grow at 70% load. `map_get` returns an `Option` by writing into an out-parameter and returning a presence flag.

- [x] **Step 4: Run to verify they pass** → PASS. Add and read the golden file.

- [x] **Step 5: Commit**

```bash
git add crates/
git commit -m "feat: string and value keyed maps over an open-addressing runtime table"
```

**As built.**

- **One table, a key-kind tag instead of function pointers.** `hashmap.c` keeps
  the three parallel byte arrays (`states` / `keys` / `values`) and a `key_size`
  / `value_size` stride, so one C implementation serves every scalar type.
  `otter_map_new(i64 key_kind, i64 key_size, i64 value_size)` records the kind
  once — `0` = `i64` (splitmix64 over the eight bytes), `1` = `bool`, `2` =
  `string` (`otter_str_hash`, FNV-1a, added to `runtime.c`) — instead of taking
  hash/equality function pointers. Generated code therefore emits no thunks: the
  whole ABI is `i8*` plus slots. Probing is linear, tombstones keep chains
  correct after removals, and the table rehashes into a fresh array once
  occupied + tombstoned entries reach 70% of capacity.
- **Ownership.** A `string` key is a handle: `insert` retains it, an overwrite
  keeps the stored key (no second retain), `remove` and `free` release exactly
  one reference. A failed allocation leaves the table unchanged; `insert`
  reports nothing, the same rule `push` follows.
- **The type and its restrictions.** `map[K]V` is parsed by name only when
  `map` is directly followed by `[`, so an ordinary type named `map` still
  resolves by name. `TypeKind::Map` is linear like a slice (one binding owns the
  table) and lowers to `i8*`. Keys are restricted to `i64`, `bool`, and
  `string` — the kinds the runtime can hash — and values to numbers and `bool`,
  because an owned value would need a destructor on removal. Both restrictions
  are diagnostics at the type expression's own span.
- **`map_new` is inferred, never guessed.** It takes no arguments, so it is only
  typed when the surrounding binding annotation, parameter, or return type is a
  `map[K]V`; `var m = map_new();` is "cannot infer the map's key and value
  types". `map_get`/`map_remove` answer `Option[V]`; `map_len` is `i64`;
  `insert`/`free` are `()`. All five take the map as their first argument and
  require a plain binding — not a temporary, not a field — because `map_free`
  moves the binding and lowering looks it up by name. `len(map)` stays an error;
  the spelling is `map_len`.
- **Inference plumbing changed.** The `var`/`const` annotation used to be
  resolved with `.ok()`, silently dropping an invalid annotation's diagnostic
  and then complaining about the initializer instead. It now reports the type
  error and stops, which is what makes the key/value restrictions visible.
- **Two runtime objects.** The driver embeds and writes both `runtime.c` and
  `hashmap.c`, caches one object per source under a single key that hashes both,
  and passes both to the link (or compiles both as inputs when the cache is
  unusable).
- **Tests as built.** Parser 4 in `tests/maps.rs` (key/value shape, nested map
  value, `Option` key, bare `map` stays a name); sema 12 (three key kinds,
  inference from a parameter and a binding, missing inference, unhashable key,
  owned value, move on `free`, linear copy, `Option` results, wrong key, wrong
  value, temporary/field receivers, `len` rejection); codegen 9
  (`tests/maps.rs`: key kinds and strides, insert slots, lookup flag → `Option`,
  remove, length, free, and the `maps` golden); runtime 10 (empty, insert/get,
  miss leaves the out slot alone, overwrite, remove twice, bool keys,
  content-equal string keys with retain counts, overwrite keeps one reference,
  1000-entry growth with removals, null/invalid tables); driver e2e 1 (`maps`)
  and leaks 1 (`maps_heap`: 200 built string keys, zero live allocations).
- **Goldens.** `golden/maps.{ot,expected.ll}` added; the ten existing goldens
  gained exactly the six `declare`s and nothing else.
- **Deferred.** `map[K]V` values with ownership (strings, slices, maps),
  user-supplied hash/equality, iteration over a map, and a literal form
  (`map[K]V { ... }`) are not implemented. `len` does not measure a map.

---

### Task 9: Module resolution

**Files:**
- Create: `crates/otter-driver/src/modules.rs`
- Modify: `crates/otter-driver/src/pipeline.rs`
- Create: `crates/otter-driver/tests/modules.rs`

**Interfaces:**
- Consumes: `Item::Import` from M0–M2, which is currently rejected
- Produces:
  - `pub fn resolve_import(spec: &str, from: Option<&Path>, roots: &[PathBuf]) -> Result<PathBuf, Diagnostic>` — maps `a.b.c` to `<root>/a/b/c.ot`, and also tries `<root>/a/b/c/mod.ot`
  - `OTTER_PATH`, a `;`-separated (Windows) or `:`-separated (Linux) root list, defaulting to the entry file's directory
  - An import cycle is an error naming the cycle
  - Per-file modules: two files importing the same module get one definition, not two

- [x] **Step 1: Write the failing tests**

```rust
#[test]
fn a_dotted_path_resolves_to_a_nested_file() {}

#[test]
fn a_module_directory_containing_mod_ot_is_found() {}

#[test]
fn a_missing_module_is_a_diagnostic_naming_every_root_tried() {}

#[test]
fn an_import_cycle_is_reported_not_looped_on() {}

#[test]
fn importing_the_same_module_from_two_files_defines_it_once() {}

#[test]
fn a_name_defined_in_two_modules_is_an_ambiguity_error() {}

#[test]
fn otter_path_overrides_the_default_root() {}
```

- [x] **Step 2: Run to verify they fail** → FAIL — `import` is still rejected.

- [x] **Step 3: Implement**

Topologically order modules, type-check each once, and merge their exported names into one root scope. Reject a name defined twice.

- [x] **Step 4: Run to verify they pass** → PASS.

- [x] **Step 5: Commit**

```bash
git add crates/otter-driver/
git commit -m "feat: module resolution, OTTER_PATH, and cycle detection"
```

**As built.**

- **One source map, many files.** The driver's CLI now builds a single
  `SourceMap` for the whole program and the loader adds each imported file to it
  as it reads that file. The entry point is still file id zero, because it is
  added first; dependencies get later ids even though they precede the entry in
  the module list. `render` therefore takes the map the diagnostics were
  produced against instead of rebuilding a one-file map, which is what lets a
  caret point into the module that made the mistake.
- **The loader** (`crates/otter-driver/src/modules.rs`) walks the import graph
  depth-first. `resolve_import(spec, from, roots)` tries, per root,
  `<root>/a/b/c.ot` and then `<root>/a/b/c/mod.ot`, and its failure message lists
  every candidate it tried. `module_roots(entry)` is `OTTER_PATH` split on `;`
  (Windows) or `:` (elsewhere) when it has a non-empty entry, and the entry
  file's directory otherwise — the variable *replaces* the default, so a build
  can be pinned. An empty `OTTER_PATH` counts as unset.
- **De-duplication and cycles.** Files are keyed by canonical path, so the same
  module reached by three importers is read, parsed and defined once. A path
  that is still on the loader's stack is a cycle, reported as
  `import cycle: a.ot -> b.ot -> a.ot` at the `import` that closed it. Loading is
  recursive, so a chain deeper than 64 modules is refused with a stack-safety
  diagnostic rather than left to overflow.
- **Checking an import graph.** `Checker::check_modules` takes
  `&[(FileId, &Program)]`, declares every module's items before any body is
  read, then checks each module with `self.file` set to that module. Errors are
  stored with the file that was current when they were reported
  (`Vec<(FileId, TypeError)>`), which is the whole of the multi-file support in
  sema; `check_program` and `check_program_with` are now one-module wrappers.
  `check_modules_with` is the public entry point.
- **Ambiguity is the existing duplicate error.** All top-level names share one
  root scope, so a `fn thing` in two modules is `duplicate definition of `thing``
  at the second module's definition — the same message and caret a redefinition
  in one file gets.
- **Pipeline split.** `compile_hir(hir, entry_name, out, toolchain, options)` is
  the emit-and-link half, `load_and_check(map, entry_path, source, release)` is
  load + parse + check, and `compile_to_binary` remains the single-file
  convenience it always was (implemented on top of `compile_hir`). The CLI uses
  the two halves directly so `check` and `--emit-ir` work without an LLVM
  toolchain and share the map with `render`.
- **Tests as built.** Ten in `crates/otter-driver/tests/modules.rs`: the seven
  named above plus `a_module_search_prefers_the_flat_file_then_the_directory`
  (direct `resolve_import` unit test and shape order),
  `check_accepts_a_multi_file_program_without_building_it` (the `check`
  subcommand resolves imports and writes no executable), and
  `an_error_in_a_module_names_the_module_not_the_entry_point` (a returning
  `string` from `-> i64` names `lib/broken.ot`, never `main.ot:2`).
- **Deferred.** There is no `pub`/private visibility — every top-level name is
  exported and a collision is an error; imports are not re-exported; a reference
  is always unqualified (`seven()`, never `lib.util.seven`); a module is
  resolved against the search roots, not relative to its importer, unless the
  importer's directory is itself a root; and module-level initialization order
  does not exist (there are no top-level statements).

---

### Task 10: Core standard library

**Files:**
- Create: `crates/otter-stdlib/otter/{io,string,math,os,alloc}.ot`
- Create: `crates/otter-driver/tests/stdlib.rs`

**Interfaces:**
- Consumes: everything above
- Produces: `io.println(string)`, `io.print`, `io.read_line() -> Result[string, string]`, `io.write_file(string, string) -> Result[i64, string]`, `io.read_file(string) -> Result[string, string]`; `string.concat`, `string.split`, `string.trim`, `string.contains`, `string.starts_with`; `math.sqrt/abs/min/max/pow`; `os.args() -> []string`, `os.exit`, `os.getenv`; `alloc` helpers

**Every exported stdlib function documents its ownership in a comment directly above it** — which function releases which allocation. Users copy this code.

- [x] **Step 1: Write the failing tests**

```rust
#[test]
fn every_stdlib_module_compiles() {}

#[test]
fn println_writes_to_stdout_and_flushed_on_exit() {}

#[test]
fn file_round_trip_writes_and_reads_back_exactly() {}

#[test]
fn read_line_returns_result_on_a_missing_stream() {}

#[test]
fn string_split_and_trim_behave_on_edge_cases() {
    // Empty input, only delimiters, leading and trailing whitespace.
}

#[test]
fn os_args_round_trips_the_command_line() {}

#[test]
fn the_stdlib_leaks_nothing_under_the_leak_checker() {
    // Every stdlib example program. This is Global Constraints' memory rule.
}
```

- [x] **Step 2: Run to verify they fail** → FAIL.

- [x] **Step 3: Implement**

Write the stdlib in Otter itself, compiled by the compiler being built. That is the first real proof the language is usable, so write it plainly.

- [x] **Step 4: Run to verify they pass** → PASS.

- [x] **Step 5: Commit**

```bash
git add crates/otter-stdlib/ crates/otter-driver/
git commit -m "feat(stdlib): io, string, math, os and alloc modules"
```

**As built.**

- **One flat namespace, so no dotted names.** Task 9 put every module's items in
  one root scope. `io.println` is therefore a name the checker cannot spell: the
  call is `println(text)`, and the module a file imports does not qualify it. The
  library keeps the plan's *functions*, not the plan's *paths*:
  `io.ot` exports `print`, `println`, `println_i64`, `println_f64`; `string.ot`
  exports `concat`, `contains`, `starts_with`, `trim`, `split`; `math.ot`
  exports `abs`, `min`, `max`, `pow`, `sqrt`; `alloc.ot` exports `alloc_zeroed`
  and `alloc_one`; `os.ot` exports `exit_with`. Qualified access is recorded
  with Task 9's other one-scope deferrals.
- **Builtins keep their own names.** `read_file`, `write_file` and `getenv` are
  runtime-backed builtins, available to every program without an import, so no
  module may also define them. `io.ot`'s header documents the file pair and
  `os.ot`'s documents `getenv`; wrapping them would have required a second name
  for the same function.
- **Ownership at the boundary.** `read_file(path)` answers with a fresh owned
  string (the empty string on failure, which is also an empty file);
  `write_file(path, body)` copies only the path to a C string, lends the body's
  bytes with its length (a body may contain a NUL, a C string may not), frees
  the path copy, and answers 1 or 0; `getenv(name)` answers with a fresh owned
  string, or the empty string when unset. `str_slice(s, start, end)` clamps both
  bounds and an inverted range is empty, so index arithmetic cannot fault. The
  ambiguities are the signatures' honest limit, not an oversight: a builtin
  cannot spell `Result` until aggregate return values carry string ownership
  (see Deferred).
- **A call now has ownership rules for borrowed arguments.** Writing the library
  exposed a general gap: a `string` temporary passed to a function the program
  wrote was never released, and a `string` a user function returned was not
  marked owned, so `println(trim(x))` leaked the trimmed text.
  `gen_borrowed_call` lowers the arguments once, releases any owned temporary
  after the call (the callee borrows it), and marks a `string` result owned (the
  callee's `return` already retains a local on the way out). The reading builtins
  (`parse_i64`, `str_slice`, ...) go through it too, which closes the same leak
  for them. Every golden's IR is unchanged except for the four new `declare`
  lines.
- **`sqrt` is Newton's method.** An `extern` declaration's name *is* the symbol
  it calls (M5 Task 2), so a wrapper named `sqrt` cannot sit beside an
  `extern fn sqrt`. Rather than add a link-name attribute, `math.ot` implements
  the iteration in Otter: 32 steps from `guess = x`, a panic on a negative
  argument. `pow` is repeated checked multiplication with a non-negative
  exponent; `abs(i64::MIN)` panics like every other checked subtraction.
- **Tests as built.** Runtime `tests/files.rs` (5): a slice clamps each bound;
  a file round-trips a body containing a NUL; a missing file and a null path
  read empty and a null path cannot be written; an unset variable reads empty;
  a set variable is an owned string with refcount 1. Sema `tests/stdlib.rs` (5):
  the four signatures and their argument type errors. Codegen
  `tests/stdlib.rs` (4): one path copy for `read_file`, one for `getenv`, the
  body lent (not copied) with its length for `write_file`, and `str_slice` taken
  over rather than retained. Driver `tests/stdlib.rs` (3): all five modules
  check together (which is also the collision test), the full report matches
  byte for byte with `0 live allocations`, and a file round trip writes, reads,
  splits and frees, matching `line one/line two/` and releasing everything.
  `tests/e2e.rs` also gained `string_slices` and `tests/leaks.rs` the matching
  heap witness for the string-element slices this task needed.
- **Deferred.** `os.args()` needs `main` to receive `argc/argv`; `main` is
  emitted as `i32 @main()`, and changing the entry signature is a change every
  program pays for, so it waits for a spec decision. `io.read_line() ->
  Result[string, string]` waits for aggregate string ownership: a `case Some(s)`
  binding is untracked, so a payload the aggregate owns has no releaser yet.
  A `!` return type cannot be *declared* (`exit_with` is `()` and still never
  returns). `trim`/`split` are byte-oriented, not character-oriented. The stdlib
  is not discovered automatically: a program finds it through `OTTER_PATH`, and
  the driver tests point that at `crates/otter-stdlib/otter`.
- **Limitations carried forward.** Cross-frame unwinding and a `break` nested in
  a block (only that block's defers run) are unchanged from Task 7 and remain
  the Task 10 review items.

---

### Task 11: Container end-to-end programs

**Files:**
- Create: `crates/otter-driver/tests/programs/{json_count,wc}.ot`
- Modify: `crates/otter-driver/tests/e2e.rs`

**Interfaces:**
- Consumes: everything
- Produces: two multi-file programs proving the language is now general purpose

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_multi_file_program_counts_json_keys() {
    // Two files, a map, a slice of strings, and a loop. Prints a count.
}

#[test]
fn a_word_count_program_works_on_a_multi_line_input() {}

#[test]
fn both_programs_leak_nothing() {}
```

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Write the programs and run them** → PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/otter-driver/
git commit -m "test: multi-file end-to-end programs using containers"
```

---

### Task 12: Run the suite on both platforms

**Files:**
- Create: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: the whole workspace test suite
- Produces: a CI matrix of `windows-latest` and `ubuntu-latest` running `cargo test --workspace`

- [ ] **Step 1: Write the workflow** with `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test --workspace`, and the 120-second fuzz targets.

- [ ] **Step 2: Push and confirm both jobs are green** before marking M5–M6 done.

- [ ] **Step 3: Commit**

```bash
git add .github/
git commit -m "ci: Windows and Linux matrix for the full workspace"
```

**As built so far (commit `ef3230a`).** The workflow landed *before* Task 1,
not at Task 12: the definition of done requires Windows and Linux from M5
onward, and no workflow existed, so waiting would have deferred the first
platform signal until after every implementation task. It runs, on
`windows-latest` and `ubuntu-latest`:

```text
cargo check --workspace --all-targets
cargo test --workspace --no-fail-fast
```

It installs LLVM on Linux and verifies `clang` on Windows before the suite,
because the driver shells out to `clang` for every end-to-end test.

Deliberately **not** in it yet: `cargo fmt --check` (rustfmt is not a gate in
this repository by standing decision, so a formatting failure would report
style rather than correctness) and `cargo clippy -- -D warnings` (blocked on
this development host by Application Control, so the workflow could not be
validated before pushing). Fuzz targets arrive with the fuzz task; the fuzzer
is not installed here, so no fuzz job is claimed. Step 2 (both jobs green)
remains open until this workflow has run on `origin`.

---

### Task 13: Close the ledger

**Files:**
- Modify: `docs/superpowers/plans/2026-10-16-otter-master-plan.md`

**Interfaces:**
- Consumes: the whole M5–M6 implementation
- Produces: L1, L2, L3 and L5 updated to match reality

- [ ] **Step 1: Compare the ledger against the code** — every runtime symbol, every `TypeKind` variant, every builtin.

- [ ] **Step 2: Write every divergence into the master plan in this commit**, not later.

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/plans/2026-10-16-otter-master-plan.md
git commit -m "docs: close the interface ledger for M5-M6"
```

---

## Definition of Done for M5–M6

- `libc.ot` calls C and prints correct results on both platforms.
- Slices, maps, `for`-in, and multi-file imports all work; `json_count.ot` and `wc.ot` run and leak nothing.
- `cargo test --workspace` is green on Windows **and** Linux in CI.
- The master plan's L1–L5 match the code exactly.
- Every stdlib export has an ownership comment.
- Review Focus item 4 holds for every operation added here: slice index, map miss, and division all produce values or panics, never UB.