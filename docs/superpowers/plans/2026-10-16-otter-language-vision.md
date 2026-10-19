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

## 3. The Minimum Viable Language (MVL)

Before listing aspirational features, define the smallest set that makes Otter *useful* for real work. If a feature is not in the MVL, it can wait.

### MVL Feature Set

```otter
// 1. Variables with type inference
mut count = 0;           // i64 by default
const name = "otter";    // string
const ratio = 3.14;      // f64
const flag = true;       // bool

// 2. Functions with multiple returns
fn add(a: i64, b: i64) -> i64 {
    return a + b;
}

// 3. Error handling without exceptions
fn parse_port(s: string) -> Result[i64, string] {
    return parse_i64(s) ?? Err("not a number");
}

// 4. Pattern matching on enums
enum Shape {
    Circle { radius: f64 },
    Rect { w: f64, h: f64 },
}

fn area(s: Shape) -> f64 {
    return switch s {
        case Circle { radius } { 3.14159 * radius * radius }
        case Rect { w, h } { w * h }
    };
}

// 5. Defer for cleanup
fn read_config(path: string) -> string {
    const f = open_file(path)?;
    defer { close_file(f); }
    return read_all(f);
}

// 6. Structs with field access
struct Point { x: i64, y: i64 }
fn distance(p: Point) -> i64 {
    return p.x * p.x + p.y * p.y;
}

// 7. Arrays and indexing
fn sum(nums: []i64) -> i64 {
    mut total = 0;
    for i in 0..len(nums) {
        total = total + nums[i];
    }
    return total;
}

// 8. String interpolation
fn greet(name: string) -> string {
    return "Hello, ${name}!";
}

// 9. C interop
extern "C" {
    fn puts(s: *const i8) -> i32;
}

fn main() {
    puts("Hello from Otter");
}
```

**If a user can write all of the above, Otter is a real language.** Everything else is polish.

---

## 4. Features Beyond M10

These are grouped by the user value they unlock, not by implementation difficulty.

### 4.1 Syntax That Disappears

**Goal:** A user can read Otter code they have never seen and guess what it means.

#### 4.1.1 `const` by default, `mut` explicit

**Current:** `var x = 5; const y = 10;`
**Proposed:** `x = 5;` is immutable by default; `mut x = 5;` is mutable.

**Why:** Signals intent. Helps escape analysis (M9) prove that a binding never changes, enabling more optimizations. Makes code easier to read — you can scan a function and immediately see what changes.

**Trade-off:** Slightly more verbose for throwaway scripts. Mitigated by type inference and the fact that most bindings in a well-written function are immutable.

#### 4.1.2 Raw strings

**Current:** `"C:\\Users\\name\\file.txt"` — escape processing required.
**Proposed:** `r"C:\Users\name\file.txt"` — no escape processing.

**Why:** Windows paths, regex patterns, and SQL queries are common in real programs. Escape processing makes them unreadable.

#### 4.1.3 Multiline strings

**Current:** `"line1\nline2\nline3"` — `\n` noise.
**Proposed:**
```otter
let sql = """
    SELECT id, name, email
    FROM users
    WHERE active = true
    ORDER BY name
""";
```

**Why:** SQL, shell commands, and embedded text are common. Multiline strings make them readable and copy-pasteable.

#### 4.1.4 Type-safe interpolation

**Current:** `"${user.name} is ${age}"` — runtime conversion.
**Proposed:** Compile-time checked. If `user.name` is not a `string` or `age` is not an `i64`, the compiler reports an error.

**Why:** Catches bugs at compile time. The compiler already knows the types; it should use them.

#### 4.1.5 Array and slice literals

**Current:** No array literals. Must use `alloc` + manual indexing.
**Proposed:**
```otter
let nums = [1, 2, 3, 4, 5];
let names = ["alice", "bob", "carol"];
let matrix = [[1, 2], [3, 4]];
let slice = nums[1..3];  // [2, 3]
```

**Why:** Arrays are fundamental. Users expect them. Without them, every program starts with manual memory management.

#### 4.1.6 Explicit error propagation (`?`)

**Current:** `panic` exits the process. No way to propagate errors.
**Proposed:**
```otter
fn read_file(path: string) -> Result[string, string] {
    return read_file_impl(path)?;  // propagate Err, unwrap Ok
}

fn main() {
    let content = read_file("data.txt")?;  // exits main with Err on failure
    let content = read_file("data.txt") ?? "default";  // coalesce
}
```

**Why:** Libraries need to return errors without killing the process. `?` is the standard solution (Rust, Swift, Zig). It makes error handling explicit but lightweight.

#### 4.1.7 No implicit conversions

**Current:** `i64` can silently become `f64` in some contexts.
**Proposed:** Never. `let x: f64 = 5;` is an error. Write `5.0` or `5 as f64`.

**Why:** Implicit conversions are a source of subtle bugs. Explicit is better.

### 4.2 Performance That Surprises

**Goal:** Otter programs run faster than users expect for a language this simple.

#### 4.2.1 Escape analysis (M9) — the hero optimization

**What it does:** If a pointer never leaves its allocating function, it is stack-allocated instead of heap-allocated. The `dealloc` is inserted automatically at scope exit.

**Example:**
```otter
fn process() -> i64 {
    let buf = alloc(1024);   // currently: heap allocation
    // ... use buf ...
    dealloc(buf);             // currently: manual
    return result;
}
```

With escape analysis:
```otter
fn process() -> i64 {
    let buf = alloc(1024);   // stack-allocated (never escapes)
    // ... use buf ...
    // dealloc inserted automatically at scope exit
    return result;
}
```

**Impact:** Eliminates most manual `alloc`/`dealloc` pairs. Makes Otter feel like it has a GC without the pauses. This is the single most impactful optimization for developer experience.

#### 4.2.2 Compile-time evaluation (`const fn`)

**What it does:** Pure functions marked `const` are evaluated at compile time.

**Example:**
```otter
const fn build_lookup_table() -> [i64; 256] {
    mut table = [0; 256];
    for i in 0..256 {
        table[i] = i * i;
    }
    return table;
}

const TABLE = build_lookup_table();  // computed at compile time

fn main() {
    print_i64(TABLE[42]);  // 1764, zero runtime cost
}
```

**Impact:** Lookup tables, computed constants, and configuration validation cost zero at runtime. Turns Otter into a language where you can pay costs once, not at runtime.

#### 4.2.3 SIMD-friendly types

**What it does:** Explicit vector types that map to SIMD instructions.

**Example:**
```otter
fn sum_four(a: i64x4, b: i64x4) -> i64x4 {
    return a + b;  // one SIMD add instead of four scalar adds
}
```

**Impact:** Numeric kernels (image processing, audio, physics) get automatic vectorization. Users don't need to write intrinsics.

#### 4.2.4 `#[inline]` + monomorphization

**What it does:** Small functions marked `#[inline]` are inlined at call sites. Generic functions are specialized per type.

**Example:**
```otter
#[inline]
fn min(a: i64, b: i64) -> i64 {
    return a < b ? a : b;
}

fn map(list: []i64, f: fn(i64) -> i64) -> []i64 {
    // specialized for f = min, f = square, etc.
}
```

**Impact:** Small function call overhead disappears. Generics don't cost anything at runtime.

#### 4.2.5 Performance Tiers

Otter's performance is not a single number — it depends on what the user is willing to do. These tiers describe the realistic performance at each level of effort.

| Tier | What the user does | Expected performance | Effort |
|------|-------------------|---------------------|--------|
| **Tier 1: Default** | Write Otter as-is, `otter build` | ~1.0–2.5× slower than C on arithmetic-heavy code; ~1.0× on call-dominated code | Zero |
| **Tier 2: Opt-outs** | Use `wrapping_*`, `--release`, `-O3` | Within 5–15% of C | Low |
| **Tier 3: Idiomatic** | Avoid hot-loop string concat, use `i64` not `Option[i64]`, keep functions small | Within 3–10% of C | Medium |
| **Tier 4: Escape analysis** | M9 escape analysis eliminates heap allocations | Within 1–5% of C for allocation-heavy code | High |

##### Tier 1: Default (Zero Effort)

```otter
fn main() {
    var sum = 0;
    for i in 0..100000000 {
        sum = sum + i;
    }
    print_i64(sum);
}
```

**Performance:** ~1.0× C (LLVM proves the loop safe and deletes the checked arithmetic)

**Why:** The default is `-O2`, and LLVM's optimizer is very good at proving checks unnecessary in simple loops.

##### Tier 2: Opt-outs (Low Effort)

```otter
fn main() {
    var sum = 0;
    for i in 0..100000000 {
        sum = wrapping_add(sum, i);
    }
    print_i64(sum);
}
```

**Performance:** Within 5–15% of C

**What changes:**
- `wrapping_add` removes the overflow check entirely
- `--release` removes `debug_assert` calls
- `-O3` enables more aggressive optimization

**Benchmark expectations:**

| Benchmark | Otter (default) | Otter (opt-outs) | C (clang -O2) | Ratio |
|-----------|-----------------|------------------|----------------|-------|
| `fib(35)` | ~150 ms | ~80 ms | ~78 ms | 1.03× |
| `loop` (portable) | ~260 ms | ~260 ms | ~262 ms | 1.01× |
| `loop` (x86-64-v3) | ~170 ms | ~70 ms | ~68 ms | 1.03× |
| `float` | ~535 ms | ~535 ms | ~530 ms | 1.01× |

##### Tier 3: Idiomatic Otter (Medium Effort)

```otter
// Bad: string concatenation in a hot loop
fn bad() -> string {
    var result = "";
    for i in 0..1000000 {
        result = result + str_from_i64(i);  // allocates every iteration
    }
    return result;
}

// Good: pre-allocate, write into buffer
fn good() -> string {
    var buf = alloc(8000000);
    var pos = 0;
    for i in 0..1000000 {
        pos = write_str(buf, pos, str_from_i64(i));  // no allocation
    }
    return string_from_buf(buf, pos);
}
```

**Performance:** Within 3–10% of C

**Key idioms:**
1. **Avoid allocation in hot loops** — use `alloc` once, reuse the buffer
2. **Use `i64` directly** — not `Option[i64]` (which is `{ i8, i64 }`, twice the size)
3. **Use `wrapping_*` in numeric kernels** — where overflow is impossible or handled separately
4. **Keep functions small** — helps LLVM inline

##### Tier 4: Escape Analysis (High Effort)

With M9 escape analysis, the compiler can stack-allocate objects that don't escape:

```otter
fn process() -> i64 {
    let buf = alloc(1024);   // currently: heap allocation
    // ... use buf ...
    dealloc(buf);             // currently: manual
    return result;
}
```

With escape analysis:
- `buf` never leaves `process()` → stack-allocated
- `dealloc` is inserted automatically at scope exit
- No manual memory management, no heap overhead

**Performance impact:** Eliminates 90%+ of heap allocations in typical code. Allocation-heavy programs (parsers, tree builders) approach C speed.

**Effort:** High. Requires implementing escape analysis in the compiler frontend (or as an LLVM pass on the emitted IR).

#### 4.2.6 Performance ceiling

| Scenario | Expected performance |
|----------|---------------------|
| `-O3 --release` with `wrapping_*` opt-outs | Within 10–30% of C/clang |
| `-O2` default with checked arithmetic | ~1.8–2× slower on arithmetic-heavy kernels |
| `-O2` default, LLVM proves checks safe | ~1.0× (checks deleted) |
| `-O3 --release --cpu=x86-64-v3` | Within 5–15% of C with same flags |

**Theoretical maximum:** Identical to C compiled with clang — because that's literally what Otter becomes. Both end at the same place: LLVM's optimizer and code generator.

### 4.3 Developer Experience: “Install and Love”

**Goal:** Otter feels modern from the first command.

#### 4.3.1 One-command installer

**Current:** Install Rust, install LLVM, build from source.
**Proposed:**
```bash
# Windows
winget install Otter.Otter

# macOS / Linux
curl -L https://otter-lang.org/install | sh
```

**Why:** Users should not need a Rust toolchain or to hunt for clang. The installer downloads a prebuilt binary and puts it on PATH.

#### 4.3.2 `otter fmt` — opinionated formatter

**Current:** No formatter. Code style is whatever the author wrote.
**Proposed:** `otter fmt` reformats all `.ot` files in the current directory to one canonical style.

**Rules:**
- 4-space indentation, no tabs.
- One statement per line.
- Braces on the same line as the keyword (`fn f() {`), closing brace on its own line.
- No trailing whitespace, single trailing newline.
- Maximum line length 100 characters (soft limit, not enforced).

**Why:** One true style. No configuration. No debates. Like `gofmt` or `rustfmt`.

#### 4.3.3 `otter test` — built-in test runner

**Current:** No test runner. Tests are external Rust tests.
**Proposed:** `otter test` discovers `#[test]` functions, compiles them into a test binary, runs them, and reports results.

**Example:**
```otter
#[test]
fn addition_works() {
    assert_eq(1 + 1, 2);
}

#[test]
fn string_concat_works() {
    assert_eq("a" + "b", "ab");
}
```

**Output:**
```
running 2 tests
test addition_works ... ok
test string_concat_works ... ok

test result: ok. 2 passed; 0 failed
```

**Why:** Tests should be written in the language, not in Rust. Built-in runner means zero configuration.

#### 4.3.4 `otter run` — compile and run in one step

**Current:** `otter build main.ot -o main && ./main`
**Proposed:** `otter run main.ot`

**Why:** For scripts and quick experiments, the two-step build-then-run is friction. `otter run` compiles to a temp directory, runs, and cleans up.

#### 4.3.5 `otter init` — project scaffolding

**Current:** No scaffolding. Users create files manually.
**Proposed:** `otter init my-project` creates:
```
my-project/
├── otter.toml          # [package] name, version
├── src/
│   └── main.ot         # fn main() { print_str("Hello, Otter!\n"); }
├── .gitignore          # target/
└── README.md           # build instructions
```

**Why:** A new project should start compiling immediately. No boilerplate.

#### 4.3.6 `otter doc` — documentation generator

**Current:** No documentation generator.
**Proposed:** `otter doc` extracts `///` comments and generates HTML or Markdown.

**Example:**
```otter
/// Adds two integers.
/// Returns the sum.
fn add(a: i64, b: i64) -> i64 {
    return a + b;
}
```

**Generated:**
```markdown
## add(a: i64, b: i64) -> i64

Adds two integers. Returns the sum.
```

**Why:** Documentation should be generated from source, not maintained separately.

#### 4.3.7 LSP — Language Server Protocol

**Current:** No editor integration.
**Proposed:** A `tower-lsp` server providing:
- Diagnostics (inline error squiggles)
- Go-to-definition
- Hover (show type)
- Auto-completion (builtins + in-scope names)
- Rename refactoring

**Why:** Without LSP, Otter feels like 2010. With LSP, it feels modern. Users get IDE features without leaving their editor.

#### 4.3.8 Teaching error messages

**Current:** Errors have spans and carets but minimal suggestions.
**Proposed:** Every error includes:
1. The file, line, and column.
2. The source line with a `^` caret under the offending span.
3. A suggestion for the most likely fix.

**Example:**
```
error: cannot assign to `x`, which is immutable
  --> main.ot:4:5
   |
 4 |     x = 10;
   |     ^^^^^^
   |
help: `x` is declared as `const`. Use `mut x` if you intend to mutate it.
   --> main.ot:2:11
    |
  2 | mut x = 5;
    |     ^^^
```

**Why:** Errors should teach, not scold. A good error message saves 10 minutes of debugging.

### 4.4 Standard Library: Small but Complete

Otter ships a **minimal, correct, fast** core. Everything else comes from packages or C FFI.

#### 4.4.1 Core modules

| Module | Functions | Why |
|--------|-----------|-----|
| `std/io` | `print`, `print_str`, `print_i64`, `print_f64`, `read_line` | Basic I/O |
| `std/string` | `len`, `concat`, `trim`, `split`, `contains`, `starts_with`, `ends_with` | String manipulation |
| `std/result` | `unwrap`, `unwrap_or`, `map`, `and_then` | Error handling |
| `std/option` | `some`, `none`, `map`, `unwrap_or` | Null safety |
| `std/time` | `now`, `sleep`, `duration_since` | Time operations |
| `std/fs` | `read_file`, `write_file`, `exists`, `list_dir` | File operations |
| `std/rand` | `int`, `float`, `bool` | Random numbers |
| `std/ffi` | `extern` helpers, `unsafe` wrappers | C interop |

#### 4.4.2 What is NOT in the core

| Module | Why not |
|--------|---------|
| JSON | Use C FFI or a package |
| HTTP | Use C FFI or a package |
| Regex | Use C FII or a package |
| Crypto | Use C FFI or a package |
| GUI | Out of scope for a systems language |

**Why:** The core stays small enough to audit in an afternoon. Everything else is a package.

### 4.5 Features Otter Deliberately Will Not Have

| Anti-feature | Why |
|--------------|-----|
| **Borrow checker** | Too complex; fights the “simple systems language” vision. Linear ownership + escape analysis is enough. |
| **Async/await runtime** | Adds a runtime dependency; violates “zero external runtime.” |
| **Garbage collector** | Non-deterministic pauses; contradicts the manual memory model. |
| **Exceptions** | Unwinding is complex; `Result` + `?` is sufficient. |
| **Implicit type coercion** | Silent bugs; Otter is explicit. |
| **Power-user macros** | Hard to read, hard to debug, slows compile times. |

---

## 5. Anti-Goals: What Otter Will Not Become

- **Not as fast as hand-tuned C** with checked arithmetic enabled. The checks are real instructions.
- **Not as safe as Rust** without a borrow checker. Use-after-free and double-free are still possible if `move`/`dealloc` are misused.
- **Not as productive as Go** until collections, tooling, and fast compile times exist.
- **Not a scripting language.** Compile time, manual memory, no REPL by default.

These are constraints, not failures. They define Otter’s identity.

---

## 6. Efficiency: How Efficient Can Otter Become?

Efficiency is not just speed. It is speed, memory, binary size, compile time, and energy use combined.

### 6.1 Runtime Efficiency

| Metric | Current | With M9 (escape analysis) | With M9 + CTE | Theoretical max |
|--------|---------|---------------------------|---------------|-----------------|
| **Heap allocations** | Every `alloc` is heap | 90% stack-allocated | 95% stack-allocated | 100% stack-allocated |
| **Memory overhead** | `Option[i64]` = 16 bytes | Same | Same | Same (representation fixed) |
| **Cache locality** | Poor (heap scattered) | Good (stack contiguous) | Excellent | Excellent |
| **Branch mispredictions** | Checked arithmetic branches | Same | Fewer (CTE removes branches) | Minimal |

**Key insight:** Escape analysis (M9) is the single biggest efficiency win. It transforms Otter from a heap-heavy language to a stack-heavy one, improving cache locality and reducing allocator pressure.

### 6.2 Binary Size Efficiency

| Component | Size | Notes |
|-----------|------|-------|
| **Hello world** | ~200 KB | Just the program + minimal runtime |
| **With `std/io`** | ~250 KB | + print/read functions |
| **With `std/string`** | ~300 KB | + string operations |
| **With full stdlib** | ~500 KB | All core modules |
| **With LTO** | ~150 KB | Link-time optimization removes dead code |
| **With `--strip`** | ~100 KB | Remove debug symbols |

**Comparison:**

| Language | Hello world binary | Runtime dependency |
|----------|-------------------|---------------------|
| **Otter** | ~200 KB | None |
| **C (clang -O2)** | ~16 KB | libc |
| **Rust** | ~300 KB | None (statically linked) |
| **Go** | ~2 MB | Go runtime |
| **Python** | ~10 MB | Python interpreter |
| **Node.js** | ~50 MB | Node.js runtime |

**Otter's advantage:** No runtime dependency. The binary is self-contained. This matters for:
- **Embedded systems** — every KB of flash counts
- **CLI tools** — fast startup, no dependency hell
- **Containers** — smaller images, faster pulls
- **Distribution** — ship one file, not a directory

### 6.3 Compile-Time Efficiency

| Scenario | Current | With improvements | Target |
|----------|---------|-------------------|--------|
| **1000 lines** | ~2 seconds | ~1 second | < 2 seconds |
| **10,000 lines** | ~15 seconds | ~5 seconds | < 10 seconds |
| **100,000 lines** | ~3 minutes | ~30 seconds | < 1 minute |
| **Incremental (1 file changed)** | ~2 seconds | ~0.5 seconds | < 1 second |

**What slows compilation:**
1. **Textual IR generation** — writing IR to a string is slower than in-memory IR
2. **clang invocation** — spawning a subprocess has overhead
3. **No incremental compilation** — every build recompiles everything

**What speeds it up:**
1. **Parallel compilation** — compile multiple files concurrently
2. **Incremental compilation** — only recompile changed files
3. **Caching** — cache IR output for unchanged files
4. **LTO** — link-time optimization (slower compile, faster binary)

### 6.4 Energy Efficiency

| Language | Energy per task | Why |
|----------|-----------------|-----|
| **Otter** | Low | Native code, no runtime, no GC |
| **C** | Lowest | Native code, no runtime |
| **Rust** | Low | Native code, no runtime |
| **Go** | Medium | GC pauses, runtime overhead |
| **Python** | High | Interpreter overhead, dynamic typing |
| **Node.js** | High | Interpreter overhead, event loop |

**Otter's advantage:** Native code with no runtime means minimal energy use. This matters for:
- **Battery-powered devices** — longer battery life
- **Data centers** — lower cooling costs
- **Embedded systems** — less heat generation

### 6.5 The Efficiency Ceiling

| Dimension | Ceiling | How to reach it |
|-----------|---------|-----------------|
| **Speed** | Equal to C | Escape analysis + opt-outs + idiomatic code |
| **Memory** | Equal to C | Escape analysis + stack allocation |
| **Binary size** | ~100 KB | LTO + strip + minimal stdlib |
| **Compile time** | ~1s for 1000 lines | Incremental compilation + caching |
| **Energy** | Equal to C | Native code, no runtime |

**The honest answer:** Otter can be **as efficient as C** for most practical purposes. The gap is 1–10% depending on the workload, which is within noise for most applications.

---

## 7. Developer-Friendliness: How Friendly Can Otter Become?

Developer-friendliness is not just syntax. It is the entire experience: writing, reading, debugging, testing, documenting, and maintaining code.

### 7.1 Writing Code

| Aspect | Current | Proposed | Impact |
|--------|---------|----------|--------|
| **Type inference** | Partial | Full | Less annotation, faster writing |
| **Error messages** | Span + caret | Span + caret + suggestion | Faster debugging |
| **Auto-completion** | None | LSP-based | Faster writing, fewer typos |
| **Syntax highlighting** | None | Editor plugins | Easier reading |
| **Formatter** | None | `otter fmt` | Consistent style, no debates |

**Example: Error message evolution**

Current:
```
error: expected `i64`, found `string`
  --> main.ot:3:10
   |
 3 |     let x: i64 = "hello";
   |          ^^^^^
```

Proposed:
```
error: expected `i64`, found `string`
  --> main.ot:3:10
   |
 3 |     let x: i64 = "hello";
   |          ^^^^^
   |
help: `x` is declared as `i64`. Did you mean to use `str_from_i64` or change the type?
```

### 7.2 Reading Code

| Aspect | Current | Proposed | Impact |
|--------|---------|----------|--------|
| **Documentation** | None | `otter doc` + `///` comments | Self-documenting code |
| **Type annotations** | Required | Inferred | Less noise, easier scanning |
| **Syntax** | C-like | Modern + familiar | Easy to learn |
| **Pattern matching** | `switch` | `switch` + exhaustiveness | Clearer intent |

**Example: Self-documenting code**

```otter
/// Reads a configuration file and returns the parsed settings.
/// Returns `Err` if the file does not exist or is malformed.
fn read_config(path: string) -> Result[Config, string] {
    // ...
}
```

### 7.3 Debugging Code

| Aspect | Current | Proposed | Impact |
|--------|---------|----------|--------|
| **Stack traces** | None | On panic | Faster debugging |
| **Debugger** | None | `otter debug` + gdb/lldb | Step-through debugging |
| **Variable inspection** | None | LSP hover | See values without printing |
| **Breakpoints** | None | Editor integration | Visual debugging |

### 7.4 Testing Code

| Aspect | Current | Proposed | Impact |
|--------|---------|----------|--------|
| **Test runner** | External Rust tests | `otter test` | Tests in the language |
| **Test discovery** | Manual | `#[test]` attribute | Automatic discovery |
| **Assertions** | `assert`/`assert_eq` | + `assert_ne`, `assert_approx` | More expressive |
| **Benchmarking** | None | `otter bench` | Performance regression testing |

### 7.5 Maintaining Code

| Aspect | Current | Proposed | Impact |
|--------|---------|----------|--------|
| **Refactoring** | None | LSP rename | Safe renames |
| **Code navigation** | None | LSP go-to-definition | Faster understanding |
| **Dependency management** | None | `otter pkg` | Easy updates |
| **Version management** | None | `otter-version` in `otter.toml` | Clear compatibility |

### 7.6 The Learning Curve

| Stage | Time to competence | What the user can do |
|-------|-------------------|----------------------|
| **Hello world** | 5 minutes | Print text, basic I/O |
| **Basic programs** | 1 hour | Variables, functions, control flow |
| **Real programs** | 1 day | Structs, enums, error handling, files |
| **Idiomatic code** | 1 week | Pattern matching, defer, ownership |
| **Advanced code** | 1 month | FFI, concurrency, optimization |

**Comparison:**

| Language | Time to "hello world" | Time to "real program" | Time to "idiomatic" |
|----------|----------------------|------------------------|---------------------|
| **Otter** | 5 minutes | 1 day | 1 week |
| **C** | 10 minutes | 1 week | 1 month |
| **Rust** | 15 minutes | 1 week | 3 months |
| **Go** | 5 minutes | 1 day | 1 week |
| **Python** | 2 minutes | 1 hour | 1 day |

**Otter's advantage:** Faster to learn than Rust (no borrow checker), safer than C (checked arithmetic), more portable than Go (no runtime).

### 7.7 The Friendliness Ceiling

| Dimension | Ceiling | How to reach it |
|-----------|---------|-----------------|
| **Error messages** | Teach, not scold | Every error has a suggestion |
| **Tooling** | Zero-config | `otter fmt`, `otter test`, `otter run` |
| **Documentation** | Self-generating | `otter doc` from `///` comments |
| **Debugging** | Visual | LSP + breakpoints + variable inspection |
| **Testing** | Built-in | `otter test` with `#[test]` |
| **Learning curve** | 1 day to real programs | MVL + good errors + tutorial |

**The honest answer:** Otter can be **as friendly as Go** for most practical purposes. The key is tooling (M11) and error messages. Without those, it is a compiler; with those, it is a language people enjoy using.

---

## 8. Success Metrics (Not Feature Count)

Measure Otter by these, not by how many keywords it has:

| Metric | Target | How to measure |
|--------|--------|----------------|
| **Time to “hello world”** | Under 5 minutes | From `curl ... \| sh` to running binary |
| **Time to debug a crash** | Under 2 minutes | Error message points to exact line with suggestion |
| **Lines of code for common tasks** | Under 10 lines | Reading a file, parsing JSON, making an HTTP request |
| **Binary size** | Under 500 KB | `otter build hello.ot -o hello && ls -la hello` |
| **Compile time** | Under 2 seconds | 1000-line program |
| **Runtime dependency count** | Zero | `ldd hello` shows only system libraries |
| **Test coverage** | Over 80% | `cargo test --workspace` with coverage reporting |

---

## 9. How Otter Earns Love

People fall in love with languages that respect their time:

1. **Fast to write** — Type inference, no boilerplate, helpful errors.
2. **Fast to run** — Native code, no GC pauses, C interop when you need it.
3. **Fast to ship** — One binary, no runtime, no dependency hell.
4. **Easy to trust** — Overflow, division by zero, and null are errors, not surprises.
5. **Easy to leave alone** — Once it compiles, it keeps working. No dependency churn.

Otter does not need to be the most powerful language. It needs to be the language a developer picks up on a Saturday afternoon and ships something with on Sunday.

---

## 10. Feature Roadmap: What to Add and How

Each feature below includes **what** it is, **why** it matters, and **how** to implement it.

### 10.1 `?` Error Propagation

**What:** Propagate errors from library functions without killing the process.

**Why:** Libraries need to return errors. `panic` exits the process — too blunt for reusable code.

**How:**
1. **Sema:** Add `Expr::Try` (already parsed as `?`). Type-check: if the expression is `Result[T, E]`, unwrap to `T` and generate an early return on `Err`.
2. **Codegen:** Lower `expr?` to:
   ```llvm
   %result = call { i8, T } @expr(...)
   %tag = extractvalue { i8, T } %result, 0
   %is_err = icmp eq i8 %tag, 1
   br i1 %is_err, label %err_block, label %ok_block
   err_block:
   %err = extractvalue { i8, T } %result, 1
   ret { i8, E } { 1, %err }   ; return Err to caller
   ok_block:
   %val = extractvalue { i8, T } %result, 1
   ```
3. **Tests:** `?` on `Ok` unwraps, `?` on `Err` returns early, `?` in a loop returns from the function.

**Effort:** 1 week. The `?` operator already parses; only sema + codegen needed.

### 10.2 Array Literals and Slicing

**What:** `[1, 2, 3]` literals and `nums[1..3]` slicing.

**Why:** Arrays are fundamental. Without them, every program starts with manual memory management.

**How:**
1. **Parser:** Already has `Expr::Index` for `p[i]`. Add array literal parsing: `[expr, expr, ...]`.
2. **Sema:** Type-check array literals: all elements must be the same type. Array type is `[T; N]` (fixed size) or `[]T` (slice).
3. **Codegen:**
   - Fixed array `[T; N]`: stack-allocate `{ T, T, ... }` aggregate.
   - Slice `[]T`: `{ i64, i64, i8* }` (length, capacity, data pointer).
   - Slicing `a[i..j]`: compute new length `j-i`, new data pointer `a.data + i`.
4. **Tests:** Array literal type-checks, slicing produces correct sub-array, out-of-bounds slice is a runtime error.

**Effort:** 2 weeks. Parser + sema + codegen + runtime (for bounds checking).

### 10.3 `const fn` — Compile-Time Evaluation

**What:** Pure functions marked `const` are evaluated at compile time.

**Why:** Lookup tables and computed constants cost zero at runtime.

**How:**
1. **Sema:** Add `const` keyword to function declarations. Mark the function as compile-time evaluable.
2. **Codegen:** When a `const fn` is called in a `const` context, evaluate it at compile time:
   - Parse the function body into an AST.
   - Interpret the AST with a simple tree-walker (no codegen).
   - Substitute the result into the calling code.
3. **Tests:** `const TABLE = build_table();` produces no runtime code for `build_table`.

**Effort:** 2 weeks. Requires an interpreter for a subset of Otter (pure functions only).

### 10.4 Escape Analysis (M9)

**What:** Stack-allocate objects that never leave their function.

**Why:** Eliminates 90%+ of heap allocations. Biggest single performance win.

**How:**
1. **Sema:** Track which pointers escape their function:
   - A pointer escapes if it is: returned, stored in a global, passed to a function that stores it, or assigned to a binding that outlives the function.
   - Use a simple intraprocedural analysis: if a pointer is only used within the function and never returned/passed to unknown functions, it does not escape.
2. **Codegen:** For non-escaping pointers:
   - Emit `alloca` instead of `call @otter_alloc`.
   - Insert automatic `dealloc` at scope exit (or just let the stack frame die).
3. **Tests:** A function that allocates and does not return the pointer uses stack allocation.

**Effort:** 3–4 weeks. The analysis is the hard part; codegen is straightforward.

### 10.5 LSP — Language Server

**What:** IDE integration: diagnostics, go-to-definition, hover, completion.

**Why:** Without LSP, Otter feels like 2010. With LSP, it feels modern.

**How:**
1. **Server:** Use `tower-lsp` (Rust LSP library). Implement:
   - `textDocument/didOpen`: parse + type-check, publish diagnostics.
   - `textDocument/definition`: resolve identifier to declaration.
   - `textDocument/hover`: show type of symbol under cursor.
   - `textDocument/completion`: suggest builtins + in-scope names.
2. **Diagnostics:** Reuse the existing diagnostic infrastructure (spans + carets).
3. **Tests:** LSP responds to `initialize`, `didOpen`, `definition`, `hover`, `completion`.

**Effort:** 2–3 weeks. `tower-lsp` handles the protocol; the work is in the Otter-specific logic.

### 10.6 `otter fmt` — Formatter

**What:** Opinionated formatter, one true style.

**Why:** No configuration, no debates. Like `gofmt`.

**How:**
1. **AST traversal:** Walk the AST and re-emit source with canonical formatting.
2. **Rules:**
   - 4-space indent, no tabs.
   - One statement per line.
   - Braces on same line as keyword.
   - Max line length 100 (soft limit).
3. **Idempotency:** Formatting twice produces identical output.
4. **Tests:** Format a messy file, verify output is canonical. Format twice, verify idempotency.

**Effort:** 1 week. Pure AST-to-text transformation.

### 10.7 `otter test` — Test Runner

**What:** Built-in test runner with `#[test]` discovery.

**Why:** Tests should be written in the language, not in Rust.

**How:**
1. **Discovery:** Scan source for `#[test]` functions.
2. **Codegen:** Generate a `main` that calls each test function and reports pass/fail.
3. **Runtime:** Run the generated binary, capture output, print summary.
4. **Tests:** `#[test]` function runs, failing test reports position, discovery skips non-test functions.

**Effort:** 1–2 weeks. Codegen for the test harness + runtime reporting.

### 10.8 Stack Traces on Panic

**What:** When a panic unwinds the stack, print each frame with file:line:col.

**Why:** Debugging is 50% of development time. Stack traces are essential.

**How:**
1. **Runtime:** Maintain a thread-local stack of function entry points (file, line, column).
2. **Panic path:** When `otter_panic` is called, walk the stack and print each frame.
3. **Tests:** A panicking program prints a stack trace with correct file:line:col for each frame.

**Effort:** 1 week. Runtime stack tracking + printing.

### 10.9 Cross-Compilation

**What:** `--target=x86_64-linux-gnu`, `--target=wasm32-unknown-wasi`.

**Why:** Cross-compilation is essential for embedded targets and CLI distribution.

**How:**
1. **CLI:** Add `--target` flag to `otter build`.
2. **Codegen:** Pass `--target` to clang.
3. **Tests:** Cross-compile to a different target, verify the binary runs (or at least links).

**Effort:** 1 week. Mostly CLI plumbing; clang does the work.

### 10.10 Package Manager

**What:** `otter pkg add`, `otter pkg remove`, `otter pkg update`.

**Why:** Package management is the foundation of an ecosystem.

**How:**
1. **Registry:** Simple HTTP registry hosting `.tar.gz` packages.
2. **CLI:** `otter pkg add <name>` downloads and extracts to `~/.otter/packages/<name>`.
3. **Build:** `otter build` searches `~/.otter/packages/` for imported modules.
4. **Tests:** Add a package, import it, compile, run.

**Effort:** 3–4 weeks. Registry + CLI + build integration.

---

## 11. Implementation Priority

| Priority | Feature | Effort | User value | Dependencies |
|----------|---------|--------|------------|--------------|
| **P0** | `?` error propagation | 1 week | High | None |
| **P0** | `otter fmt` | 1 week | High | None |
| **P0** | `otter test` | 1–2 weeks | High | None |
| **P1** | Array literals + slicing | 2 weeks | High | None |
| **P1** | LSP | 2–3 weeks | High | None |
| **P1** | Stack traces on panic | 1 week | Medium | None |
| **P2** | `const fn` | 2 weeks | Medium | None |
| **P2** | Cross-compilation | 1 week | Medium | None |
| **P3** | Escape analysis (M9) | 3–4 weeks | High | None |
| **P3** | Package manager | 3–4 weeks | Medium | None |

**Recommended order:** P0 first (error propagation, fmt, test), then P1 (arrays, LSP, stack traces), then P2 (const fn, cross-compilation), then P3 (escape analysis, package manager).

---

## 12. Migration Paths: Why Switch?

### From C

**Why switch:** Memory safety without a GC. Checked arithmetic. `defer` for cleanup. String interpolation.

**What you lose:** Direct pointer arithmetic (until `unsafe` is stable). Manual memory layout control.

**Migration path:** Wrap C libraries with `extern "C"`. Gradually replace C code with Otter.

### From Rust

**Why switch:** Simpler ownership model (no borrow checker). Faster compile times. Smaller binaries.

**What you lose:** Borrow checker guarantees. Macro system. Larger ecosystem.

**Migration path:** Rewrite performance-critical code in Otter. Keep Rust for libraries.

### From Go

**Why switch:** No GC pauses. Smaller binaries. More control over memory layout.

**What you lose:** Goroutines. Channels. Faster compile times (Go is faster to compile).

**Migration path:** Rewrite latency-sensitive services in Otter. Keep Go for infrastructure.

### From Python

**Why switch:** Native performance. Static typing. No runtime dependency.

**What you lose:** Dynamic typing. REPL. Huge ecosystem.

**Migration path:** Rewrite performance-critical paths in Otter. Keep Python for scripting.

---

## 13. Relationship to Existing Plans

This document complements, not replaces:

- `docs/superpowers/plans/2026-10-16-otter-master-plan.md` — cross-milestone contract and interface ledger.
- `docs/superpowers/plans/2026-10-16-otter-m0-m2-compiler.md` through `m11-adoption.md` — implementation task lists with checkboxes.
- `docs/superpowers/specs/2026-10-16-otter-language-design.md` — language specification.

When a feature from this vision is ready to implement, an executor writes a phase plan with checkbox tasks, adds it to the master plan's dependency order, and derives interface ledger entries from this vision's feature table.

---

## 14. Debugging & Profiling Experience

**Goal:** Users can debug Otter programs without leaving their editor.

### 10.1 Debugging

**Current state:** `panic` prints a message and exits. No debugger integration.

**Proposed:**

| Feature | Description |
|---------|-------------|
| **DWARF debug info** | Generated binaries include debug info so gdb/lldb work out of the box |
| **Breakpoint support** | `otter debug` launches a GDB session with Otter-aware pretty-printing |
| **Stack traces on panic** | When a panic unwinds the stack, print each frame with file:line:col |
| **Variable inspection** | `otter print <expr>` in the debugger shows the value at runtime |

**Example (stack trace on panic):**
```
thread 'main' panicked at 'integer overflow: 9223372036854775807 + 1'
stack trace:
  0: main::compute at src/main.ot:12:15
  1: main::run at src/main.ot:8:5
  2: main at src/main.ot:4:1
```

**Why:** Debugging is 50% of development time. A language without debugging support is a language people avoid for real work.

### 10.2 Profiling

**Current state:** No profiling tools. Users must use external profilers.

**Proposed:**

| Feature | Description |
|---------|-------------|
| **`otter profile`** | Built-in profiler that samples stack traces and outputs a flamegraph |
| **`--profile` flag** | Compile with profiling hooks; runtime writes a trace file |
| **Benchmark runner** | `otter bench` runs `#[bench]` functions and reports statistics |

**Example:**
```otter
#[bench]
fn fib_35() {
    fib(35);
}
```

**Output:**
```
running 1 benchmark
fib_35: min=79.2ms, max=81.1ms, mean=80.0ms, stddev=0.4ms
```

**Why:** Performance work without measurement is guesswork. Built-in profiling makes optimization accessible.

---

## 15. Cross-Compilation & WebAssembly

**Goal:** Otter code compiles to any target clang supports, including WebAssembly.

### 11.1 Cross-compilation

**Current state:** Compiles to the host platform only.

**Proposed:**
```bash
otter build main.ot -o main --target=x86_64-linux-gnu
otter build main.ot -o main --target=x86_64-w64-mingw32
otter build main.ot -o main --target=wasm32-unknown-emscripten
```

**Why:** Cross-compilation is essential for embedded targets, CLI distribution, and cloud deployment. Since Otter uses clang as the backend, cross-compilation is nearly free.

### 11.2 WebAssembly

**Current state:** No WASM target.

**Proposed:**
- `--target=wasm32` compiles Otter to WASM with WASI for I/O
- `otter build main.ot -o main.wasm --target=wasm32-unknown-wasi`
- JavaScript glue generated automatically for browser use

**Why:** WASM is the future of portable code. Otter's small binaries make it ideal for WASM — no runtime, no GC, just compute.

**Example use case:** Ship an Otter CLI as a WASM module that runs in the browser, with zero dependencies.

---

## 16. Package Management & Ecosystem

**Goal:** Users can share and reuse Otter code without vendoring.

### 12.1 `otter pkg` — package manager

**Current state:** No package manager. All code is local.

**Proposed:**
```bash
otter pkg add otter-http     # add a dependency
otter pkg remove otter-http   # remove a dependency
otter pkg update             # update all dependencies
```

**Registry:** A simple HTTP registry (like crates.io or npm) hosting Otter packages. Each package is a `.tar.gz` with `src/*.ot`, `otter.toml`, and a `README.md`.

### 12.2 `otter.toml` — package manifest

**Current state:** No manifest.

**Proposed:**
```toml
[package]
name = "my-app"
version = "0.1.0"
otter-version = "0.5"

[dependencies]
otter-http = "0.2"
otter-json = "1.0"

[build]
opt = "O3"
release = true
```

**Why:** Package management is the foundation of an ecosystem. Without it, every project is an island.

---

## 17. Security & Safety Features

**Goal:** Otter is the safest systems language available.

### 13.1 Sandboxing

**Proposed:**
```otter
@sandbox(files="readonly", network="none")
fn process_data(path: string) -> i64 {
    // Can only read files, cannot write or open network connections
    return count_lines(path);
}
```

**Why:** Untrusted code execution (plugins, user scripts) needs sandboxing. Otter's zero-runtime design makes sandboxing easier — intercept `open`/`socket` calls.

### 13.2 Capability-based security

**Proposed:**
```otter
fn read_file(cap: FileCap, path: string) -> string {
    // Only works if `cap` grants read access to `path`
    ...
}
```

**Why:** Unix permissions are coarse-grained. Capability-based security is fine-grained. Otter's linear types make capabilities natural — they are values that can be passed around.

### 13.3 No `unsafe` by default

**Proposed:** All safe Otter code is memory-safe by construction. `unsafe` blocks are:
- Opt-in per function or block
- Audited by the compiler (all `unsafe` uses are listed in documentation)
- Required for: raw pointer arithmetic, `extern "C"` calls, inline assembly

---

## 18. Community & Adoption Strategy

**Goal:** Otter grows from a solo project to a community language.

### 14.1 Documentation as a first-class citizen

| Resource | Purpose |
|----------|---------|
| **Getting started** | 5-minute "hello world" tutorial |
| **Language guide** | Complete reference, like the Rust Book |
| **Cookbook** | Task-oriented examples ("How do I read a JSON file?") |
| **API reference** | Generated from `///` comments |
| **FAQ** | Answers to common questions |

### 14.2 Community infrastructure

| Platform | Purpose |
|----------|---------|
| **Discord/IRC** | Real-time help |
| **Forum** | Long-form discussions |
| **GitHub Discussions** | Q&A, feature requests |
| **Blog** | Release notes, deep dives |

### 14.3 Adoption hooks

| Hook | Why it works |
|------|--------------|
| **"Otter is what C should have been"** | Appeals to C programmers tired of footguns |
| **"Otter is what Rust would be without the borrow checker"** | Appeals to Rust programmers who want simplicity |
| **"Otter binaries are 100 KB"** | Appeals to embedded/IoT developers |
| **"Otter has zero runtime dependencies"** | Appeals to DevOps/platform engineers |

---

## 19. Timeline: What to Build When

| Phase | Duration | Focus | Success criterion |
|-------|----------|-------|-------------------|
| **M0–M10** | 12–19 months | Language + compiler | Self-hosting works |
| **M11** | 4–6 months | Adoption: fmt, test, LSP, installer | A stranger can install and use Otter in 5 minutes |
| **M12** | 4–6 months | Ecosystem: package manager, registry, stdlib growth | 10 community-contributed packages exist |
| **M13** | 3–4 months | Performance: escape analysis, SIMD, CTE | Allocation-heavy code runs within 10% of C |
| **M14** | 2–3 months | Polish: tutorial, error messages, editor themes | "Learn Otter in 30 minutes" guide published |
| **M15** | Ongoing | Maturity: stability guarantees, LTS releases | Backward compatibility promised |

**Total time to a language people love:** ~2–3 years of focused work.

---

## 20. The One-Page Pitch

> **Otter** is a systems language that compiles to native binaries with zero
> runtime dependencies. It writes like Python, runs like C, and links like
> nothing.
>
> - **Safe by default:** Integer overflow, division by zero, and null
>   dereference are reported errors, not silent wrong answers.
> - **Simple by design:** No borrow checker, no GC, no async runtime, no
>   exceptions. Linear ownership + `defer` is enough.
> - **Fast to ship:** One binary, no dependencies. Cross-compiles to any
>   target clang supports, including WebAssembly.
> - **C interop:** Call any C library with `extern "C"` and `unsafe` blocks.
>
> Install with one command. Compile in seconds. Ship a single binary.
>
> **Otter: Write like a scripting language, run like C, link like nothing.**

---
