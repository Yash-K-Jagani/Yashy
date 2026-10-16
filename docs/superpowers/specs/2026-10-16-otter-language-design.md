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
manual for buffers and arrays, refcounted for strings. Concurrency uses OS
threads and channels. C is reachable through `extern` blocks.

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
| Interop | `extern` C FFI | Bootstrap path for the stdlib |
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
  `as`, `extern`, `import`, `switch`), literals.
- Semicolons are required at statement end. Braces delimit blocks.

### 3.2 Declarations and mutability

- `const x = expr;` — immutable binding. `var x = expr;` — mutable.
- `let` is **not** a keyword: a third synonym for `const` is one more thing to
  learn.
- `:=` is the one open spelling question (§11.1). If accepted, it is sugar for
  `const`, never the canonical spelling.
- Parameters, return types, and struct fields are always annotated.
- `fn main()` has no return type and returns nothing.

### 3.3 Control flow

- `if cond { } else { }`; `if let`-style binding via `if d := expr; cond { }`.
- `for i in 0..10 { }` range loop; C-style three-clause `for` retained for
  irregular iteration.
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
| Heap | `var buf = alloc(u8, 4096);` | Real `malloc` | You, via `defer dealloc(buf)` |

### 5.2 Escape analysis is an optimization, not the safety mechanism

In v1 every heap allocation is written explicitly. Escape analysis (M7) later
proves an allocation does not escape and removes it. The same program is correct
whether or not the analysis has been written, so this risk stays off the
critical path.

### 5.3 Linear ownership of heap pointers — moves without a borrow checker

`struct` values are copied on assignment. A heap pointer is *linear*: giving it
to another variable consumes it.

```c
var buf = alloc(u8, 4096);
defer dealloc(buf);

var other = move buf;   // OK: ownership transfers, buf is dead
var leak  = buf;        // ERROR: buf has no live owner
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

---

## 6. Concurrency

OS threads (`pthreads` / Win32) with message-passing channels and mutexes. No
async runtime, no scheduler, no task stack management. v1 provides no automatic
race-freedom guarantees — threads, channels, and mutexes are the primitives, and
correctness is the programmer's.

---

## 7. Interop and standard library

`extern "C"` blocks link against any C library or compiled object, letting the
stdlib bootstrap on libc and giving Otter immediate real-world reach. The cost is
that C's ABI and pointer semantics are visible at the boundary, which is
acceptable at v1.

v1 stdlib scope: `io` (stdout/stderr/files), `string`, `array`/`slice`, `math`,
`os` (args, exit, env), `sync` (mutex, channel), and `alloc`.

Dynamic loading differs by platform and is isolated in `otter-runtime`:
`LoadLibrary`/`GetProcAddress` on Windows, `dlopen`/`dlsym` on Linux. The language
never sees this difference.

---

## 8. Compiler architecture

Rust workspace, one crate per pipeline stage.

| Crate | Job | Depends on |
|---|---|---|
| `otter-lexer` | bytes → tokens, with byte spans | — |
| `otter-parser` | tokens → AST | lexer |
| `otter-sema` | scopes, inference, `Option`/`Result` checks, move checking, exhaustiveness | parser |
| `otter-codegen` | typed AST → LLVM IR **text** | sema |
| `otter-driver` | CLI, orchestration, diagnostics rendering | all |
| `otter-stdlib` | the standard library, in Otter | built compiler |
| `otter-runtime` | C shim: malloc, threads, dynamic loading | — |

```
  otter build main.ot
        │
        ▼ ┌─────────┐   ┌──────────┐   ┌─────────┐   ┌──────────┐
   │  lex │──▶│  parse   │──▶│  sema   │──▶│ codegen  │──▶ .ll ──▶ llc ──▶ exe
   └─────────┘   └──────────┘   └─────────┘   └──────────┘
                                    │
                                    └─▶ diagnostics (spans → colored carets)
```

### 8.1 Backend strategy

Hand-written textual LLVM IR emission; the compiler shells out to `llc`/`clang`.
No LLVM build dependency at compile time, so `cargo build` works on any machine.
The IR is plain text and therefore readable and diffable, which is how a young
compiler gets debugged.

`codegen` is the only crate that knows about LLVM. It consumes an already-typed
tree and is forbidden from making type decisions. This is what makes a later move
to Otter's own SSA IR a single-crate swap.

Rejected alternative: `inkwell`/Rust LLVM bindings (version lockstep, multi-minute
builds, LLVM install pain, binding churn).

### 8.2 Diagnostics

Every token and AST node carries a byte span. Errors render as `file:line:col`
with a caret under the offending token. This ships in v1: a language people
cannot debug will not become one.

---

## 9. Milestones and timeline

Estimates assume one developer at roughly 25–30 effective hours per week.
At part-time, calendar time scales accordingly.

| # | Deliverable | Proof it works | Estimate |
|---|---|---|---|
| M0 | Lexer, parser, CLI, `fn main` → `.ll` | `hello world` native binary | 2–3 weeks |
| M1 | `const`/`var`, arithmetic, `if`/`for`, `fn`, primitive types | Fibonacci, loops | 3–4 weeks |
| M2 | `struct`, `enum`, pattern matching, `Option`, `Result`, `?` | Value tree + pretty-printer | 3–5 weeks |
| M3 | `alloc`/`dealloc`/`defer`, moves, arena, refcounted strings | String builder, dynamic array | 6–10 weeks |
| M4 | `extern` C FFI + `otter-runtime`, both platforms | Calls libc; SHA-256 via a C library | 3–4 weeks |
| M5 | Slices, maps, modules, multi-file builds | Small file/HTTP tool | 4–6 weeks |
| M6 | Threads, channels, mutexes | Parallel task pool | 4–6 weeks |
| M7 | Escape analysis (removes explicit `alloc`) | Benchmark vs. M3 allocations | 6–12 weeks |
| M8 | Self-hosting bootstrap | Otter compiles its own stdlib | 6–12 weeks |

**Cumulative, honest:**

- **M0–M2** (usable little language): **8–12 weeks**
- **M0–M5** (general purpose): **21–32 weeks**, i.e. **5–8 months**
- **M0–M8** (complete, self-hosting): **33–62 weeks**, i.e. **8–15 months**

### 9.1 Where this is most likely to be wrong

- **M3 is the first slip.** Manual memory plus moves plus refcounting is three
  interacting systems. There is a material chance M3 takes 50% longer than
  estimated, or forces a simplification of the string strategy.
- **M7 is the largest single item** and is genuinely research-grade. It is also
  optional: without it the language is still correct, just allocating more.
- **M8 compounds everything.** Self-hosting surfaces every missing feature in the
  compiler, all at once. Do not start it before M5 is genuinely solid.
- **M6 is smaller than it looks** only if "no data-race guarantees" is accepted
  in v1. Adding deterministic data-race detection later is a significant project.

---

## 10. Testing strategy

- **Golden files**: source → expected AST and expected IR, so front-end changes
  are diffable and regressions are visible.
- **Unit tests** for inference, move checking, and escape analysis in isolation,
  without LLVM in the loop.
- **End-to-end suite** of real programs asserting correct output, run on
  **both Windows and Linux** in CI from M4 onward.
- **Type-error and runtime-failure tests** asserting not just that a program
  fails, but that the diagnostic names the right span.

### 10.1 Scope of the first implementation plan

This spec covers the whole language, which is too large for one implementation
plan. The first plan written from it will cover **M0–M2 only** — a working
lexer, parser, sema, codegen, and `hello world` through a typed value tree.
M3 onward each get their own spec → plan → implementation cycle.

---

## 11. Open questions for later milestones

Not blocking v1; decide when the milestone is reached.

1. Does `:=` survive as `const` sugar, or is `const` the only spelling?
2. Module system shape: flat namespace with prefixes, or per-file scoping?
3. Closures and higher-order functions — needed, and in what form?
4. Package manager: is a registry ever needed, or is git+manifest enough?
5. Generics after M5: type parameters, inference, and monomorphization vs. dictionaries.
6. Whether to add data-race detection to M6 or defer it indefinitely.