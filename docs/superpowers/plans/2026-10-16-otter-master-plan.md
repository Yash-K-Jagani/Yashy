# Otter — Master Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Coordinate the complete construction of the Otter language, from an empty repository to a self-hosting compiler, across six phase plans.

**Architecture:** This document holds no implementation tasks. It is the cross-plan contract: the interface ledger that six phase plans must agree on, the dependency order between milestones, and the global constraints that bind all of them. Read it before any phase plan.

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
| `2026-10-16-otter-m11-adoption.md` | M11 | 9 | 4–6 weeks |

Cumulative: **50–81 weeks** (12–19 months) at 25–30 effective hours/week. See
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
          │
          ▼
        M11 ─▶ M12+             adoption: formatter, test runner, LSP, installer
```

Two dependencies are load-bearing and easy to get wrong:

- **M4 depends on M3.** Unwinding runs pending `defer` handlers. A `defer` that
  does nothing cannot demonstrate unwind semantics. Do not attempt M4 first.
- **M10 depends on M9 being optional.** Self-hosting surfaces every missing
  feature at once, but it does **not** require escape analysis to be fast. M9
  may be incomplete when M10 starts; M10 may not start until M9's *correctness*
  harness is green, so a miscompile is distinguishable from a missed
  optimization.

M9, M10, and M11 are the only milestones where deferring is acceptable. Everything
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

> **L1 revision, M3 Task 5: the string half is now `otter_str*`, as the table
> above shows.** The interim this note used to describe — a `string` value as a
> bare `i8*` to NUL-terminated bytes, built in a never-freed per-thread arena —
> is gone (`547e8e1`). What the runtime defines and the generated IR calls is the
> table above with one revision: every count is `i64`/`long long` (one width, and
> the width C already uses), and a `cap` sits beside `len` for the slicing work
> M6 will want.
>
> ```c
> otter_str* otter_str_retain(otter_str*);
> void       otter_str_release(otter_str*);
> otter_str* otter_str_concat(const otter_str*, const otter_str*);
> otter_str* otter_str_from_i64(i64);
> otter_str* otter_str_from_f64(f64);
> otter_str* otter_str_from_bool(i64);   // i64, not i1: C has no one-bit type
> i64        otter_str_len(const otter_str*);
> i64        otter_str_byte_at(const otter_str*, i64);
> int        otter_str_eq(const otter_str*, const otter_str*);
> ```
>
> In the IR each of these is still spelled with `i8*` — an opaque pointer is an
> opaque pointer — but the value it points at is the `%otter_str` in L2. A
> literal is a *static* string, refcount `-1`, which makes both retain and
> release a no-op on it; everything the runtime builds (concatenation, a
> conversion, a line of input) comes back owned, refcount `1`. The lengths come
> from the struct, so measuring no longer scans, and an embedded NUL no longer
> truncates.
>
> Cost. Generated code keeps the count balanced for what it can prove. A string
> the runtime **built** is released the moment an operation has consumed it — a
> concatenation, a comparison, `len`, a parser, `print_str` — and a string a
> **binding** holds is retained on bind and released at the end of the scope that
> owns it, after that scope's `defer`s. What is deliberately *not* balanced is a
> string a user call returns (it may be a parameter the caller still owns, so it
> is treated as borrowed), one stored in a struct field, and one dropped across a
> `break` out of a nested block. Each of those leaks rather than risking a
> use-after-free; see the M3–M4 plan's Task 5 "As built" notes.

**Input (added with the M3 string operations, because a program that cannot read
is not yet a program):**

```c
i8*  otter_read_line(void);          // one line, no terminator; "" at end of input
i64  otter_parse_i64(i8*);           // 0 when the whole string is not an integer
f64  otter_parse_f64(i8*);
```

These are compiler builtins (`read_line`, `parse_i64`, `parse_f64`) resolved like
any other name, so M6's `io.readLine` replaces them rather than joining them. The
line comes back as an owned string, like every other produced string, and the two
parsers read their argument's bytes through `otter_str_data`.
`otter_read_line` returns the empty string at end of input, which the language
cannot currently distinguish from an empty line; the parsers reject trailing text
(`parse_i64("12x")` is 0, not 12) and fold overflow into the same zero, so each
has exactly one contract and no in-between. Checked forms wait for M4's `panic`.

**M4 (faults):**

```c
void otter_panic(const otter_str* message, const otter_str* file, long long line, long long column);
```

As built (M3–M4 Task 9), the `msg_len` in the sketch above is gone: a `string`
value is already length-carrying `otter_str*`, and `otter_panic` reads `message`
and `file` through `otter_str_data`, so a length here would be the one ABI that
reads a string two ways.
The three position arguments are resolved by the **checker**, not the backend — it
is the last stage holding the `SourceMap`, and the alternative was growing
`emit_program`'s signature (and every test that calls it) for one call — and they
arrive as ordinary literal arguments.

`otter_panic` does **not** unwind: it writes one `stderr` line and exits with
status 1. What runs before it is the **generated fault block**, which emits the
panicking function's open scopes' `defer` bodies (innermost first, reverse
registration order within a scope) and then calls `otter_panic`. `defer`s
registered in *calling* functions do not run, and no stack trace is printed:
both need the two-phase unwinder and cleanup landing pads. That half is
**blocked by the supported toolchain**, not deferred by choice — a landing pad
needs `__gxx_personality_v0`, which on the `x86_64-pc-windows-gnu` host is
undefined at link and would require shipping libunwind/libstdc++ (see the
M3–M4 plan's Task 10). The interim is stated rather than hidden, and it is the
seam the unwinding work attaches to when a host unwinder is available.

`otter_assert_failed` is not written, and as M4 Task 12 was built it is not
needed: an assertion is a `panic` behind a test, so `assert`,
`assert_eq` and `assert_ne` report through the `otter_panic` above and
nothing new is declared.

Checked arithmetic is **not** a runtime call. M4 emits the compare and the
conditional branch to an inline panic block in generated code (see L4).

**A leak report, for tests (not an ABI symbol):**

`runtime.c` keeps a count of live `alloc`/string allocations and, when
`OTTER_LEAK_REPORT=1` is set, prints `otter: leak report: N live allocations` to
`stderr` from an `atexit` handler. The arming function is `static`
(`otter_leak_arm`, called by the allocator and the string constructor), so it
adds nothing to the generated-code ABI; a program cannot call it. Arena blocks
are deliberately uncounted: an arena is one allocation with one
`arena_free_all`, and counting its bump blocks would report a leak for correct
code. The M3–M4 end-to-end memory tests read this report instead of requiring
`valgrind` or a debug CRT on every contributor's machine.

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

**`Str` layout changed once, in M3 Task 5, and does not change again:**

```llvm
%otter_str = type { i64, i64, i64, i8* }   ; refcount, length, capacity, data
```

M0–M2's `%otter_str = type { i32, i32, i8* }` was a placeholder that no
generated code referenced. M3 Task 5 replaced it with the layout above: `-1` in
the refcount means a static literal (never freed), `1` means owned, and
`otter_str_retain`/`otter_str_release` move the count. Every count is `i64`
rather than the `i32` sketched here originally, so one width describes the
struct on both sides of a call, and `cap` is beside `len` for M6's slicing.
**No milestone after M3 changes this layout** — which is why M3 is a milestone
boundary and not an incremental edit.

**A `Struct` is an aggregate value, laid out in declaration order:**

```llvm
%struct.Point = type { i64, i64 }
```

A literal is built with `insertvalue` into `undef` and a field is read with
`extractvalue` at the field's declaration index, so neither needs an address. The
redeclaration rule — `define_named_type` keeps the first shape it is handed —
is what makes the index mean one thing: a literal's fields reach the HIR in
declaration order, so two literals of one struct written in different orders
produce the same named type (M3–M4 Task 5e). Writing a field is the same
read-modify-write shape — load the aggregate, replace the field, store it back —
so `p.x = 1` needs no address either. What *does* need one is a struct behind a
pointer (`*mut P`), which is M6's.

### L3 — Builtin function set

The set of non-`extern` functions the compiler knows without a declaration.
Each milestone adds to it. Sema predeclares all of these in the root scope.

| Milestone | Builtins |
|---|---|
| M0–M2 | `print_str(string)`, `print_i64(i64)`, `print_f64(f64)` |
| M3 | `alloc(i64) -> *mut i64`, `dealloc(*mut i64)`, `arena_new() -> *mut i64`, `arena_alloc(*mut i64, i64) -> *mut i64`, `arena_free_all(*mut i64)`, `len(string)`, `byte_at(string, i64) -> i64`, string constructors and accessors from L1 |
| M3, as built ahead of the row above | `read_line() -> string`, `parse_i64(string) -> i64`, `parse_f64(string) -> f64`, `str_from_i64(i64) -> string`, `str_from_f64(f64) -> string`, `str_from_bool(bool) -> string` (L1's input and conversion notes) |
| M4, as built ahead of the row below | `panic(string) -> !` (M3–M4 Task 9). `!` and not `()`: a panic produces no value, so it can stand in for a value of any type — a `return`, a `switch` arm — without letting a program use one. The call carries the file, line and column the checker resolved, and ends the process with status 1 |
| M4, as built ahead of the row below | `assert(bool)`, `assert_eq`/`assert_ne` over the four types with an equality (M3-M4 Task 12, first half). One `assert_eq` name rather than one per type: the comparison forms are built in the checker, so the table's declared `i64` pair is never consulted and exists to reserve the name. `assert_ne` is the same lowering with the predicate inverted. `debug_assert(bool)` and the `--release` build mode that drops it: the checker deletes the call, so the condition is still checked but nothing is lowered. `exit(i64)` (the `!`-typed `os.exit`) ends the process with a chosen status, flushing both streams first. `main` returning `i64` is the exit status, truncated to the C `int` |
| M4 | `wrapping_add/sub/mul(i64,i64)`, `checked_add/sub/mul(i64,i64) -> Option[i64]`, `saturating_add/sub/mul(i64,i64)` |
| M6 | `len(slice) -> i64`, `slice_push`, `map_new/get/insert/remove/len`, string `len`, indexing |
| M7 | `Thread.spawn(fn, args) -> i32`, `thread_join(i32)`, `Mutex.new()`, `mutex.lock/unlock`, `Channel.new(i64)`, `chan.send/recv/close` |

> **L3 revision, M3: `alloc`/`dealloc` are over `*mut i64`, not `*mut u8`.**
> Otter has no `u8`, and M3 has neither indexing nor pointer casts, so a byte
> pointer would be a value no program could do anything with. `*mut i64` is
> usable the moment M6 adds indexing, and the M3–M4 plan's own `buffer.ot`
> already assumed `*mut i64`. Cost: a program that wants raw bytes waits for
> M6's `Slice`; nothing else moves, because `Ptr(_)` is `i8*` in the IR whatever
> the pointee type is.

> **L3 revision, M3 Task 4/6: the arena and `byte_at` spellings.** `Arena.new()`,
> `arena.alloc(n)` and `arena.free()` became `arena_new()`, `arena_alloc(arena,
> n)` and `arena_free_all(arena)`. Otter has no method-call syntax at this
> milestone, so a method table would have named operations no program could
> spell; the free functions are the same three calls with the handle explicit.
> Cost: a program repeats the handle, and M6 may add the method forms as sugar
> over these names without changing the runtime.
>
> `byte_at(string, i64) -> i64` is the language half of L1's
> `otter_str_byte_at`: 0 out of range, so a byte lookup has one answer rather
> than a fault. It is a free function for the same reason. `to_i64()` from the
> M3–M4 plan's method sketch is `parse_i64(string) -> i64` here, which folds
> failure to 0; a checked form that returns `Option` is a later milestone's
> operation.

### L4 — Codegen contracts that later milestones replace

These are the interim behaviours pinned by golden files. Each is deliberately
wrong-in-a-documented-way until the named milestone fixes it, so the change is a
diff a reviewer can read rather than a silent behaviour shift.

| Pinned in | Interim behaviour | Replaced by |
|---|---|---|
| M0–M2 Task 11 | `Add Sub Mul` emit `add/sub/mul` with `nsw`/`nuw` omitted; `Div Rem` emit bare `sdiv`/`srem` with no zero check | **Done in M4 Task 11 (first half).** `add`/`sub`/`mul` go through `llvm.s{add,sub,mul}.with.overflow.i64` — value and flag in one instruction — and `div`/`rem` test for a zero divisor *and* for `i64::MIN / -1` (where `srem` is undefined too) before emitting the operation in the fall-through block. A failing check branches to a cold block that calls `otter_panic_overflow`/`otter_panic_division` with the operator's symbol, both operands and the position the checker resolved into `FaultSite`. Negation is checked (`-i64::MIN` wraps to itself). The second half is in too: `wrapping_*` keeps the bare op, `checked_*` returns a real `Option[i64]` (a `select`, not a branch), and `saturating_*` clamps — `sadd.sat`/`ssub.sat` from LLVM, `smul.sat` by hand because LLVM does not implement it |
| M0–M2 Task 12 | `defer` lowers statically — reverse-order emission before each scope exit, and it captures the linear bindings in scope at registration (M3–M4 Task 5c) | **M4, partially.** The panic/exit/fault block runs the **panicking function's** open scopes' defers (innermost first) before `otter_panic`, and a `return`/`break`/`continue` runs only the current scope's, as before. Cross-frame cleanup — a `defer` in a caller running when a callee panics — is **not** delivered: cleanup landing pads need `__gxx_personality_v0`, undefined at link on the `x86_64-pc-windows-gnu` host, so shipping them would add libunwind/libstdc++ as a mandatory dependency. See the M3–M4 plan's Task 10; there is no stack trace for the same reason |
| M0-M2 Task 3, updated in M3 Task 5 | `Str` is an `otter_str*` (spelled `i8*` in the IR): a literal is a static string with refcount -1, and `+`, `==`/`!=` and `len` are runtime calls over it. Interpolation (M3-M4 Task 7) is lowered to exactly these calls, so it added no contract of its own. A temporary the runtime built is released when an operation consumes it, and a binding retains on bind and releases at the end of its scope (after that scope's `defer`s) | M3 Task 5's remaining interim: a string a user call returns, a struct-field string, and a string dropped across a `break` out of a nested block are leaked, not released. These are leaks, not corruptions; escape analysis (M9) may narrow them, and M6's slicing keeps the same owner rules |
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
   M3 Task 3. After M4 a panic runs the **panicking function's** open scopes'
   defers, innermost first (M4 Task 10, first half). A `defer` in a *calling*
   function still does not run on a panic — the unwinding half is blocked by the
   host toolchain and is the standing exception (M3–M4 Task 10, second half).
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
| M3–M4 | a string builder and a manual buffer run clean with the leak report at zero; deep recursion exits through a positioned fault diagnostic and the panicking function's pending `defer`s run first; integer overflow panics instead of wrapping |
| M5–M6 | a C library is called from Otter; slices, maps, multi-file modules, and the `io`/`string`/`math` stdlib modules work |
| M7–M8 | a parallel task pool produces correct results; a stranger can write and run a program using only the tutorial; `otter fmt` is idempotent |
| M9–M10 | the benchmark suite shows fewer allocations than M3 with identical output; the Otter-written compiler passes the Rust compiler's own test suite |