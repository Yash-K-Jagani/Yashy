# Profiling

`yashyc profile` is deliberately narrower than the 2.0 design's goal. This page
says what it measures today.

## Wall-clock report

```text
yashyc profile main.ya
yashyc profile --report recorded.json
```

Without `--report`, the compiler builds the program, runs it once to warm the
cache, runs it again for the measurement and prints the wall time. There are no
invented per-function counters: a fresh run reports one row for `main` and says
where the per-function table comes from. `--report <file>` renders a recorded
counter table produced by a build that collected one.

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
empty `sites` array.

The trace table is bounded at 16384 events; a run that fills it reports a
`dropped` count rather than a shorter, silently wrong report. The runtime does
no symbolisation itself: a program that does not ask for a trace pays one
cached `getenv` and records nothing.

## What is not here

- Sampling CPU profiler (`perf_event_open`, ETW, `sample`): not integrated.
  Symbols would need per-instruction LLVM debug metadata; debug builds currently
carry only a function-entry line table, which is not enough to attribute samples
inside a function.
- Allocation profiler with source spans: shipped for allocations and frees; a
  leaking program cannot be built without `--no-verify`, so `sites` is empty on
  a verified program by construction.
- Flame graph: not implemented.

A `--sample` flag is reserved in the command table but the measurement path is
the counter profiler until native sampling lands. See
[the phase plan](superpowers/plans/2026-11-15-phase-02-developer-surface.md)
for the tracked gap.
