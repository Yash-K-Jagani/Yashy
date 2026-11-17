# Coverage

`yashyc coverage <file.ya>` measures which Yashy source lines ran. It is
source-based: the debug build attaches `!dbg` locations from HIR spans, clang
writes a coverage mapping from them, and llvm-cov reads the recorded profile
back as lcov records naming Yashy lines.

```text
yashyc coverage main.ya
yashyc coverage --json main.ya
yashyc coverage --format json --fail-under 80 main.ya
```

The command builds the file with `--debug-info` and coverage instrumentation
in a scratch directory, runs it with `LLVM_PROFILE_FILE` set, merges the raw
profile with `llvm-profdata merge -sparse`, and exports lcov with
`llvm-cov export -format=lcov`. Both helpers are looked up beside the clang
used for the build; `YASHY_LLVM_PROFDATA` and `YASHY_LLVM_COV` override the
lookup. If either is missing the command is a usage error naming what to
install, never a silent empty report.

Without `--format` the command prints the lcov records. With `--format json`
(or `--json`) it prints a schema-v1 envelope with `files`, `covered_lines`,
`total_lines` and a rounded `percent`. `--fail-under <pct>` exits `1` when the
covered fraction is below the threshold; it is meant for CI and is a gate, not
a change to the report. Program arguments go after `--`.

The scratch binary, raw profile and merged profile are removed after the run.
A program that exits non-zero is refused: a partial run would report coverage
that never reached the lines after a failure.
