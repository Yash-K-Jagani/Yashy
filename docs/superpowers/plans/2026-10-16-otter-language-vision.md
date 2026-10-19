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

## 6. Success Metrics (Not Feature Count)

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

## 7. How Otter Earns Love

People fall in love with languages that respect their time:

1. **Fast to write** — Type inference, no boilerplate, helpful errors.
2. **Fast to run** — Native code, no GC pauses, C interop when you need it.
3. **Fast to ship** — One binary, no runtime, no dependency hell.
4. **Easy to trust** — Overflow, division by zero, and null are errors, not surprises.
5. **Easy to leave alone** — Once it compiles, it keeps working. No dependency churn.

Otter does not need to be the most powerful language. It needs to be the language a developer picks up on a Saturday afternoon and ships something with on Sunday.

---

## 8. Migration Paths: Why Switch?

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

## 9. Relationship to Existing Plans

This document complements, not replaces:

- `docs/superpowers/plans/2026-10-16-otter-master-plan.md` — cross-milestone contract and interface ledger.
- `docs/superpowers/plans/2026-10-16-otter-m0-m2-compiler.md` through `m11-adoption.md` — implementation task lists with checkboxes.
- `docs/superpowers/specs/2026-10-16-otter-language-design.md` — language specification.

When a feature from this vision is ready to implement, an executor writes a phase plan with checkbox tasks, adds it to the master plan's dependency order, and derives interface ledger entries from this vision's feature table.

---

## 10. Debugging & Profiling Experience

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

## 11. Cross-Compilation & WebAssembly

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

## 12. Package Management & Ecosystem

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

## 13. Security & Safety Features

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

## 14. Community & Adoption Strategy

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

## 15. Timeline: What to Build When

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

## 16. The One-Page Pitch

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
