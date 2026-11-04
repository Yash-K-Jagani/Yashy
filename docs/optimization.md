# Optimization in Yashy

Yashy does not run LLVM passes of its own: the backend emits textual LLVM IR and hands it to `clang`. What the compiler does is *spend what the verifier proved*: delete checks the verifier discharged, promote allocations that never escape, and share identical string literals. Every one of those decisions is registered in a single pass manager and held to its own benchmark row, so no pass can quietly buy one row by selling another.

This page explains what Yashy proves, what LLVM decides, the registered passes, and how to read `--opt-report`.

## What Yashy proves vs. what LLVM decides

The verifier (phase 3) knows, per function, which pointer is the unique live reference, which value cannot be optional, which integer cannot overflow, and which index is inside its slice. Codegen turns those facts into:

- **Deleted checks.** A slice index the verifier proved in range is lowered without its runtime test, and a checked `+`/`-`/`*`/`-x` whose operand ranges fit is emitted as the bare operation with no branch. A fixed array's index keeps its test: the length is in the type, but the access still has to answer `None` out of range.
- **IR attributes.** `noalias`, `nonnull`, `readonly` and `align` on function parameters, traced one-to-one to `ParamFacts`. A fact that isn't there becomes no attribute — a missing fact can never become a wrong promise. `dereferenceable` is deliberately not emitted: it needs a pointer parameter that carries a length, and a slice is an aggregate, not a pointer.
- **Promotion.** An allocation that never escapes becomes a frame `alloca` or a block-arena slot instead of a runtime allocator call. Escape analysis reads the HIR and treats any reference it cannot follow as an escape; the verifier's facts are wired through the interface, but phase 3 records no "shared" fact yet, so a doubtful site stays on the heap.

What remains after all of that is LLVM's business: inlining, GVN, unrolling, vectorisation. Yashy never promises an `inline`, never reorders struct fields, and never grows a custom register allocator. `--opt-report` says what survived and why, so a programmer can add the contract clause that removes a check rather than guessing at backend flags.

## The registered passes

Every optimization the backend applies is one entry in a registry, run in one order before any IR is written. `--opt-list` prints that order and the bench row each pass is held to:

```text
$ yashy build --opt-list
pass     row
facts    fib_run_ms
escape   maps_run_ms
intern   hello_binary_bytes
```

`--no-opt=<pass>` disables exactly one of them for bisecting. None of them changes what a program prints, so a program that behaves differently with one pass off is a bug in that pass.

| Pass | What it decides | Held to |
|------|-----------------|---------|
| `facts` | Rely on the verifier's facts: delete proved checks, emit `noalias`, `nonnull`, `readonly`, `align`. Off means an empty fact set and attribute-free IR. | `fib_run_ms` |
| `escape` | Non-escaping allocations become frame slots or block-arena slots instead of allocator calls. | `maps_run_ms` |
| `intern` | Identical string literals share one global. Off emits one global per literal. | `hello_binary_bytes` |

Anything not in the table is mandatory lowering, not a pass: there is no switch that turns an expression into a call or a field into an offset, because there would be no program left to measure. `yashy_codegen::pass::owner` names the owner of each expression form, and its exhaustive match is the check that every construct has an owner — a new form fails to compile until that is decided.

The per-pass gate (`yashy-bench --per-pass`) compares the all-passes measurement with one taken with each pass disabled and fails when disabling a pass *improves* its own row by more than 2%. See [benchmarks/README.md](../benchmarks/README.md#the-per-pass-gate) for the command that produces the table.

## The optimization report

`--opt-report` prints why each check and allocation survived the passes, grouped by function, and stops without linking:

```text
$ yashy build main.ya --opt-report
main.ya:5  fn wordcount
  promoted              alloc(4)          promoted to the stack: a literal count of 32 bytes fits in the frame and the pointer does not escape
  bounds check kept     xs[i]             the verifier could not prove this index is inside the slice
                                          fix: add `requires index < xs.len()` or iterate with `for x in xs`
  overflow check kept   n + 1             nothing proves this `+` fits in `int`
                                          fix: add a `requires` clause that bounds the operands, or use a `wrapping_*`/`checked_*` builtin when wraparound is intended
  call not inlined      labs(-3)          `labs` is a C function declared `extern`; this module has its declaration and no body
```

Every entry has a kind — `bounds`, `overflow`, `alloc`, `not-inlined`, `promoted`, or `not-promoted` — a reason, and a fix when one exists. The report is recorded while the body is lowered, so it cannot disagree with the IR: a check that was deleted has no entry, and an allocation that moved to the frame says so.

`--opt-report=json` prints the same record as schema v1 (`{"schema_version":1,"tool":"yashy","functions":[…]}`, with `fix: null` where nothing would help) for an agent. A program with nothing to explain prints one line in the human form and `"functions": []` in JSON.

The report needs the verifier's facts, so it refuses `--no-verify`; it stops before linking, so it also refuses `--emit-ir` and `--pgo`.

## Worked example: from a kept check to nothing to report

Start with a program whose two functions each carry a check the verifier cannot prove away: an index re-tested on every call and an addition re-tested on every call.

```yashy
fn pick(int i) -> int {
    let xs = []int { 10, 20, 30 };
    let o = xs[i];
    let v = switch o { case Some(value) { value } case None { 0 } };
    xs.free();
    return v;
}

fn bump(int n) -> int {
    return n + 1;
}

fn main() {
    print(pick(1));
    print(bump(2));
}
```

`xs[i]` is a runtime-checked index because `i` is a parameter, and `n + 1` is a checked addition because nothing bounds `n`. `--opt-report` says both:

```text
main.ya:1  fn pick
  bounds check kept     xs[i]             the verifier could not prove this index is inside the slice
                                          fix: add `requires index < xs.len()` or iterate with `for x in xs`
main.ya:9  fn bump
  overflow check kept   n + 1             nothing proves this `+` fits in `int`
                                          fix: add a `requires` clause that bounds the operands, or use a `wrapping_*`/`checked_*` builtin when wraparound is intended
```

Add the contract clauses:

```yashy
fn pick(int i) -> int requires 0 <= i requires i < 3 {
    let xs = []int { 10, 20, 30 };
    let o = xs[i];
    let v = switch o { case Some(value) { value } case None { 0 } };
    xs.free();
    return v;
}

fn bump(int n) -> int requires n < 9223372036854775807 {
    return n + 1;
}

fn main() {
    print(pick(1));
    print(bump(2));
}
```

Rebuild with `--opt-report` and the report is now one line:

```text
no checks or allocations to report: every check was proved and every allocation was promoted
```

The verifier discharged both obligations. `xs` is a literal declared in the
function, so the domain knows its length is three; the clauses put `i` in
`0..2`, which is inside that length. For `bump`, the clause narrows `n` to at
most `i64::MAX - 1`, so the addition cannot leave the range of `int`. Both
checks are gone from the IR.

For a slice, the length has to be one the domain knows — a literal, or a
binding whose initializer is written down. On a slice *parameter* the length is
not a fact the domain has, so a clause such as `requires 0 < xs.len()` does not
remove the check; there the other suggested fix applies, and `for value in xs`
removes the index (and its check) instead.

## Related

- [CLI reference](cli.md): `--opt-report[=json]`, `--no-opt=<pass>`, `--opt-list`, `--pgo`, `--no-lto`, `--emit-ir`.
- [Benchmarks](../benchmarks/README.md): the per-pass gate and the CI gate rows each pass is held to.
- [AGENTS.md](../AGENTS.md): repository layout, commands, and the rule that optimization is a phase, not a habit.
