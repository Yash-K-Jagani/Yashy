# Otter

Otter is a general-purpose, statically typed programming language with C-shaped
syntax, compiled to native binaries through LLVM. The compiler is written in
Rust; the generated code is textual LLVM IR that `clang` assembles and links.

There is no runtime library to install and no LLVM build dependency: the print
helpers are a C source file embedded in the compiler binary, and the only
external tool is `clang`.

**Status: milestones M0–M8 and M12–M14 are complete; M15 is in progress.** Otter lexes,
parses, type-checks and compiles programs with `i64`, `f64`, `bool`, `string`,
`struct` (with field access), `enum`, `Option<T>`, `Result<T, E>`,
`if`/`while`/`for`/`switch`, `defer`, and `?` propagation. Bindings can be
written as `var` or `let` (block-scoped and mutable) or `const` (block-scoped
and immutable). Arrays and strings have a method surface: `xs.push(1)`,
`xs.slice(0, 3)`, `s.trim()`, `s.split(",")`, and the rest of the string
method table.
A function name is a value: `fn(i64) -> i64` is a type, a top-level function
can be handed to another function and called through the parameter, and a
binding can hold one and call it. A lambda -- `|x: i64| -> i64 { return x + 1; }`
-- is a function written where a value is expected, and a *closure* is a
lambda that carries the values of the scope it was written in, so it keeps
working after that scope is gone. Three array methods take a function:
`xs.map(f)`, `xs.filter(f)` and `xs.fold(init, f)`. A type can have its own
methods, written in an `impl` block: `impl Point { fn sum(self) -> i64 { ... } }`
is called as `point.sum()`. An `object` is a type with its fields and its
methods together, and a method with no `self` is called on the type:
`Counter.new(3)`. A method that takes `self: *mut T` receives the object's
address rather than a copy, so `self[0].value = ...` writes the caller's
object, and the receiver at the call site has to be a mutable binding. A
function can be generic: `fn identity[T](x: T) -> T` is written once, checked
once for each type the program calls it with, and emitted as an ordinary
function for each, so nothing is dispatched at run time. An array whose length
is part of its type is a fixed array: `[i64; 3]` stores its elements in the
frame, `[1, 2, 3]` is its literal, and indexing is checked like any other array
(`grid[i]` is an `Option`). `grid[0..2]` is a *view*: a length and a pointer
into the array that owns nothing, with clamped bounds, and the checker refuses
to move or change the array until the view is out of scope. A tuple is an
anonymous product: `(i64, i64)` is a type, `(a, b)` is its literal, and
`const (n, rest) = pair;` unpacks one into separate bindings (`_` skips an
element). An arm may carry a guard -- `case Circle(r) if r > 10.0` -- checked
after the pattern has bound its names; a false guard sends the search on to the
next arm exactly as a failed pattern would. An integer literal may be written
with `_` between its digits (`1_000_000`) or in another base (`0xFF`, `0b1010`,
`0o777`), and `"${value:?}"` is the *debug* form of an interpolation: it shows
the surface of the value -- a struct's name and fields, an enum's variant, a
tuple's and an array's elements -- with a string quoted and escaped, so that a
newline inside it stays visible. A program can read its
input (`read_line`, `s.parse_i64()`), put a value inside a string — including with interpolation,
`"n = ${n}"` — manage its own memory (`alloc`, `dealloc`, `move`, arenas, and
pointer indexing), and stop itself with `panic("...")`, which reports the
position it was written at and exits with a failure status. Integer arithmetic
is checked, with `wrapping_*`/`checked_*`/`saturating_*` as the named opt-outs.
The assertions — `assert(cond)`, `assert_eq(a, b)`, `assert_ne(a, b)` — are the
same fault path with a test in front of it, and they quote the expression that
failed back in the message; `debug_assert` is the one `--release` compiles away.
A program can choose its exit status by returning `i64` from `main`, or by
calling `os.exit(code)` from anywhere. Faults and `defer` cleanup run for the
panicking function's scopes; full stack unwinding is the one M4 item the host
toolchain blocks (see [Roadmap](#roadmap)).

Milestones M5–M8 add the rest of the language as designed: `extern "C"`
declarations for C interop, `[]T` arrays and `map[K]V` maps, multi-file builds
with `import`, and threads, mutexes and bounded `Channel[T]`s. Programs can
declare their own structs and enums and construct enum values
(`Shape.Circle(2)`, `Shape.Rect { w: w, h: h }`, `Shape.Point`) before matching
them with `switch`. Tooling ships in the same workspace: `otter fmt` rewrites a
file in the canonical style, and `otter-lsp` is a language server for editors.
The [language tutorial](docs/tutorial/00-intro.md) walks through all of it,
example by example; a test compiles and runs every example in that tutorial, so
it cannot drift either.

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

`otter fmt file.ot` prints the canonical formatting to stdout, and
`otter fmt -w file.ot` writes it back. For a guided tour rather than a
reference, start at [`docs/tutorial/00-intro.md`](docs/tutorial/00-intro.md):
each chapter is a complete program you can build and run.

## A taste of Otter

```otter
// `let` and `var` bindings are mutable, `const` bindings are not. `for i in 0..n`
// is a range loop whose end is exclusive.
fn fib(n: i64) -> i64 {
    let a = 0;
    let b = 1;
    let next = 0;
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
// A struct is a value: built with a literal, read field by field with `.`, and
// written the same way on a `var`. The layout follows the declaration order, so
// a literal may spell the fields in any order and `p.x` still means the declared
// `x`.
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

    // A field write is a read-modify-write of the whole value, so `y` survives
    // it; `q.x += 1` is the same statement with the field read on the right.
    var q = p;
    q.x = 10;
    q.y += 1;
    print_i64(q.x + q.y);
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
// Reading and writing a pointer's elements is `p[i]`, and `alloc(n)` counts
// elements rather than bytes. `defer` releases the block on every exit from the
// scope, including a panic in this same function.
fn main() {
    var buf = alloc(4);
    defer {
        dealloc(buf);
    }
    for i in 0..4 {
        buf[i] = i * 2;
    }
    print_i64(buf[0] + buf[3]);   // 6
    print_str("\n");
}
```

```otter
// An arena is many allocations with one release. `arena_alloc` is the one
// builtin that borrows a linear handle instead of consuming it, so the same
// arena serves every block; `arena_free_all` consumes it, which is why a second
// free is a type error rather than a double free.
fn main() {
    var arena = arena_new();

    var first = arena_alloc(arena, 4);
    var second = arena_alloc(arena, 4);
    first[0] = 20;
    second[0] = 22;

    print_i64(first[0] + second[0]);
    print_str("\n");
    arena_free_all(arena);
}
```

Set `OTTER_LEAK_REPORT=1` to have the runtime print the allocations still live
when the process exits:

```console
$ OTTER_LEAK_REPORT=1 ./buffer
1047552
otter: leak report: 0 live allocations
```

Arena blocks are deliberately not counted: an arena is one allocation with one
release, so counting its bump blocks individually would report a leak for
correct code. The manual buffer and the string builder above both report zero.

```otter
// Strings are values: `+` concatenates, `len()` counts bytes, and `==`/`!=`
// compare contents rather than addresses. `str_from_i64` puts a number inside
// one, so a whole line is a single string. The string methods return new
// strings, so `trim` and `to_upper` leave `greeting` unchanged.
fn label(name: string) -> string {
    return "[" + name + "]";
}

fn main() {
    const greeting = "Hello, " + "Otter!";
    print_str(greeting + "\n");
    print_str("length = " + str_from_i64(greeting.len()) + "\n");
    print_str(label("x") + "\n");
    if greeting == "Hello, Otter!" {
        print_str("equal\n");
    }
    var padded = "  Otter  ";
    print_str(padded.trim() + "\n");
    var csv = "a,b";
    var parts = csv.split(",");
    print_i64(parts.len());
    print_str("\n");
    parts.free();
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
        total = total + line.parse_i64();
    }
    print_i64(total);
    print_str("\n");
}
```

```otter
// `${value}` shows a value the way a reader wants to read it. `${value:?}`
// shows its shape instead: a struct's name and fields, the variant an enum
// holds, the elements of a tuple or an array, and a string in quotes with its
// control characters escaped -- so a newline inside a value stays visible.
struct Point {
    x: i64,
    y: i64,
}

fn main() {
    const point = Point { x: 3, y: 4 };
    print_str("${point:?}\n");
    print_str("${[1, 2, 3]:?}\n");
    print_str("${"a\nb":?}\n");
}
```

```console
$ otter build main.ot -o main && ./main
Point { x: 3, y: 4 }
[1, 2, 3]
"a\nb"
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

```otter
// `map[K]V` is a hash map. String keys are hashed by content, so inserting
// "otter" twice overwrites the value and `len()` still counts one entry.
// A lookup answers in an `Option`, so a missing key is a `None` to handle,
// not a crash.
fn main() {
    var counts: map[string]i64 = map_new();
    counts.insert("otter", 2);
    counts.insert("river", 1);
    counts.insert("otter", 3);
    print_i64(counts.len());
    print_str("\n");

    print_str(switch counts.get("otter") {
        case Some(n) { str_from_i64(n) }
        case None { "missing" }
    });
    print_str("\n");

    counts.remove("river");
    counts.free();
}
```

```console
$ otter build main.ot -o main && ./main
2
3
```

```otter
// A channel is typed and bounded: `chan_new(1)` holds one value, so `send`
// blocks when the channel is full and `recv` blocks when it is empty. The
// worker receives, doubles, and answers; the main thread joins it and frees
// each channel exactly once.
fn worker(inbox: Channel[i64], outbox: Channel[i64]) {
    switch inbox.recv() {
        case Some(n) { outbox.send(n * 2); }
        case None { outbox.send(-1); }
    }
}

fn main() {
    var inbox: Channel[i64] = chan_new(1);
    var outbox: Channel[i64] = chan_new(1);
    var t = thread_spawn(worker, inbox, outbox);
    inbox.send(21);
    switch outbox.recv() {
        case Some(n) { print_i64(n); }
        case None { print_str("closed"); }
    }
    print_str("\n");
    thread_join(t);
    outbox.free();
    inbox.free();
}
```

```console
$ otter build main.ot -o main && ./main
42
```

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

### What the features cost

The tables above measure arithmetic and calls. Six more programs, in
[`benchmarks/features/`](benchmarks/features), measure the features that make a
program rather than a loop: FFI, arrays, maps, enums, channels and mutexes.
Each is compared with the nearest built-in equivalent in another language, not
with an identical source file, and each row must print the same expected
number — a wrong hash or a lost update fails the run instead of becoming a fast
row. Times are milliseconds, lower is better, and the ratio is Otter's time
divided by the comparison's, so below 1.0 means Otter was faster. Published
numbers are 9 runs over 3 passes at `-O2` on the same machine as the tables
above.

| Feature | Otter | Fastest comparison | Otter / comparison |
|---|---:|---:|---:|
| FFI, 100,000,000 `getpid` calls | 248 ms | C (clang) 237 ms | 1.05× |
| Arrays, 10,000,000 appends | 207 ms | C (clang) 141 ms | 1.47× |
| Maps, 1,000,000 inserts + lookups | 322 ms | Rust 282 ms | 1.14× |
| Enums, 50,000,000 builds + matches | 150 ms | C (clang) 101 ms | 1.48× |
| Channels, 200,000 round trips | 179 ms | Go 296 ms | 0.61× |
| Mutexes, 1,000,000 lock pairs | 74 ms | Go 66 ms | 1.12× |

Read these as cost checks, not a scoreboard. Otter's `push` is an out-of-line
runtime call; C's `realloc` loop inlines the common path. Otter's map is an
open-addressed table; Rust's `HashMap` and Go's map use their own layouts.
Otter's threads are OS threads and its channels block on condition variables;
Go's goroutine is multiplexed by the Go scheduler. The full workload sizes,
methodology and caveats are in
[`benchmarks/README.md`](benchmarks/README.md#feature-benchmarks).

### What the numbers do not prove

The runtime rows are measured by `benchmarks/run.ps1`, which rebuilds every row
whenever the compiler's output could change. `--cpu` and `--fast-math` are extra
arguments on the `clang` command line and nothing else — they cannot change a
portable build — so the portable rows move only with run-to-run variation, and the
tiered and fused rows are this run's measurement. The feature rows are measured
by `benchmarks/run-features.ps1` with the same warm-up and minimum rule; that
script refuses a row whose output does not match the number every language is
expected to print. See
[`docs/performance-improvements.md`](docs/performance-improvements.md) for the
round-by-round record, including the changes that were measured and *not* made.

Checked integer arithmetic has landed since these tables were first written, and
the prediction that the `loop` row would get slower was half right: at the default
tier LLVM deletes the checks and the row did not move, while at `x86-64-v3` it did,
by 2.5×, because the check is what stops the loop from vectorising. See [what the
check costs](#what-the-check-costs). Either way the cost is now measured rather
than a surprise, which is what the numbers were for.

These benchmarks exercise scalar arithmetic and function calls; they do not
exercise heap allocation, collections, strings or threads. Those have their own
suite under [Feature benchmarks](benchmarks/README.md#feature-benchmarks),
which measures the cost of each feature against a comparable built-in
equivalent. It is a cost check per feature, not a second scalar comparison.
The scalar programs are also a fixed algorithm in each language, which is what
makes them comparable and what makes them narrow: `fib` measures call overhead
and little else. Measured on one machine: ratios should be stable, absolute
times will not be.

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
| `otter-runtime` | the C shim: print helpers, allocation, arenas, refcounted strings, array storage, hash maps, threads/channels/mutexes, and fault diagnostics |
| `otter-stdlib` | Otter-source library modules: `alloc`, `io`, `math`, `os`, `string` |
| `otter-fmt` | the canonical formatter behind `otter fmt` |
| `otter-lsp` | the language server: diagnostics, hover, go-to-definition, and semantic tokens |

The compiler never panics on malformed input; that is an acceptance criterion,
not an aspiration, and it has its own test suite
(`crates/otter-driver/tests/never_panics.rs`) that feeds it malformed programs
and asserts a diagnostic comes back instead of a Rust backtrace.

## Documentation

- [`docs/tutorial/00-intro.md`](docs/tutorial/00-intro.md) — the language tutorial, in eleven chapters
- [`docs/superpowers/specs/2026-10-16-otter-language-design.md`](docs/superpowers/specs/2026-10-16-otter-language-design.md) — the language design
- [`docs/superpowers/plans/`](docs/superpowers/plans) — the milestone plans
- [`benchmarks/README.md`](benchmarks/README.md) — benchmark methodology and results
- [`docs/performance-improvements.md`](docs/performance-improvements.md) — where the remaining performance work is

## Roadmap

| Milestone | Deliverable |
|---|---|
| M0–M2 ✅ | Lexer, parser, sema, codegen, CLI: a usable little language |
| M3 ✅ | Heap, strings and input: `alloc`/`dealloc`, `move`, linear ownership, pointer indexing (`p[i]`), arenas (`arena_new` / `arena_alloc` / `arena_free_all`), `defer`, and string methods `len` / `byte_at` / `slice` / `parse_i64` / `parse_f64` plus `+` / `==`, `read_line`, `str_from_*`, and interpolation. A `string` is a length-prefixed refcounted `%otter_str`: a literal is static (refcount -1), everything built is owned, a temporary is released when an operation consumes it, and a binding's reference is released at the end of its scope after that scope's `defer`s. Known leaks, not corruptions: a string a user call returns, one stored in a struct field, or one dropped across a `break` out of a nested block |
| M4 ✅ | Faults: `panic` reports `file:line:col` and exits 1, and `!` lets it stand in for a value. Integer arithmetic is checked — overflow, division by zero and `i64::MIN / -1` name the operands and the position — with `wrapping_*`, `checked_*` and `saturating_*` as the opt-outs. `assert` / `assert_eq` / `assert_ne` quote the failed expression (and both values); `debug_assert` is the one `--release` drops. Exit codes: `i64`-returning `main`, and `os.exit(code)`. A `defer` in the panicking function's open scopes runs before the diagnostic, innermost first. **Not delivered:** cross-frame unwinding (a caller's `defer`, and stack traces) — cleanup landing pads need `__gxx_personality_v0`, which is undefined at link on this `x86_64-pc-windows-gnu` host, and shipping an unwinder would add a mandatory C++ dependency. See the M3–M4 plan's Task 10 |
| M5 ✅ | `extern "C"` FFI: C type spellings, `unsafe` declarations, `link` libraries, and calls into libc. The C runtime (print, allocation, arenas, refcounted strings, array storage, maps, threads, fault diagnostics) is embedded in the compiler and built into every program on Windows and Linux |
| M6 ✅ | Arrays (`[]T`, `push`/`pop`/`len`/indexing, `free`), hash maps (`map_new` plus `.insert`/`.get`/`.remove`/`.len`/`.free`), `import` for multi-file builds, and the `io`, `string`, `math`, `os`, `alloc` modules |
| M7 ✅ | Threads (`thread_spawn`/`thread_join`), linear mutexes with `.lock`/`.unlock`/`.free`, and bounded `Channel[T]` channels with blocking `.send`/`.recv`, `.close` to unblock both sides, and copyable elements |
| M8 ✅ | `otter fmt`, the eleven-chapter tutorial (every example compiled and run by a test), an LSP server, and editor modes for VS Code, Vim, and Emacs |
| M9 | Escape analysis |
| M10 | Self-hosting bootstrap |
| M11 | Adoption: formatter polish, test runner, LSP, installer |
| M12 ✅ | Language surface: JavaScript-style `let`, array and string methods, and one public method spelling for arrays, strings, maps, channels, and mutexes |
| M13 ✅ | Function values: `fn(T) -> R` types, top-level function values, indirect calls, non-capturing lambdas, capturing closures with refcounted environments, the `map`/`filter`/`fold` array methods, `impl` methods, `object` declarations, static methods, and generic functions by monomorphization |
| M14 ✅ | Objects: field-and-method declarations, static methods and constructors, and pointer receivers (`self: *mut T`) that mutate the object they are called on |
| M15 (in progress) | Correctness gaps: `?` enforcement, returns, short-circuiting, struct equality, array moves, owning struct fields, fixed arrays `[T; N]` with borrowed `a[start..end]` views, tuples with destructuring, and pattern guards (arrays, views, tuples and guards done) |
