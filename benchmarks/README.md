# Yashy benchmarks

Two suites live here:

- **Scalar** (`run.ps1`) — `fib`, `loop` and `float`, written in six languages,
  answering **how fast is the code Yashy produces** and **how fast does Yashy
  compile it**.
- **Features** (`run-features.ps1`) — FFI, arrays, maps, enums, channels,
  mutexes, characters and printing, answering **what each language feature
  costs** against the same idea in a language that has it built in.

Everything is reproducible from the repository root after
`cargo build --release --workspace`:

```powershell
pwsh benchmarks/run.ps1            # the scalar suite
pwsh benchmarks/run-features.ps1   # the feature suite
```

## The programs

| Program | Source | Measures |
|---|---|---|
| `fib` | recursive `fib(35)` | function calls, call overhead, how well recursion is optimised |
| `loop` | 200,000,000 iterations of an integer loop with a data-dependent branch | loop code, integer arithmetic, branch handling |
| `float` | 100,000,000 iterations of `x = x * a + b` | `float` arithmetic on a serial dependency chain |
| `fib-wrap`, `loop-wrap` | the two integer programs again, with `wrapping_*` in place of bare `+`, `-`, `*` | what Yashy's checked arithmetic costs, and the row that is strictly comparable with C, Rust, Go, Node and Python — all of whose arithmetic wraps |

Yashy's arithmetic is checked, so `fib.ya` is not quite the same program as
`fib.c`: it stops with a message if an addition overflows, where C carries on
with a wrapped value. The `-wrap` variants withdraw that promise using the
language's own opt-out, and they are what a like-for-like comparison runs on.
Both variants print the same number here — none of these inputs can overflow — so
the difference between them is entirely in the instructions emitted, which is
what makes the pair a measurement of the check rather than of the workload.

`float` has no `-wrap` variant because it contains no integer arithmetic: only
integer operations carry a check.

Each is deliberately small enough that the *whole* generated program fits in a
few dozen instructions. That is the point: any difference between two rows is a
difference in the code the compiler emitted, not in library code.

`fib` cannot be flattened by an optimiser. `loop`'s accumulator is a sum with a
branch, which resists collapsing to a closed form while keeping every
intermediate value well inside `int` — no language's overflow rule changes the
answer, and C's signed-overflow undefined behaviour never fires. `float` is a
strict dependency chain, so no compiler may reassociate or vectorise it without
a wide target — and every row built for the portable baseline computes
bit-identical results. The rows that do not are those built for a wider one:
`clang -march=x86-64-v3` fuses `x * a + b` into a single FMA, and Yashy does the
same only under `--fast-math`. That turns two roundings into one and moves the
last six digits. Those rows are labelled, and they are discussed below.

### The feature programs

These run under `run-features.ps1` and are not part of the scalar table. Each
one exercises one language feature end to end; the rows are the languages that
have a comparable standard-library primitive.

| Program | Workload | Compared with |
|---|---|---|
| `ffi` | 100,000,000 calls to libc's `getpid` | C, clang and gcc |
| `array` | 10,000,000 appends to a growable array, a sum, then one query/edit/search/extend/copy and string-method pass | C (`realloc`), Go `append`, Rust `Vec` |
| `map` | 1,000,000 `int` inserts, then 1,000,000 lookups | Go's built-in map, Rust's `HashMap` |
| `map_array` | 10,000,000 elements mapped into a second array, then summed | C, Go and Rust loops that build the second array |
| `map_scan` | a 1,000,000-entry map, its keys and values copied into two arrays ten times over, then 2,000,000 `contains_key` probes | Go and Rust loops that collect the table into two slices or vectors |
| `debug_format` | 1,000,000 debug-formatted points (`${value:?}`), and the total length of the text built | Rust's derived `Debug` |
| `char` | 1 MiB of text scanned one byte at a time with `byte_at`, each byte compared with `'a'` and `'z'`, and the lowercase count printed | none: other languages compare bytes as integers, which is not the `char` type being measured |
| `print` | 200,000 `println` lines, with stdout redirected to a file and the byte count checked (1,288,890) | none: the other languages' print APIs do different work |
| `method` | 10,000,000 calls to a method that takes `self: *mut T` and increments one field | C struct and pointer, Go pointer receiver, Rust `&mut self` |
| `generics` | 100,000,000 calls to a generic function, and the same loop with the type written out | C (the specialized function), Go generics, Rust generics |
| `fixed_array` | 50,000,000 traversals of an 8-element fixed array (400,000,000 element adds), once with checked `+` and once with `wrapping_add` | C stack array (clang and gcc), Go array, Rust array |
| `tuple` | 50,000,000 calls returning a `(int, int)`, each destructured by the caller and both elements folded into the accumulator | C struct returned by value (clang and gcc), Go multiple returns, Rust tuple |
| `enum` | 50,000,000 three-variant enum constructions and matches | C tagged struct |
| `threads` | 200,000 channel round trips between two threads | Go channels |
| `mutex` | 1,000,000 contended lock/unlock pairs across four threads | Go `sync.Mutex` |
| `function` | 100,000,000 calls through a function value, and the same 100,000,000 calls made directly | C function pointer, Go func value, Rust fn pointer |

The `function` workload is the only one with two rows per language. It is
there because "what does a function value cost" is a question about the
difference between two calls, not about one number: every language has a
direct call, and the interesting measurement is what the *value* adds to it.
Each language therefore contributes a `value` row and a `direct` row, and the
gap between them is the price of the function value. Yashy contributes two
more: a `lambda` row, which calls through a lambda, and a `closure` row,
which calls through a closure that captured a value.

The `value` rows keep the call indirect on purpose, because a call the
optimizer can see through is not a call through a value:

- Yashy marks a function that *receives* a function value `noinline`, so the
  call it makes through that value stays indirect. This is the same reason
  WebAssembly keeps `call_indirect` indirect.
- C hides the pointer behind an empty `asm` barrier.
- Rust uses `black_box`.
- Go relies on `//go:noinline`: the Go compiler does not promote an indirect
  call to a direct one.

Without those, every row would measure a direct call and the workload would
answer a question nobody asked.

There is no C row for `map`: C has no hash map in its standard library, and a
hand-written one would measure the author of that C file rather than Yashy.
There is likewise no C row for `threads` and `mutex`: the portable C answer is
pthreads on Linux and the Win32 API on Windows, so a single source would not be
the same program on both. Go is the comparison there, with the caveat that a
goroutine is not an OS thread.

## What the comparison does and does not mean

It means: **for scalar arithmetic and function calls, Yashy's output is
competitive with what C, Rust and Go produce.** That is the claim being tested,
and the numbers support or refute it.

It does not mean "Yashy is as fast as C". Read the caveats:

- **Yashy's binary is clang's output.** `yashy-codegen` emits unoptimized
  textual LLVM IR and `clang -O2` optimizes it. So the honest comparison is "the
  IR Yashy emits vs. the IR clang and rustc emit, given the same optimizer",
  which is a real and useful measurement, but it is not a claim that
  `yashy-codegen` contains optimization passes — it contains none. Yashy's
  advantage, where it has one, is that it hands LLVM clean, fully-typed IR.
- **The scalar suite does not allocate; the feature suite does.** `fib`, `loop`
  and `float` still measure arithmetic and calls only, so that table remains a
  lower bound rather than a preview of a real program. Arrays, maps, strings and
  threads are measured separately under [Feature
  benchmarks](#feature-benchmarks), where allocation *is* the workload. The two
  tables answer different questions and are not comparable row for row.
- **Yashy's arithmetic is checked, and the others' is not.** `a + b` in Yashy
  reports an integer overflow at the position it happened and ends the program; in
  C, Rust, Go, Node and Python it wraps. That is a difference in what the program
  *promises*, not in how well it is compiled, so the benchmark set carries
  `fib-wrap` and `loop-wrap`: the same source with the opt-out
  (`wrapping_add`/`_sub`/`_mul`) in place of `+ - *`. Those are the rows to read
  against the other languages, and the gap between a benchmark's two Yashy rows is
  what the check costs there: nothing at all on `loop` at the default tier, 1.9× on
  `fib`, and 2.5× on `loop` at `--cpu=x86-64-v3`, where the check is what stops the
  loop from vectorising. The README's [what the check
  costs](../README.md#what-the-check-costs) section has the measurements.
- **The same source, not the same program.** `loop` in JavaScript runs on
  doubles because JavaScript has no 64-bit integer. Its printed total differs
  from the others above 2^53. The timing is comparable, the arithmetic is not.
- **One machine, no PGO, no fast-math.** All numbers come from one laptop, with
  no profile-guided optimization and no relaxed floating-point flags. The
  `-march=native` rows are the single exception to the portable target, and they
  are labelled wherever they appear. Absolute times will differ on your hardware;
  the ratios should be stable.

## Methodology

- **Same algorithm, same input** for every language, so a row is a difference in
  the compiler and not in the workload.
- **Native rows compile at `-O2`** (`rustc -O`, which is equivalent), each with
  its own native compiler. `clang` is given the same `--target=` triple Yashy
  uses, read back out of Yashy's own IR — on Windows a bare `clang` defaults to
  the MSVC triple and cannot find `stdio.h` on a machine without the MSVC CRT.
- **Two C compilers** (`clang` and `gcc`), so the Yashy numbers are not graded
  against one backend's quirks.
- **Yashy is built twice**, at `-O0` and at `-O2`, so the table shows what
  enabling optimization actually buys instead of asserting that it helps. The
  integer benchmarks are built a third time from `<name>-wrap.ya`, which is what
  the checked-arithmetic comparison uses.
- **Wider-target rows are opt-in and always paired.** `--cpu=x86-64-v3` and
  `--native` each build a Yashy row *and* the matching `clang` row
  (`-march=x86-64-v3`, `-march=native`), so the table shows what the *flag* buys
  rather than what Yashy buys over a compiler that was not given it. They are
  never the headline; the portable rows are. The `--fast-math` Yashy row is
  paired against the `clang -march=x86-64-v3` row, because clang contracts
  floating-point by default and that is the C row doing the same arithmetic.
- **One untimed warm-up run per program**, then the reported number is the
  **minimum** of N runs. For a short-lived process the minimum is the run least
  disturbed by everything else on the machine, and it is far more stable than
  the mean. The median is printed beside it as a check on how noisy the run was.
  A large min/median gap means the machine was busy and the row should be
  re-measured.
- **Each result table is two passes of the script**, and each row reports the
  better of the two minima. This is not belt and braces: the machine used for
  these numbers has a clock that varies by up to 3× between windows (a background
  scan or a thermal state — the cause was never identified), and a *single* pass
  carries that state into every row of the benchmark it was measuring last. Two
  passes at different times give every row a chance to be measured in the good
  state. What does not survive this is the absolute scale: only ratios measured
  inside one pass are meaningful, and the tables say so. If a row is missing from
  one pass — Device Guard refused a binary — the other pass supplies it.
- **Interpreted rows get fewer samples** (3 vs 5). They cost seconds each and
  their variance is negligible next to the interpreter/native gap.
- **`ns/op` is reported** as well as milliseconds, where one op is one
  iteration (`loop`, `float`) or one call into the recursion tree (`fib`), so
  rows can be compared even if the inputs ever change.
- **Timing is process wall-clock**, which includes process start-up. For these
  programs the work dominates start-up several hundredfold; it does not matter
  for the `loop` and `float` rows at all.
- **A run that produced no output is not a run.** On a machine with an
  Application Control (Device Guard) policy, a freshly linked binary can be
  refused outright — *"was blocked by your organization's Device Guard policy"*.
  That message goes to the host's error stream, not the program's, so a harness
  that only collects stdout would time a refusal and record it as a suspiciously
  fast row. Check that the output is non-empty (and correct) before accepting a
  measurement, and rebuild if one binary is refused: the linker stamps a fresh
  timestamp into every build, so a rebuild is a different hash and usually a
  different verdict.

## Compile time

`fib` is used, and the `clang`/`rustc`/`go` rows are their full build, so the
comparison is end-to-end invocations rather than self-reported phase times.

The interesting row is **Yashy (front end only)**: `yashy build --emit-ir` runs
lexing, parsing, type checking and IR emission, and stops before any LLVM
process starts. That is the part of the pipeline this repository actually
implements — and it is a small fraction of the full build, because the rest is
clang doing for Yashy exactly what it does for C.

### A `const fn` call is free at run time

`benchmarks/const-fn.ya` asks the compiler for two tables: one hundred
squares, summed by a recursive `const fn`, and the tenth Fibonacci number.
`benchmarks/const-fn-values.ya` is the same program with the answers written
out. The compiler folds the calls, so the first program carries the numbers
and never calls anything at run time.

| Measurement | `const-fn.ya` | `const-fn-values.ya` |
|---|---:|---:|
| Front end (`yashy build --emit-ir`), best of 9 | 19.7 ms | 20.0 ms |
| Run (whole process), best of 9 | 15.7 ms | 15.4 ms |

Both rows are the same within the noise, and that is the result: the folded
call costs nothing at run time, and the hundred calls the compiler made while
folding are not visible in a measurement whose floor is the process start-up.
A `const fn` is a compile-time feature, so the honest measurement of it is the
front end — and here even that is below the noise, because the evaluator's
hundred calls take less time than the process takes to start.

The numeric literal forms are the same case and a stronger one:
`1_000_000`, `0xF4240` and `1000000` are one constant, so the three spellings
produce the same code and the same number, and there is nothing to time. The
lexer reads the digits and the prefix; everything downstream sees the value.

## Reproducing

```
cargo build --release --workspace
pwsh benchmarks/run.ps1                 # default: 5 native runs, 3 interpreted
pwsh benchmarks/run.ps1 -Runs 9         # more samples
pwsh benchmarks/run.ps1 -SkipBuild      # re-time existing binaries

pwsh benchmarks/run-features.ps1         # feature suite, 5 runs x 2 passes
pwsh benchmarks/run-features.ps1 -Runs 9 # more samples
pwsh benchmarks/run-features.ps1 -SkipBuild
```

Tested with clang 23.1.3, gcc 16.2.0 (MSYS2 UCRT64), rustc 1.99.1, go 1.27.0,
Node 24.20.0 and CPython 3.14.7 on Windows x86-64.

## Results

Windows x86-64, 13th Gen Intel(R) Core(TM) i5-13420H. clang 23.1.3, gcc 16.2.0
(MSYS2 UCRT64), rustc 1.99.1, go 1.27.0, Node 24.20.0, CPython 3.14.7. Each row is
the better of two passes of the script — best of 5 runs within a pass, after one
untimed warm-up, and best of 3 for the interpreted rows. Two passes rather than
one because of the clock-state problem described under Methodology.

Rows in **bold** are the portable Yashy rows. Rows named
``Yashy (`-O2`, `wrapping_*`)`` are the same program with the arithmetic opt-out;
they are the rows to compare against every other language here, and the plain
Yashy rows are the checked default.

### `fib(35)` — about 30 million calls

| Language | min ms | median ms | ns/call |
|---|---:|---:|---:|
| C (gcc -O2) | 48.6 | 51.7 | 1.628 |
| C (clang -O2, `-march=x86-64-v3`) | 80.0 | 81.9 | 2.679 |
| C (clang -O2, `-march=native`) | 81.5 | 83.8 | 2.729 |
| C (clang -O2) | 82.0 | 84.9 | 2.746 |
| **Yashy (`-O2`, `wrapping_*`)** | **82.2** | **89.6** | **2.753** |
| Rust (rustc -O) | 86.7 | 87.3 | 2.904 |
| Go (go build) | 139.6 | 150.4 | 4.675 |
| Yashy (`-O2 --cpu=x86-64-v3 --fast-math`) | 142.5 | 157.8 | 4.772 |
| Yashy (`-O2 --cpu=x86-64-v3`) | 146.1 | 151.5 | 4.893 |
| Yashy (`-O2 --native`) | 146.3 | 156.3 | 4.900 |
| **Yashy (clang -O2)** | **149.0** | **150.3** | **4.990** |
| Yashy (clang -O0) | 176.4 | 178.1 | 5.908 |
| Node 24 | 362.1 | 364.9 | 12.126 |
| CPython 3.14 | 3417.9 | 3497.9 | 114.463 |

`gcc` is consistently the fastest here and the reason is not codegen quality: it
inlines across the recursive call differently. Between `clang`, Rust and Yashy's
`wrapping_*` row the spread is about 6%, and which of the three wins moves from
run to run — `fib` is a small function called 30 million times, so it is dominated
by how the recursion is inlined and by call overhead, not by the quality of the
surrounding IR. Yashy's opt-out row is within a percent of `clang`'s here and ahead
of Rust's, which is the honest reading: a tie inside the noise.

The checked default is a different story — it is the only row in this whole suite
where Yashy lands behind Go. Three checks inside a function called thirty million
times cost 1.8–1.9×, and the tier rows are the checked build too, so the tier does
not buy anything back: the cost is the checks, not the instruction set.

### `loop` — 200,000,000 iterations

| Language | min ms | median ms | ns/iteration |
|---|---:|---:|---:|
| C (clang -O2, `-march=x86-64-v3`) | 63.5 | 64.3 | 0.318 |
| C (clang -O2, `-march=native`) | 68.3 | 69.5 | 0.342 |
| Yashy (`-O2 --cpu=x86-64-v3 --fast-math`) | 168.5 | 171.2 | 0.843 |
| Yashy (`-O2 --native`) | 170.4 | 177.9 | 0.852 |
| Yashy (`-O2 --cpu=x86-64-v3`) | 184.1 | 188.5 | 0.921 |
| **Yashy (clang -O2)** | **240.8** | **261.9** | **1.204** |
| C (gcc -O2) | 242.7 | 243.6 | 1.214 |
| Rust (rustc -O) | 249.1 | 272.9 | 1.246 |
| **Yashy (`-O2`, `wrapping_*`)** | **250.6** | **254.8** | **1.253** |
| C (clang -O2) | 266.5 | 275.3 | 1.333 |
| Go (go build) | 293.4 | 346.9 | 1.467 |
| Node 24 | 688.4 | 697.3 | 3.442 |
| Yashy (clang -O0) | 1716.5 | 1729.9 | 8.583 |
| CPython 3.14 | 26475.8 | 34453.2 | 132.379 |

Node's total is `29999999922509360`, not `29999999900000000`: JavaScript has no
64-bit integers and this sum exceeds 2^53. See the caveats above.

The whole `loop` story is the CPU tier *and* the check, and the two interact. On
the portable baseline the check is free: LLVM proves this loop cannot overflow and
deletes it, so the checked and opted-out rows are the same number inside the noise
(240.8 and 250.6 ms, against gcc's 242.7 and clang's 266.5). At `x86-64-v3` the
loop would vectorise — four `i64` additions per instruction instead of two — and
the checked build cannot: a potentially-faulting branch in the body is not
something LLVM's vectoriser will speculate. So the tier is worth 4.09× in the
pre-check numbers this file used to carry, 1.31× for the checked default, and ~3.8×
again once the loop is opted out (69 ms against clang's 68 ms, measured in one
pass). This row is the clearest example of what the check costs and of what the
opt-out is for: see [what the check
costs](../README.md#what-the-check-costs).

### `float` — 100,000,000 iterations

| Language | min ms | median ms | ns/iteration |
|---|---:|---:|---:|
| Yashy (`-O2 --cpu=x86-64-v3 --fast-math`) | 319.9 | 331.6 | 3.199 |
| C (clang -O2, `-march=native`) | 320.0 | 331.9 | 3.200 |
| C (clang -O2, `-march=x86-64-v3`) | 321.8 | 324.0 | 3.218 |
| **Yashy (clang -O2)** | **534.5** | **542.6** | **5.345** |
| C (gcc -O2) | 537.0 | 555.6 | 5.370 |
| Rust (rustc -O) | 541.7 | 567.5 | 5.417 |
| Yashy (`-O2 --cpu=x86-64-v3`) | 542.7 | 556.6 | 5.427 |
| C (clang -O2) | 545.6 | 551.4 | 5.456 |
| Yashy (`-O2 --native`) | 546.3 | 555.5 | 5.463 |
| Go (go build) | 549.3 | 593.6 | 5.493 |
| Node 24 | 684.3 | 689.9 | 6.843 |
| Yashy (clang -O0) | 827.1 | 934.0 | 8.271 |
| CPython 3.14 | 9421.3 | 9430.3 | 94.213 |

There is no `float-wrap.ya`: the benchmark contains no integer arithmetic, so
there is no check in it to opt out of, and the portable Yashy row is the best in
the table.

The three fast rows print `44051.909807502794`; every other row prints
`44051.909822450085`. That is not a typo. `clang` contracts `x * a + b` into a
single fused multiply-add at `-march=x86-64-v3` (C contracts by default), and
Yashy does the same under `--fast-math`. The FMA rounds once instead of twice, so
the last six digits differ — a 1.7× win bought by changing the answer. The two
fused rows land at 320 and 322 ms, which is the point: given the same target and
the same permission, Yashy and clang produce the same speed. On the portable
baseline, where no row is allowed to fuse, Yashy is 534.5 ms against clang's 545.6
and gcc's 537.0.

### The effect of the CPU tier

`--cpu=<tier>` stops targeting the x86-64 baseline and targets an instruction-set
level: `x86-64-v2`, `x86-64-v3`, `x86-64-v4`, or `native` (the CPU running the
build). Each C row is built with the matching `-march=`, because the flag — not
the compiler — is what does the work.

| Benchmark | portable `-O2` | `--cpu=x86-64-v3` | Speedup | `--native` | Speedup |
|---|---:|---:|---:|---:|---:|
| `fib` | 149.0 ms | 146.1 ms | 1.02× | 146.3 ms | 1.02× |
| `loop` | 240.8 ms | 184.1 ms | 1.31× | 170.4 ms | 1.41× |
| `float` | 534.5 ms | 542.7 ms | 0.99× | 546.3 ms | 0.98× |

Two of the three rows say the flag is worth almost nothing, and one says it is
worth a good deal — but less than it used to, and the arithmetic check is why. The
wider target unlocks AVX2 and the wider vector unit, so it pays exactly where the
backend has something to do with them and nowhere else. `float` is unaffected
because its dependency chain is serial — no vectorisation is possible — and `fib`
is unaffected because it is calls, not arithmetic. On `loop` the tier would be
worth ~3.8× and is held to 1.31× by the check in the loop body; the opted-out build
collects the whole 3.8× (69 ms against clang's 68 ms in one pass). `x86-64-v3`
edges out `native` on `loop` here; the two are within run-to-run noise of each
other, and the fixed tier is the one to quote because the same binary runs on every
v3 machine.

Three things are deliberately *not* the default, and each was measured:

- **The tier is not the default.** A binary built for a wide CPU dies with an
  illegal instruction on an older one. The default has to run on a machine the
  compiler has never seen, so the tier is opt-in and the portable row stays the
  headline.
- **FMA contraction is not the default.** `--fast-math` puts LLVM's `contract`
  flag on our `fmul`/`fadd`, and at `x86-64-v3` the backend fuses them — that is
  how Yashy reaches 312 ms on `float`, matching clang. It is opt-in because it
  changes the number a program prints, and the portable rows are the ones the
  suite promises compute the same answer. (An earlier revision of this file said
  the `contract` flag did not fuse. That was measured on the portable baseline,
  which has no FMA unit to fuse into; on a target that has one, it fuses.)
- **The tier is a `clang` argument, never an IR change.** A codegen test pins
  that: the emitted IR is byte-identical whether or not a tier is set. So a tier
  cannot change the *meaning* of a program, only whether it runs on a given CPU.

### The effect of the optimization level

Yashy built at `-O0` and at `-O2`, same source, same machine:

| Benchmark | `-O0` | `-O2` | Speedup |
|---|---:|---:|---:|
| `fib` | 176.4 ms | 149.0 ms | 1.18× |
| `loop` | 1716.5 ms | 240.8 ms | 7.13× |
| `float` | 827.1 ms | 534.5 ms | 1.55× |

### Compile time (`fib`, best of 5)

| Toolchain | min ms | median ms |
|---|---:|---:|
| Yashy (front end only, `--emit-ir`) | 25.4 | 28.3 |
| C (`clang -O2`) | 244.8 | 252.4 |
| Yashy (full build, `-O2`) | 256.2 | 263.0 |
| Go (`go build`) | 261.9 | 265.6 |
| Rust (`rustc -O`) | 558.9 | 600.2 |

`yashy build` is a single `clang` invocation now — compile the IR and link, with
no intermediate object file — which is why a full build lands 5% above
`clang -O2` on the same program rather than double it. It used to run `clang`
twice and recompile the runtime's C source on every build (135 ms of a 403 ms
build); the runtime object is cached in a content-addressed directory now, so the
first build on a machine pays for filling that cache and every build after it
does not. The front end — everything this repository implements — is 25 ms of the
256.

## Feature benchmarks

The scalar table answers "how fast is the arithmetic?"; this one answers "what
does a feature cost?". Every row runs the same algorithm and must print the
same expected number — the harness throws on a mismatch, so a wrong hash, a
lost update, or a broken array growth becomes a failed run rather than a fast
row.

Methodology, beyond the shared rules above:

- **9 runs, 3 passes** in the published run (the script defaults to 5 × 2),
  one warm-up, minimum wins. These programs are short, so the same clock-state
  caveat applies; a row measured in a bad window gets two more chances. The
  first draft of these numbers was taken with 5 × 2 and the channel row moved
  by almost 2× between runs — the reason this table uses more samples.
- **No preallocation** in any array or map row. Yashy's `map_new()` and
  `[]int {}` start empty; Go's `make(map)` and `Vec::new()` do too, and the C
  array doubles from four elements like the Yashy runtime does.
- **The implementations are not the same code.** Yashy's map is an
  open-addressed table with linear probing that grows at 70% occupancy; Go's
  map and Rust's `HashMap` have their own layouts and hash functions. Yashy's
  array grows by doubling from four; Go and Rust use their own growth
  policies. The row measures each language's implementation, not an algorithm
  race.
- **Threads are not goroutines.** Yashy spawns an OS thread and blocks on
  condition variables; Go multiplexes a goroutine onto its runtime scheduler.
  The comparison is useful as a cost check, not as a claim that the two are the
  same mechanism. Mutex rows differ too: Yashy takes the OS mutex immediately,
  while Go's `sync.Mutex` spins briefly before parking.
- **`ns/op` counts the unit in the `Workload` column**: one C call, one array
  element, one map insert-or-lookup, one enum iteration, one channel round
  trip, one lock/unlock pair, one scanned byte, or one printed line. Compare
  rows within a benchmark, not across benchmarks.

### Results

`ns/op` tables, Windows x86-64, same machine and toolchains as the scalar
results. Each row is the better of the three passes above; a missing row was
blocked by Application Control (Device Guard) and is reported as such by the
script rather than timed.

#### FFI — 100,000,000 `getpid` calls

| Language | min ms | median ms | ns/call |
|---|---:|---:|---:|
| C (clang -O2) | 237.0 | 247.4 | 2.37 |
| C (gcc -O2) | 239.6 | 249.6 | 2.40 |
| Yashy -O2 | 247.8 | 267.7 | 2.48 |

`getpid` is opaque to the optimizer, so every iteration is a real call; the
loop cannot be folded away, which an earlier draft using `llabs` was. Yashy is
4.6% behind clang and 3.4% behind gcc, all in the same band. There is no thunk
or wrapper in that path: an `extern` call is lowered as an ordinary LLVM call,
with the C ABI's argument and return conventions.

#### Arrays — 10,000,000 appends, a sum, then query, edit, search, and copy methods

| Language | min ms | median ms | ns/element |
|---|---:|---:|---:|
| C (clang -O2) | 141.1 | 143.8 | 14.11 |
| C (gcc -O2) | 143.3 | 146.1 | 14.33 |
| Rust (rustc -O) | 159.4 | 165.6 | 15.94 |
| Yashy -O2 | 207.2 | 218.9 | 20.72 |
| Go (go build) | 258.2 | 294.8 | 25.82 |

Yashy's `push` is an out-of-line runtime call: check capacity, maybe grow and
copy, then `memcpy` the element. The C loop inlines the common case and only
calls `realloc` every doubling, and Rust's `Vec::push` inlines its fast path;
that is the 1.5× gap to C and 1.3× to Rust. Go's `append` is slower here than
Yashy's push, despite the same doubling idea. The 10,000,000-element array is
about 80 MB, so allocation and the growth copies are part of the measurement,
not background noise.

The trailing query/edit/search/extend/copy calls are outside the timed loop.
They keep the M12 array method path and the new array runtime entry points in a
benchmark that is built and run with the rest of the suite; they do not change
the growth numbers. A final string-method pass covers `trim`, case conversion,
`replace`, `slice`, `repeat`, `split`, `lines`, `contains`, `starts_with`,
`ends_with`, `byte_at`, `get`, `index_of`, `is_empty`, `parse_i64`, and
`parse_f64` for the same reason: a broken method resolver or string runtime
entry point fails the run instead of hiding behind a number.

#### Maps — 1,000,000 inserts and 1,000,000 lookups

| Language | min ms | median ms | ns/operation |
|---|---:|---:|---:|
| Rust (rustc -O) | 281.5 | 321.1 | 140.75 |
| Yashy -O2 | 322.1 | 329.9 | 161.05 |
| Go (go build) | 370.7 | 420.0 | 185.35 |

Yashy's map is an open-addressed table with linear probing that grows at 70%
occupancy; Rust and Go have their own layouts and hash functions. On this
integer-key workload Yashy is 14% behind Rust and 15% ahead of Go. Nothing is
preallocated: the inserts include every rehash. A `string`-key workload would
measure the hash function and key retention instead, so this table is about
`int` keys only.

#### Walking a map — 1,000,000 entries, copied into two arrays ten times over

| Language | min ms | median ms | ns/element |
|---|---:|---:|---:|
| Rust (rustc -O) | 567.3 | 574.8 | 28.36 |
| Go (go build) | 761.4 | 784.3 | 38.07 |
| Yashy -O2 | 1065.4 | 1077.7 | 53.27 |

The workload is `keys()` and `values()`: the whole table copied into two
arrays, ten times over, and then 2,000,000 `contains_key` probes of a key that
is there and one that is not. Go's row collects the map into two slices and
Rust's into two vectors, which is the same work; the `contains_key` probe is
Go's `v, ok := m[k]` and Rust's `contains_key`.

Yashy is 1.9× behind Rust and 1.4× behind Go here, and the gap is the copy.
Yashy's `keys()` and `values()` are runtime calls that walk the table's slots
and `memcpy` each entry out into a fresh array; Rust's `keys()` and
`values()` are borrows of the table's own storage, so the collect is the only
copy either of them makes. That is the price of the shape Yashy chose: the
arrays are the caller's to keep and free, and the map may be freed while they
are still alive.

#### Debug formatting — 1,000,000 formatted points

| Language | min ms | median ms | ns/format |
|---|---:|---:|---:|
| Rust (rustc -O) | 513.4 | 530.7 | 513.40 |
| Yashy -O2 | 1753.6 | 1772.4 | 1753.60 |

`${value:?}` builds the text a programmer reads, and the two rows print the
same text: `Point { x: 1, y: 2 }` in both, so the totals match and the
comparison is of the same work. Rust's derived `Debug` is 3.4× faster here.
The difference is where the formatting happens: Rust's `format!` writes into
a `String` it grows in place, while Yashy's interpolation builds the text in
the runtime's own buffer and then copies it into the string the binding
holds. The `{:?}` machinery is a `Debug` implementation in both, and the
per-field work — the field names, the braces, the two numbers — is the same
in both.

These two tables are from the script's default run (5 runs, 2 passes) rather
than the published 9 × 3, so their numbers carry a little more noise than the
tables above them; the gaps they report are far larger than that noise.

#### Enums — 50,000,000 constructions and matches

| Language | min ms | median ms | ns/iteration |
|---|---:|---:|---:|
| C (clang -O2) | 101.2 | 130.2 | 2.02 |
| C (gcc -O2) | 120.4 | 129.4 | 2.41 |
| Yashy -O2 | 150.1 | 166.6 | 3.00 |

Both sides build a 16-byte tagged value and switch on the tag. Yashy's enum is
`{ i8, i64 }` with the tag in field 0 and the payload in field 1; the C struct
uses an `int` tag. The 1.5× gap is LLVM's handling of the aggregate through the
two calls, not a different data representation: the value does not allocate and
the match has no fallback path.

#### Channels — 200,000 round trips

| Language | min ms | median ms | ns/round trip |
|---|---:|---:|---:|
| Yashy -O2 | 179.3 | 253.0 | 896.5 |
| Go (go build) | 296.2 | 318.1 | 1481.0 |

One capacity-one channel in each direction, so the two sides alternate strictly
and every send waits for the matching receive. Yashy pays for two condition
variables and an OS thread; Go pays for a goroutine wakeup in its scheduler.
The minimum and median disagree by 40% on the Yashy row, which is scheduling
noise, not a bimodal implementation — the earlier 5 × 2 run had Yashy at
1822 ns and Go at 1764 ns on the same binaries. Treat this row as "the same
order", not as a scheduler comparison.

#### Mutexes — 1,000,000 contended lock/unlock pairs

| Language | min ms | median ms | ns/pair |
|---|---:|---:|---:|
| Go (go build) | 65.6 | 67.4 | 65.60 |
| Yashy -O2 | 73.5 | 84.5 | 73.50 |

Four threads and four goroutines, an empty critical section, one million pairs
in total. Yashy takes the OS mutex immediately; Go's `sync.Mutex` spins briefly
before parking. The 12% gap is what that difference and the lock implementation
cost under contention. A shared counter cannot be written in Yashy user code
yet: a `*mut` may not cross a thread boundary, so the runtime's own C test
covers that case.

#### Mapping an array — 10,000,000 elements into a second array

| Language | min ms | median ms | ns/element |
|---|---:|---:|---:|
| C (gcc -O2) | 110.8 | 118.0 | 11.08 |
| C (clang -O2) | 112.6 | 125.4 | 11.26 |
| Rust (rustc -O) | 116.2 | 123.1 | 11.62 |
| Go (go build) | 144.9 | 167.8 | 14.49 |
| Yashy -O2 | 417.1 | 458.4 | 41.71 |

Every row builds an array of ten million elements, maps it into a second
array, and sums the second one. The C, Go and Rust rows map with a plain
loop; Yashy's row maps through a function value, so every element costs a
call through a pointer as well.

That is the 30 ns an element Yashy spends over C's 11: the call through the
function value is about 4 ns of it, and the rest is the array the mapping
builds — Yashy's `push` is an out-of-line runtime call that checks the
capacity and copies the element, where C's loop writes into an array it
already knows the size of. The [function
values](#function-values--100000000-calls-through-a-value-and-directly)
table has the same story in one number: a call through a value costs about
2.5 ns more than a direct one.

#### Function values — 100,000,000 calls, through a value and directly

| Language | min ms | median ms | ns/call |
|---|---:|---:|---:|
| C (clang -O2, direct) | 158.8 | 165.9 | 1.59 |
| Yashy -O2 (direct) | 160.7 | 163.9 | 1.61 |
| Rust (rustc -O, direct) | 162.1 | 165.4 | 1.62 |
| C (gcc -O2, direct) | 201.2 | 207.7 | 2.01 |
| Go (go build, direct) | 232.3 | 241.0 | 2.32 |
| C (gcc -O2, value) | 306.3 | 314.9 | 3.06 |
| C (clang -O2, value) | 343.5 | 355.9 | 3.44 |
| Rust (rustc -O, value) | 374.9 | 389.8 | 3.75 |
| Yashy -O2 (lambda) | 414.6 | 423.8 | 4.15 |
| Yashy -O2 (closure) | 417.3 | 426.1 | 4.17 |
| Yashy -O2 (value) | 453.4 | 466.1 | 4.53 |
| Go (go build, value) | 548.5 | 569.2 | 5.49 |

The direct rows call the function by name; the value rows call the same
function through a value, so the difference between the two rows of a language
is the price of the value and nothing else.

The price is about 1.5 ns more per call in C, 2.1 ns in Rust, 2.9 ns in Yashy
and 3.2 ns in Go. A function value is a pair of pointers — the code and the
environment — and the call goes through the code pointer, which the processor
cannot predict. The direct rows are the same program with the call folded to a
direct one, which is why they are 1.6 to 2.3 ns.

Yashy's value row is 31% behind clang's and 18% ahead of Go's. Yashy's
function value is a `{ i8* code, i8* env }` pair in memory: the call site
loads the pair out of the binding's slot, takes the two halves, and calls the
code pointer with the environment as its first argument. The environment is
null for a top-level function, but the argument is still passed, because that
is the convention a closure will use.

The two Yashy rows that are not a direct call are the same program through
different values. The `lambda` row calls through a lambda written inside the
loop, so the value is built on every iteration exactly as the value row builds
it, and the two differ only in what the value points at. The `closure` row
calls through a closure that captured a value, so the call passes an
environment that is not null and the closure reads the captured value out of
it. The two cost the same to within a percent: 4.15 ns and 4.17 ns, against
the named function's 4.53. Carrying a captured value is not what makes a call
through a value slow; going through the pointer is.

When these numbers change, update this file and the `Speed` section of the
top-level `README.md` in the same commit, so the two cannot drift.

#### Methods — 10,000,000 calls through a pointer receiver

| Language | min ms | median ms | ns/call |
|---|---:|---:|---:|
| C (gcc -O2) | 16.8 | 17.5 | 1.68 |
| C (clang -O2) | 18.0 | 19.2 | 1.80 |
| Yashy -O2 | 19.0 | 20.3 | 1.90 |
| Rust (rustc -O, `&mut self`) | 19.4 | 22.0 | 1.94 |
| Go (go build, pointer receiver) | 36.4 | 40.5 | 3.64 |

Every row increments one field of one struct ten million times, through a
method that takes the object's address: Yashy's `self: *mut Counter`, C's
`Counter *`, Rust's `&mut self`, Go's pointer receiver. The counter lives in
the caller's frame and is never reallocated, so the number is the call and the
store.

Yashy is 5.6% behind clang and 2% ahead of Rust. A pointer receiver lowers to
a direct call whose first argument is the caller's slot — the local's own
`alloca`, not a copy — and the field write is a `getelementptr` at the field's
offset plus a `store`. There is nothing to box, no vtable to look up, and no
aggregate to rebuild. The 1.6 ns gap to gcc is the checked add on
`self[0].value + 1`: Yashy's `+` traps on overflow, and that check is a
`llvm.sadd.with.overflow` plus a branch, which gcc's `-O2` is free to omit.

#### Generic functions — 100,000,000 calls

| Language | min ms | median ms | ns/call |
|---|---:|---:|---:|
| Yashy -O2 (generic) | 162.1 | 173.3 | 1.62 |
| C (clang -O2) | 163.7 | 200.6 | 1.64 |
| Yashy -O2 (direct) | 164.6 | 172.9 | 1.65 |
| Rust (rustc -O) | 169.0 | 179.2 | 1.69 |
| C (gcc -O2) | 209.4 | 241.1 | 2.09 |
| Go (go build) | 264.2 | 290.1 | 2.64 |

Every row runs the same loop: `scale(i, 1000)` over 100,000,000 iterations,
summed with wrapping addition, and every row prints
`4999999950099999999`. The two Yashy rows are `scale[T]` and a `scale` written
with `int`; the other languages use their own generic function, except C, which
has none — its row is the specialized function a generic call would compile to.

The two Yashy rows differ by 1.5%, which is inside this machine's run-to-run
spread (the direct row's median is the *lower* of the two). That is the result:
a generic function costs what the concrete one costs, because it *is* the
concrete one. `scale(i, 1000)` infers `T = int`, the checker checks one body at
that substitution, and the backend emits `scale_i64` as an ordinary function
with an ordinary name. The call site is a direct call; no value is built, no
type is inspected, and nothing is dispatched. Rust and Go monomorphize the same
way, which is why their rows sit in the same band.

The gcc row is slower than clang's for the same reason it is slower on the
`function_direct` row above: `-O2` leaves the signed `% 3` in a form clang
turns into a multiply-and-shift. The gap is the compiler's arithmetic, not the
call.

A generic function that is never called emits nothing at all: the body is
checked at a substitution, and a program with no substitution has no body to
check. That is a compile-time property, not a run-time one, and this benchmark
does not measure it.

#### Fixed arrays — 50,000,000 traversals of 8 elements

| Language | min ms | median ms | ns/element |
|---|---:|---:|---:|
| C (clang -O2) | 16.9 | 19.9 | 0.042 |
| Yashy -O2 (`wrapping_add`) | 17.9 | 19.8 | 0.045 |
| Rust (rustc -O) | 21.4 | 25.6 | 0.053 |
| C (gcc -O2) | 176.3 | 196.6 | 0.441 |
| Yashy -O2 (`+`) | 192.8 | 218.3 | 0.482 |
| Go (go build) | 333.6 | 352.6 | 0.834 |

Every row walks the same eight-element `[1, 2, 3, 4, 5, 6, 7, 8]` fifty
million times and adds each element into one accumulator, then prints
`1800000000`. The array is in the frame in every language: `[int; 8]` in Yashy,
`int64_t grid[8]` in C, `[8]int64` in Go, `[i64; 8]` in Rust. Nothing is
allocated, nothing is freed, and no bounds field is read.

The table is really two measurements. The `wrapping_add` row is the array: 17.9
ms against clang's 16.9 (5.9% behind) and Rust's 21.4 (16% ahead). Iterating a
fixed array is a pointer bump and a load; the inner `for` reads the length from
the type, so the loop's bound is a constant and LLVM folds it like any other
counted loop. The Go row is slow for an unrelated reason — Go's compiler does
not reassociate the sum — and gcc's `-O2` does not either, which is why those
two rows are near 180 ms.

The second measurement is the checked add. Yashy's `+` is an overflow test plus
a branch on every element, and that branch is loop-carried, so the optimiser
cannot reassociate the sum away. That row is 11x the `wrapping_add` row with
the *same* array code, and it is the one place in this suite where the cost of
Yashy's arithmetic rules is larger than the cost of the feature being measured.
C and Rust get the fast row by default: C's signed overflow is undefined, and
Rust's `-O` turns overflow checks off. Go defines wrapping but does not
reassociate, so its row is the slow one despite having no checks at all. The
honest reading is that the fixed array costs about what C's array costs, and
that a checked add is what it is.

A view (`grid[0..2]`, or the whole `grid` in the `for` above) is not measured
separately: it is the same header-free pointer-and-length loop as iterating the
array, and its length is already a word in a register.

#### Tuples — 50,000,000 calls returning two values

| Language | min ms | median ms | ns/iteration |
|---|---:|---:|---:|
| C (gcc -O2) | 74.7 | 80.1 | 1.494 |
| C (clang -O2) | 90.6 | 97.8 | 1.812 |
| Rust (rustc -O) | 97.7 | 102.9 | 1.953 |
| Yashy -O2 | 133.6 | 135.4 | 2.672 |
| Go (go build) | BLOCKED | — | — |

Every row runs the same loop: `min_max(i, rounds - i)` returns the smaller and
the larger of its two arguments as one value, the caller destructures that value
into `lo` and `hi`, and folds `3 * lo + hi` into an accumulator. The program
prints `3750000000000000`. The pair travels by value: Yashy returns a
`{ i64, i64 }` literal struct, C returns a two-field struct, Rust returns a
tuple (also a two-field struct), and Go returns two results — which its ABI also
turns into a register pair.

The rows are within 1.8x of each other and the ordering is arithmetic policy
again, not layout. All four compilers inline a function this small, so the
aggregate return and the destructuring disappear and what is left is the
`if`, the multiply, and the adds. Yashy's `+`, `-`, and `*` each test for
overflow and branch, and this loop does five of them per iteration (the
subtraction in the argument, the multiply, two adds into the accumulator, and
the increment), so the Yashy row is 1.47x clang and 1.79x gcc. Rust's `-O`
turns overflow checks off and lands next to clang; gcc reassociates the sum
into one multiply-add chain and wins. A tuple itself has no run-time
representation to pay for: the number would be the same if `min_max` returned
one `int` and the loop multiplied it.

The Go row is BLOCKED: Application Control refused every freshly linked
`tuple-go` binary during the run, and this suite does not time a binary that
has not printed its expected output. The Yashy row was produced by the same
compiler source built in debug mode (`-O2` output is unchanged by the compiler
binary's own optimization), because the freshly built release driver was
blocked at the time; re-running `run-features.ps1` rebuilds and times every row
through the release driver.

#### Characters — 1 MiB scanned one byte at a time

| Language | min ms | median ms | ns/byte |
|---|---:|---:|---:|
| Yashy -O2 | 31.6 | 32.1 | 30.14 |

The program builds a 1 KiB pattern, repeats it to 1 MiB, and walks it with
`s.byte_at(i)`, counting the bytes in `'a'..='z'`; it prints `393216`, which the
harness checks. Measured 2026-10-24 on the Windows x86-64 machine above with the
plan's `-Runs 3 -Passes 1`. The row is short enough that process start-up is most
of the number: a no-op program costs about 20-30 ms here, so the scan itself is
about 8 ms and the 30 ns/byte figure is an upper bound. It proves the `char`
load and the two byte-order comparisons are real work rather than a cast to
`int` that disappears. There is no C or Rust row because those languages compare
`unsigned char`/`u8` as integers, which is not the `char` type this row exists to
measure.

#### Printing — 200,000 lines

| Language | min ms | median ms | ns/line |
|---|---:|---:|---:|
| Yashy -O2 | 159.4 | 201.5 | 797.00 |

The program writes `println(i)` for `i` from 0 to 199,999. The runner hands the
child's stdout straight to a file (PowerShell's `>` would decode the text and
rewrite the line endings) and checks the byte count, `1288890` — the digits of
0..199,999 plus one newline each — so a missing newline or a wrong conversion is
a failed run, not a fast one. It proves the print path converts through
`str_from_i64` and writes through `yashy_print_line_str` for every line, at
about 1.25 million lines per second. Measured 2026-10-24 on the same machine with
`-Runs 3 -Passes 1`; the gap between the minimum and the median is the file
system's write buffering, not a different amount of work.


