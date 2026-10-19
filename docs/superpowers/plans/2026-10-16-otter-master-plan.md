# Otter — Master Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Coordinate the complete construction of the Otter language, from an empty repository to a self-hosting compiler, across five phase plans.

**Architecture:** This document holds no implementation tasks. It is the cross-plan contract: the interface ledger that five phase plans must agree on, the dependency order between milestones, and the global constraints that bind all of them. Read it before any phase plan.

**Tech Stack:** Rust 1.99 (edition 2021), LLVM IR emitted as text, `llc`/`clang` as subprocesses, a C runtime shim, `cargo-fuzz`.

**Spec:** `docs/superpowers/specs/2026-10-16-otter-language-design.md`

## The Phase Plans

| Phase plan | Milestones | Tasks | Estimate |
|---|---|---|---|
| `2026-10-16-otter-m0-m2-compiler.md` | M0–M2 | 14 | 8–12 weeks |
| `2026-10-16-otter-m3-m4-memory-and-faults.md` | M3–M4 | 13 | 11–17 weeks |
| `2026-10-16-otter-m5-m6-ffi-and-containers.md` | M5–M6 | 13 | 7–10 weeks |
| `2026-10-16-otter-m7-m8-concurrency-and-tooling.md` | M7–M8 | 10 | 8–12 weeks |
| `2026-10-16-otter-m9-m10-optimization-and-bootstrap.md` | M9–M10 | 8 | 12–24 weeks |

Cumulative: **46–75 weeks** (11–18 months) at 25–30 effective hours/week. See
spec §10.1.

## Dependency Order

```
  M0 ─▶ M1 ─▶ M2          front-end + codegen scaffold
          │
          ▼
        M3 ──────────▶ M4        memory, then faults
          │  (M4 needs M3's defer paths)
          ▼
        M5 ──────────▶ M6        FFI, then containers/modules
          │
          ▼
        M7 ──────────▶ M8        threads, then tooling
          │
          ▼
        M9 ──────────▶ M10       escape analysis, then bootstrap
```

Two dependencies are load-bearing and easy to get wrong:

- **M4 depends on M3.** Unwinding runs pending `defer` handlers. A `defer` that
  does nothing cannot demonstrate unwind semantics. Do not attempt M4 first.
- **M10 depends on M9 being optional.** Self-hosting surfaces every missing
  feature at once, but it does **not** require escape analysis to be fast. M9
  may be incomplete when M10 starts; M10 may not start until M9's *correctness*
  harness is green, so a miscompile is distinguishable from a missed
  optimization.

M9 and M10 are the only milestones where deferring is acceptable. Everything
else is strictly ordered.

---

## Interface Ledger

**This is the single source of truth for every signature that crosses a
milestone boundary.** A phase plan may add to these lists but may not change an
entry already here. If implementation forces a change, the change is made in
this file first, with a ledger entry explaining the cost.

### L1 — `otter_runtime` C ABI

Every symbol below is callable by generated Otter code. The `otter-runtime`
crate owns the implementations; `otter-codegen` owns the emission. Adding a
symbol here is a two-place change and both tasks must land together.

**M0–M2 (Task 0, 10):**

```c
void otter_print_str(i8*);
void otter_print_i64(i64);
void otter_print_f64(double);
```

**M3 (memory, strings):**

```c
i8*   otter_alloc(i64 size, i64 align);
void  otter_free(i8* ptr);

otter_arena* otter_arena_new(void);
i8*   otter_arena_alloc(otter_arena*, i64 size, i64 align);
void  otter_arena_free_all(otter_arena*);

otter_str* otter_str_new(i8* data, i64 len);
void       otter_str_retain(otter_str*);
void       otter_str_release(otter_str*);
otter_str* otter_str_concat(otter_str*, otter_str*);
otter_str* otter_str_from_i64(i64);
otter_str* otter_str_from_f64(double);
otter_str* otter_str_from_bool(i1);
i64        otter_str_len(otter_str*);
i8*        otter_str_data(otter_str*);
i8         otter_str_byte_at(otter_str*, i64);   // returns 0 out of range
```

> **L1 revision, M3 interim: the string half ships in the M0–M2 representation.**
> A `string` value is an `i8*` to NUL-terminated bytes, not an `otter_str*`, so
> the six signatures above that mention `otter_str*` are not what M3 emits. What
> M3 emits and the runtime defines is:
>
> ```c
> i64         otter_str_len(i8*);
> i64         otter_str_byte_at(i8*, i64);
> i8*         otter_str_concat(i8*, i8*);
> i8*         otter_str_from_i64(i64);
> i8*         otter_str_from_f64(double);
> i8*         otter_str_from_bool(i64);   // i64, not i1: C has no one-bit type
> i1          otter_str_eq(i8*, i8*);
> ```
>
> The lengths are recovered with `strlen`, the results are built in one internal
> arena **per thread** that is released at process exit, and nothing is retained
> or released. (Per thread, not shared: an arena is an unsynchronized bump
> pointer, so a shared one handed two threads the same block — see M3–M4 Task 5f.)
> Cost: a built string is held until the program exits, and an embedded NUL byte
> truncates. The `otter_str*` half of this table lands together with the
> `%otter_str` retype below, in M3's remaining string task; until then a program
> that builds strings is correct and simply does not give the memory back.

**Input (added with the M3 string operations, because a program that cannot read
is not yet a program):**

```c
i8*  otter_read_line(void);          // one line, no terminator; "" at end of input
i64  otter_parse_i64(i8*);           // 0 when the whole string is not an integer
f64  otter_parse_f64(i8*);
```

These are compiler builtins (`read_line`, `parse_i64`, `parse_f64`) resolved like
any other name, so M6's `io.readLine` replaces them rather than joining them. The
line is built in the same never-freed string arena as every other produced string.
`otter_read_line` returns the empty string at end of input, which the language
cannot currently distinguish from an empty line; the parsers reject trailing text
(`parse_i64("12x")` is 0, not 12) and fold overflow into the same zero, so each
has exactly one contract and no in-between. Checked forms wait for M4's `panic`.

**M4 (faults):**

```c
noreturn void otter_panic(i8* msg, i64 msg_len, i8* file, i64 line, i64 col);
noreturn void otter_assert_failed(i8* expr_src, i8* file, i64 line, i64 col);
```

Checked arithmetic is **not** a runtime call. M4 emits the compare and the
conditional branch to an inline panic block in generated code (see L4).

**M6 (containers):**

```c
typedef struct { i8* data; i64 len; i64 cap; } otter_slice;

void  otter_slice_push(otter_slice*, i8* elem, i64 elem_size);
void  otter_slice_reserve(otter_slice*, i64 needed_elems, i64 elem_size);
void  otter_slice_free(otter_slice*);

otter_map* otter_map_new(i8* hash_fn, i8* eq_fn, i64 key_size, i64 val_size);
i8         otter_map_get(otter_map*, i8* key, i8* out_val);  // 1 if present
void       otter_map_insert(otter_map*, i8* key, i8* val);
void       otter_map_remove(otter_map*, i8* key);
i64        otter_map_len(otter_map*);
void       otter_map_free(otter_map*);
```

**M7 (concurrency):**

```c
i32 otter_thread_spawn(i8* fn_ptr, i8* arg, i64 arg_size);
i32 otter_thread_join(i32 handle);

otter_mutex* otter_mutex_new(void);
void         otter_mutex_lock(otter_mutex*);
void         otter_mutex_unlock(otter_mutex*);
void         otter_mutex_free(otter_mutex*);

otter_chan*  otter_chan_new(i64 capacity);
i8           otter_chan_send(otter_chan*, i8* item, i64 size);
i8           otter_chan_recv(otter_chan*, i8* out_item, i64 size);
void         otter_chan_close(otter_chan*);
void         otter_chan_free(otter_chan*);
```

### L2 — Type representation

`TypeKind` in `otter-sema::ty` grows by variant. Each addition changes LLVM
layout, so it also changes what every previously emitted program means. A
variant may only be added by a milestone that also regenerates every golden
file.

| Milestone | `TypeKind` additions | LLVM representation |
|---|---|---|
| M0–M2 | `I64 F64 Bool Str Unit Struct Enum Option Result Func Never` | see plan L-equivalent in `otter-codegen::ir` |
| M3 | `Ptr(Type)` | `i8*` — a raw pointer to an LLVMLayout of the pointee |
| M6 | `Slice(Type)` | `{ i8*, i64, i64 }` matching `otter_slice` |
| M6 | `Map(Type, Type)` | `{ i8* }` — pointer to the opaque `otter_map` |
| M5 | `CInt(i32) CUInt(i32) CFloat(i32) CVoid CPtr(Type) CSize` | C spellings, `i1..i64`, `double`, `void`, `i8*`, `i64` |

**`Str` layout changes exactly once, in M3, and never again:**

```llvm
%otter_str = type { i32, i64, i8* }   ; refcount (atomic), length, data
```

M0–M2 uses `%otter_str = type { i32, i32, i8* }` with the first field always
`1` and no refcount operations. M3 widens it to `i64` and adds the retain and
release calls. **No milestone after M3 changes this layout** — which is why M3
is a milestone boundary and not an incremental edit.

**A `Struct` is an aggregate value, laid out in declaration order:**

```llvm
%struct.Point = type { i64, i64 }
```

A literal is built with `insertvalue` into `undef` and a field is read with
`extractvalue` at the field's declaration index, so neither needs an address. The
redeclaration rule — `define_named_type` keeps the first shape it is handed —
is what makes the index mean one thing: a literal's fields reach the HIR in
declaration order, so two literals of one struct written in different orders
produce the same named type (M3–M4 Task 5e). A struct *behind* a pointer
(`*mut P`, `p.x = 1`) does need an address, and is M6's.

### L3 — Builtin function set

The set of non-`extern` functions the compiler knows without a declaration.
Each milestone adds to it. Sema predeclares all of these in the root scope.

| Milestone | Builtins |
|---|---|
| M0–M2 | `print_str(string)`, `print_i64(i64)`, `print_f64(f64)` |
| M3 | `alloc(i64) -> *mut i64`, `dealloc(*mut i64)`, `Arena.new()`, `arena.alloc(i64) -> *mut i64`, `arena.free()`, `len(string)`, string constructors and accessors from L1 |
| M3, as built ahead of the row above | `read_line() -> string`, `parse_i64(string) -> i64`, `parse_f64(string) -> f64`, `str_from_i64(i64) -> string`, `str_from_f64(f64) -> string`, `str_from_bool(bool) -> string` (L1's input and conversion notes) |
| M4 | `panic(string)`, `assert(bool)`, `debug_assert(bool)`, `assert_eq` (typed per pair), `os.exit(i32)`, `wrapping_add/sub/mul(i64,i64)`, `checked_add/sub/mul(i64,i64) -> Option[i64]`, `saturating_add/sub/mul(i64,i64)` |
| M6 | `len(slice) -> i64`, `slice_push`, `map_new/get/insert/remove/len`, string `len`, indexing |
| M7 | `Thread.spawn(fn, args) -> i32`, `thread_join(i32)`, `Mutex.new()`, `mutex.lock/unlock`, `Channel.new(i64)`, `chan.send/recv/close` |

> **L3 revision, M3: `alloc`/`dealloc` are over `*mut i64`, not `*mut u8`.**
> Otter has no `u8`, and M3 has neither indexing nor pointer casts, so a byte
> pointer would be a value no program could do anything with. `*mut i64` is
> usable the moment M6 adds indexing, and the M3–M4 plan's own `buffer.ot`
> already assumed `*mut i64`. Cost: a program that wants raw bytes waits for
> M6's `Slice`; nothing else moves, because `Ptr(_)` is `i8*` in the IR whatever
> the pointee type is.

### L4 — Codegen contracts that later milestones replace

These are the interim behaviours pinned by golden files. Each is deliberately
wrong-in-a-documented-way until the named milestone fixes it, so the change is a
diff a reviewer can read rather than a silent behaviour shift.

| Pinned in | Interim behaviour | Replaced by |
|---|---|---|
| M0–M2 Task 11 | `Add Sub Mul` emit `add/sub/mul` with `nsw`/`nuw` omitted; `Div Rem` emit bare `sdiv`/`srem` with no zero check | M4: compare + branch to an inline panic block; `wrapping_*` keeps the bare op |
| M0–M2 Task 12 | `defer` lowers statically — reverse-order emission before each scope exit, and it captures the linear bindings in scope at registration (M3–M4 Task 5c) | M4: landing pad with `cleanup`, so defers also run on unwind |
| M0–M2 Task 3, updated in M3 | `Str` is an `i8*` to NUL-terminated bytes; `+`, `==`/`!=` and `len` are runtime calls, and a built string lives in one arena that is never freed (ledger L1's M3 note). Interpolation (M3–M4 Task 7) is lowered to exactly these calls, so it added no contract of its own | M3's remaining string task: length-prefixed `%otter_str` with retain/release; indexing a pointer in M6 |
| M3 | every `alloc` is explicit, but overwriting a live linear binding elides the *free* the source never spells (see L5) | M9: escape analysis removes provably non-escaping allocations |

### L5 — Ownership and linearity

Introduced in M3, unchanged until M10.

- A binding whose type is `Ptr(T)` or `Slice(T)` is **linear**: it has at most
  one live owner.
- `move x` transfers ownership; `x` is dead afterwards and any read is an
  error.
- Assigning a linear binding to another name without `move` is an error.
- Rebinding a linear name with `var` installs a fresh owner: whatever the binding
  held — live or moved — it owns a value again afterwards. This is deliberately
  permissive (the phase plan first said "error if the compiler cannot prove the
  old value was not moved"): `dealloc(buf); buf = alloc(32);` is a correct
  re-initialisation, and rejecting it would enforce a rule the checker cannot
  justify. Ownership state is restored per branch across an `if` and merged by
  union, so a value moved in either branch counts as moved afterwards.
- Sema tracks this per binding, not per allocation. Two `move`s of the same
  binding are caught; aliasing a raw pointer obtained through `extern` is not,
  which is what `unsafe` marks (spec §8.1).
- Overwriting a live linear binding releases the old value first. The checker
  decides this per assignment (`release_previous` in the HIR) rather than letting
  the backend load the slot and free it blindly: after `dealloc(buf)`, or after
  `consume(buf)` where `consume` takes `*mut i64`, the slot still holds the
  address but the block is already free. Only the ownership state distinguishes
  "a pointer to release" from "a dangling pointer the program already released",
  so the flag travels with the assignment rather than being recomputed.
- A `defer` body is checked when the `defer` is **reached** but runs at scope
  exit, so the backend captures the linear bindings in scope at registration into
  snapshot slots and the body reads those (Task 5c of the M3–M4 plan). Otherwise
  the state the checker approved and the state the body executes against can
  differ, and the difference is a double free rather than a diagnostic. The
  capture covers **linear** bindings only: a deferred body that reads a
  copy-typed binding still sees its exit-time value, which cannot free anything
  twice and avoids a load and a store per binding per registration.

---

## Global Constraints

These bind every phase plan. A phase plan may add constraints; it may not
weaken these.

- **Platforms:** Windows x86-64 and Linux x86-64, both first-class from M0.
  Every end-to-end test runs on both from M5 onward.
- **No LLVM build dependency.** The crate tree never links LLVM. LLVM is a
  subprocess only. The reason: a contributor can `cargo build` with no LLVM
  development environment.
- **`llc` does not exist on Windows.** The Windows LLVM distribution ships
  `clang.exe` and `clang-cl.exe` but no `llc.exe`. `otter-driver::toolchain`
  records which candidate won in `Toolchain.found`, and the pipeline branches on
  it. Only Linux has a real `llc` path; Windows always goes through
  `clang -x ir`.
- **Assume no MSVC.** A machine may have no Visual Studio and no MSVC CRT.
  Windows linking goes through `clang`, never a hand-written `link.exe` command
  line, so the same code works with or without MSVC present.
- **The Rust toolchain target is not pinned to MSVC.** On a machine with no MSVC,
  a `rustup` directory override to a `-gnu` toolchain is required. That is
  machine state, not repository state; the repository must not hard-code a
  target that only works on the author's machine.
- **LLVM 17+ opaque pointers.** All pointers are `i8*`; typed pointer syntax
  (`i64*`) is never emitted. `target datalayout` and `target triple` come from
  the M0–M2 module header and are not restated by later tasks.
- **The compiler never panics on user input.** No `panic!`, no `unwrap()` on a
  value derived from source text, no stack overflow on deep nesting (a 256-deep
  limit, from M0–M2 Task 14) in any milestone. A malformed program yields a
  diagnostic and exit code 1.
- **Every diagnostic carries a span and renders with a caret.** New error sites
  in later milestones use `otter_common::Diagnostic`, never `eprintln!`.
- **Naming is frozen.** The type keywords, the builtin names in L3, and the
  runtime symbols in L1 do not get renamed by a later milestone. A rename is a
  spec change, not an implementation choice.
- **Golden files are the contract.** Any change to emitted IR must be a
  deliberate, reviewed diff of a `.expected.ll` file. The implementer reads
  each regenerated golden file; an unread regeneration is an incomplete task.
- **Mutation testing must not leave stale build artifacts.** `otter-runtime`'s
  `build.rs` uses `cargo:rerun-if-changed`, which is mtime-based. Restoring a C
  source with a tool that preserves the old mtime leaves a stale static library
  and a test that passes against old code or fails against new code for no real
  reason. This actually happened: a stale `libotter_runtime.a` made
  `otter_print_str` appear to append a CRLF. After any mutation-testing session
  touching a C file, run `cargo clean -p <crate>` before trusting results.
- **No premature features.** Generics, closures, operator overloading, Wasm
  output, C++ interop, package-manager version resolution, and any form of GC
  remain out of scope for M0–M10 per spec §1. The bootstrap in M10 uses only
  features M0–M8 deliver.
- **Commit format:** conventional commits, `git add` with explicit paths, no
  `git push` from an implementer subagent.
- **Memory-leak freedom in the stdlib.** Every stdlib allocation has a visible
  owner and a `defer` release. The stdlib is reviewed against this rule
  specifically, because users will copy it as a template.

## Review Focus

Failure modes the spec implies that no milestone names explicitly. Each has an
owning task with a test. A reviewer of any phase should read this list first.

1. **A `defer` that does not run when its scope exits early.** Every `return`,
   `break`, and `continue` inside a scope with a pending `defer` must run it.
   M3 Task 3. After M4 this extends to panics, and that is the test M4 Task 3
   exists for.
2. **A double free or use-after-free from a missed or duplicated `move`.**
   M3 Task 2's linearity checker must reject re-reading a moved binding, and M3
   Task 3's golden IR must show the move as a bitwise copy plus a register
   clobber, so the compiler cannot emit it twice.
3. **A string whose refcount reaches zero while another binding still holds
   it.** Two bindings to one `string` must balance retain/release exactly.
   M3 Task 6. The failure is a use-after-free that passes every functional test
   and shows up only under memory pressure — so the test asserts refcount
   *balance*, not just output correctness.
4. **An Otter program that can segfault instead of producing an Otter
   diagnostic.** Division by zero (M0–M2, fixed M4), a slice index (M6, must
   yield `Option`), a map lookup miss (M6), and a `None` unwrap (M2/M4) are all
   runtime faults or values, never undefined behaviour. Every milestone that
   adds an operation that can fail names which of the two it chose.
5. **A silent ABI or layout break across a milestone boundary.** The L1/L2/L4
   tables are the contract. A task that changes a layout must regenerate every
   golden file from the previous milestone and the diff must be reviewed as a
   whole, not per-file.

## What Each Phase Must Prove

| Phase | Proof it works |
|---|---|
| M0–M2 | `hello world`, Fibonacci, `Option` matching, `Result` propagation, and recursion each compile to native binaries with correct output |
| M3–M4 | a string builder and a manual buffer run clean; deep recursion panics with a caret diagnostic and every pending `defer` runs on the way out; integer overflow panics |
| M5–M6 | a C library is called from Otter; slices, maps, multi-file modules, and the `io`/`string`/`math` stdlib modules work |
| M7–M8 | a parallel task pool produces correct results; a stranger can write and run a program using only the tutorial; `otter fmt` is idempotent |
| M9–M10 | the benchmark suite shows fewer allocations than M3 with identical output; the Otter-written compiler passes the Rust compiler's own test suite |