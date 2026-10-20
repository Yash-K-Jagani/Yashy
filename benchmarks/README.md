# Otter benchmarks

Two suites live here:

- **Scalar** (`run.ps1`) — `fib`, `loop` and `float`, written in six languages,
  answering **how fast is the code Otter produces** and **how fast does Otter
  compile it**.
- **Features** (`run-features.ps1`) — FFI, slices, maps, enums, channels and
  mutexes, answering **what each language feature costs** against the same idea
  in a language that has it built in.

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
| `float` | 100,000,000 iterations of `x = x * a + b` | `f64` arithmetic on a serial dependency chain |
| `fib-wrap`, `loop-wrap` | the two integer programs again, with `wrapping_*` in place of bare `+`, `-`, `*` | what Otter's checked arithmetic costs, and the row that is strictly comparable with C, Rust, Go, Node and Python — all of whose arithmetic wraps |

Otter's arithmetic is checked, so `fib.ot` is not quite the same program as
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
intermediate value well inside `i64` — no language's overflow rule changes the
answer, and C's signed-overflow undefined behaviour never fires. `float` is a
strict dependency chain, so no compiler may reassociate or vectorise it without
a wide target — and every row built for the portable baseline computes
bit-identical results. The rows that do not are those built for a wider one:
`clang -march=x86-64-v3` fuses `x * a + b` into a single FMA, and Otter does the
same only under `--fast-math`. That turns two roundings into one and moves the
last six digits. Those rows are labelled, and they are discussed below.

### The feature programs

These run under `run-features.ps1` and are not part of the scalar table. Each
one exercises one language feature end to end; the rows are the languages that
have a comparable standard-library primitive.

| Program | Workload | Compared with |
|---|---|---|
| `ffi` | 100,000,000 calls to libc's `getpid` | C, clang and gcc |
| `slice` | 10,000,000 appends to a growable array, then a sum | C (`realloc`), Go `append`, Rust `Vec` |
| `map` | 1,000,000 `i64` inserts, then 1,000,000 lookups | Go's built-in map, Rust's `HashMap` |
| `enum` | 50,000,000 three-variant enum constructions and matches | C tagged struct |
| `threads` | 200,000 channel round trips between two threads | Go channels |
| `mutex` | 1,000,000 contended lock/unlock pairs across four threads | Go `sync.Mutex` |

There is no C row for `map`: C has no hash map in its standard library, and a
hand-written one would measure the author of that C file rather than Otter.
There is likewise no C row for `threads` and `mutex`: the portable C answer is
pthreads on Linux and the Win32 API on Windows, so a single source would not be
the same program on both. Go is the comparison there, with the caveat that a
goroutine is not an OS thread.

## What the comparison does and does not mean

It means: **for scalar arithmetic and function calls, Otter's output is
competitive with what C, Rust and Go produce.** That is the claim being tested,
and the numbers support or refute it.

It does not mean "Otter is as fast as C". Read the caveats:

- **Otter's binary is clang's output.** `otter-codegen` emits unoptimized
  textual LLVM IR and `clang -O2` optimizes it. So the honest comparison is "the
  IR Otter emits vs. the IR clang and rustc emit, given the same optimizer",
  which is a real and useful measurement, but it is not a claim that
  `otter-codegen` contains optimization passes — it contains none. Otter's
  advantage, where it has one, is that it hands LLVM clean, fully-typed IR.
- **The scalar suite does not allocate; the feature suite does.** `fib`, `loop`
  and `float` still measure arithmetic and calls only, so that table remains a
  lower bound rather than a preview of a real program. Slices, maps, strings and
  threads are measured separately under [Feature
  benchmarks](#feature-benchmarks), where allocation *is* the workload. The two
  tables answer different questions and are not comparable row for row.
- **Otter's arithmetic is checked, and the others' is not.** `a + b` in Otter
  reports an integer overflow at the position it happened and ends the program; in
  C, Rust, Go, Node and Python it wraps. That is a difference in what the program
  *promises*, not in how well it is compiled, so the benchmark set carries
  `fib-wrap` and `loop-wrap`: the same source with the opt-out
  (`wrapping_add`/`_sub`/`_mul`) in place of `+ - *`. Those are the rows to read
  against the other languages, and the gap between a benchmark's two Otter rows is
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
  its own native compiler. `clang` is given the same `--target=` triple Otter
  uses, read back out of Otter's own IR — on Windows a bare `clang` defaults to
  the MSVC triple and cannot find `stdio.h` on a machine without the MSVC CRT.
- **Two C compilers** (`clang` and `gcc`), so the Otter numbers are not graded
  against one backend's quirks.
- **Otter is built twice**, at `-O0` and at `-O2`, so the table shows what
  enabling optimization actually buys instead of asserting that it helps. The
  integer benchmarks are built a third time from `<name>-wrap.ot`, which is what
  the checked-arithmetic comparison uses.
- **Wider-target rows are opt-in and always paired.** `--cpu=x86-64-v3` and
  `--native` each build an Otter row *and* the matching `clang` row
  (`-march=x86-64-v3`, `-march=native`), so the table shows what the *flag* buys
  rather than what Otter buys over a compiler that was not given it. They are
  never the headline; the portable rows are. The `--fast-math` Otter row is
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

The interesting row is **Otter (front end only)**: `otter build --emit-ir` runs
lexing, parsing, type checking and IR emission, and stops before any LLVM
process starts. That is the part of the pipeline this repository actually
implements — and it is a small fraction of the full build, because the rest is
clang doing for Otter exactly what it does for C.

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

Rows in **bold** are the portable Otter rows. Rows named
``Otter (`-O2`, `wrapping_*`)`` are the same program with the arithmetic opt-out;
they are the rows to compare against every other language here, and the plain
Otter rows are the checked default.

### `fib(35)` — about 30 million calls

| Language | min ms | median ms | ns/call |
|---|---:|---:|---:|
| C (gcc -O2) | 48.6 | 51.7 | 1.628 |
| C (clang -O2, `-march=x86-64-v3`) | 80.0 | 81.9 | 2.679 |
| C (clang -O2, `-march=native`) | 81.5 | 83.8 | 2.729 |
| C (clang -O2) | 82.0 | 84.9 | 2.746 |
| **Otter (`-O2`, `wrapping_*`)** | **82.2** | **89.6** | **2.753** |
| Rust (rustc -O) | 86.7 | 87.3 | 2.904 |
| Go (go build) | 139.6 | 150.4 | 4.675 |
| Otter (`-O2 --cpu=x86-64-v3 --fast-math`) | 142.5 | 157.8 | 4.772 |
| Otter (`-O2 --cpu=x86-64-v3`) | 146.1 | 151.5 | 4.893 |
| Otter (`-O2 --native`) | 146.3 | 156.3 | 4.900 |
| **Otter (clang -O2)** | **149.0** | **150.3** | **4.990** |
| Otter (clang -O0) | 176.4 | 178.1 | 5.908 |
| Node 24 | 362.1 | 364.9 | 12.126 |
| CPython 3.14 | 3417.9 | 3497.9 | 114.463 |

`gcc` is consistently the fastest here and the reason is not codegen quality: it
inlines across the recursive call differently. Between `clang`, Rust and Otter's
`wrapping_*` row the spread is about 6%, and which of the three wins moves from
run to run — `fib` is a small function called 30 million times, so it is dominated
by how the recursion is inlined and by call overhead, not by the quality of the
surrounding IR. Otter's opt-out row is within a percent of `clang`'s here and ahead
of Rust's, which is the honest reading: a tie inside the noise.

The checked default is a different story — it is the only row in this whole suite
where Otter lands behind Go. Three checks inside a function called thirty million
times cost 1.8–1.9×, and the tier rows are the checked build too, so the tier does
not buy anything back: the cost is the checks, not the instruction set.

### `loop` — 200,000,000 iterations

| Language | min ms | median ms | ns/iteration |
|---|---:|---:|---:|
| C (clang -O2, `-march=x86-64-v3`) | 63.5 | 64.3 | 0.318 |
| C (clang -O2, `-march=native`) | 68.3 | 69.5 | 0.342 |
| Otter (`-O2 --cpu=x86-64-v3 --fast-math`) | 168.5 | 171.2 | 0.843 |
| Otter (`-O2 --native`) | 170.4 | 177.9 | 0.852 |
| Otter (`-O2 --cpu=x86-64-v3`) | 184.1 | 188.5 | 0.921 |
| **Otter (clang -O2)** | **240.8** | **261.9** | **1.204** |
| C (gcc -O2) | 242.7 | 243.6 | 1.214 |
| Rust (rustc -O) | 249.1 | 272.9 | 1.246 |
| **Otter (`-O2`, `wrapping_*`)** | **250.6** | **254.8** | **1.253** |
| C (clang -O2) | 266.5 | 275.3 | 1.333 |
| Go (go build) | 293.4 | 346.9 | 1.467 |
| Node 24 | 688.4 | 697.3 | 3.442 |
| Otter (clang -O0) | 1716.5 | 1729.9 | 8.583 |
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
| Otter (`-O2 --cpu=x86-64-v3 --fast-math`) | 319.9 | 331.6 | 3.199 |
| C (clang -O2, `-march=native`) | 320.0 | 331.9 | 3.200 |
| C (clang -O2, `-march=x86-64-v3`) | 321.8 | 324.0 | 3.218 |
| **Otter (clang -O2)** | **534.5** | **542.6** | **5.345** |
| C (gcc -O2) | 537.0 | 555.6 | 5.370 |
| Rust (rustc -O) | 541.7 | 567.5 | 5.417 |
| Otter (`-O2 --cpu=x86-64-v3`) | 542.7 | 556.6 | 5.427 |
| C (clang -O2) | 545.6 | 551.4 | 5.456 |
| Otter (`-O2 --native`) | 546.3 | 555.5 | 5.463 |
| Go (go build) | 549.3 | 593.6 | 5.493 |
| Node 24 | 684.3 | 689.9 | 6.843 |
| Otter (clang -O0) | 827.1 | 934.0 | 8.271 |
| CPython 3.14 | 9421.3 | 9430.3 | 94.213 |

There is no `float-wrap.ot`: the benchmark contains no integer arithmetic, so
there is no check in it to opt out of, and the portable Otter row is the best in
the table.

The three fast rows print `44051.909807502794`; every other row prints
`44051.909822450085`. That is not a typo. `clang` contracts `x * a + b` into a
single fused multiply-add at `-march=x86-64-v3` (C contracts by default), and
Otter does the same under `--fast-math`. The FMA rounds once instead of twice, so
the last six digits differ — a 1.7× win bought by changing the answer. The two
fused rows land at 320 and 322 ms, which is the point: given the same target and
the same permission, Otter and clang produce the same speed. On the portable
baseline, where no row is allowed to fuse, Otter is 534.5 ms against clang's 545.6
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
  how Otter reaches 312 ms on `float`, matching clang. It is opt-in because it
  changes the number a program prints, and the portable rows are the ones the
  suite promises compute the same answer. (An earlier revision of this file said
  the `contract` flag did not fuse. That was measured on the portable baseline,
  which has no FMA unit to fuse into; on a target that has one, it fuses.)
- **The tier is a `clang` argument, never an IR change.** A codegen test pins
  that: the emitted IR is byte-identical whether or not a tier is set. So a tier
  cannot change the *meaning* of a program, only whether it runs on a given CPU.

### The effect of the optimization level

Otter built at `-O0` and at `-O2`, same source, same machine:

| Benchmark | `-O0` | `-O2` | Speedup |
|---|---:|---:|---:|
| `fib` | 176.4 ms | 149.0 ms | 1.18× |
| `loop` | 1716.5 ms | 240.8 ms | 7.13× |
| `float` | 827.1 ms | 534.5 ms | 1.55× |

### Compile time (`fib`, best of 5)

| Toolchain | min ms | median ms |
|---|---:|---:|
| Otter (front end only, `--emit-ir`) | 25.4 | 28.3 |
| C (`clang -O2`) | 244.8 | 252.4 |
| Otter (full build, `-O2`) | 256.2 | 263.0 |
| Go (`go build`) | 261.9 | 265.6 |
| Rust (`rustc -O`) | 558.9 | 600.2 |

`otter build` is a single `clang` invocation now — compile the IR and link, with
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
lost update, or a broken slice growth becomes a failed run rather than a fast
row.

Methodology, beyond the shared rules above:

- **9 runs, 3 passes** in the published run (the script defaults to 5 × 2),
  one warm-up, minimum wins. These programs are short, so the same clock-state
  caveat applies; a row measured in a bad window gets two more chances. The
  first draft of these numbers was taken with 5 × 2 and the channel row moved
  by almost 2× between runs — the reason this table uses more samples.
- **No preallocation** in any slice or map row. Otter's `map_new()` and
  `[]i64 {}` start empty; Go's `make(map)` and `Vec::new()` do too, and the C
  slice doubles from four elements like the Otter runtime does.
- **The implementations are not the same code.** Otter's map is an
  open-addressed table with linear probing that grows at 70% occupancy; Go's
  map and Rust's `HashMap` have their own layouts and hash functions. Otter's
  slice grows by doubling from four; Go and Rust use their own growth
  policies. The row measures each language's implementation, not an algorithm
  race.
- **Threads are not goroutines.** Otter spawns an OS thread and blocks on
  condition variables; Go multiplexes a goroutine onto its runtime scheduler.
  The comparison is useful as a cost check, not as a claim that the two are the
  same mechanism. Mutex rows differ too: Otter takes the OS mutex immediately,
  while Go's `sync.Mutex` spins briefly before parking.
- **`ns/op` counts the unit in the `Workload` column**: one C call, one slice
  element, one map insert-or-lookup, one enum iteration, one channel round
  trip, or one lock/unlock pair. Compare rows within a benchmark, not across
  benchmarks.

### Results

`ns/op` tables, Windows x86-64, same machine and toolchains as the scalar
results. Each row is the better of two passes; a missing row was blocked by
Application Control (Device Guard) and is reported as such by the script
rather than timed.

#### FFI — 100,000,000 `getpid` calls

| Language | min ms | median ms | ns/call |
|---|---:|---:|---:|
| C (clang -O2) | 237.0 | 247.4 | 2.37 |
| C (gcc -O2) | 239.6 | 249.6 | 2.40 |
| Otter -O2 | 247.8 | 267.7 | 2.48 |

`getpid` is opaque to the optimizer, so every iteration is a real call; the
loop cannot be folded away, which an earlier draft using `llabs` was. Otter is
4.6% behind clang and 3.4% behind gcc, all in the same band. There is no thunk
or wrapper in that path: an `extern` call is lowered as an ordinary LLVM call,
with the C ABI's argument and return conventions.

#### Slices — 10,000,000 appends, then a sum

| Language | min ms | median ms | ns/element |
|---|---:|---:|---:|
| C (clang -O2) | 141.1 | 143.8 | 14.11 |
| C (gcc -O2) | 143.3 | 146.1 | 14.33 |
| Rust (rustc -O) | 159.4 | 165.6 | 15.94 |
| Otter -O2 | 207.2 | 218.9 | 20.72 |
| Go (go build) | 258.2 | 294.8 | 25.82 |

Otter's `push` is an out-of-line runtime call: check capacity, maybe grow and
copy, then `memcpy` the element. The C loop inlines the common case and only
calls `realloc` every doubling, and Rust's `Vec::push` inlines its fast path;
that is the 1.5× gap to C and 1.3× to Rust. Go's `append` is slower here than
Otter's push, despite the same doubling idea. The 10,000,000-element array is
about 80 MB, so allocation and the growth copies are part of the measurement,
not background noise.

#### Maps — 1,000,000 inserts and 1,000,000 lookups

| Language | min ms | median ms | ns/operation |
|---|---:|---:|---:|
| Rust (rustc -O) | 281.5 | 321.1 | 140.75 |
| Otter -O2 | 322.1 | 329.9 | 161.05 |
| Go (go build) | 370.7 | 420.0 | 185.35 |

Otter's map is an open-addressed table with linear probing that grows at 70%
occupancy; Rust and Go have their own layouts and hash functions. On this
integer-key workload Otter is 14% behind Rust and 15% ahead of Go. Nothing is
preallocated: the inserts include every rehash. A `string`-key workload would
measure the hash function and key retention instead, so this table is about
`i64` keys only.

#### Enums — 50,000,000 constructions and matches

| Language | min ms | median ms | ns/iteration |
|---|---:|---:|---:|
| C (clang -O2) | 101.2 | 130.2 | 2.02 |
| C (gcc -O2) | 120.4 | 129.4 | 2.41 |
| Otter -O2 | 150.1 | 166.6 | 3.00 |

Both sides build a 16-byte tagged value and switch on the tag. Otter's enum is
`{ i8, i64 }` with the tag in field 0 and the payload in field 1; the C struct
uses an `int` tag. The 1.5× gap is LLVM's handling of the aggregate through the
two calls, not a different data representation: the value does not allocate and
the match has no fallback path.

#### Channels — 200,000 round trips

| Language | min ms | median ms | ns/round trip |
|---|---:|---:|---:|
| Otter -O2 | 179.3 | 253.0 | 896.5 |
| Go (go build) | 296.2 | 318.1 | 1481.0 |

One capacity-one channel in each direction, so the two sides alternate strictly
and every send waits for the matching receive. Otter pays for two condition
variables and an OS thread; Go pays for a goroutine wakeup in its scheduler.
The minimum and median disagree by 40% on the Otter row, which is scheduling
noise, not a bimodal implementation — the earlier 5 × 2 run had Otter at
1822 ns and Go at 1764 ns on the same binaries. Treat this row as "the same
order", not as a scheduler comparison.

#### Mutexes — 1,000,000 contended lock/unlock pairs

| Language | min ms | median ms | ns/pair |
|---|---:|---:|---:|
| Go (go build) | 65.6 | 67.4 | 65.60 |
| Otter -O2 | 73.5 | 84.5 | 73.50 |

Four threads and four goroutines, an empty critical section, one million pairs
in total. Otter takes the OS mutex immediately; Go's `sync.Mutex` spins briefly
before parking. The 12% gap is what that difference and the lock implementation
cost under contention. A shared counter cannot be written in Otter user code
yet: a `*mut` may not cross a thread boundary, so the runtime's own C test
covers that case.

When these numbers change, update this file and the `Speed` section of the
top-level `README.md` in the same commit, so the two cannot drift.
