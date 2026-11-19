# Profiling

`yashyc profile` is deliberately narrower than the 2.0 design's goal. This page
says what it measures today.

## Wall-clock report

```text
yashyc profile main.ya
yashyc profile --report recorded.json
```

Without `--report`, the compiler builds the program, runs it once and prints the
wall time. There are no invented per-function counters: a fresh run reports one
row for `main` and says where the per-function table comes from. `--report
<file>` renders a recorded counter table produced by a build that collected one.

The measured program's stdout is discarded so the report is the only thing on
stdout and `--json` stays parseable; its stderr still reaches the terminal, so a
fault is not hidden. The same is true of `--sample`.

`--json` renders either form as the schema-v1 envelope (`program`,
`overhead_pct`, `functions`) instead of the table.

```text
yashyc profile --json main.ya
yashyc profile --json --report recorded.json
```

## Allocation total

```text
yashyc profile --alloc-report main.ya
```

The program is rebuilt with debug info and run with `YASHY_TRACE_FILE` set. The
runtime records every allocation and free with its byte size, its address, the
caller's return address and a bounded three-frame stack; the driver symbolises
those addresses with `llvm-symbolizer` and emits a schema-v1 envelope. It has
`live_allocations`, an `allocations` array grouping every allocation by its
source span (count and bytes), and a `sites` array with the same shape for the
blocks still live at exit. A program that frees everything reports `0` and an
empty `sites` array. A verified program cannot leak, so measuring a deliberate
leak is the one case for `--no-verify`; the build is stamped unverified in the
usual way and the leak's span is named.

The trace table is bounded at 16384 events; a run that fills it reports a
`dropped` count rather than a shorter, silently wrong report. The runtime does
no symbolisation itself: a program that does not ask for a trace pays one
cached `getenv` and records nothing.

## Native sampling

```text
yashyc profile --sample main.ya
yashyc profile --sample --json main.ya
```

The program is rebuilt with debug info and at `-O0`, so a source function is a
distinct symbol rather than an inlined copy of its caller. The runtime then
starts a native sampler before `main`:

- Windows: a watchdog thread suspends the program's thread for a
  `GetThreadContext` and records the instruction pointer;
- Linux and macOS: `ITIMER_PROF` delivers `SIGPROF`, whose `ucontext` names the
  interrupted instruction.

Samples are image-relative and are resolved with the same `llvm-symbolizer` and
debug info the event trace uses. The table reports each function's sample count
and share; `hot()` (and the `--json` form) says whether one function owns at
least 80% of them, which is the attribution bar. The report never turns a
counter into a sample: a program that finishes before the first tick writes an
empty sample file, the driver says so, and the wall-clock counter path runs
instead.

`--json` adds `mode: "sample"`, `total`, `dropped`, `symbolized`, `hot` and a
`functions` array of `{name, samples, percent}`. `hot` names the function that
owns at least 80% of the samples, or is `null` when none does. Without a
symbolizer the rows keep their image-relative addresses and `symbolized` is
`false`.

## What is not here

- Stack walking: a sample is the interrupted function only. The debug info that
  would name a caller chain is present, but no platform unwind API is wired in.
- Flame graph: not implemented.
- Per-instruction attribution: a sample names the enclosing source line through
  the line table. It is not an instruction-level profile.

A verified program cannot leak, so the allocation report's `sites` array is
empty on one by construction; it is the trace of what the runtime recorded, not
a guess. See
[the phase plan](superpowers/plans/2026-11-15-phase-02-developer-surface.md)
for the tracked gap.
