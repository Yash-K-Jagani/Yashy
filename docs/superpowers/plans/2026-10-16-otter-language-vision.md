# Otter Language Vision — Beyond M10

> **Status:** aspirational design document. This file holds the long-term
> direction for Otter after M0–M10 are complete. It is not an implementation
> plan with task checkboxes; it is a compass. Implementation tasks derived from
> it live in their own phase plans.
>
> **Read first:** `docs/superpowers/plans/2026-10-16-otter-master-plan.md`
> (cross-milestone contract) and `docs/superpowers/specs/2026-10-16-otter-language-design.md`
> (language specification).

---

## 1. The Core Promise

Otter’s pitch is one sentence:

> **Write like a scripting language, run like C, link like nothing.**

A user should be able to:

- Install Otter with one command and compile their first program in under five minutes.
- Write code that feels lightweight — type inference, string interpolation, no header files, no build system configuration.
- Get native binaries with no external runtime dependency: no `.dll`, no `.so`, no GC, no runtime DLL to ship.
- Call C libraries directly through `extern "C"` and `unsafe` blocks.
- Trust that integer overflow, division by zero, and null dereference are *reported errors*, not silent wrong answers.
- Know that every `defer` runs, every `assert` checks, and every `panic` tells them exactly where it happened and why.

If Otter delivers on that sentence, people will use it for CLI tools, small services, embedded utilities, and systems glue — not because it has every feature, but because it gets out of the way.

---

## 2. What M0–M10 Buys (Baseline)

| Milestone | Delivers |
|-----------|----------|
| **M0–M2** | Working compiler: lexer, parser, sema, codegen, CLI. `i64`, `f64`, `bool`, `string`, `struct`, `enum`, `Option<T>`, `Result<T,E>`, `if`/`while`/`for`/`switch`, `defer`. |
| **M3** | Heap (`alloc`/`dealloc`), strings, input, `move`/linear ownership, arenas. |
| **M4** | `panic`, checked arithmetic, `assert`/`debug_assert`, `--release`, `os.exit`, defer on panic path. |
| **M5** | `extern "C"` FFI, `unsafe` blocks, C interop on Windows and Linux. |
| **M6** | Collections: arrays, slices, maps, indexing, `for i in 0..n`. |
| **M7** | Concurrency: OS threads, message-passing channels, mutexes. |
| **M8** | Tooling: `fmt`, tutorial, LSP, syntax highlighting. |
| **M9** | Escape analysis (optional optimization). |
| **M10** | Self-hosting bootstrap: Otter compiles Otter. |

M0–M10 makes Otter a *complete* language. It does not yet make Otter a *beloved* language.

This document is about what comes after: the features, polish, and UX that turn a correct compiler into a language developers reach for by default.

---

## 3. Features Beyond M10

These are grouped by the user value they unlock, not by implementation difficulty.

### 3.1 Syntax That Disappears

**Goal:** A user can read Otter code they have never seen and guess what it means.

| Feature | What it changes | Example |
|---------|----------------|---------|
| **`const` by default, `mut` explicit** | Signals intent; helps escape analysis | `mut x = 5;` vs `x = 5;` |
| **Raw strings** | No escape-processing for paths, regex, SQL | `r"C:\Users\name\file.txt"` |
| **Multiline strings** | Embedded text without `\n` noise | ```otter\nlet sql = """\n  SELECT * FROM users\n  WHERE active = true\n""";\n``` |
| **Type-safe interpolation** | Compile-time checked `${expr}` | `"${user.name} is ${age}"` — errors if types don’t match |
| **Array / slice literals** | Fundamental data structures feel native | `[1, 2, 3]`, `["a", "b"]`, `nums[1..3]` |
| **Explicit error propagation (`?`)** | Libraries are pleasant to write and call | `read_file("x.txt")?` |
| **No implicit conversions** | `i64` never silently becomes `f64`; `string` never becomes `i8*` | Eliminates an entire class of subtle bugs |

### 3.2 Performance That Surprises

**Goal:** Otter programs run faster than users expect for a language this simple.

| Feature | What it changes |
|---------|----------------|
| **Escape analysis (M9)** | Short-lived `alloc` becomes stack allocation; automatic `dealloc` elimination. Manual memory management without the manual work. |
| **Compile-time evaluation (`const fn`)** | Pure functions run at compile time; lookup tables and computed constants cost zero at runtime. |
| **SIMD-friendly types** | `i64x4`, `f64x2` for numeric kernels; emits IR that clang vectorizes. |
| **`#[inline]` + monomorphization** | Small functions and generics specialize to fast, direct machine code. |
| **`-O3 --release` as default path** | The optimizer level a shipped binary actually wants; `debug_assert` compiled out automatically. |

**Performance ceiling:** Within 10–30% of C/clang on the same algorithm when using opt-outs (`wrapping_*`, `--release`). With checked arithmetic on, expect ~1.8–2× on arithmetic-heavy kernels; LLVM proves many loops safe and deletes the checks entirely.

### 3.3 Developer Experience: “Install and Love”

**Goal:** Otter feels modern from the first command.

| Feature | What it changes |
|---------|----------------|
| **One-command installer** | `curl ... \| sh` or `winget install Otter.Otter` — no Rust toolchain, no clang hunting, no PATH manipulation. |
| **`otter fmt`** | Opinionated formatter, one true style, zero configuration (like `gofmt`). |
| **`otter test`** | Built-in test runner; `#[test]` functions discovered automatically. |
| **`otter run`** | Compile and run in one step; no `-o` required for scripts. |
| **`otter init`** | Scaffolds `otter.toml`, `src/main.ot`, `.gitignore`, `README.md`. |
| **`otter doc`** | Generates documentation from `///` comments. |
| **LSP** | Go-to-definition, rename, inline error squiggles, auto-completion in VS Code and other editors. |
| **Teaching error messages** | Errors point at the exact span, show the line with a caret, and suggest a fix. |

### 3.4 Standard Library: Small but Complete

Otter ships a **minimal, correct, fast** core. Everything else comes from packages or C FFI.

| Module | Purpose |
|--------|---------|
| `std/io` | `print`, `read_line`, file read/write |
| `std/string` | `len`, `concat`, `trim`, `split`, `contains` |
| `std/result` | `Result` helpers: `unwrap`, `unwrap_or`, `map` |
| `std/option` | `Option` helpers: `some`, `none`, `map` |
| `std/time` | `now`, `sleep`, `duration` |
| `std/fs` | File operations |
| `std/rand` | Basic random numbers |
| `std/ffi` | `extern "C"` helpers |

JSON, HTTP, regex, and crypto live in packages, not the core. The core stays small enough to audit in an afternoon.

### 3.5 Features Otter Deliberately Will Not Have

| Anti-feature | Why |
|--------------|-----|
| **Borrow checker** | Too complex; fights the “simple systems language” vision. Linear ownership + escape analysis is enough. |
| **Async/await runtime** | Adds a runtime dependency; violates “zero external runtime.” |
| **Garbage collector** | Non-deterministic pauses; contradicts the manual memory model. |
| **Exceptions** | Unwinding is complex; `Result` + `?` is sufficient. |
| **Implicit type coercion** | Silent bugs; Otter is explicit. |
| **Power-user macros** | Hard to read, hard to debug, slows compile times. |

---

## 4. Anti-Goals: What Otter Will Not Become

- **Not as fast as hand-tuned C** with checked arithmetic enabled. The checks are real instructions.
- **Not as safe as Rust** without a borrow checker. Use-after-free and double-free are still possible if `move`/`dealloc` are misused.
- **Not as productive as Go** until collections, tooling, and fast compile times exist.
- **Not a scripting language.** Compile time, manual memory, no REPL by default.

These are constraints, not failures. They define Otter’s identity.

---

## 5. Success Metrics (Not Feature Count)

Measure Otter by these, not by how many keywords it has:

| Metric | Target |
|--------|--------|
| **Time to “hello world”** | Under 5 minutes from zero to running binary. |
| **Time to debug a crash** | Error message points to exact line with caret and a suggestion. |
| **Lines of code for common tasks** | Reading a file, parsing JSON, making an HTTP request — under 10 lines each. |
| **Binary size** | Hello world under 500 KB. |
| **Compile time** | 1000 lines under 2 seconds. |
| **Runtime dependency count** | Zero. Ship the binary, nothing else. |

---

## 6. How Otter Earns Love

People fall in love with languages that respect their time:

1. **Fast to write** — Type inference, no boilerplate, helpful errors.
2. **Fast to run** — Native code, no GC pauses, C interop when you need it.
3. **Fast to ship** — One binary, no runtime, no dependency hell.
4. **Easy to trust** — Overflow, division by zero, and null are errors, not surprises.
5. **Easy to leave alone** — Once it compiles, it keeps working. No dependency churn.

Otter does not need to be the most powerful language. It needs to be the language a developer picks up on a Saturday afternoon and ships something with on Sunday.

---

## 7. Relationship to Existing Plans

This document complements, not replaces:

- `docs/superpowers/plans/2026-10-16-otter-master-plan.md` — cross-milestone contract and interface ledger.
- `docs/superpowers/plans/2026-10-16-otter-m0-m2-compiler.md` through `m9-m10` — implementation task lists with checkboxes.
- `docs/superpowers/specs/2026-10-16-otter-language-design.md` — language specification.

When a feature from this vision is ready to implement, an executor writes a phase plan with checkbox tasks, adds it to the master plan’s dependency order, and derives interface ledger entries from this vision’s feature table.

---
