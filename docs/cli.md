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
| `--release` | Compile out `debug_assert`; default optimization becomes `-O3` unless `-O` is given. |
| `--cpu <tier>` | `portable` (default), `x86-64-v2`, `x86-64-v3`, `x86-64-v4`, `native`. |
| `--fast-math` | Allow fused multiply-add; results may differ. |
| `--emit-ir` | Print LLVM IR and stop; do not link. |
| `--timings` | Print how long each pipeline phase took. See [Timings](#timings). |

## `yashy run <file.ya>`

Compiles and runs in one step. It accepts the same optimization, CPU, release
and timing options as `build`. Arguments after `--` go to the program.

## `yashy check <file.ya> [--json] [--timings]`

Lexes, parses, and type-checks without emitting an executable. Use it for fast
feedback.

`--json` prints the [schema v1 report](#json-diagnostics-schema-v1). The command
exits `0` when there are no errors and `1` when there is at least one error; a
missing file is a usage error (`2`). A diagnostic with a catalog code ends with
a `docs:` line naming its page, for example
`docs: https://yashy.dev/errors/E0100`.

## `yashy verify <file.ya> [--json] [--timings]`

Runs the front half of the pipeline and then the verifier: the pass that proves
a well-typed program is also *sound*. In 1a that means two things: every linear
value -- every pointer, slice, map, mutex and channel -- is released on every
path out of its scope, and every `requires`, `ensures` and loop `invariant` is
discharged. A clause the verifier cannot prove is a warning in this slice
(see [E1100](errors/E1100.md)), so the program still builds.

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

`verify` exits `0` when every diagnostic is a warning. Two kinds are warnings in
1a: a viewed `[]T`, which the view keeps alive until the function returns so the
language gives no way to release it ([E1000](errors/E1000.md)), and a contract
clause the verifier's domain cannot prove ([E1100](errors/E1100.md)).

`build` and `run` verify by default. An unverified binary is not a build, so a
program that leaks is refused before `clang` is started. `test` still runs the
checker alone in R1.

## `yashy test [path] [--filter <substring>] [--list] [--json] [--timings]`

Runs every `.ya` file under `tests/` (or `path`). Each file is an ordinary
program with a `main`; exit `0` is a pass, a panic or non-zero exit is a
failure.

| Option | Meaning |
|---|---|
| `--filter <substring>` | Run only tests whose path contains the substring. |
| `--list` | Print the tests that would run and exit. |
| `--json` | Print the [schema v1 report](#json-diagnostics-schema-v1) instead of the progress lines. |
| `--timings` | Print the suite's total time per pipeline phase. See [Timings](#timings). |

`--json` writes one envelope to stdout. Compile diagnostics are nested under
the test that produced them; top-level `diagnostics` holds setup errors such as
a missing toolchain. `--list` and `--json` cannot be combined. A failing test
makes the command exit `1`; a setup problem exits `2`; a directory that cannot
be created exits `3`.

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
  "yashy_version": "0.1.0",
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
timing: link 253 ms
```

The phases are `load` (reading and parsing the entry point and its imports),
`check` (names and types), `verify` (soundness), `emit` (LLVM IR) and `link`
(the single `clang` invocation). A command reports only the phases it runs:
`check` has no `verify`, `emit` or `link`, and `verify` has no `emit` or `link`.
A phase that fails still gets a line, because a build that is slow *and* broken
is when the number is most useful.

`test --timings` prints one total for the whole suite rather than five lines per
test, so the report answers where the run's wall time went.

## JSON diagnostics (schema v1)

`check --json`, `verify --json` and `test --json` share one envelope. The
`tool` field names the command. The schema is v1 for R1: additive changes are
allowed within 0.x, and it is frozen at 1.0. Output is pretty-printed JSON on
stdout; `--json` implies no color and no prompt.

```json
{
  "schema_version": 1,
  "tool": "check",
  "yashy_version": "0.1.0",
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
  milliseconds. `test --json --timings` carries the suite's totals.

`test --json` uses the same envelope plus `tests` and a test summary:

```json
{
  "schema_version": 1,
  "tool": "test",
  "yashy_version": "0.1.0",
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
  "summary": { "passed": 0, "failed": 1, "skipped": 0 }
}
```

`status` is `passed` or `failed`; `skipped` is reserved for a later release.
`duration_ms` covers compiling and running the test. A runtime failure has a
synthetic diagnostic with the exit status and no code.

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
  "yashy_version": "0.1.0",
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
  "next": null
}
```

In JSON mode all output goes to stdout; human diagnostics are suppressed. A
failure has `"ok": false`, an `error` on the failing section, and a non-null
`next`.

## `yashy fmt [-w] <files...>`

Prints the canonical formatting to stdout. With `-w`, rewrites the files in
place.

## `yashy version`

Prints the compiler version.

## Environment

| Variable | Effect |
|---|---|
| `YASHY_STDLIB` | Directory containing `io.ya`; overrides bundled stdlib discovery. |
| `YASHY_PATH` | Replaces the entry-directory import roots. The bundled stdlib is still appended. |
| `YASHY_LLC` | Names the `llc` executable directly. Builds still need `clang` on `PATH`. |
| `YASHY_LEAK_REPORT=1` | Print live allocations at process exit. |

## Installer flags

The installers are separate scripts shipped with a release. They support
`--prefix` / `-Prefix`, `--no-path` / `-NoPath`, `--no-doctor` / `-NoDoctor`,
and `--uninstall` / `-Uninstall`. See the [quickstart](quickstart.md).
