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
a wide target — and every row built for the portable baseline computes
bit-identical results. The rows that do not are those built for a wider one:
`clang -march=x86-64-v3` fuses `x * a + b` into a single FMA, and Otter does the
same only under `--fast-math`. That turns two roundings into one and moves the
last six digits. Those rows are labelled, and they are discussed below.

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
| C (gcc -O2) | 48.7 | 49.8 | 1.631 |
| Otter (`-O2 --cpu=x86-64-v3`) | 76.4 | 80.5 | 2.559 |
| C (clang -O2, `-march=x86-64-v3`) | 77.0 | 79.8 | 2.579 |
| C (clang -O2, `-march=native`) | 78.1 | 80.3 | 2.615 |
| Otter (`-O2 --cpu=x86-64-v3 --fast-math`) | 78.1 | 81.7 | 2.615 |
| C (clang -O2) | 79.8 | 81.9 | 2.672 |
| **Otter (clang -O2)** | **81.2** | **84.3** | **2.719** |
| Otter (`-O2 --native`) | 82.4 | 89.2 | 2.759 |
| Rust (rustc -O) | 82.8 | 85.3 | 2.773 |
| Otter (clang -O0) | 140.3 | 144.0 | 4.698 |
| Go (go build) | 144.4 | 152.9 | 4.836 |
| Node 24 | 333.1 | 334.8 | 11.155 |
| CPython 3.14 | 2934.0 | 2938.1 | 98.256 |

`gcc` is consistently the fastest here and the reason is not codegen quality: it
inlines across the recursive call differently. Between `clang`, Rust and Otter
the spread is about 7%, and which of the three wins moves from run to run —
`fib` is a small function called 30 million times, so it is dominated by how the
recursion is inlined and by call overhead, not by the quality of the surrounding
IR. Otter's portable row is within 2% of `clang`'s and ahead of Rust's in this
run, which is the honest reading: a tie inside the noise.

### `loop` — 200,000,000 iterations

| Language | min ms | median ms | ns/iteration |
|---|---:|---:|---:|
| C (clang -O2, `-march=x86-64-v3`) | 60.6 | 62.9 | 0.303 |
| Otter (`-O2 --cpu=x86-64-v3`) | 61.8 | 64.1 | 0.309 |
| Otter (`-O2 --cpu=x86-64-v3 --fast-math`) | 62.4 | 63.1 | 0.312 |
| C (clang -O2, `-march=native`) | 68.7 | 73.1 | 0.344 |
| Otter (`-O2 --native`) | 69.4 | 75.1 | 0.347 |
| C (gcc -O2) | 239.8 | 249.4 | 1.199 |
| Rust (rustc -O) | 251.6 | 255.4 | 1.258 |
| **Otter (clang -O2)** | **253.0** | **257.3** | **1.265** |
| C (clang -O2) | 254.3 | 269.3 | 1.272 |
| Go (go build) | 269.9 | 280.4 | 1.350 |
| Node 24 | 616.2 | 622.9 | 3.081 |
| Otter (clang -O0) | 1475.5 | 1488.9 | 7.378 |
| CPython 3.14 | 50885.8 | 51133.0 | 254.429 |

Node's total is `29999999922509360`, not `29999999900000000`: JavaScript has no
64-bit integers and this sum exceeds 2^53. See the caveats above.

The whole `loop` story is the CPU tier: the accumulator is vectorisable, the
x86-64 baseline is not allowed to use AVX2, and the moment the target permits it
four `i64` additions happen per instruction instead of two — a 4.1× win, and the
largest effect anywhere in this suite. At the same tier Otter and clang are 62 vs
61 ms, effectively a tie; on the portable baseline they are 253 vs 254 ms, also a
tie. The tier, not the compiler, is what moves this row.

### `float` — 100,000,000 iterations

| Language | min ms | median ms | ns/iteration |
|---|---:|---:|---:|
| C (clang -O2, `-march=native`) | 307.3 | 312.8 | 3.073 |
| C (clang -O2, `-march=x86-64-v3`) | 310.5 | 314.3 | 3.105 |
| Otter (`-O2 --cpu=x86-64-v3 --fast-math`) | 311.6 | 312.5 | 3.116 |
| C (gcc -O2) | 523.8 | 529.1 | 5.238 |
| Otter (`-O2 --cpu=x86-64-v3`) | 526.6 | 531.8 | 5.266 |
| Otter (`-O2 --native`) | 528.5 | 531.8 | 5.285 |
| **Otter (clang -O2)** | **529.0** | **533.7** | **5.290** |
| Rust (rustc -O) | 530.3 | 534.8 | 5.303 |
| C (clang -O2) | 530.9 | 532.6 | 5.309 |
| Go (go build) | 537.8 | 541.0 | 5.378 |
| Node 24 | 644.2 | 647.2 | 6.442 |
| Otter (clang -O0) | 887.7 | 899.5 | 8.877 |
| CPython 3.14 | 12870.0 | 13021.5 | 128.700 |

The three fast rows print `44051.909807502794`; every other row prints
`44051.909822450085`. That is not a typo. `clang` contracts `x * a + b` into a
single fused multiply-add at `-march=x86-64-v3` (C contracts by default), and
Otter does the same under `--fast-math`. The FMA rounds once instead of twice, so
the last six digits differ — a 1.7× win bought by changing the answer. The two
fused rows land at 312 and 311 ms, which is the point: given the same target and
the same permission, Otter and clang produce the same speed. On the portable
baseline, where no row is allowed to fuse, Otter is 529 ms against clang's 531 ms
and gcc's 524 ms.

### The effect of the CPU tier

`--cpu=<tier>` stops targeting the x86-64 baseline and targets an instruction-set
level: `x86-64-v2`, `x86-64-v3`, `x86-64-v4`, or `native` (the CPU running the
build). Each C row is built with the matching `-march=`, because the flag — not
the compiler — is what does the work.

| Benchmark | portable `-O2` | `--cpu=x86-64-v3` | Speedup | `--native` | Speedup |
|---|---:|---:|---:|---:|---:|
| `fib` | 81.2 ms | 76.4 ms | 1.06× | 82.4 ms | 0.99× |
| `loop` | 253.0 ms | 61.8 ms | 4.09× | 69.4 ms | 3.65× |
| `float` | 529.0 ms | 526.6 ms | 1.00× | 528.5 ms | 1.00× |

Two of the three rows say the flag is worth almost nothing, and one says it is
worth a great deal. That is the honest shape of it: the wider target unlocks
AVX2 and the wider vector unit, so it pays exactly where the backend has
something to do with them and nowhere else. `float` is unaffected because its
dependency chain is serial — no vectorisation is possible — and `fib` is
unaffected because it is calls, not arithmetic. `x86-64-v3` happens to edge out
`native` on `loop` here; the two are within run-to-run noise of each other, and
the fixed tier is the one to quote because the same binary runs on every v3
machine.

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
| `fib` | 140.3 ms | 81.2 ms | 1.73× |
| `loop` | 1475.5 ms | 253.0 ms | 5.83× |
| `float` | 887.7 ms | 529.0 ms | 1.68× |

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

When these numbers change, update this file and the `Speed` section of the
top-level `README.md` in the same commit, so the two cannot drift.
