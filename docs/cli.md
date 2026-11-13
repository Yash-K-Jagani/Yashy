# Yashy CLI reference

The `yashy` binary has one command per job. Exit codes are shared:

| Code | Meaning |
|---|---|
| `0` | success |
| `1` | diagnostics or tests failed |
| `2` | usage or configuration error: a bad argument, a missing source, or a toolchain the command needs |
| `3` | internal error: the compiler could not do work it had accepted |

A program's `main` may return an `int`, which becomes its process exit status
when run through `yashy run`.

## `yashy build <file.ya>`

Compiles one program to a native executable.

```text
yashy build main.ya
yashy build main.ya -o wordcount --release
yashy build main.ya --emit-ir
```

| Option | Meaning |
|---|---|
| `-o <path>` | Output path. Default: the source's stem beside the source. |
| `-O0` ... `-O3` | Optimization level passed to `clang`. Default `-O2`. |
| `--release` | Compile out `debug_assert`; default optimization becomes `-O3` unless `-O` is given, and link with ThinLTO. |
| `--no-lto` | Link without ThinLTO. Only for attributing a suspected LTO-only miscompile; see [ThinLTO and `--pgo`](#thinlto-and---pgo). |
| `--pgo` | Build instrumented, train by running the program once, then rebuild with the profile. See [ThinLTO and `--pgo`](#thinlto-and---pgo). |
| `--cpu <tier>` | `portable` (default), `x86-64-v2`, `x86-64-v3`, `x86-64-v4`, `native` (`-march=native`, or `-mcpu=native` on aarch64). The x86-64 tiers are refused for a target of another architecture. |
| `--fast-math` | Allow fused multiply-add; results may differ. |
| `--no-opt=<pass>` | Disable one registered optimization pass for bisecting. See [Optimization passes](#optimization-passes). |
| `--opt-list` | List the passes in order with the bench row each is held to; needs no input. |
| `--opt-report[=json]` | Explain every check and allocation that survived, grouped by function, and stop. See [The optimization report](#the-optimization-report). |
| `--emit-ir` | Print LLVM IR and stop; do not link. |
| `--stdlib-source` | Derive the program from source instead of reusing cached IR. The IR cache is neither read nor written, for bisecting a suspected cache problem. |
| `--no-verify` | Build without the verifier and stamp `<output>.unverified`. Refused for a program with a contract, or when `YASHY_REQUIRE_VERIFY` is set. See [`--no-verify`](#--no-verify). |
| `--timings` | Print how long each pipeline phase took. See [Timings](#timings). |
| `--jobs <n>` | Threads the front end may parse on. `0` (default) uses one per core; `1` is serial. See [`--jobs`](#--jobs-n). |
| `--target <triple>` | Build for this platform rather than the host. See [`cfg` and `--target`](#cfg-and---target). |
| `--reproducible` | Build for bytes that depend only on the tree: a fixed timestamp, no absolute paths in the IR, and a reproducible link. See [Reproducible builds](#reproducible-builds). |

## `yashy run <file.ya>`

Compiles and runs in one step. It accepts the same optimization, CPU, release,
`--no-lto`, verification and timing options as `build`. Arguments after `--` go
to the program. It refuses `--pgo`, because that would train on the program and
then run it a second time; use `build --pgo` and run the resulting binary.

| Option | Meaning |
|---|---|
| `--debug` | Run the child with `YASHY_DEBUG=1`: a panic or an arithmetic fault also prints every live task (id, async function and resume index) before the process ends. See [`yashy tasks`](#yashy-tasks-fileya). |
| `--target <triple>` | Build for this platform rather than the host. See [`cfg` and `--target`](#cfg-and---target). |

## `yashy tasks <file.ya>`

Runs the program like `run --debug`, and prints the executor's live-task
table: the async analogue of a thread dump, and the only reason a hung
program is debuggable. Debug builds only: `--release` is refused. The
table goes to standard error, one line per task, sorted by task id:

```text
tasks: 2
task 1 main_task resume=0 state=running
task 2 blocked_on_channel resume=0 state=pending
```

`resume` is the task's resume index — the next suspension point its poll
dispatches on. `state` is `running` while the task is inside its poll and
`pending` when it is queued or suspended. The table prints on a panic
(`YASHY_DEBUG=1`) and while the run spins on a pending task
(`YASHY_TASKS_DUMP=1`, which `tasks` sets), so a program that never
finishes still shows why.

The same switches name the executor's mode: `YASHY_EXECUTOR_THREADS=<n>`
is the worker count of the default executor (`0`, unset or non-numeric
means single-threaded), so one binary runs both modes without rebuilding.

## ThinLTO and `--pgo`

`--release` links the generated IR and the C runtime as one ThinLTO unit
(`-flto=thin`), so a runtime helper can be inlined into the program that calls
it. A debug build keeps the separate-object model: it is the edit/run loop, and
paying for link-time optimization on every build would make it slower without
changing the answer.

`--no-lto` takes the release link apart again. It exists so a suspected
LTO-only miscompile can be *attributed* rather than worked around: a program
that changes behaviour when the flag is added is a bug to fix, not a reason to
ship without LTO.

`--pgo` is a profile-guided build, in three steps inside one `build`:

1. compile with `-fprofile-generate=<dir>` to an instrumented binary;
2. run that binary once — the training run — which writes raw counters;
3. merge the counters with `llvm-profdata` and rebuild with
   `-fprofile-use=<file>`.

The instrumented binary is a build artifact beside the requested output, and
the profile lives under `<output-dir>/.yashy/pgo/`. Nothing is written outside
that directory. A training run that exits non-zero, or one that writes no
counter file, is an error rather than a profile built from a program that could
not finish. `--pgo` is refused with `--emit-ir` and `--opt-report` because there
is no binary to train.

The training corpus is checked in under [`benchmarks/pgo/`](../benchmarks/pgo);
[`scripts/pgo.ps1`](../scripts/pgo.ps1) and
[`scripts/pgo.sh`](../scripts/pgo.sh) build every member twice — once with the
ThinLTO release link, once with `--pgo` — and report the size and time
difference. A profile is per program, so each member trains on itself.

## Optimization passes

Every optimization the backend applies is one entry in a registry, run in one
order before any IR is written. `--opt-list` prints that order and the bench row
each pass is held to:

```text
$ yashy build --opt-list
pass     row
facts    fib_run_ms
escape   maps_run_ms
intern   hello_binary_bytes
async    async_run_ms
```

`--no-opt=<pass>` disables exactly one of them. None of them changes what a
program prints, so the switch is for *attributing* a suspected miscompile: a
program that behaves differently with one pass off is a bug in that pass.

| Pass | What it decides | Held to |
|---|---|---|
| `facts` | Rely on the verifier's facts: delete proved checks, emit `noalias`, `nonnull`, `readonly` and `align`. Off means an empty fact set and attribute-free IR. | `fib_run_ms` |
| `escape` | Non-escaping allocations become frame slots or block-arena slots instead of allocator calls. | `maps_run_ms` |
| `intern` | Identical string literals share one global. Off emits one global per literal. | `hello_binary_bytes` |
| `async` | Generate the poll function (the explicit-frame state machine) for each `async fn`. Off drives every `await` inline, without suspending. | `async_run_ms` |

Anything not in the table is mandatory lowering, not a pass: there is no switch
that turns an expression into a call or a field into an offset, because there
would be no program left to measure. `yashy_codegen::pass::owner` names the
owner of each expression form, and its exhaustive match is the check that every
construct has an owner — a new form fails to compile until that is decided.

## The optimization report

`--opt-report` prints why each check and allocation survived the passes,
grouped by function, and stops without linking:

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

Every entry has a kind — `bounds`, `overflow`, `alloc`, `not-inlined`,
`promoted`, or `not-promoted` — a reason, and a fix when one exists. The report
is recorded while the body is lowered, so it cannot disagree with the IR: a
check that was deleted has no entry, and an allocation that moved to the frame
says so. `--opt-report=json` prints the same record as schema v1
(`{"schema_version":1,"tool":"yashy","functions":[…]}`, with `fix: null` where
nothing would help) for an agent. A program with nothing to explain prints one
line in the human form and `"functions": []` in JSON. The report needs the
verifier's facts, so it refuses `--no-verify`, and it stops before linking, so
it also refuses `--emit-ir` and `--pgo`. The full treatment — what Yashy proves,
what LLVM decides, the pass list, and a worked example from a kept check to
nothing to report — lives in [The optimization program](optimization.md).

The per-pass gate is `yashy-bench --per-pass`. It compares the all-passes
measurement with one taken with a single pass disabled and fails when disabling
a pass *improves* its own row by more than 2%, because a pass that does not pay
for its row is not earning its place:

```text
$ yashy-bench --per-pass --baseline build/all.bench \
    --without facts=build/no-facts.bench \
    --without escape=build/no-escape.bench \
    --without intern=build/no-intern.bench
pass     | row                |   baseline |    current |     delta | verdict
facts    | fib_run_ms         |       11.2 |       14.9 |    +33.0% | pass
escape   | maps_run_ms        |       88.1 |       90.0 |     +2.2% | pass
intern   | hello_binary_bytes |    64000.0 |    65024.0 |     +1.6% | pass
```

The script that produced either file is the measurement's business
([`benchmarks/ci.ps1`](../benchmarks/ci.ps1) takes `-NoOpt <pass>`, its Linux
counterpart `BENCH_NO_OPT`). A registered pass with no `--without` measurement
is an error: the gate cannot skip a pass by not measuring it. `--report-only`
prints the table and exits 0 for a pre-existing regression you are still
bisecting.

## `yashy check <file.ya> [--json] [--timings] [--jobs <n>]`

Lexes, parses, and type-checks without emitting an executable. Use it for fast
feedback.

`--json` prints the [schema v1 report](#json-diagnostics-schema-v1). The command
exits `0` when there are no errors and `1` when there is at least one error; a
missing file is a usage error (`2`). A diagnostic with a catalog code ends with
a `docs:` line naming its page, for example
`docs: https://yashy.dev/errors/E0100`.

| Option | Meaning |
|---|---|
| `--print-cfg` | Print the `cfg` keys that are true for this target, one per line, sorted, and stop. Needs no source file. See [`cfg`](#cfg-and---target). |
| `--target <triple>` | Check for this target rather than the host. See [`cfg` and `--target`](#cfg-and---target). |

## `cfg` and `--target`

`cfg(..)` guards an item, a statement block or a struct field, and the guarded
code exists only on the targets the condition names:

```yashy
cfg(windows) fn platform() -> string {
    return "windows";
}

fn main() {
    cfg(unix) {
        print("unix");
    }
}
```

A disabled branch is **not resolved, not checked, not verified and not
emitted**. That is the whole difference between `cfg` and `if false`: the
branch may call a function that exists nowhere, because the filter runs on the
parsed program before name resolution.

The keys are `windows`, `linux`, `macos`, `unix`, `arch_x86_64`,
`arch_aarch64`, `debug` and `release`, combined with `not(..)`, `any(..)` and
`all(..)`. A key the compiler does not define is [`E2202`](errors/E2202.md), not
a condition that quietly evaluates to false, and `cfg` anywhere it cannot apply
is [`E2203`](errors/E2203.md).

```text
yashy check --print-cfg
arch_aarch64
debug
linux
unix
```

`--target <triple>` names the platform a build is for. The supported triples
are `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`,
`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
`x86_64-apple-darwin` and `aarch64-apple-darwin`; any other is
[`E2200`](errors/E2200.md). The target decides the `cfg` keys, the data layout
in the emitted IR, the platform libraries and the `--target` line `clang` is
given, so one build is never for two machines at once.

A cross build needs the target's C toolchain: Yashy's runtime is C and the
program is linked by `clang`, and the compiler ships neither a linker nor a
cross runtime. A target whose toolchain is missing is
[`E2201`](errors/E2201.md), naming the program it looked for.
`yashy doctor --target <triple>` answers the same question without building.

`run` and `test` refuse a target other than the host: they run what they
build, and a binary for another platform cannot start here. `build` is the way
to make a program for another platform.

## Reproducible builds

`yashy build --reproducible` builds for bytes that depend only on the tree
and not on the machine:

```text
yashy build --reproducible main.ya -o hello
```

Three things normally make two builds of the same source differ, and the flag
answers all three:

| What differs | What the flag does |
|---|---|
| The clock | The timestamp is fixed: `SOURCE_DATE_EPOCH` if the environment sets it, otherwise the commit time of the tree, otherwise zero. |
| The build directory | The source is named relative to the working directory, and `clang` is told `-ffile-prefix-map` so the debug info and the `__FILE__` macros do not carry the path either. |
| The linker's own idea of an identity | A PE image carries the time it was made in its header; the linker is asked for a reproducible image, so that stamp comes from the content. |

Two builds of the same source in two different directories then produce the
same bytes, which is what lets a stranger recompute the hash of a release.

`scripts/repro-check.sh` (and `scripts/repro-check.ps1` on Windows) checks
the claim on a machine: it builds `examples/hello` twice, in two temporary
directories, and compares the SHA-256 of the two binaries. When they differ
it prints the first differing byte offset, because "the builds differ" is not
an answer a person can act on.

The compiler's own output is already ordered: the one map the emitter
iterates is a `BTreeMap`, and the maps that are only looked up never reach
the output.

## `yashy expand <file.ya> [--json]`

Prints the fully expanded program: every macro invocation is replaced by its
arm's tokens, in source order. The command expands and formats; it does not
type-check or verify, and its exit code is only about whether expansion and
formatting succeeded (`0` success, `1` a macro or parse diagnostic, `2` for a
usage error).

```text
yashy expand main.ya
yashy expand --json main.ya
```

`--json` prints `{"schema_version":1,"tool":"yashy","expansions":[...]}`, one
entry per invocation, with `macro`, `arm_span`, `invocation_span` and
`output_tokens`.

`check`, `build`, `run`, `test` and `verify` all expand before anything else,
so a program written with a shorthand is checked and compiled as the ordinary
syntax it expands to. `--emit-ir` shows IR from expanded code, as it must.

## `yashy verify <file.ya> [--json] [--timings] [--jobs <n>] [--inventory]`

Runs the front half of the pipeline and then the verifier: the pass that proves
a well-typed program is also *sound*. That means every linear value -- every
pointer, slice, map, mutex and channel -- is released on every path out of its
scope, and every `requires`, `ensures` and loop `invariant` is discharged. A
clause the verifier cannot prove is an error
(see [E1100](errors/E1100.md)), so the program is refused.

```text
yashy verify main.ya
yashy verify --json main.ya
```

Exit codes are the same as `check`: `0` when the verifier found no errors, `1`
when it found at least one, `2` for a usage error. A program that does not
type-check fails before the verifier runs, and its check diagnostics are the
whole report: there is no typed program to reason about.

A verifier diagnostic carries a proof trace in its JSON form -- the obligation,
the fact that could not be established, and the remedy -- so a tool can act on
it without parsing the message.

`verify` exits `0` when every diagnostic is a warning. One kind is a warning: a
viewed `[]T`, which the view keeps alive until the function returns so the
language gives no way to release it ([E1000](errors/E1000.md)). An unproved
contract is an error, not a warning.

`build` and `run` verify by default. An unverified binary is not a build, so a
program that leaks or breaks a contract is refused before `clang` is started.
`test` verifies too, and both surfaces agree: a leaking test fails whether you
run `test` or `test --json`, and a verifier warning is reported without failing
the run.

### `--inventory`

A proof rests on assumptions, and the assumptions are worth naming. `--inventory`
lists every `extern` declaration and every call the source marked `unsafe`: the
places where the program trusts something the verifier cannot see.

```text
$ yashy verify --inventory libc.ya
assumptions: 3
  extern_decl  malloc  libc.ya:2:5
  extern_decl  free    libc.ya:3:5
  unsafe_call  free    libc.ya:8:12
```

The list is in source order, one line per assumption, and it is printed even
when it is empty (`assumptions: 0`): "this program trusts nothing" is an answer.
`--inventory --json` puts the same list in the envelope as `assumptions`, an
array of `{kind, name, file, span}` where `kind` is `extern_decl` or
`unsafe_call`. Without the flag the key is absent, which means "not computed",
not "none".

The inventory never changes the exit code. A program the verifier accepted
before the flag existed is still accepted with it; what changed is that its
assumptions can be read instead of guessed at.

## Warnings

`yashy` emits four coded warnings (E2000-E2003): unused binding, unused
import, unreachable code, and unused parameter. A warning is reported to
stderr and does not stop the build: `yashy build` exits 0 with a warning.

- `--deny warnings` treats every warning as an error: the build exits 1, and
  in `--json` the diagnostic's `severity` becomes `"error"` with the same
  `code`.
- `--allow warnings` suppresses all warnings entirely.
- `-W none` is a spelling of `--allow warnings`.
- `YASHY_DENY_WARNINGS=1` in the environment has the same effect as
  `--deny warnings`.

## `--no-verify`

An escape hatch for the case where the verifier is wrong and a binary is still
needed. It skips the proof, not the type system: `check` still runs, so a
program that does not type-check is still refused. What it skips is everything
in [`yashy verify`](#yashy-verify-fileya---json---timings---inventory): the leak pass and
contract discharge.

The flag is narrow, loud and traceable:

```text
$ yashy build main.ya --no-verify
warning: built without verification (`--no-verify`); wrote main.unverified
```

The stamp is the output path plus `.unverified`, so `main.exe` on Windows
becomes `main.exe.unverified`. It contains one line of JSON:

```json
{"schema_version":1,"verified":false,"reason":"--no-verify","yashy_version":"1.0.0"}
```

A verified build removes a stale stamp. If the stamp cannot be written, the
binary is removed too and the build fails: an unverified binary must not exist
without one.

The flag is refused, as a usage error (`2`) rather than a diagnostic, in four
cases, because nothing is wrong with the program:

- the program has a `requires`, `ensures` or loop `invariant`; the message names
  the first function that carries one, because only the verifier can honour a
  contract;
- `YASHY_REQUIRE_VERIFY` is set (to anything but empty, `0` or `false`), so a CI
  can demand proof without trusting every command line it runs;
- `--emit-ir` is also given, because IR is not a binary and there is nothing to
  stamp;
- `--opt-report` is also given, because the report explains what the verifier
  proved and has nothing to say when the proof was skipped.

`run --no-verify` forwards the flag to `build`. The temporary binary and its
stamp live in the run directory and are removed with it; `run` keeps no binary.

## `yashy test [path] [--filter <substring>] [--list] [--json] [--timings] [--jobs <n>]`

Runs every `.ya` file under `tests/` (or `path`). Each file is an ordinary
program with a `main`; exit `0` is a pass, a panic or non-zero exit is a
failure.

| Option | Meaning |
|---|---|
| `--filter <substring>` | Run only tests whose path contains the substring. |
| `--list` | Print the tests that would run and exit. |
| `--json` | Print the [schema v1 report](#json-diagnostics-schema-v1) instead of the progress lines. |
| `--timings` | Print the suite's total time per pipeline phase. See [Timings](#timings). |
| `--target <triple>` | Build the tests for this platform rather than the host. A test is built *and* run, so a target other than the host is refused. See [`cfg` and `--target`](#cfg-and---target). |

`--json` writes one envelope to stdout. Diagnostics -- check or verifier -- are
nested under the test that produced them; top-level `diagnostics` holds setup
errors such as a missing toolchain. `--list` and `--json` cannot be combined. A
failing test makes the command exit `1`; a setup problem exits `2`; a directory
that cannot be created exits `3`.

## `yashy explain <code> [--json]`

Prints the long-form catalog page for an error code: the cause, a minimal
example, and the fix. The pages are embedded in the binary, so the command
works offline.

```text
yashy explain E0100
yashy explain --json E0100
```

The human form is the Markdown page verbatim. The JSON form is a separate
schema v1 envelope carrying the same facts:

```json
{
  "schema_version": 1,
  "tool": "explain",
  "yashy_version": "1.0.0",
  "success": true,
  "code": "E0100",
  "title": "type mismatch",
  "summary": "An expression's type does not match its context.",
  "page": "# E0100 — type mismatch\n\nAn expression's type ...",
  "docs_url": "https://yashy.dev/errors/E0100"
}
```

An unknown code is a usage error (`2`). When the code is a near miss, the
message names the nearest known code, for example
``unknown error code `E010`; did you mean `E0100`?``. With `--json`, an unknown
code still writes that human message to stderr; the JSON envelope is for a
catalog entry that exists.

## Timings

`--timings` makes the compiler report how long each pipeline phase took. It is
accepted by `build`, `run`, `check`, `verify` and `test`, and it never writes to
stdout: the human report goes to stderr, one line per phase, and `--json` puts
the same numbers in the envelope instead.

```text
$ yashy run hello.ya --timings
hello
timing: load 1 ms
timing: check 4 ms
timing: verify 0 ms
timing: emit 2 ms
lto=off
timing: link 253 ms
```

The phases are `load` (reading the entry point and its imports, and parsing
them), `check` (names and types), `verify` (soundness), `emit` (LLVM IR) and
`link` (the single `clang` invocation). A command reports only the phases it
runs: `check` has no `verify`, `emit` or `link`, and `verify` has no `emit` or
`link`. A phase that fails still gets a line, because a build that is slow *and*
broken is when the number is most useful.

Two lines describe the link strategy rather than a phase. `lto=thin` is a
release link (ThinLTO); `lto=off` is anything else, including `--no-lto`. A
`--pgo` build adds one `pgo=<stage>` line per step: `pgo=instrumented` after the
instrumented binary is linked, `pgo=trained` after the training run, and
`pgo=applied` after the rebuild with the profile. A build without `--pgo` prints
no `pgo` line.

Two more lines describe the front end rather than a phase:

```text
query_hits=0 query_misses=1 recomputed=check
frontend_threads=8
```

`frontend_threads=N` is how many threads parsed the program's files, which is
what [`--jobs`](#--jobs-n) sets. `query_hits`/`query_misses` are how many cached
front-end queries were reused and how many ran; `recomputed` names what ran.

Two lines describe what the build reused:

```text
ir_cache=hit
stdlib_modules=1
```

Every build is a pure function of the sources it read, the codegen flags, the
compiler and the stdlib, so the emitted LLVM IR is stored in a
content-addressed cache and reused when all of those agree. `ir_cache=hit` means
the IR was read (its `emit` row is `0 ms`); `ir_cache=miss` means it was emitted
and stored. The key includes every source byte and path, the optimization and
codegen flags, the target triple, the compiler version and executable timestamp,
the IR format version and the stdlib version, so a hit can only hand back the IR
those inputs produce. The cache shares
[`YASHY_CACHE_DIR`](#environment) with the runtime objects and the package
store. `--stdlib-source` neither reads nor writes it and prints no `ir_cache`
line.

`stdlib_modules=N` counts the modules the build loaded from the standard
library. The stdlib is not ambient: the loader adds only modules the program
imports, so `hello` reports `stdlib_modules=0` and an `import JSON;` reports
one. A stdlib module is typechecked only because it was imported, not because
the compiler was installed with one.

`test --timings` prints one total for the whole suite rather than five lines per
test, so the report answers where the run's wall time went.

## `--jobs <n>`

The loader discovers the program's files, then parses them on a thread pool.
`--jobs` sets the pool size: `0` (the default) uses one thread per available
core, `1` parses serially. The size is capped by the number of files, so a
one-file program always reports `frontend_threads=1`.

Parallelism does not change the output. Results are collected per file and
merged in source order, so diagnostics, the `--json` envelope and a built binary
are identical at any `--jobs` setting; only `frontend_threads` differs. A file
that fails to parse is reported exactly as the serial loader reported it: the
first failure in discovery order, with that file's errors in source order.

## JSON diagnostics (schema v1)

`check --json`, `verify --json` and `test --json` share one envelope. The
`tool` field names the command. The schema is v1 for R1: additive changes are
allowed within 0.x, and it is frozen at 1.0. Output is pretty-printed JSON on
stdout; `--json` implies no color and no prompt.

```json
{
  "schema_version": 1,
  "tool": "check",
  "yashy_version": "1.0.0",
  "success": false,
  "diagnostics": [
    {
      "code": "E0100",
      "severity": "error",
      "message": "expected `int`, found `string`",
      "file": "main.ya",
      "span": { "start": 42, "end": 47, "line": 3, "column": 12 },
      "labels": [
        { "span": { "start": 42, "end": 47 }, "message": "expected `int`" }
      ],
      "notes": ["..."],
      "docs_url": "https://yashy.dev/errors/E0100"
    }
  ],
  "summary": { "errors": 1, "warnings": 0 }
}
```

- `code` is `null` for a diagnostic without a catalog entry.
- `labels` and `notes` are omitted when empty; `fix` is not part of R1.
- `docs_url` is present only when `code` is present.
- `assumptions` is present only on `verify --inventory`, and is an array of
  `{kind, name, file, span}` in source order, where `kind` is `extern_decl` or
  `unsafe_call`. An absent key means the inventory was not computed.
- `span.start` and `span.end` are byte offsets; `line` and `column` are
  one-based and refer to `span.start`.
- `proof` is present only on a verifier diagnostic, and describes why an
  obligation was not met:

  ```json
  "proof": {
    "obligation": "leak",
    "missing_fact": "`buf` is not released on every path out of its scope",
    "remedy": "release it with `buf.free()`, `defer { buf.free(); }`, or by returning it"
  }
  ```

  `obligation` is a stable name: `leak` for ownership, and `requires`,
  `ensures` or `invariant` for a contract clause. `missing_fact` says what the
  verifier could not prove, and `remedy` says what to write instead. A check
  diagnostic never carries one: a type error is a fact about the program, not a
  missing proof.
- `timings` is present only when `--timings` was passed, and is omitted
  otherwise so a report without the flag stays byte-identical:

  ```json
  "timings": [
    { "phase": "load", "ms": 1 },
    { "phase": "check", "ms": 4 },
    { "phase": "verify", "ms": 0 }
  ]
  ```

  `phase` is the stable name from [Timings](#timings) and `ms` is whole
  milliseconds. Two rows describe the front end rather than a phase:
  `{"phase":"query","hits":0,"misses":1,"recomputed":["check"]}` carries the
  memo counters and `{"phase":"frontend_threads","threads":8}` the pool size.
  The link strategy is `{"phase":"lto","mode":"thin"}`; a profile-guided build
  adds one `{"phase":"pgo","stage":"instrumented"}` per stage (`trained`,
  `applied`). The cache decision is `{"phase":"ir_cache","hit":true}` and the
  stdlib count is `{"phase":"stdlib_modules","modules":1}`; the first appears
  only on a command that runs the backend (`build`, `run`, `test`), never for
  `check`, `verify` or `--emit-ir`, while the second appears on every command
  that loads sources. `test --json --timings` carries the suite's totals.

`test --json` uses the same envelope plus `tests` and a test summary:

```json
{
  "schema_version": 1,
  "tool": "test",
  "yashy_version": "1.0.0",
  "success": false,
  "diagnostics": [],
  "tests": [
    {
      "name": "smoke",
      "file": "tests/smoke.ya",
      "status": "failed",
      "duration_ms": 12,
      "diagnostics": []
    }
  ],
  "summary": { "passed": 0, "failed": 1, "compile_errors": 0, "skipped": 0 }
}
```

`status` is `passed`, `failed` or `compile_error`. `compile_error` means the
test never ran: it did not type-check, the verifier refused it, or code
generation failed. `failed` means the test ran and exited non-zero. The two are
distinct on purpose — a tool that counts red tests must not count a build
failure as one, and a reader must not go looking for an assertion that fired.
`summary` counts the two separately, and `passed + failed + compile_errors +
skipped` is the number of tests. `duration_ms` covers compiling and running the
test. A runtime failure has a synthetic diagnostic with the exit status and no
code; a compile error carries the same diagnostic the compiler prints.

## `yashy init [path]`

Creates a project in an empty directory. It writes `yashy.toml`, `main.ya`,
`tests/smoke.ya`, `README.md`, `AGENTS.md`, and `.gitignore`. It refuses a
directory that already contains anything.

## `yashy doctor [--json]`

Checks the toolchain and standard library. It exits `1` when `clang` cannot be
run or the stdlib is missing, and prints a final `next:` line with the fix. The
`path` check is informational and does not affect the exit status.

`--json` prints the stable schema v1 report:

```json
{
  "schema_version": 1,
  "tool": "doctor",
  "ok": true,
  "yashy_version": "1.0.0",
  "target": "x86_64-unknown-linux-gnu",
  "clang": {
    "ok": true,
    "found": "clang",
    "path": "/usr/bin/clang",
    "version": "clang version 18.1.8",
    "error": null
  },
  "stdlib": {
    "ok": true,
    "path": "/home/you/.local/share/yashy/stdlib/yashy",
    "error": null
  },
  "path": {
    "executable": "/home/you/.local/bin/yashy",
    "on_path": true,
    "hint": null
  },
  "cross": null,
  "next": null
}
```

In JSON mode all output goes to stdout; human diagnostics are suppressed. A
failure has `"ok": false`, an `error` on the failing section, and a non-null
`next`.

`--target <triple>` asks about another platform: the report names that target
and adds a `cross` section saying whether the target's C toolchain can link
for it. `null` means the question was not asked, which is different from
`{"ok": false, ...}` — the toolchain is missing.

## `yashy playground [--port <n>] [--host <address>]`

Serves the playground: a static front end and a small server that compiles
each program on the server, in a sandbox of its own. See
[the playground's own page](../playground/README.md) for the limits and the
API.

```text
yashy playground --port 8080
```

## `yashy fmt [-w | --check] [--json] <files...>`

Prints the canonical formatting to stdout. With `-w`, rewrites the files in
place.

`--check` is the CI gate: it writes nothing, prints one line per file that is
not already canonical, and exits `1` if there is at least one. `--check` with
`--write` is a usage error (`2`), because a gate that quietly rewrites files is
not a gate.

The comparison normalises CRLF first. `core.autocrlf=true` is the default on
Windows, and a gate that fails on a clean checkout is a gate people disable.

`--json` prints the same answer as a document, so CI need not parse text:

```json
{
  "schema_version": 1,
  "tool": "fmt",
  "success": false,
  "files": [{ "file": "main.ya", "changed": true }]
}
```

`--json` needs `--check`: there is nothing to list otherwise.

## `yashy version`

Prints the compiler version.

## `yashy index [path] [--json]`

Walks a directory (or one file) and prints the symbol index: every top-level
function, struct, enum, trait and impl method, with signature, effects,
contracts, typestate, visibility and docs. A fact the compiler does not know
is `null`. Symbols are sorted by `(file, span.start)`, so `--json` is
byte-identical across runs.

## `yashy outline <file.ya>` / `yashy graph [path] [--json|--dot]` / `yashy search <query> [path]`

`outline` prints one file's symbols in source order, one line each.
`graph` prints the module graph plus the call graph; `--json` is
`{"nodes":[...],"edges":[{"from","to","kind":"import"|"call"}]}` and
`--dot` is Graphviz. `search` matches symbols and doc text, ranked exact
name, then prefix, then doc match; no match exits 0 with an empty result.

## `yashy context --task "..." [path] [--budget <n>] [--json]`

A token-budgeted pack for one task: the diagnostic, the enclosing function,
its callees' signatures, the tests that cover it, and the contracts
involved, added in that relevance order until the next section would exceed
the budget. `--json` reports `tokens`, `budget`, `included` and `dropped`;
what did not fit is named in `dropped`. The estimator is documented in
`crates/yashy-driver/src/context.rs`: one token per four bytes of ASCII,
one per non-ASCII character, rounded up. Exit 2 when `--task` names nothing
in the workspace.

## `yashy mcp --root <dir>`

JSON-RPC 2.0 over stdio with seven tools: `check`, `build`, `test`,
`verify`, `explain`, `context`, `index`. Every tool returns the schema-v1
payload the CLI returns; a tool error is `code: -32000` with the diagnostic
array in `data`. No tool accepts a path outside `--root` (refused with
`-32001`), no tool accepts a shell string, and `build` writes only under
`<root>/.yashy/` and the requested output.

## `yashy run --sandbox [--allow-read <path>] [--allow-write <path>] [--time <ms>] [--memory <mb>]`

Runs the program with a default-deny capability set: the named paths, and
nothing else. A read or write outside the allowlist is `E1900` naming the
capability; a time, memory or output breach is `E1901` naming the limit.
Windows enforces with a Job Object (memory, process count, kill-on-close);
Linux with rlimits in the child. `test --sandbox` applies the same set to
every test program.

## `yashy run --deterministic`

Fixes the clock at a given instant, seeds the RNG, sorts map iteration
where observable, and pins the process id field, so `log_info`/`log_error`
and `metrics_add`/`metrics_set` produce byte-identical stderr across runs.
`YASHY_DETERMINISTIC=1` does the same for a test run.

## `yashy doc [path] [--out DIR] [--json] [--private]`

Renders the public surface from the compiler's own tables into `target/doc/`
(one Markdown file per module plus `index.md`). A private item appears only
with `--private`; an undocumented public item appears with
`No documentation.` `--json` emits the same model the Markdown renders
from. The output is byte-identical across runs and OSes. HTML is out of
scope: the Markdown is what `llms.txt` and the index already consume.

## `yashy lsp`

Execs the sibling `yashy-lsp` binary on stdio, so an editor needs one
command. A missing sibling exits 2 naming the path; the server's exit code
is propagated. `yashy doctor` reports the lookup.

## `yashy profile <file.ya>`

Builds and runs the program, printing the measured wall time. A recorded
per-function counter report renders with `--report <file.json>`: one line
per function, sorted by self time, with the instrumentation overhead
measured and printed. No sampling thread.
`run --debug` also leaves `.yashy/debug.lldb` in the current directory —
a generated command file that sets a breakpoint on `main` and dumps the
async task table on stop; the same table is available without a debugger
through `YASHY_DEBUG=1`.

## `yashy.toml` and `yashy lock --check`

A project may carry a manifest beside its source. The reader is a documented
subset of TOML, hand-written so the compiler keeps building offline. The full
story — lockfile, store, audit and the local registry — is in
[packages](packages.md).

```text
file       := line*
line       := ws (comment | table | array-table | assignment) ws comment? ws
table      := '[' key ('.' key)* ']'
array-table:= '[[' key ('.' key)* ']]'
assignment := key ('.' key)* '=' ws value
key        := bare-key | '"' basic-string '"' | "'" literal-string "'"
value      := string | integer | boolean | array
array      := '[' ws (value ws (',' value ws)* ','?)? ']'
comment    := '#' any
```

Bare keys are letters, digits, `_` and `-`. Strings are single-line: a basic
string understands `\n`, `\r`, `\t`, `\\` and `\"`, and a literal string
understands nothing. There are no inline tables, no multi-line strings, no
floats and no dates. `[[package]]` is the array-of-tables form the lockfile
uses; a plain `[table]` may not repeat. The whole file is capped at 1 MiB and
every rejection is [E1800](errors/E1800.md) at a line and column.

```toml
[package]
name = "hello"
version = "0.1.0"

[dependencies]
http = "0.3.1"

[dependencies.net]
path = "../net"

[dependencies.extra]
git = "https://example.invalid/extra"
rev = "0123456789abcdef0123456789abcdef01234567"
```

Versions are exact pins: `1.2.3`, never `^1.2.3`, `~1.2.3`, `*` or a range. A
git dependency names a commit. Unknown tables and keys are refused, so a
misspelled `[dependecies]` cannot silently vanish.

`yashy lock --check` compares `yashy.toml` with `yashy.lock` and exits 1 with
[E1802](errors/E1802.md) when a version, source or content hash differs. The
lockfile is deterministic: packages sorted by name then version, dependency
lists sorted, LF endings, and a `sha256:` hash over a canonical source tree
(sorted paths, CRLF normalised, no mtimes). `yashy build` runs the same check
when a lockfile is present.

### Resolution: `update`, `fetch`, `add`, `--offline`

Resolution is exact-pin only; there is no version solver. A path dependency is
read in place and a git dependency is cloned into a global store keyed by
`(url, rev)`, checked out at that commit and made read-only.

```text
yashy update [--offline] [dir]     resolve the manifests and rewrite yashy.lock
yashy fetch  [--offline] [dir]     put every package in the store, without building
yashy add <name> --path <dir>      append a path dependency, then update
yashy add <name> --git <url> --rev <commit>   append a pinned git dependency
```

`--offline` (also accepted by `build` and `run`) never invokes git or touches
the network: a package that is not already in the store stops the build with
[E1803](errors/E1803.md), which names the package, its pin and the command that
would fetch it. A dependency cycle is [E1801](errors/E1801.md) and prints the
path it followed.

A package's modules are found under its `src/` directory when it has one, and
under its root otherwise. The build puts the entry directory and every resolved
package root at the front of `YASHY_PATH` for the duration of the build, so
`import net;` finds the `net` package without any manual environment setup.

The store lives under `YASHY_PACKAGE_STORE` when set, otherwise under the
compiler's cache directory: `YASHY_CACHE_DIR/packages`, or the platform
convention (`%LOCALAPPDATA%/yashy/packages`, `$XDG_CACHE_HOME/yashy/packages`,
`~/.cache/yashy/packages`). A registry dependency is looked up by exact version
in `YASHY_REGISTRY` (or `store/registry`), its tarball hash is checked, and the
tree is unpacked into the store.

### `audit`, `publish` and a local registry

```text
yashy audit [dir] [--json]
yashy publish [dir] --registry <absolute-path-or-file-url>
yashy add <name>@<version> --registry <dir>
```

`yashy audit` reads the resolved graph and reports missing exact versions,
wrong sources, a lockfile that disagrees with the manifest, and an untrusted
registry; each finding is [E1804](errors/E1804.md). A clean run prints
`ok: N packages, M bytes` and exits 0.

A registry is a directory: `index.json` plus `packages/<hash>.tar`, written
through staging files and renamed into place. `publish` takes the name,
version and description from the package's `yashy.toml` and hashes the whole
tree. `YASHY_REGISTRY` names the registry a later resolution uses; `add
<name>@<version> --registry <dir>` sets it for the update it runs. The worked
publish → depend → build cycle is in [packages](packages.md#a-local-registry).

## Flags the native foundation did not add

The `link_name`, variadic, `repr(C)` and byte-buffer work extended the language,
not the command line: it added no option, no environment variable and no JSON
key. A build script written against an earlier version still parses, and a
`--json` consumer sees the same envelope and the same diagnostic shape it
already knew.

The one behavioural change is a signature change, not a flag: the builtins that
used to report failure as `""` or `0` now answer `Option` or `Result`, so a
program that called them must match the value. That is a source change a
diagnostic points at, not a switch to set.

## Environment

| Variable | Effect |
|---|---|
| `YASHY_STDLIB` | Directory containing `io.ya`; overrides bundled stdlib discovery. |
| `YASHY_PATH` | Replaces the entry-directory import roots. The bundled stdlib is still appended. |
| `YASHY_LLC` | Names the `llc` executable directly. Builds still need `clang` on `PATH`. |
| `YASHY_LEAK_REPORT=1` | Print live allocations at process exit. |
| `YASHY_REQUIRE_VERIFY` | When set to anything but empty, `0` or `false`, `build` and `run` refuse `--no-verify`. |
| `YASHY_DEBUG=1` | A panic or arithmetic fault also prints every live task (id, async function, resume index) to standard error before the process ends. Set by `run --debug` and `tasks`. |
| `YASHY_TASKS_DUMP=1` | The executor prints its live-task table while the run spins on a pending task. Set by `tasks`. |
| `YASHY_EXECUTOR_THREADS=<n>` | Worker count of the default executor; `0`, unset or non-numeric means single-threaded. |
| `YASHY_CACHE_DIR` | Root of the compiler's caches: compiled runtime objects, emitted IR and the package store (`<dir>/packages`). A private value isolates a test or a build system. |

## Installer flags

The installers are separate scripts shipped with a release. They support
`--prefix` / `-Prefix`, `--no-path` / `-NoPath`, `--no-doctor` / `-NoDoctor`,
and `--uninstall` / `-Uninstall`. See the [quickstart](quickstart.md).
