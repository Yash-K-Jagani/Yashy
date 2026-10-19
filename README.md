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
interpolation, `"n = ${n}"` — manage its own memory (`alloc`, `dealloc`, `move`),
and stop itself with `panic("...")`, which reports the position it was written at
and exits with a failure status. The assertions — `assert(cond)`,
`assert_eq(a, b)`, `assert_ne(a, b)` — are the same fault path with a test in
front of it, and they quote the expression that failed back in the message;
`debug_assert` is the one `--release` compiles away. A program can choose its
exit status by returning `i64` from `main`, or by calling `os.exit(code)` from
anywhere.
C interop, collections, concurrency and the
standard library are later milestones; see [Roadmap](#roadmap).

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
`--release` is a separate question from optimization: it is the flag that makes
this a build a user runs, and the one build-mode decision the compiler makes —
`debug_assert` is compiled out. A `main` returning `i64` is the process exit
status, and `os.exit(code)` ends the program with a chosen status from anywhere.

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

```otter
// Integer arithmetic is checked. `big + 1` stops the program with
// `panic: integer overflow: 9223372036854775807 + 1` rather than continuing with
// a wrapped number that looks exactly like an answer -- a language with no
// exceptions has nowhere else to put the error. Code that wants the other
// behaviour asks for it by name, and the three ways of asking are different
// questions with three different answers.
fn main() {
    var big = 9223372036854775807;      // the largest i64
    print_i64(wrapping_add(big, 1));    // wraps:  -9223372036854775808
    print_str("\n");
    print_i64(saturating_add(big, 1));  // clamps:  9223372036854775807
    print_str("\n");
    switch checked_add(big, 1) {        // reports: the caller decides
        case Some(v) { print_i64(v) }
        case None { print_str("no answer") }
    }
    print_str("\n");
}
```

The same family exists for `-` and `*` (`wrapping_sub`, `checked_mul`,
`saturating_mul`, ...). There is deliberately no `wrapping_div`: a wrapped
quotient is not the quotient of anything, and the two ways a division has no
answer -- a zero divisor, and `i64::MIN / -1` -- are reported either way.

```otter
// An assertion is the same fault path with a test in front of it. The message
// quotes the condition that was written, and the comparison forms print both
// values, so a failure says what was expected and not merely that something was.
fn main() {
    const n = 3;
    assert(n > 0);                              // passes, prints nothing
    assert_eq(n, 4);                            // this one stops the program
}
```

```console
$ otter build main.ot -o main && ./main
main.ot:4:5: panic: assertion failed: n == 4, left = 3, right = 4
```

`debug_assert` is the same assertion, compiled out by `--release`: a claim worth
checking while writing the program and worth nothing in front of a user.

## Speed

Same algorithm, same input, every language compiled by its own native
optimizer at `-O2`. Lower is better. Full methodology and caveats in
[`benchmarks/README.md`](benchmarks/README.md).

One row needs explaining before the numbers mean anything. Otter's arithmetic is
**checked**, and C's, Rust's, Go's, Node's and Python's is not: `a + b` in Otter
stops the program if it overflows. Rows labeled ``Otter (-O2, `wrapping_*`)`` are
the same program with the opt-out (§ Checks in [`README`](#arithmetic-is-checked)),
and they are the strictly like-for-like rows; the plain Otter rows promise more
and sometimes pay for it.

**`fib(35)` — about 30 million calls** (function call overhead):

| Language | ms | ns/call |
|---|---:|---:|
| C (gcc -O2) | 49 | 1.63 |
| C (clang -O2) | 82 | 2.75 |
| **Otter (-O2, `wrapping_*`)** | **82** | **2.75** |
| Rust (rustc -O) | 87 | 2.91 |
| Go (go build) | 140 | 4.67 |
| **Otter (clang -O2)** | **149** | **4.99** |
| Otter (clang -O0) | 176 | 5.91 |
| Node 24 | 362 | 12.1 |
| CPython 3.14 | 3418 | 114 |

**A 200,000,000-iteration integer loop with a branch:**

| Language | ms | ns/iteration |
|---|---:|---:|
| **Otter (clang -O2)** | **241** | **1.20** |
| C (gcc -O2) | 243 | 1.21 |
| Rust (rustc -O) | 249 | 1.25 |
| **Otter (-O2, `wrapping_*`)** | **251** | **1.25** |
| C (clang -O2) | 267 | 1.33 |
| Go (go build) | 293 | 1.47 |
| Node 24 | 688 | 3.44 |
| Otter (clang -O0) | 1717 | 8.58 |
| CPython 3.14 | 26476 | 132 |

**100,000,000 iterations of an `f64` dependency chain:**

| Language | ms | ns/iteration |
|---|---:|---:|
| **Otter (clang -O2)** | **535** | **5.35** |
| C (gcc -O2) | 537 | 5.37 |
| Rust (rustc -O) | 542 | 5.42 |
| C (clang -O2) | 546 | 5.46 |
| Go (go build) | 549 | 5.49 |
| Node 24 | 684 | 6.84 |
| Otter (clang -O0) | 827 | 8.27 |
| CPython 3.14 | 9421 | 94.2 |

Laptops are laptops, and these three tables come from two runs of one script on
one machine, each row reporting the best of the ten runs. The ordering of rows
that are close together — Otter, `clang` and Rust on `loop`, for instance — moves
between runs. The `ns/iteration` column is there because it travels better than
the millisecond column, and so does a ratio measured within one pass: this
machine's clock varies by up to 3× between windows (a background scan or a
thermal state; the cause was not identified), which moves every row of a table
together and moves nothing between them.

What the tables say:

- **Otter's scalar code is in the same band as C and Rust, and that is the whole
  claim.** On `loop` it is ahead of `clang` and level with `gcc`; on `float` it is
  ahead of both C compilers; on `fib` with the arithmetic opt-out it is within a
  percent of `clang` and ahead of Rust. It is sometimes ahead of C built by the
  same `clang` — evidence the IR is clean, not evidence that Otter is better than
  C. It is not a claim that `otter-codegen` optimizes anything: it contains no
  optimization passes, and hands everything to LLVM.
- **Checked arithmetic is free where the backend can see the answer and 1.8× on
  `fib` where it cannot**, which is the one place Otter's default is behind Go.
  See [what the check costs](#what-the-check-costs) below.
- **Two opt-in flags buy most of the remaining speed.** `--cpu=<tier>` spends
  portability for instruction-set width (which is what lets the backend vectorise the
  loop); `--fast-math` lets the backend fuse a floating-point multiply and add into
  one instruction. Both are off by default, because the first can stop a binary
  running on an older CPU and the second changes the arithmetic result.

| Benchmark | portable | `--cpu=x86-64-v3` | Speedup | `+ --fast-math` | Speedup |
|---|---:|---:|---:|---:|---:|
| `fib` | 149 ms | 146 ms | 1.02× | 143 ms | 1.04× |
| `loop` | 241 ms | 184 ms | 1.31× | 169 ms | 1.43× |
| `float` | 535 ms | 543 ms | 0.99× | 320 ms | 1.67× |

  `fib` gets nothing from the tier — it is calls, not instruction width. `float`
  gets the fusion, and that is the whole story there: `clang` gets it for free at
  `-march=x86-64-v3` (C's `-ffp-contract=on` is the default), which is why the
  `float` row that matches `clang` needs the flag — it is Otter asking for what C
  does by default. `loop` is the one that changed: the tier buys 1.31× where it
  used to buy 4.1×, because the check is what stops the loop from vectorising.
  With `wrapping_*` the same build reaches 69 ms. That is [what the check
  costs](#what-the-check-costs), and this row is the strongest argument for the
  opt-out.
- **The tiers are graded against the same tier in C.** `--cpu=x86-64-v3` is compared
  with `clang -march=x86-64-v3`, never against a baseline C build that went without.
  At that tier the like-for-like rows are within a percent of `clang` — `float` 320
  vs 322 ms, and `loop` with `wrapping_*` 69 vs 68 ms in one pass — while Otter's
  default trails on `fib` (146 vs 80 ms) and on `loop` (184 vs 64 ms), both because
  of the same check. `--native` is the last tier, tuned to the build machine; its
  numbers travel worse between machines, which is why the portable scale and
  `x86-64-v3` are the ones quoted.
- **Enabling optimization mattered enormously.** `otter build` used to pass `clang`
  no optimization flag at all, so every Otter binary ran at `-O0`. Fixing that is
  worth **7.1×** on the loop, 1.2× on `fib` and 1.5× on the float chain — the rows
  above labeled `-O0` are what Otter shipped before.

| Benchmark | `-O0` | `-O2` | Speedup |
|---|---:|---:|---:|
| `fib` | 176 ms | 149 ms | 1.2× |
| `loop` | 1717 ms | 241 ms | 7.1× |
| `float` | 827 ms | 535 ms | 1.5× |

**Compile time** (the `fib` program, best of 5):

| Toolchain | ms |
|---|---:|
| Otter (front end only: lex, parse, check, emit IR) | 22 |
| C (`clang -O2`) | 252 |
| Otter (full build) | 259 |
| Go (`go build`) | 281 |
| Rust (`rustc -O`) | 602 |

The front end — everything this repository implements — is 22 ms. The rest of
`otter build` is `clang` doing the same job it does for C, and it is now a *single*
`clang` invocation: compile the IR and link, in one step, with no intermediate
object file. That is why `otter build` sits about 3% above `clang -O2` on the same
program instead of double it. An earlier version ran `clang` twice and compiled the
runtime's C source from scratch on *every* build, which cost 135 ms of the 403 ms
it took then; the runtime object is cached in a content-addressed directory now.
The first build on a machine still pays for filling that cache. See
[`docs/performance-improvements.md`](docs/performance-improvements.md) for the
breakdown.

### What the check costs

The arithmetic is checked, so a program that would overflow stops with a message
naming the operands instead of continuing with a wrapped number. That promise has
a price, and it is not the same price everywhere: it is free where the backend can
prove the operation cannot overflow, and charged per operation where it cannot.
Every number below is one interleaved pass — thermal effects cancel within a pass,
which they do not across passes — minimum of seven runs, `-O2`.

| Benchmark | checked | `wrapping_*` | C (`clang -O2`) | Cost |
|---|---:|---:|---:|---:|
| `fib(35)` | 158 ms | 84 ms | 84 ms | 1.9× |
| `loop`, portable | 261 ms | 262 ms | 273 ms | 1.0× |
| `loop`, `--cpu=x86-64-v3` | 171 ms | 69 ms | 68 ms | 2.5× |
| `float` | 535 ms | — | 546 ms | 1.0× |

- **`float` costs nothing**, because there is no integer arithmetic in it: only
  integer operations carry a check.
- **`loop` costs nothing at the default tier.** Its induction variable and its sum
  are provably within range, so LLVM deletes every check: 261 ms checked against
  262 ms opted out, and both are ahead of `clang`'s 273 ms.
- **`fib` costs 1.8–1.9×**: three checks inside a function called thirty million
  times. This is the one row where Otter's default falls behind Go, and the one
  where the opt-out matters most.
- **At `--cpu=x86-64-v3` the check costs 2.5× on `loop`, for a specific reason: it
  stops the loop from vectorising.** The baseline loop is scalar and the checks are
  proved away. The `v3` build's loop would vectorise, and a potentially-faulting
  branch in the body is not something LLVM's vectoriser will speculate. With
  `wrapping_*` the same build vectorises to exactly C's code: 69 ms against 68 ms
  in the same pass.

So the opt-out is not a claim that checking was a mistake; it is how a program
says *this one cannot overflow* where the backend cannot see that for itself. It
recovers all of the cost every time, which is why it shipped in the same commit as
the check rather than in a later one. Making the checked form itself vectorisable
is an open optimisation, recorded in
[`docs/performance-improvements.md`](docs/performance-improvements.md).

### How fast can Otter get

Otter's ceiling is C's ceiling, and for a plain reason: the backend *is* `clang`.
Compiling with `--cpu=x86-64-v3 --fast-math` puts Otter within a few percent of
`clang -O2 -march=x86-64-v3` on every benchmark here — the same compiler on the same
IR shape — which is the honest upper bound, not a promise of beating it. Measured
against what each language actually reached in the run above, that ceiling is:

| Row | Otter's best | Language's own best | Otter's ratio |
|---|---:|---:|---:|
| `loop`, vs `gcc -O2` | 69 ms (`v3`, `wrapping_*`) | 243 ms | 3.5× faster |
| `loop`, vs Rust | 69 ms | 249 ms | 3.6× faster |
| `loop`, vs Go | 69 ms | 293 ms | 4.2× faster |
| `loop`, vs Node 24 | 69 ms | 688 ms | 10× faster |
| `loop`, vs CPython | 69 ms | 26476 ms | 380× faster |
| `float`, vs the portable C/Rust cluster | 320 ms (`v3`, `--fast-math`) | 535 ms | 1.7× faster |

The `loop` rows are with `wrapping_*`, because that is what the program should say:
its arithmetic provably cannot overflow, and the opt-out is what lets the loop
vectorise. (`--fast-math` does nothing for an integer loop.) Left checked, the same
build is 169 ms and the ratio against `gcc` is 1.4×. `float` needs no opt-out —
there is no integer arithmetic in it — so its ceiling is available by default.

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

Checked integer arithmetic has landed since these tables were first written, and
the prediction that the `loop` row would get slower was half right: at the default
tier LLVM deletes the checks and the row did not move, while at `x86-64-v3` it did,
by 2.5×, because the check is what stops the loop from vectorising. See [what the
check costs](#what-the-check-costs). Either way the cost is now measured rather
than a surprise, which is what the numbers were for.

These benchmarks exercise scalar arithmetic and function calls; they exercise
none of the costs a real program pays — no heap, no collections, no string
operations — so they are a lower bound on the work to come, not a preview of it.
They are also a fixed algorithm in each language, which is what makes them
comparable and what makes them narrow: `fib` measures call overhead and little
else. Measured on one machine: ratios should be stable, absolute times will not
be.

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
| M3 🚧 | Heap, strings and input in progress: `alloc`/`dealloc`, `move`, linear ownership, `defer`, `+` / `len` / `==` on strings, `read_line` / `parse_i64` / `parse_f64`, `str_from_i64` / `str_from_f64` / `str_from_bool`, and string interpolation (`"n = ${n}"`). A `string` is now a length-prefixed refcounted value (`%otter_str`, `{ refcount, len, cap, data }`): a literal is a static string with refcount -1, and everything built — concatenation, a conversion, a line of input — comes back owned. Both halves of the release are in: a temporary is dropped the moment an operation has consumed it, and a string a binding holds is kept with `otter_str_retain` and dropped at the end of the scope that owns it — after that scope's `defer`s have run. Still to come: a string a *user call returns*, one stored in a struct field, or one dropped across a `break` out of a nested block is leaked rather than released; M4 adds `defer` on the unwind path |
| M4 🚧 | Faults: `panic("...")` is in — it reports `file:line:col` and exits with a failure status, and `!` (the type of an expression that produces no value) lets it stand in for a `return` or a `switch` arm. Integer arithmetic is checked too: overflow, division by zero and `i64::MIN / -1` report both operands and the position instead of wrapping, with `wrapping_*`, `checked_*` and `saturating_*` as the named opt-outs. The assertions are in: `assert(cond)`, `assert_eq(a, b)` and `assert_ne(a, b)` quote the expression that failed and, for the comparison forms, both values; `debug_assert` is the one `--release` compiles out. Exit codes are in: `main` returning `i64` is the process status (the `i32` the specification names is Otter's `i64`, truncated to the C `int`), and `os.exit(code)` ends the program with a chosen status from anywhere. `defer` now runs on the panic path for the panicking function's scopes (innermost first), before the process ends. Still to come: full stack unwinding (callers' defers and stack traces), which needs landing pads and `invoke` |
| M5 | `extern` C FFI and the runtime, on Windows and Linux |
| M6 | Slices, maps, modules, multi-file builds |
| M7 | Threads, channels, mutexes |
| M8 | `otter fmt`, tutorial, LSP, syntax highlighting |
| M9 | Escape analysis |
| M10 | Self-hosting bootstrap |
