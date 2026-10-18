# Performance improvements

This document records what was measured, what was fixed, and what is worth
doing next. It is ordered by (confidence × impact) ÷ effort, not by ambition.

The measure-first rule applies throughout: every item below states the evidence
that it matters, and items whose magnitude is unknown say so explicitly rather
than being presented as wins.

## The defect that mattered

`otter build` invoked `clang` without an optimization flag, which means `-O0`.
Every binary Otter produced ran unoptimized, including the runtime C source
compiled at link time. The compiler's design puts *all* optimization in LLVM —
`otter-codegen` deliberately emits readable, unoptimized IR — so omitting the
flag silently discarded the entire performance strategy of the language.

The fix is `OptLevel` in `crates/otter-driver/src/pipeline.rs`, defaulting to
`-O2`, with `-O0`…`-O3` selectable on the command line. The benchmark suite
builds Otter at both `-O0` and `-O2` so the difference is measured rather than
asserted; see [`../benchmarks/README.md`](../benchmarks/README.md).

A second defect surfaced while building the suite: `1.0 + 2.0` lowered to
`add double`, and `3.0 * 4.0` to `mul double`. `add` and `mul` are LLVM's
*integer* opcodes, so the emitted IR was rejected by `clang` outright — nothing
had ever compiled an `f64` arithmetic expression end to end. Worse, the golden
file `crates/otter-codegen/tests/golden/ops.expected.ll` had **pinned the
invalid IR as the expected contract**, so the test that should have caught it
was actively protecting it. Both are fixed, and the golden file now matches
valid IR.

That is the lesson worth carrying forward, and it is why item 9 below exists: a
test that compares generated text against a recorded expectation cannot tell
whether the text is *valid*, let alone fast.

## What was done

Three items were implemented. The first two make Otter *compile* faster; the
third makes the programs it produces *run* faster, and it is the largest single
result in this document.

### 1. Hoist every `alloca` into the entry block — done

`alloca` instructions were emitted at the current insertion point:
`expr_gen.rs` (a binding's slot), `patterns.rs` (a match result and its
bindings), and `stmt_gen.rs` (a `for` loop's limit and index). A binding or loop
declared *inside* another loop therefore emitted a fresh `alloca` in a block that
runs once per iteration.

**Evidence it was real.** This program

```otter
fn f(n: i64) { for i in 0..n { for j in 0..n { var x = i * j; } } }
```

put the inner loop's counter slots in the *outer* loop's body block — `%slot3`
and `%slot4` were allocated in `bb1_body`, once per outer iteration. The `-O0`
IR, which is kept readable and close to the source on purpose, was describing a
dynamic allocation where the source has a static one.

**What changed.** `FnCtx` buffers the allocas it needs (`push_alloca`) and the
`begin_entry` / `finish` pair splices them in directly after the `entry:` label,
so no block other than `entry` ever allocates. The invariant is pinned by
`every_alloca_lands_in_the_entry_block`, which asserts that no `alloca` line
appears after any label, rather than by a golden file that a regeneration would
quietly update.

**Evidence it is safe.** Every slot is addressed only by its own `store` and
`load` — no slot's address escapes, checked across all three benchmarks and the
whole golden corpus — so `mem2reg` promotes them at `-O2` and the optimized code
is unchanged. Re-measuring the three benchmarks after the change reproduced the
runtime table below within run-to-run variation (fib `-O2` 79.2 → 87.8 ms, loop
`-O2` 246.2 → 261.1 ms, float `-O2` 566.5 → 539.6 ms). Those `-O2` rows cannot
move by construction; the movement is the machine, not the change.

### 2. Compile the runtime once per toolchain, not once per build — done

The link step passed the runtime's C source to `clang` along with the object
file, so **every** `otter build` recompiled an identical `runtime.c`. The item
said to measure that invocation before doing anything, and the answer was
decisive: it was **135 ms of a 403 ms build**, a third of every build, to
recompile a file whose bytes never change.

| step | ms |
| --- | ---: |
| front end (lex, parse, check, emit IR) | 21.8 |
| generated IR → object | 103.0 |
| **compile `runtime.c`** | **135.0** |
| link | 159.2 |
| **total** | **402.9** |

**What changed.** `cached_runtime_object` compiles the runtime to
`<cache>/otter-runtime-<hash>.o` on first use and links that object afterwards.
The hash covers the runtime bytes, the header, the target triple, the
optimization level, and the toolchain's path and modification time, so a changed
runtime, a different target, a different `-O`, or an upgraded `clang` each
address a different object — the hash *is* the staleness check, which is what
keeps the original "no archive can go stale" decision intact. The object is
written to a staged name and renamed into place, so an interrupted build cannot
leave a truncated object behind. `OTTER_CACHE_DIR` relocates the cache. Any
failure — an unwritable directory, a toolchain that will not compile the runtime
on its own — returns `None` and the build falls back to the previous link,
because a cache that can fail a build is worse than no cache.

| build | before | after |
| --- | ---: | ---: |
| first build on a machine (cold cache) | 402.9 ms | 517.6 ms |
| every build after that | 402.9 ms | **282.8 ms** |

The cold path is 115 ms slower because it compiles the runtime and links it in
two invocations where the old path did both in one; that is paid once per
(toolchain, optimization level) pair. The steady state is **30% faster**, and it
matters for how the language is perceived: `otter build` is now within 8% of
`clang -O2` on the equivalent C file (282.8 ms against 263.0 ms), where it was
53% slower (402.9 against 263.0).

**Verified neutral for the output.** The cached path and the fallback path
produce byte-identical executables (same SHA-256), so this is purely a
build-time change.

### 3. Offer `--native` as an opt-in, never a default — done

The only change so far that makes a *program* run faster rather than making the
compiler compile faster. `--native` adds `-march=native` when the user's IR is
compiled, so the backend may use the CPU it is actually running on instead of
the x86-64 baseline.

**Measured effect.** The same IR, the flag the only difference, from the
benchmark suite's own run:

| Benchmark | portable `-O2` | `-O2 --native` | Speedup |
| --- | ---: | ---: | ---: |
| `fib` | 90.8 ms | 84.6 ms | 1.07× |
| `loop` | 252.2 ms | 68.4 ms | 3.69× |
| `float` | 535.5 ms | 542.2 ms | 0.99× |

`loop` is the result: a **3.7× speed-up**, and the fastest native row in the
whole table. It is a vectorisable accumulator — the baseline is allowed to add
two `i64` per instruction and AVX2 adds four — and the C reference built with the
same flag lands in the same place (`clang -O2 -march=native`, 74.9 ms), which is
the point: what is being unlocked is the instruction set, not Otter. The other
two rows are unmoved because there is nothing to unlock — `float` is a serial
dependency chain that cannot be vectorised, and `fib` is calls rather than
arithmetic. A separate interleaved 21-run measurement of `fib` put `-O2` at
78.3 ms and `--native` at 79.0 ms, i.e. the 1.07× above is run-to-run variation
and not a real gain.

**Why the runtime stays portable.** The flag is applied to the generated IR and
to nothing else. The runtime is a few `printf` wrappers called once per program,
so the flag would buy it nothing measurable, while leaving it on the baseline
means one cached runtime object serves both portable and native builds —
enabling `--native` does not throw the cache away, and a cache shared between
machines never hands one machine another machine's instructions.

**Why it is not the default.** A binary built for one CPU dies with an illegal
instruction on an older one. The default has to run on a machine the compiler has
never seen, so `--native` is opt-in and the portable numbers stay the headline.

Two tests pin it: `only_the_native_target_un_conserves_the_instruction_set` for
the flag itself, and the end-to-end
`native_optimization_does_not_change_what_the_program_prints`, which builds the
same `f64` program both ways and requires the identical printed value.

## Measured and deliberately not done

Each of these was measured in the form it would ship, and the measurement said
no. They are recorded so the same idea is not re-evaluated from scratch, and so
that "not done" is visibly a decision rather than an oversight.

### 1. Build the IR in one buffer instead of one `String` per line — no

`Module::push` takes a `String` per instruction and `finish` joins them, so a
module allocates once per emitted instruction. A single `String` with `write!`
would remove that. The item said to measure with a large generated program
before deciding, so here is that measurement — `otter build --emit-ir` on
generated sources of *n* small functions, output discarded, best of 3:

| functions | IR lines | front end | front end + codegen | codegen |
| ---: | ---: | ---: | ---: | ---: |
| 1,500 | 55,515 | 101 ms | 148 ms | 47 ms |
| 3,000 | 111,015 | 146 ms | 279 ms | 133 ms |
| 6,000 | 222,015 | 227 ms | 422 ms | 195 ms |

Both stages are linear in program size, at roughly a microsecond per emitted IR
line. A 5 MB module costs about 200 ms of codegen on top of 230 ms for the front
end — real, but not the shape of a bottleneck, and the whole measurement is
smaller than one `clang` invocation. Not worth a rewrite; revisit only if
compile time becomes a complaint at that scale.

### 2. Make `-O3` the default — no

`-O3` is already selectable; the question was only whether the default should
move from `-O2`. Same IR, same machine, both levels, 21 timed runs *interleaved*
so the machine's drift hits each level equally, on `fib`:

| level | min | p25 | median | p75 |
| --- | ---: | ---: | ---: | ---: |
| `-O2` | 78.3 ms | 80.9 ms | 88.2 ms | 97.8 ms |
| `-O3` | 79.3 ms | 81.5 ms | 83.4 ms | 93.2 ms |

The medians favour `-O3` and the minimum favours `-O2`. `loop` moved 2.5% in one
run and not at all in the next; `float` was identical. That is a coin flip with a
table attached, and a default changed on a coin flip is indistinguishable from a
default changed on vibes — so `-O2` stays, one argument away from anyone who
wants to measure their own program.

### 3. Contract `a * b + c` into an FMA — no

On the `float` benchmark this is worth **1.7×** (`clang -O2 -march=native` runs
it in 311 ms against Otter's 536 ms) — and it is visible in the output. The FMA
row prints `44051.909807502794` where every other row prints
`44051.909822450085`, because a fused multiply-add rounds once instead of twice.
Every C compiler does this by default, and it is within the letter of IEEE-754.
Otter still will not, because it changes the number a program prints, and a
benchmark allowed to change the answer in order to be faster is not a benchmark.

The small version of it does not work here either: adding LLVM's `contract` flag
to the emitted `fmul`/`fadd` was tried, and the backend still produced
`vmulsd`/`vaddsd`. `clang` gets an FMA by emitting an explicit `llvm.fmuladd`
intrinsic from its front end, not by a backend pass, so taking it would mean
teaching `otter-codegen` a fast-math rule. That is a language-semantics decision
belonging to a `--fast-math` flag, not a performance tweak.

### 4. Link with `lld` — no

The link is 158.5 ms of a 283 ms build, so it was worth asking whether LLVM's own
linker is faster than the GNU `ld` that `clang` picks up from MSYS2. It is
barely: 152.4 ms, a 4% difference that is inside the spread of repeated identical
links, in exchange for depending on a linker that is not guaranteed to be
installed. `-fuse-ld=lld` stays unset.

## Suggested work

### 5. Intern identifiers and type spellings — parser, sema, codegen, large

`FnCtx::locals` is a `HashMap<String, (String, TypeKind)>` looked up and cloned
on every identifier, sema clones `Type` (which may own a `Vec`), and type
spellings like `i64` are re-formatted per instruction. Interned `Symbol` ids
would remove the string work from all three stages at once.

This is the item with the largest gap between effort and current payoff: the
whole front end is a few milliseconds for a small program, so nothing here is
*currently* worth doing for speed. It becomes worth doing when the compiler is
pointed at programs of the size that make self-hosting (M10) plausible, so
schedule it with that, not with M3.

### 6. Replace the recursion-depth cap with an iterative parser — parser, large

`MAX_NESTING_DEPTH` is `48`. That is not a design choice; it is the measured
point below which a debug build stops overflowing a 1 MiB stack. Deeply nested
expressions are rejected with a diagnostic, which is safe but is also a
capability ceiling the language will eventually have to remove. A large-stack
worker thread is the cheap version; an explicit stack in the parser is the real
one.

### 7. Lower `switch` to an LLVM `switch` instead of a chain of `icmp` — codegen, small

`patterns.rs` emits one `icmp` per arm (its own module comment shows the
shape). For `Option` and `Result` this is one comparison and is not worth
touching. For a user enum with fifty variants it is fifty. LLVM has a pass that
recovers a jump table from an `icmp` chain, so the real cost may be near zero —
measure with a wide enum before changing anything.

### 8. Re-measure when checked arithmetic lands — M4, process

The language contract says integer overflow panics (§6.1 of the design spec);
at M0–M2 it wraps, and the benchmarks depend on wrapping. M4 will add the
checks, and Otter's `loop` row will get slower. Having the before-number is
what turns that from a surprise into a measured cost.

### 9. Make the benchmarks a CI gate with stored baselines — process

The f64 defect above was found by a human running a new program, not by the
test suite, because the suite compared IR text against a recorded expectation
and never asked whether the text was valid. Two additions close that class of
hole: run every benchmark program through `clang` in CI (which the end-to-end
suite now does for `floats.ot`), and store a baseline so a codegen regression
shows up as a number that moved.

### 10. Add size and speed checks for aggregate passing — codegen, unknown

`struct` values are LLVM aggregate types passed and returned by value
(`%otter_str = type { i32, i32, i8* }`). LLVM's SROA splits small ones into
scalars, which is why this has not hurt yet. Once `struct`s can be large (M6)
the right shape is `byval`/`sret` pointers, or emitting scalars directly. No
evidence yet that it matters; no struct-heavy benchmark exists.

### 5. Delete the `cc` build of the runtime from `otter-runtime` — no

`otter-runtime`'s `build.rs` compiles `runtime.c` with the `cc` crate at *cargo*
build time, and the driver never calls those symbols — it embeds the C source and
hands it to `clang`. That looks like dead weight in every `cargo build`, and it
was the next thing to delete. It is not dead: `crates/otter-runtime/tests/print.rs`
is a `harness = false` test binary that calls `otter_print_str`/`_i64`/`_f64`
through the `extern` declarations in `lib.rs`, so the static library this
`build.rs` produces is what links those calls. Removing it would not speed the
driver up at all; it would break the only test that exercises the runtime's
formatting. Left alone.

## Gaps in the tooling, not in the code

- **Timing a command that writes a lot to stdout through PowerShell times
  PowerShell, not the command.** Piping `otter build --emit-ir` into `Out-Null`
  reported 2,492 ms for a 3,000-function program — PowerShell materialises one
  object per output line. The same command redirected with `cmd /c … > nul`
  reported 279 ms. That 9× artifact briefly looked like a quadratic codegen bug.
  Whenever a command's stdout is large, discard it through `cmd` or redirect it
  to a file.
- **PowerShell's `Measure` is an alias for `Measure-Object`.** A timing helper
  named `Measure` in a probe script resolves to the cmdlet instead, which fails on
  any parameter it does not recognise; the timing rows then come back *blank*
  rather than wrong, so it is easy to miss. Name timing helpers `Time-Runs` or
  similar.
- **An Application Control (Device Guard) policy on this machine blocks a
  content-defined subset of newly built executables.** A binary is refused with
  "blocked by your organization's Device Guard policy" no matter where it is
  copied to, and changing a single byte flips the verdict — so it is a hash
  decision, not a complaint about the binary. Because the linker stamps a fresh
  timestamp into every build, the *same source* gets a different verdict from one
  build to the next, which makes any test that runs what it just built
  intermittently red for reasons that have nothing to do with the compiler. The
  same policy is why `cargo clippy` cannot run here. Treat a blocked `spawn` as
  an environment result, not a test failure, and rebuild when a benchmark binary
  is refused.
- **`cargo clippy` cannot run in this environment.** An Application Control
  policy blocks `cargo-clippy` (`os error 4551`). The gates used instead are
  `cargo check --workspace --all-targets` and `cargo test --workspace`. Restoring
  clippy would add a lint pass this project currently does not have.
- **`cargo-fuzz` and a nightly toolchain are not installed**, so the targets in
  `fuzz/` have never been executed. The deterministic no-panic suite
  (`crates/otter-driver/tests/never_panics.rs`) stands in: it runs several
  thousand malformed and mutated inputs in well under a second and is the reason
  the never-panics criterion is verified at all rather than merely intended.

## The honest summary

The compiler is not the bottleneck. For a small program, lexing through IR
emission is about 20 ms — and it is linear in program size, so it stays
proportional. The build's real cost was the second `clang` invocation
recompiling a runtime that never changes; caching it took a steady-state build
from 402.9 ms to 282.8 ms, within 8% of `clang -O2` on the same C file.

The largest single result in this document is not a build-time number at all.
`--native` takes the `loop` benchmark from 252 ms to 68 ms — a 3.7× speed-up — by
letting the backend use AVX2 on the machine that compiles the program. It is
opt-in because it costs portability, and it is honest because the C reference,
given the same flag, lands in the same place: the flag is what is being
unlocked, not Otter.

What is left is not in the front end. It is the memory system (M3), the cost
checked arithmetic adds (M4), and escape analysis (M9). This document lists the
front-end items for completeness, but nothing here should be done before
measuring, and the measurements so far say the front end is not where the time
goes.
