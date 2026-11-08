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
yashy verify --json --timings --inventory examples/hello/main.ya
cargo test -p yashy-driver --test doctor
cargo test -p yashy-driver --test tutorial
cargo test -p yashy-driver --test examples
cargo test -p yashy-driver --test verify
cargo test -p yashy-driver --test unverified
cargo test -p yashy-driver --test init
cargo test -p yashy-driver --test explain
cargo test -p yashy-driver --test diagnostics_json
cargo test -p yashy-driver --test docs
cargo test -p yashy-driver --test agents
cargo fmt --all
```

The active phase plan under `docs/superpowers/plans/` is the source of truth for
what to build next; the ledger under `.superpowers/sdd/` records what landed and
which acceptance commands ran. Do not start a later slice before the current
one's gate passes.

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
yashy verify --json examples/hello/main.ya
yashy test --json examples/hello
yashy explain E0100
yashy explain --json E0100
```

`check --json`, `verify --json` and `test --json` share
[schema v1](docs/cli.md#json-diagnostics-schema-v1). The envelope has
`schema_version`, `tool`, `yashy_version`, `success`, `diagnostics`, and
`summary`; a diagnostic has `code` (null when uncoded), `severity`, `message`,
`file`, `span`, optional `labels`, optional `notes`, and `docs_url` only when
coded. A verifier diagnostic also carries a `proof` object naming the
`obligation`, the `missing_fact`, and the `remedy`. `test --json` adds `tests`
and its own summary. `--json` implies no color and no prompt; exit codes are `0`
success, `1` diagnostics or tests failed, `2` usage or configuration error, and
`3` internal error.

Workflow for a compiler error:

1. Run `yashy check --json <file.ya>` and read `diagnostics[]`.
2. Take a `code` and run `yashy explain --json <code>` for `title`, `summary`,
   `page`, and `docs_url`; `yashy explain <code>` prints the human page.
3. Fix the cause, rerun `check`, then run `yashy test --json` and inspect
   `tests[]` and `summary`.
4. Run `yashy verify --json <file.ya>` before claiming a build: `build` and
   `run` refuse an unsound program, and `proof` says what is missing.

The catalog lives in [docs/errors.md](docs/errors.md); every code has a page such
as [E0100](docs/errors/E0100.md) with a cause, a minimal example, and the fix.

## Layout

| Path | Job |
|---|---|
| `crates/yashy-lexer` | bytes to tokens |
| `crates/yashy-parser` | tokens to AST |
| `crates/yashy-sema` | name resolution, types, exhaustiveness, ownership |
| `crates/yashy-query` | memoised pipeline: content-hashed query cache and parallel front end |
| `crates/yashy-codegen` | typed AST to textual LLVM IR |
| `crates/yashy-runtime` | the C shim linked into every program |
| `crates/yashy-driver` | CLI, pipeline, module loader (graph, cycles, aliases), toolchain and stdlib discovery |
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
  power-user procedural macros. Macros are declarative only: a macro is data,
  not a program the compiler runs — expansion happens before parsing, once,
  in `yashy-expand`, with the caps in its spec (`E1600` no match,
  `E1601` depth/size, `E1602` fragment kind, `E1603` duplicate name).
- **Async is one executor.** `async fn` returns `Task[T]` and suspends at
  `.await`; `scope` joins or cancels its children before it is left. There is
  no pluggable runtime, no `async` trait method (`E1703`), no `Future`
  trait and no async closures. A lock held across `.await` is `E1702`, a
  task escaping its scope is `E1701`, and an `await` outside an `async fn`
  is `E1700`.
- **Patterns are irrefutable in binding position.** A pattern in `let`,
  `const` and `for` must match every value of its type; a refutable pattern
  there is `E0100` with the fix "use `switch`". In `switch`, every
  alternative of an or-pattern binds the same names with the same types
  (`E1002` otherwise), and a struct pattern writes `field: binding`, never
  the shorthand.
- **Sugar desugars; it does not add HIR.** Phase 5's forms (`if let`,
  `?` on `Option`, trailing blocks, spread, chained comparisons, `for i, x`,
  field defaults, `arena { }`, labels) and phase 7's (or/range/nested
  patterns, patterns in binding position, named and default arguments) lower
  into HIR that already exists — `crates/yashy-sema/tests/desugar.rs` is the
  proof. Named arguments reach the HIR positional; defaults are inserted at
  resolution. The two recorded
  exceptions are `HirStmt::IfLet` (two blocks, which a `switch` arm cannot
  hold) and `HirBlock::arena` (one placement bit). `///` is documentation,
  not an attribute: it is stored on the item for `yashy doc` and interpreted
  nowhere else.
- **The object model is Rust-style, not class-based.** Traits are requirement
  sets with no state; `impl` is inherent or `impl Trait for Type`; dispatch is
  static only, so there is no `dyn`, no vtable and no runtime type information;
  receivers are `self` or `self: *mut T`; associated functions are `Type::name`
  and are not methods. There is no `Drop`: release is `defer` and `.free()`.
  A trait never gains fields, a type never gains a parent.
- **Generic types are schemes, not templates.** `Pair[T]` is a scheme and
  `Pair[int]` is one monomorphic type; a generic function is instantiated
  per substitution, capped at 64 per function and 4096 per program (`E1505`).
  `Iterator[T]` is the one compiler-known generic trait; user generic traits
  are `E1511`. The excluded forms are named in writing: inheritance `E1512`,
  `Drop` `E1513`, trait fields `E1514`, operator overloading `E1515`.
- **Optimization is a phase, not a habit.** Do not add a backend trick, an
  attribute or an `inline` keyword to make one benchmark faster. The optimizer
  spends what the verifier proved (phase 4); a pass is registered in the pass
  manager and held to its own benchmark row, and `--opt-report` explains every
  check and allocation that survived.
- **Tests are ordinary programs.** A file under `tests/` is a Yashy program
  with a `main`; exit 0 is a pass. Do not add `#[test]` attributes or test
  macros.
- **No new runtime dependencies** without a plan and a reason. The compiler is
  deliberately dependency-light; the runtime is C and is embedded in the
  compiler binary.
- **A malformed program must produce a diagnostic, not a Rust panic.** The
  `never_panics` suite enforces this.
- **A declaration writes the type first.** `let int x = 5;`,
  `const string name = "x";`, `fn add(int a, int b) -> int`, `|int x| -> int`.
  The name-first spelling `let x: int = 5;` is `E0109`, not a second form. An
  untyped `let x = 5;` still infers from its initialiser. A field list and a
  function *type* keep the colon: `struct Point { x: int }`, `fn(int) -> int`,
  and the receiver stays `self: *mut T`.
- **Ownership is explicit.** `move` transfers; `defer` releases at scope exit;
  arrays, maps, channels, mutexes, and built strings use `.free()`.
- **Errors are values.** `Option[T]` for a possibly missing value and
  `Result[T, E]` for a fallible operation; `?` propagates. There are no
  exceptions.
- **A builtin never answers with a sentinel.** `read_file` is
  `Result[string, io.Error]`, `getenv` is `Option[string]`, `parse_i64` is
  `Option[int]`. `""` and `0` are values a program can produce legitimately, so
  they cannot also mean "failed".
- **Bitwise arithmetic is checked and total.** `& | ^ ~ << >>` take integers
  (and `bool` for the first three) with no implicit widening; a shift count out
  of range is a fault at run time and `E0105` in a `const`. `>>` is arithmetic
  on a signed integer.
- **`repr(C)` is a keyword form, not an attribute system.** It is one clause on
  a struct, enum or object, and it is the only thing that licenses the C
  spellings `u8`, `i32` and friends. Do not grow it into `#[...]` syntax.
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
- [Optimization](docs/optimization.md)
- [Error catalog](docs/errors.md)
- [Examples](examples)
- [llms.txt](llms.txt)

## Working with the plans

Each slice plan has a task list with checkboxes, acceptance criteria, and
verification commands. Keep the ledger under
`.superpowers/sdd/<date>-<slice>/progress.md` current, mark plan boxes as they
land, and record the commit hash in the plan when the slice is complete. Do not
start a later slice before the current one's acceptance commands pass.
