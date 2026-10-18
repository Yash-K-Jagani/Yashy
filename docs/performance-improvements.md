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

## Suggested work

### 1. Hoist every `alloca` into the entry block — codegen, small

`alloca` instructions are emitted at the current insertion point:
`expr_gen.rs:203` (a binding's slot), `patterns.rs:53,170,193` (a match
result and its bindings), and `stmt_gen.rs:239,242` (a `for` loop's limit and
index). A binding or loop declared *inside* another loop therefore emits a
fresh `alloca` in a block that runs once per iteration.

Nothing is wrong with the generated machine code today, because LLVM's
`mem2reg` and SROA hoist those allocas — but that is LLVM's guarantee, not the
compiler's, and it is not free even when it works. The evidence that this is a
real problem is that `fib.ot` carries a comment explaining that its accumulator
must be declared *before* the loop, because declaring it inside would leak a
stack slot per iteration. That is a workaround in user-facing example code for a
compiler bug.

The fix is contained: collect allocas in a side buffer while lowering a
function and splice them at the top of the entry block. The benefit is that the
`-O0` IR — which is kept readable on purpose — stops being wrong, and the
user-visible workaround disappears.

### 2. Compile the runtime once per toolchain, not once per build — driver, moderate

The link step passes the runtime's C source to `clang` along with the object
file, so **every** `otter build` recompiles an identical `runtime.c`. The plan
chose this deliberately (`pipeline.rs`, "Where the runtime comes from") so no
prebuilt archive can go stale — a good decision. A content-addressed cache keeps
the good property while removing the cost: hash the runtime source, the target
triple and the optimization level, compile to an object in a cache directory on
first use, and link the cached object afterwards. The hash *is* the staleness
check.

Measure the second `clang` invocation before doing this; it is one small C file,
and if it turns out to be a few milliseconds the change is not worth its
complexity.

### 3. Build the IR in one buffer instead of one `String` per line — codegen, small

`Module::push` takes a `String` per instruction and `finish` joins them, so a
module allocates once per emitted instruction. A single `String` with `write!`
would remove that. The magnitude is unknown; measure with a large generated
program (`otter build --emit-ir` on a 10,000-statement file) before deciding.

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
emission is a few milliseconds and `clang` is the rest — the same cost C pays
for the same work. The remaining large performance work for Otter is not in the
front end at all: it is the memory system (M3), the cost that checked arithmetic
adds (M4), and escape analysis (M9). This document lists the front-end items for
completeness, but nothing here should be done before measuring, and the
measurements so far say the front end is not where the time goes.
