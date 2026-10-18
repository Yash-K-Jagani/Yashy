# Otter

Otter is a general-purpose, statically typed programming language with C-shaped
syntax, compiled to native binaries through LLVM. The compiler is written in
Rust; the generated code is textual LLVM IR that `clang` assembles and links.

There is no runtime library to install and no LLVM build dependency: the print
helpers are a C source file embedded in the compiler binary, and the only
external tool is `clang`.

**Status: milestones M0–M2 are complete.** Otter lexes, parses, type-checks and
compiles programs with `i64`, `f64`, `bool`, `string`, `struct`, `enum`,
`Option<T>`, `Result<T, E>`, `if`/`for`/`switch`, `defer`, and `?` propagation.
Memory management, panics, C interop, collections, concurrency and the standard
library are later milestones; see [Roadmap](#roadmap).

## Quick start

```console
$ cargo build --release --workspace
$ ./target/release/otter build crates/otter-driver/tests/programs/fib.ot -o fib
$ ./fib
55
```

(The programs under `crates/otter-driver/tests/programs/` are the language's
worked examples — they are what the end-to-end suite compiles and runs, so they
cannot drift out of date.)

`otter build` compiles to a native executable, `otter check` type-checks without
emitting anything, and `otter build --emit-ir` prints the generated LLVM IR and
stops — useful on a machine with no LLVM backend at all. `-O0` through `-O3`
control the optimization level passed to `clang`; the default is `-O2`.

## A taste of Otter

```otter
// `var` bindings are mutable, `const` bindings are not. `for i in 0..n` is a
// range loop whose end is exclusive.
fn fib(n: i64) -> i64 {
    var a = 0;
    var b = 1;
    var next = 0;
    for step in 0..n {
        next = a + b;
        a = b;
        b = next;
    }
    return a;
}

fn main() {
    print_i64(fib(10));
    print_str("\n");
}
```

```otter
// Sum types are switched over, and the checker requires the arms to be
// exhaustive. A `switch` used as an expression yields a value.
fn describe(o: Option[i64]) -> i64 {
    return switch o {
        case Some(x) { x }
        case None { 0 }
    };
}
```

## Speed

Same algorithm, same input, every language compiled by its own native
optimizer at `-O2`. Lower is better. Full methodology and caveats in
[`benchmarks/README.md`](benchmarks/README.md).

**`fib(35)` — about 30 million calls** (function call overhead):

| Language | ms | ns/call |
|---|---:|---:|
| C (gcc -O2) | 51 | 1.70 |
| C (clang -O2) | 80 | 2.68 |
| Rust (rustc -O) | 88 | 2.96 |
| **Otter (clang -O2)** | **91** | **3.04** |
| Go | 145 | 4.85 |
| Otter (clang -O0) | 153 | 5.13 |
| Node 24 | 393 | 13.1 |
| CPython 3.14 | 3526 | 118 |

**A 200,000,000-iteration integer loop with a branch:**

| Language | ms | ns/iteration |
|---|---:|---:|
| C (gcc -O2) | 243 | 1.22 |
| **Otter (clang -O2)** | **252** | **1.26** |
| C (clang -O2) | 259 | 1.30 |
| Rust (rustc -O) | 261 | 1.31 |
| Go | 286 | 1.43 |
| Node 24 | 671 | 3.36 |
| Otter (clang -O0) | 1529 | 7.65 |
| CPython 3.14 | 35216 | 176 |

**100,000,000 iterations of an `f64` dependency chain:**

| Language | ms | ns/iteration |
|---|---:|---:|
| **Otter (clang -O2)** | **536** | **5.36** |
| C (gcc -O2) | 540 | 5.40 |
| C (clang -O2) | 542 | 5.42 |
| Rust (rustc -O) | 547 | 5.47 |
| Go | 562 | 5.62 |
| Node 24 | 682 | 6.82 |
| Otter (clang -O0) | 936 | 9.36 |
| CPython 3.14 | 9586 | 95.9 |

Laptops are laptops, and these three tables come from one run of one script on
one machine: the ordering of rows that are close together — Otter, `clang` and
Rust on `fib`, for instance — moves between runs. The `ns/iteration` column is
there because it travels better than the millisecond column.

What the tables say:

- **Otter's scalar code is in the same band as C and Rust, and that is the whole
  claim.** On `loop` and `float` it is within 4% of the fastest portable row — and
  on `float` it *is* the fastest portable row, ahead of both C compilers. On `fib`
  it is about 10% behind `clang`, which is a fact about how each compiler inlines
  a 30-million-call recursion rather than about the quality of the emitted IR. It
  is sometimes ahead of C built by the same `clang` — evidence the IR is clean,
  not evidence that Otter is better than C. It is not a claim that `otter-codegen`
  optimizes anything: it contains no optimization passes, and hands everything to
  LLVM.
- **There is an opt-in `--native` that is worth 3.7× on the loop, and nothing on
  the other two.** It lets the backend use *this* CPU's instruction set instead of
  the x86-64 baseline, which is what allows the loop's accumulator to be
  vectorised four integers at a time rather than two. It is opt-in because the
  binary may then die with an illegal instruction on an older CPU, and it is
  measured against another `clang` build with the same flag rather than against a
  C build that went without it: on the loop, Otter `--native` is 68 ms and
  `clang -march=native` is 75 ms.

| Benchmark | portable | `--native` | Speedup |
|---|---:|---:|---:|
| `fib` | 91 ms | 85 ms | 1.07× |
| `loop` | 252 ms | 68 ms | 3.69× |
| `float` | 536 ms | 542 ms | 0.99× |

- **Enabling optimization mattered enormously.** `otter build` used to pass
  `clang` no optimization flag at all, so every Otter binary ran at `-O0`. Fixing
  that is worth **6.1×** on the loop, 1.7× on `fib` and 1.7× on the float chain —
  the rows above labeled `-O0` are what Otter shipped before.

| Benchmark | `-O0` | `-O2` | Speedup |
|---|---:|---:|---:|
| `fib` | 153 ms | 91 ms | 1.7× |
| `loop` | 1529 ms | 252 ms | 6.1× |
| `float` | 936 ms | 536 ms | 1.7× |

**Compile time** (the `fib` program, best of 9):

| Toolchain | ms |
|---|---:|
| Otter (front end only: lex, parse, check, emit IR) | 20 |
| C (`clang -O2`) | 263 |
| Go (`go build`) | 266 |
| Otter (full build) | 283 |
| Rust (`rustc -O`) | 620 |

The front end — everything this repository implements — is 20 ms. The rest of
`otter build` is `clang` doing the same job it does for C: Otter compiles the IR
to an object, then links it. An earlier version compiled the runtime's C source
from scratch on *every* build, which cost 135 ms of the 403 ms it took then; that
object is now cached in a content-addressed directory, which is where the
415 → 283 ms comes from. The first build on a machine still pays 518 ms while it
fills the cache. See [`docs/performance-improvements.md`](docs/performance-improvements.md)
for the breakdown.

### What the numbers do not prove

The runtime rows are re-measured whenever the compiler's output could change.
`--native` cannot change a portable build — it is one extra argument on the
`clang` command line and no more — so the portable rows moved only with
run-to-run variation, and the `--native` rows are the new measurement. See
[`docs/performance-improvements.md`](docs/performance-improvements.md) for the
round-by-round record, including the changes that were measured and *not* made.

M0–M2 has no heap, no collections, no string operations and no checked
arithmetic. These benchmarks exercise scalar arithmetic and function calls —
they exercise none of the costs a real program pays, so they are a lower bound
on the work to come, not a preview of it. When M4 adds checked integer
arithmetic, the `loop` row is *expected* to get slower; having the number before
is what turns that into a measured cost instead of a surprise. Measured on one
machine with one run of the script: ratios should be stable, absolute times will
not be.

The benchmark suite exists in part because it caught two real compiler bugs that
the test suite could not: `f64` `+`, `-` and `*` were lowering to LLVM's
*integer* opcodes, so any program with float arithmetic failed to compile — and
the golden file had pinned that invalid IR as the expected output. See
[`docs/performance-improvements.md`](docs/performance-improvements.md).

## Building from source

```console
$ cargo build --release --workspace
$ cargo test  --workspace
```

The workspace is one crate per pipeline stage:

| Crate | Job |
|---|---|
| `otter-common` | `Span`, `SourceMap`, `Diagnostic`, and diagnostic rendering |
| `otter-lexer` | bytes to tokens, with byte spans |
| `otter-parser` | tokens to AST |
| `otter-sema` | name resolution, type inference, exhaustiveness checking |
| `otter-codegen` | typed AST to textual LLVM IR |
| `otter-driver` | CLI, pipeline orchestration, toolchain detection |
| `otter-runtime` | the C shim: print helpers, and later allocation and threads |

The compiler never panics on malformed input; that is an acceptance criterion,
not an aspiration, and it has its own test suite
(`crates/otter-driver/tests/never_panics.rs`) that feeds it malformed programs
and asserts a diagnostic comes back instead of a Rust backtrace.

## Documentation

- [`docs/superpowers/specs/2026-10-16-otter-language-design.md`](docs/superpowers/specs/2026-10-16-otter-language-design.md) — the language design
- [`docs/superpowers/plans/`](docs/superpowers/plans) — the milestone plans
- [`benchmarks/README.md`](benchmarks/README.md) — benchmark methodology and results
- [`docs/performance-improvements.md`](docs/performance-improvements.md) — where the remaining performance work is

## Roadmap

| Milestone | Deliverable |
|---|---|
| M0–M2 ✅ | Lexer, parser, sema, codegen, CLI: a usable little language |
| M3 | `alloc`/`dealloc`/`defer`, moves, arenas, refcounted strings |
| M4 | Panic and unwinding, checked arithmetic, `assert`, exit codes |
| M5 | `extern` C FFI and the runtime, on Windows and Linux |
| M6 | Slices, maps, modules, multi-file builds |
| M7 | Threads, channels, mutexes |
| M8 | `otter fmt`, tutorial, LSP, syntax highlighting |
| M9 | Escape analysis |
| M10 | Self-hosting bootstrap |
