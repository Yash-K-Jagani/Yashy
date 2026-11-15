# Yashy

Yashy is a general-purpose, statically typed programming language that compiles
to native binaries through LLVM. The compiler is `yashyc`, written in Rust; what
it emits is textual LLVM IR that `clang` assembles and links. There is no runtime
library to install and no LLVM build dependency: the print helpers are a C source
file embedded in the compiler binary, and the only external tool is `clang`.

**Status: [1.0.0](docs/1.0.md).** Windows x86-64 and Linux x86-64 are the claimed
platforms. macOS archives are built and install-tested by every release, but the
claim waits for a signed and notarised archive, which needs an Apple Developer
Program membership. The release note is explicit about what 1.0 does *not*
promise; read it before you rely on something.

## Install

You need `clang` and LLVM first: `winget install LLVM.LLVM` on Windows,
`sudo apt install clang llvm` on Debian or Ubuntu, `brew install llvm` on macOS.
Then download the archive for your platform from the
[releases page](https://github.com/Yash-K-Jagani/Yashy/releases), unpack it, and
run the installer inside it (`./install.ps1` or `./install.sh`).

```text
yashyc doctor
yashyc init wordcount
cd wordcount
yashyc run main.ya
```

[Quickstart](docs/quickstart.md) is the whole path on one page, including the
`SHA256SUMS` check and the one-time SmartScreen and Gatekeeper overrides.
[Your first CLI](docs/tutorial/12-first-cli.md) takes the same path to a tested
release binary with a dependency. macOS has its own
[install page](docs/install/macos.md).

## Hello

```yashy
fn main() {
    print("hello, world");
}
```

```console
$ yashyc run hello.ya
hello, world
```

`yashyc build` produces a native executable, `yashyc check` type-checks without
emitting one, `yashyc test` runs every program under `tests/`, and
`yashyc build --emit-ir` prints the generated IR and stops. The
[language tutorial](docs/tutorial/00-intro.md) builds the language up in twenty
chapters; a test compiles and runs every example in it, so it cannot drift.

## What the compiler checks

- **Types, exhaustively.** No implicit coercion, no operator overloading, no
  class inheritance. A `switch` must cover every variant, and a pattern in
  binding position must match every value of its type.
- **Ownership, before it runs.** Every linear value is released on every path
  out of its scope, and every `requires`, `ensures` and loop `invariant` is
  discharged. `build` and `run` verify by default, so a program that leaks is
  refused before `clang` starts; `yashyc verify` answers with a proof object
  naming the obligation, the missing fact and the remedy.
- **Arithmetic, at run time.** `a + b` stops the program if it overflows instead
  of continuing with a wrapped number that looks exactly like an answer, and the
  message names the operands and the position. `wrapping_*`, `checked_*` and
  `saturating_*` are the named opt-outs.
- **Malformed input, without panicking.** The compiler returns a diagnostic for
  any program, and a dedicated suite feeds it garbage to prove it.

There is no garbage collector, no borrow checker, no exceptions, no `Drop` trait
and no procedural macros. Memory is owned explicitly with `move`, `defer` and
`.free()`; errors are `Option` and `Result` values and `?` propagates them.

A coded diagnostic ends with a `docs:` line, and `yashyc explain <code>` prints
the same long-form page offline. `check`, `verify` and `test` share one
[JSON schema](docs/cli.md#json-diagnostics-schema-v1), which is what the
[tooling](docs/tutorial/19-tooling.md) and the `mcp` server are built on.

## Performance

Same algorithm, same input, every language compiled by its own native optimizer
at `-O2`. Lower is better. Full methodology, all tiers and every caveat are in
[`benchmarks/README.md`](benchmarks/README.md).

| Benchmark | Yashy | C (`clang -O2`) | Rust (`-O`) |
|---|---:|---:|---:|
| `fib(35)`, ~30M calls | 149 ms | 82 ms | 87 ms |
| `loop`, 200M iterations | 241 ms | 267 ms | 249 ms |
| `float`, 100M iterations | 535 ms | 546 ms | 542 ms |

Yashy's scalar code is in the same band as C and Rust, and that is the whole
claim: `yashy-codegen` contains no optimization passes and hands everything to
LLVM. `fib` is the one row where the checked default falls behind, and the
`wrapping_*` build of the same program is within a percent of `clang` (82 ms).
The front end — lex, parse, check, emit IR — is 22 ms of a 259 ms full build;
the rest is `clang` doing the same job it does for C, in a single invocation.

### What the check costs

The arithmetic is checked, so a program that would overflow stops with a message
naming the operands instead of continuing with a wrapped number. That promise has
a price, and it is not the same price everywhere: it is free where the backend can
prove the operation cannot overflow, and charged per operation where it cannot.
Every number below is one interleaved pass — thermal effects cancel within a pass,
which they do not across passes — minimum of seven runs, `-O2`.

| Benchmark | checked | `wrapping_*` | C (`clang -O2`) | Cost |
|---|---:|---:|---:|---:|
| `fib(35)` | 158 ms | 84 ms | 84 ms | 1.9x |
| `loop`, portable | 261 ms | 262 ms | 273 ms | 1.0x |
| `loop`, `--cpu=x86-64-v3` | 171 ms | 69 ms | 68 ms | 2.5x |
| `float` | 535 ms | — | 546 ms | 1.0x |

- **`float` costs nothing**, because there is no integer arithmetic in it: only
  integer operations carry a check.
- **`loop` costs nothing at the default tier.** Its induction variable and its sum
  are provably within range, so LLVM deletes every check.
- **`fib` costs 1.8–1.9x**: three checks inside a function called thirty million
  times. This is the one row where Yashy's default falls behind Go, and the one
  where the opt-out matters most.
- **At `--cpu=x86-64-v3` the check costs 2.5x on `loop`, for a specific reason: it
  stops the loop from vectorising.** A potentially-faulting branch in the body is
  not something LLVM's vectoriser will speculate. With `wrapping_*` the same
  build vectorises to exactly C's code: 69 ms against 68 ms in the same pass.

So the opt-out is not a claim that checking was a mistake; it is how a program
says *this one cannot overflow* where the backend cannot see that for itself.
`--cpu=<tier>` and `--fast-math` are the other two opt-ins, and both trade
something away: instruction-set width costs portability, fused multiply-add
changes the arithmetic result. Making the checked form itself vectorisable is an
open optimisation, recorded in
[`docs/performance-improvements.md`](docs/performance-improvements.md).

## Documentation

| Start here | |
|---|---|
| [Quickstart](docs/quickstart.md) | install, doctor, init, run, test |
| [Language tutorial](docs/tutorial/00-intro.md) | twenty chapters, every example compiled and run by a test |
| [What 1.0 promises](docs/1.0.md) | the release note: numbers, corpus, platform table, and the explicit limits |
| [`examples/`](examples) | `hello`, `wordcount` and `json-report`, each with tests |
| [`book/`](book/src/SUMMARY.md) | the same material as an mdBook, if you prefer to read it that way |

| Reference | |
|---|---|
| [CLI reference](docs/cli.md) | every command, option, exit code and JSON schema |
| [Standard library](docs/stdlib.md) | every public function, generated from the compiler's own doc tables |
| [Packages](docs/packages.md) | `yashy.toml`, the lockfile, the store, `yashyc audit`, and publishing |
| [Error catalog](docs/errors.md) | stable codes, each with a cause, an example and a fix |
| [Optimization](docs/optimization.md) | what the verifier proves, what LLVM decides, and how to read `--opt-report` |
| [Benchmarks](benchmarks/README.md) | methodology, the CI gate, and the full result tables |
| [Release](docs/release.md) | how a release is built, signed and verified |

The design record is
[`docs/superpowers/specs/2026-10-27-yashy-master-design.md`](docs/superpowers/specs/2026-10-27-yashy-master-design.md).
The completed 1.0 plan set, and the earlier milestones, sit under
[`docs/superpowers/plans/history/`](docs/superpowers/plans/history); nothing
there is authoritative.

## Repository layout

The compiler is one crate per pipeline stage:

| Crate | Job |
|---|---|
| `yashy-lexer` | bytes to tokens, with byte spans |
| `yashy-parser` | tokens to AST |
| `yashy-expand` | macro expansion, before parsing |
| `yashy-sema` | name resolution, types, exhaustiveness, ownership |
| `yashy-hir` | the typed HIR the back end consumes |
| `yashy-verify` | the verifier: contracts, leaks, effects, typestate |
| `yashy-query` | the memoised, parallel front end |
| `yashy-codegen` | typed AST to textual LLVM IR |
| `yashy-runtime` | the C shim linked into every program |
| `yashy-stdlib` | Yashy-source modules: `io`, `string`, `math`, `fs`, `path`, `JSON`, ... |
| `yashy-driver` | the `yashyc` CLI, the pipeline, the module loader, the toolchain lookup |
| `yashy-fmt` | the canonical formatter |
| `yashy-lsp` | the language server: diagnostics, hover, go-to-definition, semantic tokens |
| `yashy-manifest` | the manifest, the lockfile and the store |
| `yashy-common` | `Span`, `SourceMap`, `Diagnostic` and its rendering |
| `yashy-cache` | content-addressed cache for IR and runtime objects |
| `yashy-bench` | the benchmark harness and the CI gate |

`docs/tutorial` is the executable language tutorial, `crates/yashy-driver/tests/programs`
holds the end-to-end programs, `packages` holds the workspace's own packages,
`examples` holds complete projects, and `benchmarks` holds the measurement suite.

## Build from source

```console
$ cargo build --release --workspace
$ cargo test --workspace
$ ./target/release/yashyc doctor
```

Contributors should read [`AGENTS.md`](AGENTS.md) first: it states the rules the
codebase is held to, the commands CI runs, and the diagnostic workflow.
[`docs/gates.md`](docs/gates.md) is the runbook for every gate and what "green"
means. Release notes live in [`CHANGELOG.md`](CHANGELOG.md).

## License

MIT. See [`LICENSE`](LICENSE).
