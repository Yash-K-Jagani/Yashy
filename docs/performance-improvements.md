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

Two items from the list below were implemented, both because the measurement
that the list asked for came back decisive.

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

## Suggested work

### 3. Build the IR in one buffer instead of one `String` per line — measured, not worth doing

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

### 4. Intern identifiers and type spellings — parser, sema, codegen, large

`FnCtx::locals` is a `HashMap<String, (String, TypeKind)>` looked up and cloned
on every identifier, sema clones `Type` (which may own a `Vec`), and type
spellings like `i64` are re-formatted per instruction. Interned `Symbol` ids
would remove the string work from all three stages at once.

This is the item with the largest gap between effort and current payoff: the
whole front end is a few milliseconds for a small program, so nothing here is
*currently* worth doing for speed. It becomes worth doing when the compiler is
pointed at programs of the size that make self-hosting (M10) plausible, so
schedule it with that, not with M3.

### 5. Replace the recursion-depth cap with an iterative parser — parser, large

`MAX_NESTING_DEPTH` is `48`. That is not a design choice; it is the measured
point below which a debug build stops overflowing a 1 MiB stack. Deeply nested
expressions are rejected with a diagnostic, which is safe but is also a
capability ceiling the language will eventually have to remove. A large-stack
worker thread is the cheap version; an explicit stack in the parser is the real
one.

### 6. Lower `switch` to an LLVM `switch` instead of a chain of `icmp` — codegen, small

`patterns.rs` emits one `icmp` per arm (its own module comment shows the
shape). For `Option` and `Result` this is one comparison and is not worth
touching. For a user enum with fifty variants it is fifty. LLVM has a pass that
recovers a jump table from an `icmp` chain, so the real cost may be near zero —
measure with a wide enum before changing anything.

### 7. Offer `-march=native` as an opt-in, never a default — driver, small

The benchmarks currently compare portable codegen. A `--native` flag would let
the suite show what the same programs do with the full instruction set, without
making non-portable binaries the default.

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

## Gaps in the tooling, not in the code

- **Timing a command that writes a lot to stdout through PowerShell times
  PowerShell, not the command.** Piping `otter build --emit-ir` into `Out-Null`
  reported 2,492 ms for a 3,000-function program — PowerShell materialises one
  object per output line. The same command redirected with `cmd /c … > nul`
  reported 279 ms. That 9× artifact briefly looked like a quadratic codegen bug.
  Whenever a command's stdout is large, discard it through `cmd` or redirect it
  to a file.
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
from 402.9 ms to 282.8 ms, within 8% of `clang -O2` on the same C file. The
remaining large performance work for Otter is not in the front end at all: it is
the memory system (M3), the cost that checked arithmetic adds (M4), and escape
analysis (M9). This document lists the front-end items for completeness, but
nothing here should be done before measuring, and the measurements so far say the
front end is not where the time goes.
