# Otter — Design Specification

**Date:** 2026-10-16
**Status:** Draft for review
**Author:** Brainstormed with the project owner

---

## 1. Summary

Otter is a general-purpose, statically typed programming language with C-shaped
syntax and semantics borrowing the clearest parts of Go, Python, and Rust. It
compiles to native binaries through an LLVM backend, is written in Rust, and is
intended first for its creator and then for wider use.

Otter has **no null**, **no exceptions**, and **no garbage collector**. Memory is
manual for buffers and arrays, refcounted for strings. Runtime faults `panic`
and unwind, running every pending `defer`. Concurrency uses OS threads and
channels. C is reachable through `unsafe`-marked `extern` blocks.

### Goals

1. Become a general-purpose language its author and other people can write real
   programs in.
2. Be genuinely pleasant to read: self-documenting keywords, interpolation,
   named fields, `const`/`var` rather than silent mutability.
3. Compile to fast native binaries via LLVM.
4. Eventually self-host: Otter compiles its own standard library and then itself.

### Non-goals for the foreseeable future

- Compiling to WebAssembly
- C++ interop
- Operator overloading
- First-class closures / higher-order functions as a v1 feature
- A package manager with version resolution
- Any form of garbage collection
- Generics (deferred past v1 by deliberate decision — see §4.6)

---

## 2. Design decisions of record

| Area | Decision | Rationale |
|---|---|---|
| Level | "Middle level" — C-like explicit control, high-level conveniences | Core request |
| Syntax family | C / Go / Python, C-shaped | Core request |
| Blocks | Braces + semicolons | Familiarity, unambiguous parsing |
| Compilation | LLVM backend, native binaries | Best long-term performance |
| Compiler host language | Rust, then self-hosting | Memory safety suits an AST-heavy program |
| Type system | Static, local inference, no null | Safety without ceremony |
| Memory | Manual + escape analysis | Core request |
| Concurrency | OS threads + channels, no async runtime | Smallest correct runtime |
| Interop | `extern` C FFI, `unsafe`-marked, copying | Bootstrap path for the stdlib |
| Runtime faults | `panic` with unwinding; checked arithmetic | No exceptions, so faults need one defined path |
| Platforms | Windows x86-64 and Linux x86-64 | Author's machine, plus server targets |

---

## 3. Language surface

```c
import std.io;
import crypto.sha256;

// Named fields, so field order never matters to the reader.
struct Point {
    x: f64,
    y: f64,
}

enum Shape {
    Circle(f64),
    Rect { w: f64, h: f64 },
}

enum Color {
    Red,
    Green,
    Blue,
}

fn distance(a: Point, b: Point) -> f64 {
    const dx = a.x - b.x;
    const dy = a.y - b.y;
    return sqrt(dx * dx + dy * dy);
}

fn main() {
    const p = Point { x: 3.0, y: 4.0 };

    defer io.println("done");            // no GC: cleanup is explicit

    if d := distance(p, origin); d > 5.0 {
        io.println("far: ${d}");        // interpolation, not printf
    } else {
        io.println("close");
    }

    for i in 0..10 {                      // range loop
        if i % 2 == 0 { continue; }
        io.println(i);
    }

    switch color {
        case Circle(r):    return 3.14159 * r * r;
        case Rect { w, h }: return w * h;
        case Color.Red:    io.println("red");
        default:           io.println("other");
    }
}
```

### 3.1 Lexical

- Line comments `//`, block comments `/* */` (nesting permitted).
- Identifiers, keywords (`fn`, `struct`, `enum`, `const`, `var`, `defer`, `move`,
  `as`, `extern`, `import`, `switch`, `if`, `else`, `for`, `while`, `in`, `break`,
  `continue`, `return`, `case`, `default`, `true`, `false`), literals.
- String literals are `"..."` with the escapes `\n`, `\r`, `\t`, `\0`, `\\`, `\"`,
  `\'`, `\$`. The set is closed: `\q` is a diagnostic, not a literal `q`.
- `${ expr }` is **interpolation** (§3.3): the text before, between and after the
  interpolations is a string literal, and each `expr` is converted to text and
  concatenated. `\$` is how a literal `$` before `{` is written — without it a
  `$` followed by `{` could not appear in a string at all — and it means the two
  characters, never an interpolation.
- Interpolation is **sugar, lowered by the type checker** into a call to the
  matching `str_from_i64` / `str_from_f64` / `str_from_bool` (a `string` operand
  needs no call) joined by `+`. There is no interpolation node in the typed tree
  and nothing for code generation to lower differently, so `"n = ${n}"` and
  `"n = " + str_from_i64(n)` are the same program. A part whose type has no text
  form — a struct, an enum, a pointer — is a compile error rather than a printed
  address. See the M3–M4 plan, Task 7.
- Semicolons are required at statement end. Braces delimit blocks.

### 3.2 Declarations and mutability

- `const x = expr;` — immutable binding. `var x = expr;` — mutable.
- `let` is **not** a keyword: a third synonym for `const` is one more thing to
  learn.
- Assignment is spelled `x = v;` and only a `var` binding may be the target.
- `x op= v;` for `+=`, `-=`, `*=`, `/=`, `%=` is **sugar**: it is exactly
  `x = x op v`, and there is no compound-assignment node. So the right operand
  is checked against the operator and the target against mutability, and a
  program that writes the long form gets the same result.
- `:=` is the one open spelling question (§12.1). If accepted, it is sugar for
  `const`, never the canonical spelling.
- Parameters, return types, and struct fields are always annotated.
- `fn main()` may return nothing or return `i32` (§6.3).

### 3.3 Control flow

- `if cond { } else { }`; `if let`-style binding via `if d := expr; cond { }`.
- `while cond { }` — the condition-only loop, and nothing more than that: it
  parses to the same node as `for ; cond; { }`, so the two are interchangeable
  and `while` cannot drift from `for`. The condition is at the top, which is
  where a reader looks for it; the update is in the body.
- `for i in 0..10 { }` range loop; C-style three-clause `for` retained for
  irregular iteration.
- A `{` after an expression is a block unless it holds `field:`, which is a
  struct literal. So `while running { }` is a loop with an empty body, not a
  struct literal named `running` — an ambiguity that used to be resolved the
  wrong way for the empty form.
- `break`, `continue`.
- `switch` with `case` arms, `default`, and exhaustiveness checking on enums.
  Struct/enum payloads destructure: `case Rect { w, h }`.
- `defer` — statement or expression queued to run at scope exit, LIFO.

### 3.4 Imports

`import std.io;` reads as a path, not a string literal. Resolved through the
module system in M5; single-file before that.

---

## 4. Type system

### 4.1 Types

Primitives: `i8 i16 i32 i64`, `u8 u16 u32 u64`, `f32 f64`, `bool`, `char`,
`string`, and `()` for "no value".

Nominal types: `struct`, `enum`, and (v1) opaque function references.

### 4.2 Inference

Locals infer from their initializer: `const x = 3;` is `i32`. Annotations are
required at the boundaries — parameters, return types, struct fields — where
inference would force the reader to look elsewhere. Inference is bidirectional:
top-down for annotated signatures, unification within a function body.

### 4.3 No implicit conversions

`const x: i64 = 3;` is allowed (literal widening). `const y: i64 = some_i32;`
is an error; write `some_i32 as i64`. Silent numeric conversions are how
overflow bugs become invisible.

### 4.4 Option and Result

- `Option<T>` — `None` / `Some(x)`. Replaces null.
- `Result<T, E>` — `Ok(v)` / `Err(e)`. Replaces exceptions.
- `?` postfix on a call returning `Result` unwraps `Ok` and early-returns `Err`.
- `??` / `unwrap_or(default)` on an `Option`, and a `default` value on the type,
  so unwrapping is not verbose in the common case.
- Indexing a `[]T` yields `Option<T>` rather than trapping.

### 4.5 Sum types

`enum` with unit, tuple, and struct variants. `switch` over an enum must be
exhaustive; the compiler lists missing cases.

### 4.6 Generics — deliberately deferred

Generics are not in v1. They are the point where a type checker's complexity
curve turns upward, and a young language that ships them wrong becomes
permanently unpleasant to use. Structs with a few specialized variants, plus
`Option`/`Result`, carry the early load. Revisit after M5.

### 4.7 Casts and FFI types

`expr as T` for explicit casts. `extern` blocks use C spellings (`i32`, `f64`,
`*mut u8`, `usize`) so declarations read like the header they wrap.

---

## 5. Memory model

The riskiest part of the design, and the one most likely to change schedule.

### 5.1 Three tiers

| Tier | How you get it | Cost | Who frees it |
|---|---|---|---|
| Stack | `const p = Point { x: 3.0, y: 4.0 }` | Free | Nobody; scope exit |
| Arena | `arena = Arena.new();` | Bump allocation | `arena.free()` |
| Heap | `var buf = alloc(i64, 4096);` | Real `malloc` | You, via `defer dealloc(buf)` |

> **M3 interim form.** `alloc(T, count)` needs a type in argument position and a
> size for `T`, and neither exists until generics and slices land (M6). M3
> therefore ships `alloc(count) -> *mut i64` — the element type is fixed at
> `i64` and the count is in elements — with `dealloc(*mut i64)`. The two-argument
> spelling above is what M6 delivers, and no program changes meaning when it
> arrives, because `alloc(count)` is exactly `alloc(i64, count)`. The master
> plan's ledger L3 records the same revision.

### 5.2 Escape analysis is an optimization, not the safety mechanism

In v1 every heap allocation is written explicitly. Escape analysis (M7) later
proves an allocation does not escape and removes it. The same program is correct
whether or not the analysis has been written, so this risk stays off the
critical path.

### 5.3 Linear ownership of heap pointers — moves without a borrow checker

`struct` values are copied on assignment. A heap pointer is *linear*: giving it
to another variable consumes it.

```c
var buf = alloc(4096);  // 4096 i64s (see the M3 interim form in §5.1)
var other = move buf;   // OK: ownership transfers, buf is dead
var leak  = buf;        // ERROR: buf has no live owner
dealloc(other);         // the one free
```

Tracking which pointer variables are live and who owns each allocation is far
smaller than a full borrow checker, and it removes double-free and
use-after-free — the failures that actually bite. Fine-grained aliasing is
given up and can return later if the stdlib needs it.

### 5.4 Strings

Strings are immutable, length-prefixed, and **refcounted**. Refcounting is
cheap for immutable data because reference cycles are impossible, so no cycle
collector is required. Cost is one 8-byte header and refcount logic confined to
string-producing expressions.

An earlier draft placed strings in arenas. That was rejected: strings are the
most common heap value in real programs, and arena discipline would make the
language about lifetimes instead of about problems.

**The rule:** arrays and buffers are manual, strings are refcounted.

> **M3 interim representation.** The design above is what the language ships;
> M3 ships the *shape* of the operations without the refcount. A `string` value
> is an `i8*` to NUL-terminated bytes, `+` concatenates (through
> `otter_str_concat`), `==`/`!=` compare contents, and `len` counts bytes. The
> runtime's string operations (`otter_str_len`, `otter_str_concat`,
> `otter_str_eq`, and the `otter_str_from_*` conversions) build their results in
> one internal arena **per thread** that is released at process exit. That is a
> correctness requirement, not a tuning choice: an arena is an unsynchronized bump
> pointer, so a single shared one handed two threads the same block and one
> thread's bytes appeared in another's string.
>
> That is a leak, chosen deliberately over a use-after-free: a built string has
> no owner to release — the value is a bare `i8*`, with no refcount and no
> `release` operation reachable from the language — and a refcount with no
> `release` point would be the wrong half of this design. Nothing is corrupted; a
> program is correct, it just holds its built strings until it exits. The
> length-prefixed, refcounted layout in this section, and the release points that
> go with it, are the next step (M3's remaining string task; see the milestone
> plans and ledger L4 in the master plan).

### 5.5 `defer`, `move`, and arenas together

These three interact, so the rules are stated explicitly.

- **`defer` captures at registration time.** A `defer` expression is evaluated
  when the `defer` statement executes, not at scope exit. `defer dealloc(buf)`
  therefore captures the pointer *now*.
- **You cannot `move` a pointer captured by a live `defer`.** The compiler
  rejects it: the `defer` would free memory the new owner is using. The fix is to
  register the `defer` after any `move`, or to transfer the `defer` too.
- **Arena allocations are not linear.** Moving an arena-owned value transfers no
  ownership obligation; the arena keeps it alive. `move` on an arena pointer is a
  no-op for lifetime purposes and is allowed freely.
- **`defer` runs during panic unwinding** (§6), so cleanup is not skipped on a
  fault. A `defer` that panics during unwinding aborts the process.

At M0–M2 there are no panics (§10, M4), so `defer` lowers statically: each
scope's deferred expressions are emitted in reverse order before every exit path
out of that scope — `return`, `break`, `continue`, and normal fallthrough.

### 5.6 Strings and threads

Strings are **UTF-8** byte sequences with an explicit length. A `char` is a
Unicode scalar value, not a byte.

Refcounts are **atomic** when a string may be shared across threads, which is the
default for values reachable from more than one thread. The compiler tracks this
statically per binding: a `string` that never escapes to another thread gets a
non-atomic refcount and pays nothing for it. This is decided at M7 and is a
performance optimization, not a correctness requirement — before it lands,
refcounts are atomic everywhere.

A `string` shared between threads therefore requires no annotation; there is no
`Arc`/`Rc` split in Otter, because refcounted strings are already the common case
and adding two string types would be a worse trade than a few extra atomics.

---

## 6. Runtime faults and unwinding

There are no exceptions, so runtime faults need one defined mechanism.

**Every runtime fault panics.** Division by zero, integer overflow, `unwrap()`
on a `None`, a failed `assert`, and an internal compiler-invariant violation all
take the same path:

1. Print `file:line:col`, the panic message, and a caret under the offending
   expression in a source span.
2. Unwind: run every pending `defer` on the stack, innermost first.
3. Exit with a non-zero status.

A fault never becomes a segfault, an abort with no message, or silent
undefined behaviour. This costs real codegen work — landing pads and unwind
tables — but without it, "no exceptions" and "no null" would leave programs
dying in ways nobody can debug.

**Unwinding is bounded, not a coroutine.** Otter supports panic-unwinding
across `fn` frames, not `catch`/`throw`. Once a panic starts, the only choice
the language offers is whether `defer` cleanup runs first.

### 6.1 Checked arithmetic

Integer arithmetic is **checked by default**: `i32` overflow panics rather than
wrapping or invoking UB. Silent overflow is worse than a loud fault, and a
language without exceptions has nowhere else to put the error.

Wrapping and non-panicking forms are explicit opt-ins:
`wrapping_add`, `checked_add` (returns `Option`), `saturating_add`.

### 6.2 `assert`

`assert(cond);` and `assert_eq(a, b);` panic with source spans on failure and
compile away under `--release` when the assertion is marked `debug_assert`.

### 6.3 Exit codes

`fn main()` may return nothing or return `i32`, which becomes the process exit
status. `os.exit(code)` is available anywhere.

---

## 7. Concurrency

OS threads (`pthreads` / Win32) with message-passing channels and mutexes. No
async runtime, no scheduler, no task stack management. v1 provides no automatic
race-freedom guarantees — threads, channels, and mutexes are the primitives, and
correctness is the programmer's.

---

## 8. Interop and standard library

### 8.1 The FFI trust boundary

Every `extern` declaration requires the `unsafe` marker:

```c
extern "C" unsafe fn puts(s: *const u8) -> i32;
```

Rules:

- **Strings are UTF-8**, and `*const u8` is a NUL-terminated C string.
- **Crossing the boundary copies.** `string` and `[]T` arguments are copied in
  and out of C, so no aliased Otter memory ever escapes. Passing raw pointers is
  possible but requires explicit `unsafe` at the call site.
- The compiler's linearity invariants hold only over memory it can see. C code
  can free an Otter buffer; that is precisely what `unsafe` marks as the
  programmer's responsibility.

### 8.2 Standard library

v1 stdlib scope: `io` (stdout/stderr/files), `string`, `array`/`slice`, `math`,
`os` (args, exit, env), `sync` (mutex, channel), and `alloc`.

Until that library exists, the functions a program can call are the compiler's
builtins, and they are exactly the ones a program cannot be written without:
`print_str`, `print_i64`, `print_f64` on the way out, `str_from_i64`,
`str_from_f64`, `str_from_bool` to put a value *inside* a string, `read_line` and
`parse_i64`/`parse_f64` on the way in, `alloc`/`dealloc` for memory, and
`len` for a string's byte length. They are ordinary names resolved like any
other function, so `io.readLine` arriving in M6 replaces `read_line` rather than
living beside it. Three of them have a contract worth stating: `read_line`
returns the empty string at end of input (which the language cannot yet
distinguish from an empty line — the checked form waits for `panic` or `Option`
in M4), and the parsers return zero when their argument is not a number, with
trailing text making the whole string a non-number rather than being ignored.
`str_from_bool` takes a `bool` in the language and an `i64` at the ABI, because
C has no one-bit type; the generated code widens.

Dynamic loading differs by platform and is isolated in `otter-runtime`:
`LoadLibrary`/`GetProcAddress` on Windows, `dlopen`/`dlsym` on Linux. The language
never sees this difference.

---

## 9. Compiler architecture

Rust workspace, one crate per pipeline stage.

| Crate | Job | Depends on |
|---|---|---|
| `otter-common` | `Span`, `SourceMap`, `Diagnostic`, rendering | — |
| `otter-lexer` | bytes → tokens, with byte spans | common |
| `otter-parser` | tokens → AST | lexer, common |
| `otter-sema` | scopes, inference, `Option`/`Result` checks, move checking, exhaustiveness | parser, common |
| `otter-codegen` | typed AST → LLVM IR **text** | sema, common |
| `otter-driver` | CLI, orchestration, toolchain detection, diagnostics rendering | all |
| `otter-stdlib` | the standard library, in Otter | built compiler |
| `otter-runtime` | C shim: malloc, threads, dynamic loading, print helpers | — |

```
  otter build main.ot
        │
        ▼ ┌─────────┐   ┌──────────┐   ┌─────────┐   ┌──────────┐
   │  lex │──▶│  parse   │──▶│  sema   │──▶│ codegen  │──▶ .ll ──▶ llc ──▶ exe
   └─────────┘   └──────────┘   └─────────┘   └──────────┘
                                    │
                                    └─▶ diagnostics (spans → colored carets)
```

### 9.1 Backend strategy

Hand-written textual LLVM IR emission; the compiler shells out to `llc`/`clang`.
No LLVM build dependency at compile time, so `cargo build` works on any machine.
The IR is plain text and therefore readable and diffable, which is how a young
compiler gets debugged.

`codegen` is the only crate that knows about LLVM. It consumes an already-typed
tree and is forbidden from making type decisions. This is what makes a later move
to Otter's own SSA IR a single-crate swap.

Rejected alternative: `inkwell`/Rust LLVM bindings (version lockstep, multi-minute
builds, LLVM install pain, binding churn).

### 9.2 Diagnostics

Every token and AST node carries a byte span. Errors render as `file:line:col`
with a caret under the offending token. This ships in v1: a language people
cannot debug will not become one.

---

### 9.3 Unwinding support

Codegen emits landing pads and unwind tables for every call site that can panic,
and `otter-runtime` provides the personality routine and phase-1/phase-2 landing
pad helpers. The Rust compiler's `libunwind` is the reference implementation.

---

## 10. Milestones and timeline

Estimates assume one developer at roughly 25–30 effective hours per week.
At part-time, calendar time scales accordingly.

| # | Deliverable | Proof it works | Estimate |
|---|---|---|---|
| M0 | Lexer, parser, CLI, `fn main` → `.ll` | `hello world` native binary | 2–3 weeks |
| M1 | `const`/`var`, arithmetic, `if`/`for`, `fn`, primitive types | Fibonacci, loops | 3–4 weeks |
| M2 | `struct`, `enum`, pattern matching, `Option`, `Result`, `?` | Value tree + pretty-printer | 3–5 weeks |
| M3 | `alloc`/`dealloc`/`defer`, moves, arena, refcounted strings | String builder, dynamic array | 7–11 weeks |
| M4 | Panic/unwinding, checked arithmetic, `assert`, exit codes | Deep recursion unwinds cleanly | 4–6 weeks |
| M5 | `extern` C FFI + `otter-runtime`, both platforms | Calls libc; SHA-256 via a C library | 3–4 weeks |
| M6 | Slices, maps, modules, multi-file builds | Small file/HTTP tool | 4–6 weeks |
| M7 | Threads, channels, mutexes | Parallel task pool | 4–6 weeks |
| M8 | `otter fmt`, tutorial book, LSP + syntax highlighting | A stranger writes a program from the tutorial | 4–6 weeks |
| M9 | Escape analysis (removes explicit `alloc`) | Benchmark vs. M3 allocations | 6–12 weeks |
| M10 | Self-hosting bootstrap | Otter compiles its own stdlib | 6–12 weeks |

### 10.1 Cumulative, honest

- **M0–M2** (usable little language): **8–12 weeks**
- **M0–M6** (general purpose): **26–39 weeks**, i.e. **6–10 months**
- **M0–M10** (complete, self-hosting, teachable): **46–75 weeks**, i.e. **11–18 months**

### 10.2 Language surface available in M0–M2

Deliberately narrow, so the front-end and codegen stay small and fully tested
before memory and containers arrive.

Available: `i64`, `f64`, `bool`, `string`, `()`, `struct`, `enum`, `Option<T>`,
`Result<T,E>`. Declarations `fn`, `const`, `var`, `import`. Statements:
expression, `const`/`var` binding, `return`, `if`/`else`, `for` (range and
three-clause), `switch`, `break`, `continue`, `defer`. Expressions: literals,
identifiers, binary and unary operators, calls, struct literals, field access,
`as` casts, `?`, `switch` as an expression.

Not yet available — each arrives in a later milestone:

| Feature | Milestone |
|---|---|
| String and `char` operations, interpolation | M3 |
| `alloc`/`dealloc`, `move`, arenas, refcounted string drop | M3 |
| Slices `[]T`, arrays, maps `map[K]V`, indexing | M6 |
| `extern` C FFI | M5 |
| `switch` as an expression, exhaustiveness with bindings | M2 |
| Checks for missing `main`, zero division, overflow | M4 |

At M0–M2 `i64` division by zero and integer overflow are **not** trapped: `sdiv`
by zero is undefined behaviour in LLVM. Both become defined, panicking
operations in M4. The M0–M2 codegen tests pin the interim wrapping behaviour so
the transition to checked arithmetic is a deliberate, tested change.

### 10.4 Where this is most likely to be wrong

- **M3 is the first slip.** Manual memory plus moves plus refcounting is three
  interacting systems. There is a material chance M3 takes 50% longer than
  estimated, or forces a simplification of the string strategy.
- **M4 (unwinding) is new relative to the original estimate** and adds roughly a
  month. It is non-negotiable if "no exceptions" is to be usable.
- **M9 is the largest single item** and is genuinely research-grade. It is also
  optional: without it the language is still correct, just allocating more.
- **M10 compounds everything.** Self-hosting surfaces every missing feature in
  the compiler, all at once. Do not start it before M6 is genuinely solid.
- **M7 is smaller than it looks** only if "no data-race guarantees" is accepted
  in v1. Adding deterministic data-race detection later is a significant project.
- **M8 is not optional for the stated goal.** A language with no tutorial, no
  formatter, and no editor support does not become one that "other people" use,
  no matter how good the compiler is.

---

## 11. Testing strategy

- **Golden files**: source → expected AST and expected IR, so front-end changes
  are diffable and regressions are visible.
- **Unit tests** for inference, move checking, panic propagation, and escape
  analysis in isolation, without LLVM in the loop.
- **End-to-end suite** of real programs asserting correct output, run on
  **both Windows and Linux** in CI from M5 onward.
- **Type-error and runtime-failure tests** asserting not just that a program
  fails, but that the diagnostic names the right span.
- **Robustness: the compiler never panics on malformed input.** This is an
  explicit M0 acceptance criterion. The lexer, parser, and sema are fuzzed from
  M0 with `cargo-fuzz`; any input produces a diagnostic and a non-zero exit, never
  a Rust backtrace.

### 11.1 Scope of the implementation plans

This spec covers the whole language. It is planned up front, in full, as five
documents that execute in order:

| Plan | Covers |
|---|---|
| `plans/2026-10-16-otter-m0-m2-compiler.md` | M0–M2: lexer, parser, sema, codegen, driver |
| `plans/2026-10-16-otter-m3-m4-memory-and-faults.md` | M3–M4: memory model, panic and unwinding |
| `plans/2026-10-16-otter-m5-m6-ffi-and-containers.md` | M5–M6: C FFI, slices, maps, modules, stdlib |
| `plans/2026-10-16-otter-m7-m8-concurrency-and-tooling.md` | M7–M8: threads, formatter, LSP, tutorial |
| `plans/2026-10-16-otter-m9-m10-optimization-and-bootstrap.md` | M9–M10: escape analysis, self-hosting |

`plans/2026-10-16-otter-master-plan.md` holds the cross-plan interface ledger and
the ordering and dependencies between milestones. Execution still proceeds one
milestone at a time; a later plan's task never starts before the earlier plan's
milestone is complete and reviewed.

---

## 13. Development approach

Otter will be written with heavy AI assistance: an AI agent writes most of the
code, guided by specs, milestone plans, and tests written by the author.

### 13.1 What AI assistance changes

- **Interface design matters more, not less.** Agents work within a context
  window. The crate boundaries in §9 exist partly so each is small enough to be
  reasoned about correctly in isolation.
- **Tests are the specification.** Golden files, unit tests, and the
  end-to-end suite are the primary channel through which the author verifies
  agent output, since the author will not read every line.
- **Slice size matters.** Milestones must be split into slices small enough to
  complete and verify in one sitting, each ending in working, tested code.
- **Diagnostics as a first-class feature (§9.2) become more important, not
  less**, because a failing agent-written test needs a good error to debug from.

### 13.2 What it does not change

The timeline in §10 assumes one author making design decisions, reviewing
generated code, and debugging compiler bugs. AI assistance helps unevenly:

- **Faster:** boilerplate-heavy code (lexer rules, parser combinators, AST
  definitions, repetitive codegen, test fixtures, stdlib wrappers).
- **Not faster:** deciding what the type system should do, diagnosing a wrong
  diagnostic, chasing a miscompile, and reviewing for correctness the compiler
  itself cannot catch. Escape analysis (M9) and self-hosting (M10) are the
  milestones where AI assistance helps least, because the difficulty is in the
  design, not the typing.

The estimates in §10 are therefore left unchanged. They are already conservative
for the code-authoring portion and realistic for the design-and-debug portion.
A useful early signal is whether M0–M2 lands ahead of or behind the stated
8–12 weeks; that single data point should calibrate the rest of the schedule.

---

## 12. Open questions for later milestones

Not blocking v1; decide when the milestone is reached.

1. Does `:=` survive as `const` sugar, or is `const` the only spelling?
2. Module system shape: flat namespace with prefixes, or per-file scoping?
3. Closures and higher-order functions — needed, and in what form?
4. Package manager: is a registry ever needed, or is git+manifest enough?
5. Generics after M6: type parameters, inference, and monomorphization vs. dictionaries.
6. Whether to add data-race detection to M7 or defer it indefinitely.
7. Whether `struct` values inside slices may be self-referential (a linked list in
   an array). If so, how are elements pinned?