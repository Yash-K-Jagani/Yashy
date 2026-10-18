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
a fast-math flag, and every language computes bit-identical results.

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
  no profile-guided optimization and no relaxed floating-point flags. Absolute
  times will differ on your hardware; the ratios should be stable.

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
| C (gcc -O2) | 49.0 | 50.1 | 1.641 |
| **Otter (clang -O2)** | **79.2** | **80.7** | **2.652** |
| Rust (rustc -O) | 87.0 | 100.1 | 2.914 |
| C (clang -O2) | 90.9 | 95.8 | 3.044 |
| Otter (clang -O0) | 139.8 | 144.0 | 4.682 |
| Go (go build) | 141.3 | 144.4 | 4.732 |
| Node 24 | 341.3 | 380.7 | 11.430 |
| CPython 3.14 | 3525.7 | 3525.9 | 118.072 |

### `loop` — 200,000,000 iterations

| Language | min ms | median ms | ns/iteration |
|---|---:|---:|---:|
| C (gcc -O2) | 236.7 | 239.9 | 1.184 |
| **Otter (clang -O2)** | **246.2** | **252.7** | **1.231** |
| C (clang -O2) | 250.7 | 252.5 | 1.254 |
| Rust (rustc -O) | 259.1 | 261.2 | 1.296 |
| Go (go build) | 298.2 | 318.1 | 1.491 |
| Node 24 | 707.8 | 709.6 | 3.539 |
| Otter (clang -O0) | 1535.6 | 1571.1 | 7.678 |
| CPython 3.14 | 35183.0 | 36450.3 | 175.915 |

Node's total is `29999999922509360`, not `29999999900000000`: JavaScript has no
64-bit integers and this sum exceeds 2^53. See the caveats above.

### `float` — 100,000,000 iterations

| Language | min ms | median ms | ns/iteration |
|---|---:|---:|---:|
| C (clang -O2) | 540.5 | 544.3 | 5.405 |
| Go (go build) | 549.0 | 552.0 | 5.490 |
| Rust (rustc -O) | 553.7 | 581.0 | 5.537 |
| C (gcc -O2) | 560.2 | 582.9 | 5.602 |
| **Otter (clang -O2)** | **566.5** | **577.0** | **5.665** |
| Node 24 | 654.7 | 654.9 | 6.547 |
| Otter (clang -O0) | 952.5 | 953.7 | 9.525 |
| CPython 3.14 | 9585.7 | 10077.7 | 95.857 |

### The effect of the optimization level

Otter built at `-O0` and at `-O2`, same source, same machine:

| Benchmark | `-O0` | `-O2` | Speedup |
|---|---:|---:|---:|
| `fib` | 139.8 ms | 79.2 ms | 1.77× |
| `loop` | 1535.6 ms | 246.2 ms | 6.24× |
| `float` | 952.5 ms | 566.5 ms | 1.68× |

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
once per toolchain and optimization level. See item 2 in
`../docs/performance-improvements.md`.

When these numbers change, update this file and the `Speed` section of the
top-level `README.md` in the same commit, so the two cannot drift.
