# AGENTS.md

Yashy is a small statically typed language compiled to native binaries through
LLVM. The compiler is Rust; the language and its runtime are not. This file is
the short guide for a coding agent working in the repository. The design is
`docs/superpowers/specs/2026-10-27-yashy-master-design.md`; the master plan under
`docs/superpowers/plans/` is the source of truth for what to build next. Read the
active phase before changing behavior. Superseded plans and specs sit in
`history/` inside those two folders and are not authoritative.

## Commands

```text
cargo build --release --workspace
cargo check --workspace --all-targets
cargo test --workspace
cargo test -p yashy-driver --test doctor
cargo test -p yashy-driver --test tutorial
cargo test -p yashy-driver --test examples
cargo test -p yashy-driver --test init
cargo test -p yashy-driver --test explain
cargo test -p yashy-driver --test diagnostics_json
cargo test -p yashy-driver --test docs
cargo test -p yashy-driver --test agents
cargo fmt --all
```

On Windows, the default MSVC toolchain links the compiler, but a local
`x86_64-pc-windows-gnu` toolchain is the one used by some contributors:
`cargo +stable-x86_64-pc-windows-gnu test ...`. CI uses the MSVC toolchain.

The end-to-end programs under `crates/yashy-driver/tests/programs/` are the
language's worked examples. The tutorial harness compiles and runs every
`yashy` block in `docs/tutorial/`, and the examples harness runs each project
under `examples/`. A change to the language is not done until those still pass.

## Diagnostics and JSON

Human diagnostics are the primary interface. A coded diagnostic ends with a
`docs:` line, and `yashy explain <code>` prints the same long-form page offline.

```text
yashy check --json examples/hello/main.ya
yashy test --json examples/hello
yashy explain E0100
yashy explain --json E0100
```

`check --json` and `test --json` share
[schema v1](docs/cli.md#json-diagnostics-schema-v1). The envelope has
`schema_version`, `tool`, `yashy_version`, `success`, `diagnostics`, and
`summary`; a diagnostic has `code` (null when uncoded), `severity`, `message`,
`file`, `span`, optional `labels`, optional `notes`, and `docs_url` only when
coded. `test --json` adds `tests` and its own summary. `--json` implies no color
and no prompt; exit codes are `0` success, `1` diagnostics or tests failed, `2`
usage or configuration error, and `3` internal error.

Workflow for a compiler error:

1. Run `yashy check --json <file.ya>` and read `diagnostics[]`.
2. Take a `code` and run `yashy explain --json <code>` for `title`, `summary`,
   `page`, and `docs_url`; `yashy explain <code>` prints the human page.
3. Fix the cause, rerun `check`, then run `yashy test --json` and inspect
   `tests[]` and `summary`.

The catalog lives in [docs/errors.md](docs/errors.md); every code has a page such
as [E0100](docs/errors/E0100.md) with a cause, a minimal example, and the fix.

## Layout

| Path | Job |
|---|---|
| `crates/yashy-lexer` | bytes to tokens |
| `crates/yashy-parser` | tokens to AST |
| `crates/yashy-sema` | name resolution, types, exhaustiveness, ownership |
| `crates/yashy-codegen` | typed AST to textual LLVM IR |
| `crates/yashy-runtime` | the C shim linked into every program |
| `crates/yashy-driver` | CLI, pipeline, toolchain and stdlib discovery |
| `crates/yashy-stdlib` | Yashy-source modules (`io`, `string`, `math`, `JSON`, ...) |
| `crates/yashy-fmt` | the canonical formatter |
| `crates/yashy-lsp` | the language server |
| `docs/tutorial` | the executable language tutorial |
| `docs/superpowers` | design records and milestone plans |
| `scripts` | release packaging and installer tests |
| `examples` | complete projects used by the examples test |

## Rules

- **Keep the language's identity.** No garbage collector, borrow checker,
  exceptions, implicit coercion, operator overloading, class inheritance, or
  power-user procedural macros.
- **The object model is Rust-style, not class-based.** Traits are requirement
  sets with no state; `impl` is inherent or `impl Trait for Type`; dispatch is
  static only, so there is no `dyn`, no vtable and no runtime type information;
  receivers are `self` or `self: *mut T`; associated functions are `Type::name`
  and are not methods. There is no `Drop`: release is `defer` and `.free()`.
  A trait never gains fields, a type never gains a parent.
- **Optimization is a phase, not a habit.** Do not add a backend trick, an
  attribute or an `inline` keyword to make one benchmark faster. The optimizer
  spends what the verifier proved (phase 4); a pass is registered in the pass
  manager and held to its own benchmark row.
- **Tests are ordinary programs.** A file under `tests/` is a Yashy program
  with a `main`; exit 0 is a pass. Do not add `#[test]` attributes or test
  macros.
- **No new runtime dependencies** without a plan and a reason. The compiler is
  deliberately dependency-light; the runtime is C and is embedded in the
  compiler binary.
- **A malformed program must produce a diagnostic, not a Rust panic.** The
  `never_panics` suite enforces this.
- **Ownership is explicit.** `move` transfers; `defer` releases at scope exit;
  arrays, maps, channels, mutexes, and built strings use `.free()`.
- **Errors are values.** `Option[T]` for a possibly missing value and
  `Result[T, E]` for a fallible operation; `?` propagates. There are no
  exceptions.
- **Every feature ships with tests, docs, diagnostics, and a machine-readable
  form** where the plan requires one.
- **Format touched files.** `cargo fmt --all --check` has pre-existing drift;
  do not reformat unrelated files.
- **Do not claim macOS support** until a signed and tested macOS archive ships.
  macOS is a 1.0 target.

## Docs

- [Quickstart](docs/quickstart.md)
- [Tutorial](docs/tutorial/00-intro.md)
- [First CLI](docs/tutorial/12-first-cli.md)
- [CLI reference](docs/cli.md)
- [Error catalog](docs/errors.md)
- [Examples](examples)
- [llms.txt](llms.txt)

## Working with the plans

Each slice plan has a task list with checkboxes, acceptance criteria, and
verification commands. Keep the ledger under
`.superpowers/sdd/<date>-<slice>/progress.md` current, mark plan boxes as they
land, and record the commit hash in the plan when the slice is complete. Do not
start a later slice before the current one's acceptance commands pass.
