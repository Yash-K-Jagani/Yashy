# Yashy CLI reference

The `yashy` binary has one command per job. A command that fails exits `1`; a
successful command exits `0`. A program's `main` may return an `int`, which
becomes its process exit status.

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

## `yashy run <file.ya>`

Compiles and runs in one step. It accepts the same optimization, CPU, and
release options as `build`. Arguments after `--` go to the program.

## `yashy check <file.ya>`

Lexes, parses, and type-checks without emitting an executable. Use it for fast
feedback.

## `yashy test [path] [--filter <substring>] [--list]`

Runs every `.ya` file under `tests/` (or `path`). Each file is an ordinary
program with a `main`; exit `0` is a pass, a panic or non-zero exit is a
failure.

| Option | Meaning |
|---|---|
| `--filter <substring>` | Run only tests whose path contains the substring. |
| `--list` | Print the tests that would run and exit. |

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
