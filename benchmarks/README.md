# Otter benchmarks

Three small programs, written in six languages, used to answer two questions:

1. **How fast is the code Otter produces?** — the runtime table.
2. **How fast does Otter compile it?** — the compile-time table.

Everything here is reproducible with one command. Run it from the repository
root after `cargo build --release --workspace`:

```powershell
pwsh benchmarks/run.ps1
```

## The programs

| Program | Source | Measures |
|---|---|---|
| `fib` | recursive `fib(35)` | function calls, call overhead, how well recursion is optimised |
| `loop` | 200,000,000 iterations of an integer loop with a data-dependent branch | loop code, integer arithmetic, branch handling |
| `float` | 100,000,000 iterations of `x = x * a + b` | `f64` arithmetic on a serial dependency chain |

Each is deliberately small enough that the *whole* generated program fits in a
few dozen instructions. That is the point: any difference between two rows is a
difference in the code the compiler emitted, not in library code.

`fib` cannot be flattened by an optimiser. `loop`'s accumulator is a sum with a
branch, which resists collapsing to a closed form while keeping every
intermediate value well inside `i64` — no language's overflow rule changes the
answer, and C's signed-overflow undefined behaviour never fires. `float` is a
strict dependency chain, so no compiler may reassociate or vectorise it without
a fast-math flag, and every portable row computes bit-identical results. The one
row that does not is `float` built with `-march=native`: clang fuses `x * a + b`
into a single FMA there, which rounds once instead of twice and moves the last
six digits. It is labelled, and it is discussed separately below.

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
- **Nothing allocates.** M0–M2 has no heap, no strings beyond literals, no
  collections and no refcounting. These benchmarks therefore exercise none of
  the costs a real program pays: allocation, deallocation, string handling,
  bounds checks. They are a lower bound on the work to come, not a preview of
  it.
- **Checked arithmetic is not implemented yet.** The language contract says
  integer overflow panics (§6.1 of the design spec). At M0–M2 it wraps, and
  these benchmarks rely on wrapping. When M4 adds the checks, expect the `loop`
  row for Otter to get slower — that will be the price of the safety, and this
  suite is how it gets measured rather than guessed.
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
  enabling optimization actually buys instead of asserting that it helps.
- **`--native` rows are opt-in and always paired.** The Otter row and the clang
  row are built with the flag together, so the table shows what the *flag* buys
  rather than what Otter buys over a compiler that was not given it. They are
  never the headline; the portable rows are.
- **One untimed warm-up run per program**, then the reported number is the
  **minimum** of N runs. For a short-lived process the minimum is the run least
  disturbed by everything else on the machine, and it is far more stable than
  the mean. The median is printed beside it as a check on how noisy the run was.
  A large min/median gap means the machine was busy and the row should be
  re-measured.
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
```

Tested with clang 23.1.3, gcc 16.2.0 (MSYS2 UCRT64), rustc 1.99.1, go 1.27.0,
Node 24.20.0 and CPython 3.14.7 on Windows x86-64.

## Results

Windows x86-64, 13th Gen Intel(R) Core(TM) i5-13420H. clang 23.1.3, gcc 16.2.0
(MSYS2 UCRT64), rustc 1.99.1, go 1.27.0, Node 24.20.0, CPython 3.14.7. Best of 5
runs for the native rows, best of 3 for the interpreted rows, each after one
untimed warm-up.

### `fib(35)` — about 30 million calls

| Language | min ms | median ms | ns/call |
|---|---:|---:|---:|
| C (gcc -O2) | 50.7 | 62.6 | 1.698 |
| C (clang -O2) | 80.1 | 91.4 | 2.682 |
| C (clang -O2, `-march=native`) | 84.1 | 90.1 | 2.816 |
| Otter (`-O2 --native`) | 84.6 | 87.7 | 2.833 |
| Rust (rustc -O) | 88.3 | 90.5 | 2.957 |
| **Otter (clang -O2)** | **90.8** | **96.0** | **3.041** |
| Go (go build) | 144.8 | 152.5 | 4.849 |
| Otter (clang -O0) | 153.2 | 161.4 | 5.130 |
| Node 24 | 392.6 | 395.3 | 13.148 |
| CPython 3.14 | 3525.6 | 3531.6 | 118.068 |

`gcc` is consistently the fastest here and the reason is not codegen quality: it
inlines across the recursive call differently. Between `clang`, Rust and Otter
the spread is about 10%, and which of the three wins moves from run to run —
`fib` is a small function called 30 million times, so it is dominated by how the
recursion is inlined and by call overhead, not by the quality of the surrounding
IR.

### `loop` — 200,000,000 iterations

| Language | min ms | median ms | ns/iteration |
|---|---:|---:|---:|
| Otter (`-O2 --native`) | 68.4 | 68.9 | 0.342 |
| C (clang -O2, `-march=native`) | 74.9 | 75.7 | 0.374 |
| C (gcc -O2) | 243.1 | 251.4 | 1.216 |
| **Otter (clang -O2)** | **252.2** | **267.7** | **1.261** |
| C (clang -O2) | 259.2 | 283.1 | 1.296 |
| Rust (rustc -O) | 261.3 | 277.6 | 1.306 |
| Go (go build) | 286.1 | 321.9 | 1.430 |
| Node 24 | 671.0 | 723.8 | 3.355 |
| Otter (clang -O0) | 1529.0 | 1544.5 | 7.645 |
| CPython 3.14 | 35215.5 | 35771.9 | 176.078 |

Node's total is `29999999922509360`, not `29999999900000000`: JavaScript has no
64-bit integers and this sum exceeds 2^53. See the caveats above.

The two `-march=native` rows are three to four times faster than everything else
in the table, and that is the whole `loop` story: the accumulator is vectorisable,
the x86-64 baseline is not allowed to use AVX2, and the moment the target permits
it four `i64` additions happen per instruction instead of two. Otter's row is
ahead of clang's here by 9%; on a benchmark this size that is closer to a tie than
a result, and the honest reading is that the two moved together.

### `float` — 100,000,000 iterations

| Language | min ms | median ms | ns/iteration |
|---|---:|---:|---:|
| C (clang -O2, `-march=native`) | 311.5 | 325.3 | 3.115 |
| **Otter (clang -O2)** | **535.5** | **545.9** | **5.355** |
| C (gcc -O2) | 540.1 | 553.5 | 5.401 |
| C (clang -O2) | 541.8 | 549.2 | 5.418 |
| Otter (`-O2 --native`) | 542.2 | 553.2 | 5.422 |
| Rust (rustc -O) | 547.3 | 556.6 | 5.473 |
| Go (go build) | 561.6 | 570.8 | 5.616 |
| Node 24 | 681.6 | 692.1 | 6.816 |
| Otter (clang -O0) | 936.3 | 955.0 | 9.363 |
| CPython 3.14 | 9586.3 | 9628.1 | 95.863 |

The fastest row prints `44051.909807502794`; every other row prints
`44051.909822450085`. That is not a typo. `clang` contracts `x * a + b` into a
single fused multiply-add when the target has an FMA unit, which rounds once
instead of twice, and the last six digits are the difference. It is a 1.7× win
bought by changing the answer, and Otter does not take it — see below.

### The effect of `--native`

`--native` (clang's `-march=native`) stops targeting the x86-64 baseline and
targets the CPU that is running the build. The C row is built with the same flag,
because the flag — not the compiler — is what does the work.

| Benchmark | portable `-O2` | `-O2 --native` | Speedup |
|---|---:|---:|---:|
| `fib` | 90.8 ms | 84.6 ms | 1.07× |
| `loop` | 252.2 ms | 68.4 ms | 3.69× |
| `float` | 535.5 ms | 542.2 ms | 0.99× |

Two of the three rows say the flag is worth almost nothing, and one says it is
worth a great deal. That is the honest shape of it: `-march=native` unlocks AVX2
and the wider vector unit, so it pays exactly where the backend has something to
do with them and nowhere else. `float` is unaffected because its dependency
chain is serial — no vectorisation is possible — and `fib` is unaffected because
it is calls, not arithmetic.

Two things are deliberately *not* done, and both were measured:

- **The flag is not the default.** A binary built for one CPU dies with an
  illegal instruction on an older one. The default has to run on a machine the
  compiler has never seen, so `--native` is opt-in and the portable row stays the
  headline.
- **Otter does not contract float multiplies into FMAs.** It would win the
  `float` row outright (that is what clang's 311 ms above is), and it would be
  within the letter of IEEE-754 — a fused multiply-add rounds once, and every C
  compiler does it by default. But it changes the number a program prints, and
  the premise this suite is built on is that every row computes the same answer,
  so the one thing a benchmark *cannot* be allowed to buy is a different result.
  Even the LLVM `contract` flag on our own `fmul`/`fadd` does not fuse here (it
  was tried: the backend still emits `vmulsd`/`vaddsd`; clang gets an FMA by
  emitting an explicit `llvm.fmuladd` intrinsic from the front end), so taking it
  would mean teaching codegen a fast-math rule, not flipping a switch.

### The effect of the optimization level

Otter built at `-O0` and at `-O2`, same source, same machine:

| Benchmark | `-O0` | `-O2` | Speedup |
|---|---:|---:|---:|
| `fib` | 153.2 ms | 90.8 ms | 1.69× |
| `loop` | 1529.0 ms | 252.2 ms | 6.06× |
| `float` | 936.3 ms | 535.5 ms | 1.75× |

### Compile time (`fib`, best of 9)

| Toolchain | min ms | median ms |
|---|---:|---:|
| Otter (front end only, `--emit-ir`) | 20.0 | 21.3 |
| C (`clang -O2`) | 263.0 | 294.7 |
| Go (`go build`) | 266.0 | 281.2 |
| Otter (full build, `-O2`, warm cache) | 282.8 | 318.8 |
| Otter (full build, `-O2`, first build on a machine) | 517.6 | — |
| Rust (`rustc -O`) | 619.7 | 696.6 |

`otter build` used to be 415 ms here, and the gap to C's 263 ms was the second
`clang` invocation recompiling the runtime C source on every build — 135 ms of a
403 ms build. That object is now cached in a content-addressed directory, so a
steady-state build is within 8% of the same C file built by the same `clang`. The
first build on a machine is slower than before (518 vs 403 ms) because it now
compiles the runtime and links it in two invocations instead of one; that is paid
once per toolchain and optimization level. A cold *cache* is not the only cold
cost — the first build after the toolchain itself has not run is slower still, so
treat 518 ms as the measured case rather than as an upper bound. See item 2 in
`../docs/performance-improvements.md`.

When these numbers change, update this file and the `Speed` section of the
top-level `README.md` in the same commit, so the two cannot drift.
