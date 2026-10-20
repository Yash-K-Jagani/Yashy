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
  uses. `void` as a *parameter* is therefore also rejected today as "C type
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
- Modify: `crates/otter-codegen/src/expr_gen.rs`
- Create: `crates/otter-codegen/tests/golden/ffi_copy.ot`, `tests/golden/ffi_copy.expected.ll`

**Interfaces:**
- Consumes: Task 2
- Produces: `pub fn gen_ffi_call(ctx: &mut FnCtx, decl: &HirExternDecl, args: &[GenValue]) -> String` — copies each `string` and `Slice` argument into temporary runtime-owned storage before the call, and copies an out-parameter result back

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_string_argument_to_an_extern_call_is_copied_not_passed_by_pointer() {
    // The IR must show a copy into a temp before the call. This is spec §8.1's
    // "crossing the boundary copies".
}

#[test]
fn a_slice_argument_is_copied_with_its_length() {}

#[test]
fn a_raw_pointer_argument_is_passed_unchanged_and_requires_an_unsafe_call() {}

#[test]
fn the_temporary_is_released_after_the_call() {}
```

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement**

An `unsafe` marker on the *call site* is required to bypass copying. Without it, string and slice arguments are copied and C cannot retain them.

- [ ] **Step 4: Run to verify they pass** → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-codegen/
git commit -m "feat(ffi): copy string and slice arguments at the C boundary"
```

---

### Task 4: Linking and platform toolchain

**Files:**
- Modify: `crates/otter-driver/src/pipeline.rs`
- Modify: `crates/otter-driver/src/toolchain.rs`
- Create: `crates/otter-driver/tests/link.rs`

**Interfaces:**
- Consumes: M0–M2 driver, Task 2's link directives
- Produces: `pub fn link_directives(hir: &HirProgram) -> Vec<String>`; platform-specific library flags; a test that builds and runs the same program on both platforms

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_link_directive_adds_the_library_flag() {}

#[test]
fn math_functions_link_without_an_explicit_directive() {
    // sqrt needs libm on Linux, is folded into the CRT on Windows.
}

#[test]
fn the_same_program_produces_identical_output_on_both_platforms() {
    // Runs only on the host; the CI matrix covers the other.
}
```

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement**

Collect directives, append platform defaults (`-lm -lpthread -ldl` on Linux, the CRT on Windows), and pass them to the linker. Report a clear diagnostic naming the library when linking fails for that reason.

- [ ] **Step 4: Run to verify they pass** → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-driver/
git commit -m "feat(driver): link directives and platform library flags"
```

---

### Task 5: Call libc

**Files:**
- Create: `crates/otter-driver/tests/programs/libc.ot`
- Modify: `crates/otter-driver/tests/e2e.rs`

**Interfaces:**
- Consumes: Tasks 1–4
- Produces: an end-to-end program calling `puts`, `strlen`, and `malloc`/`free` through `extern`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn libc_is_reachable_from_otter() {
    // Prints a line via puts and prints the length strlen returns.
}

#[test]
fn a_wrong_signature_against_a_real_c_function_is_a_type_error() {
    // Declaring strlen as returning i32 and printing it still links; the point
    // is that Otter's checker, not C's, catches the mismatch.
}
```

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Write the program and run it** → PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/otter-driver/
git commit -m "test: end-to-end libc interop"
```

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

- [ ] **Step 1: Write the failing tests**

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

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement**

`{ i8*, i64, i64 }` exactly matching `otter_slice`. `otter_slice_push` reallocs with a doubling policy when `len == cap`. Indexing yields `Option[T]`, never a trap — Review Focus item 4.

- [ ] **Step 4: Run to verify they pass** → PASS. Add and read the golden file.

- [ ] **Step 5: Commit**

```bash
git add crates/
git commit -m "feat: slices as linear values with a doubling backing array"
```

---

### Task 7: Slice operations and iteration

**Files:**
- Modify: `crates/otter-sema/src/builtins.rs`
- Modify: `crates/otter-codegen/src/slices.rs`
- Modify: `crates/otter-parser/src/stmts.rs` (`for x in slice`)

**Interfaces:**
- Consumes: Task 6
- Produces: `len(slice) -> i64`, `push`, `pop() -> Option[T]`, `slice[i64] -> Option[T]`, `slice[a..b] -> Slice[T]`, `contains`, and `for x in slice { }`

- [ ] **Step 1: Write the failing tests**

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

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement**

`for x in slice` allocs a hidden index, loads the element each iteration, and re-reads `len` each time so a `push` inside the loop is visible. Moving the slice inside the body is a linearity error.

- [ ] **Step 4: Run to verify they pass** → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/
git commit -m "feat: slice operations and for-in iteration"
```

---

### Task 8: Maps

**Files:**
- Create: `crates/otter-runtime/hashmap.c`, modify `include/otter_runtime.h`
- Modify: `crates/otter-parser/src/exprs.rs` (`map[K]V`)
- Modify: `crates/otter-sema/src/ty.rs`, `builtins.rs`
- Create: `crates/otter-codegen/src/maps.rs`
- Create: `crates/otter-codegen/tests/golden/maps.ot`, `tests/golden/maps.expected.ll`

**Interfaces:**
- Consumes: ledger L1 `otter_map_*`
- Produces: `TypeKind::Map(Box<TypeKind>, Box<TypeKind>)`; `map_new`, `map_get(K) -> Option[V]`, `map_insert(K, V)`, `map_remove(K) -> Option[V]`, `map_len(map) -> i64`, `map_free`

- [ ] **Step 1: Write the failing tests**

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

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement**

An open-addressing table in C taking function pointers for hash and equality, so codegen emits small Otter-visible thunks per key type. Grow at 70% load. `map_get` returns an `Option` by writing into an out-parameter and returning a presence flag.

- [ ] **Step 4: Run to verify they pass** → PASS. Add and read the golden file.

- [ ] **Step 5: Commit**

```bash
git add crates/
git commit -m "feat: string and value keyed maps over an open-addressing runtime table"
```

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

- [ ] **Step 1: Write the failing tests**

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

- [ ] **Step 2: Run to verify they fail** → FAIL — `import` is still rejected.

- [ ] **Step 3: Implement**

Topologically order modules, type-check each once, and merge their exported names into one root scope. Reject a name defined twice.

- [ ] **Step 4: Run to verify they pass** → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-driver/
git commit -m "feat: module resolution, OTTER_PATH, and cycle detection"
```

---

### Task 10: Core standard library

**Files:**
- Create: `crates/otter-stdlib/otter/{io,string,math,os,alloc}.ot`
- Create: `crates/otter-driver/tests/stdlib.rs`

**Interfaces:**
- Consumes: everything above
- Produces: `io.println(string)`, `io.print`, `io.read_line() -> Result[string, string]`, `io.write_file(string, string) -> Result[i64, string]`, `io.read_file(string) -> Result[string, string]`; `string.concat`, `string.split`, `string.trim`, `string.contains`, `string.starts_with`; `math.sqrt/abs/min/max/pow`; `os.args() -> []string`, `os.exit`, `os.getenv`; `alloc` helpers

**Every exported stdlib function documents its ownership in a comment directly above it** — which function releases which allocation. Users copy this code.

- [ ] **Step 1: Write the failing tests**

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

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement**

Write the stdlib in Otter itself, compiled by the compiler being built. That is the first real proof the language is usable, so write it plainly.

- [ ] **Step 4: Run to verify they pass** → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-stdlib/ crates/otter-driver/
git commit -m "feat(stdlib): io, string, math, os and alloc modules"
```

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