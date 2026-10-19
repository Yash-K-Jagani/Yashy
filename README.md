# Otter

Otter is a general-purpose, statically typed programming language with C-shaped
syntax, compiled to native binaries through LLVM. The compiler is written in
Rust; the generated code is textual LLVM IR that `clang` assembles and links.

There is no runtime library to install and no LLVM build dependency: the print
helpers are a C source file embedded in the compiler binary, and the only
external tool is `clang`.

**Status: milestones M0–M2 are complete, and M3 is in progress.** Otter lexes,
parses, type-checks and compiles programs with `i64`, `f64`, `bool`, `string`,
`struct` (with field access), `enum`, `Option<T>`, `Result<T, E>`,
`if`/`while`/`for`/`switch`, `defer`, and `?` propagation. A program can read its input
(`read_line`, `parse_i64`), put a value inside a string — including with
interpolation, `"n = ${n}"` — and manage its own memory (`alloc`, `dealloc`,
`move`). Panics, C interop, collections, concurrency and the standard library are
later milestones; see [Roadmap](#roadmap).

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

```otter
// A struct is a value: built with a literal and read field by field with `.`.
// The layout follows the declaration order, so a literal may spell the fields
// in any order and `p.x` still means the declared `x`.
struct Point {
    x: i64,
    y: i64,
}

fn manhattan(p: Point) -> i64 {
    return p.x + p.y;
}

fn main() {
    const p = Point { y: 4, x: 3 };
    print_i64(manhattan(p));
    print_str("\n");
}
```

```otter
// Heap memory is explicit and linear. `alloc(n)` reserves room for n `i64`s and
// returns a `*mut i64`; `defer` releases it when the scope exits, and the
// checker rejects a second use before code generation ever runs:
//
//     var buf = alloc(16);
//     defer { dealloc(buf); }
//     buf = alloc(32);        // the deferred free still releases the first block
//     dealloc(buf);
//
// Ownership transfers with `move`, so only one name can free a block:
//
//     var buf = alloc(16);
//     dealloc(buf);
//     dealloc(buf);   // error: value `buf` has already been moved
//
// Reading a pointer's elements needs indexing, which arrives with slices in M6;
// until then a pointer can be allocated, moved, and freed.
fn main() {
    var buf = alloc(16);
    defer {
        dealloc(buf);
    }
    print_str("memory ok\n");
}
```

```otter
// Strings are values: `+` concatenates, `len` counts bytes, and `==`/`!=`
// compare contents rather than addresses. `str_from_i64` puts a number inside
// one, so a whole line is a single string.
fn label(name: string) -> string {
    return "[" + name + "]";
}

fn main() {
    const greeting = "Hello, " + "Otter!";
    print_str(greeting + "\n");
    print_str("length = " + str_from_i64(len(greeting)) + "\n");
    print_str(label("x") + "\n");
    if greeting == "Hello, Otter!" {
        print_str("equal\n");
    }
}
```

```otter
// A program can read its input. `for ;;` is the unbounded loop; `read_line`
// returns the empty string at end of input, which the loop's `break` uses as its
// only stopping condition.
fn main() {
    var total = 0;
    for ;; {
        const line = read_line();
        if line == "" {
            break;
        }
        total = total + parse_i64(line);
    }
    print_i64(total);
    print_str("\n");
}
```

## Speed

Same algorithm, same input, every language compiled by its own native
optimizer at `-O2`. Lower is better. Full methodology and caveats in
[`benchmarks/README.md`](benchmarks/README.md).

**`fib(35)` — about 30 million calls** (function call overhead):

| Language | ms | ns/call |
|---|---:|---:|
| C (gcc -O2) | 49 | 1.63 |
| C (clang -O2) | 80 | 2.67 |
| **Otter (clang -O2)** | **81** | **2.72** |
| Rust (rustc -O) | 83 | 2.77 |
| Otter (clang -O0) | 140 | 4.70 |
| Go | 144 | 4.84 |
| Node 24 | 333 | 11.2 |
| CPython 3.14 | 2934 | 98.3 |

**A 200,000,000-iteration integer loop with a branch:**

| Language | ms | ns/iteration |
|---|---:|---:|
| C (gcc -O2) | 240 | 1.20 |
| Rust (rustc -O) | 252 | 1.26 |
| **Otter (clang -O2)** | **253** | **1.27** |
| C (clang -O2) | 254 | 1.27 |
| Go | 270 | 1.35 |
| Node 24 | 616 | 3.08 |
| Otter (clang -O0) | 1476 | 7.38 |
| CPython 3.14 | 50886 | 254 |

**100,000,000 iterations of an `f64` dependency chain:**

| Language | ms | ns/iteration |
|---|---:|---:|
| C (gcc -O2) | 524 | 5.24 |
| **Otter (clang -O2)** | **529** | **5.29** |
| Rust (rustc -O) | 530 | 5.30 |
| C (clang -O2) | 531 | 5.31 |
| Go | 538 | 5.38 |
| Node 24 | 644 | 6.44 |
| Otter (clang -O0) | 888 | 8.88 |
| CPython 3.14 | 12870 | 128.7 |

Laptops are laptops, and these three tables come from one run of one script on
one machine: the ordering of rows that are close together — Otter, `clang` and
Rust on `fib`, for instance — moves between runs. The `ns/iteration` column is
there because it travels better than the millisecond column.

What the tables say:

- **Otter's scalar code is in the same band as C and Rust, and that is the whole
  claim.** On `loop` it is ahead of `clang` and within 6% of `gcc`; on `float` it is
  ahead of both C compilers; on `fib` it is within 2% of `clang` and ahead of Rust. It
  is sometimes ahead of C built by the same `clang` — evidence the IR is clean, not
  evidence that Otter is better than C. It is not a claim that `otter-codegen`
  optimizes anything: it contains no optimization passes, and hands everything to
  LLVM.
- **Two opt-in flags buy most of the remaining speed.** `--cpu=<tier>` spends
  portability for instruction-set width (which is what lets the backend vectorise the
  loop); `--fast-math` lets the backend fuse a floating-point multiply and add into
  one instruction. Both are off by default, because the first can stop a binary
  running on an older CPU and the second changes the arithmetic result.

| Benchmark | portable | `--cpu=x86-64-v3` | Speedup | `+ --fast-math` | Speedup |
|---|---:|---:|---:|---:|---:|
| `fib` | 81 ms | 76 ms | 1.06× | 78 ms | 1.04× |
| `loop` | 253 ms | 62 ms | 4.09× | 62 ms | 4.06× |
| `float` | 529 ms | 527 ms | 1.00× | 312 ms | 1.70× |

  On `fib` and `loop` the tiers barely matter; on `float` the fusion is the whole
  story. `clang` gets that fusion for free at `-march=x86-64-v3` (C's
  `-ffp-contract=on` is the default), which is why the `float` row that matches
  `clang` needs the flag: it is Otter asking for what C does by default.
- **The tiers are graded against the same tier in C.** `--cpu=x86-64-v3` is compared
  with `clang -march=x86-64-v3`, never against a baseline C build that went without.
  At that tier Otter is within a few percent of `clang` on all three rows — `loop`
  62 vs 61 ms, `float` 312 vs 311 ms, `fib` 76 vs 77 ms. `--native` is the last tier,
  tuned to the build machine; its numbers travel worse between machines, which is why
  the portable scale and `x86-64-v3` are the ones quoted.
- **Enabling optimization mattered enormously.** `otter build` used to pass `clang`
  no optimization flag at all, so every Otter binary ran at `-O0`. Fixing that is
  worth **5.8×** on the loop, 1.7× on `fib` and 1.7× on the float chain — the rows
  above labeled `-O0` are what Otter shipped before.

| Benchmark | `-O0` | `-O2` | Speedup |
|---|---:|---:|---:|
| `fib` | 140 ms | 81 ms | 1.7× |
| `loop` | 1476 ms | 253 ms | 5.8× |
| `float` | 888 ms | 529 ms | 1.7× |

**Compile time** (the `fib` program, best of 5):

| Toolchain | ms |
|---|---:|
| Otter (front end only: lex, parse, check, emit IR) | 25 |
| C (`clang -O2`) | 245 |
| Otter (full build) | 256 |
| Go (`go build`) | 262 |
| Rust (`rustc -O`) | 559 |

The front end — everything this repository implements — is 25 ms. The rest of
`otter build` is `clang` doing the same job it does for C, and it is now a *single*
`clang` invocation: compile the IR and link, in one step, with no intermediate
object file. That is why `otter build` sits about 5% above `clang -O2` on the same
program instead of double it. An earlier version ran `clang` twice and compiled the
runtime's C source from scratch on *every* build, which cost 135 ms of the 403 ms
it took then; the runtime object is cached in a content-addressed directory now.
The first build on a machine still pays for filling that cache. See
[`docs/performance-improvements.md`](docs/performance-improvements.md) for the
breakdown.

### How fast can Otter get

Otter's ceiling is C's ceiling, and for a plain reason: the backend *is* `clang`.
Compiling with `--cpu=x86-64-v3 --fast-math` puts Otter within a few percent of
`clang -O2 -march=x86-64-v3` on every benchmark here — the same compiler on the same
IR shape — which is the honest upper bound, not a promise of beating it. Measured
against what each language actually reached in the run above, that ceiling is:

| Row | Otter (`v3`, `--fast-math`) | Language's own best | Otter's ratio |
|---|---:|---:|---:|
| `loop`, vs `gcc -O2` | 62 ms | 240 ms | 3.9× faster |
| `loop`, vs Rust | 62 ms | 252 ms | 4.1× faster |
| `loop`, vs Go | 62 ms | 270 ms | 4.3× faster |
| `loop`, vs Node 24 | 62 ms | 616 ms | 9.9× faster |
| `loop`, vs CPython | 62 ms | 50886 ms | 820× faster |
| `float`, vs the portable C/Rust cluster | 312 ms | ~524 ms | 1.7× faster |

Both flags are opt-in because they trade something away — the tier gives up running
on older CPUs, `--fast-math` gives up exact floating-point results. The portable
default is the row the other languages are compared against in the three tables at
the top, and it is already in C's band.

### What the numbers do not prove

The runtime rows are measured by `benchmarks/run.ps1`, which rebuilds every row
whenever the compiler's output could change. `--cpu` and `--fast-math` are extra
arguments on the `clang` command line and nothing else — they cannot change a
portable build — so the portable rows move only with run-to-run variation, and the
tiered and fused rows are this run's measurement. See
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
| M3 🚧 | Heap, strings and input in progress: `alloc`/`dealloc`, `move`, linear ownership, `defer`, `+` / `len` / `==` on strings, `read_line` / `parse_i64` / `parse_f64`, `str_from_i64` / `str_from_f64` / `str_from_bool`, and string interpolation (`"n = ${n}"`). Still to come: arenas and refcounted (non-leaking) strings; M4 adds `defer` on the unwind path |
| M4 | Panic and unwinding, checked arithmetic, `assert`, exit codes |
| M5 | `extern` C FFI and the runtime, on Windows and Linux |
| M6 | Slices, maps, modules, multi-file builds |
| M7 | Threads, channels, mutexes |
| M8 | `otter fmt`, tutorial, LSP, syntax highlighting |
| M9 | Escape analysis |
| M10 | Self-hosting bootstrap |
