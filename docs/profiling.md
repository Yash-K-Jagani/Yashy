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

The program is rebuilt and run with `YASHY_LEAK_REPORT=1`. The C runtime prints
its live-allocation total at exit; the command parses that line and emits a
schema-v1 envelope with `live_allocations` and a note explaining what is not
there. A program that frees everything it allocates reports `0`.

The report has **no allocation site, size histogram or bounded stack**. The
runtime counts allocations but does not carry a site id or a size per live
block, so a leaking site cannot be named. That is the next slice of work, not a
number this command guesses.

## What is not here

- Sampling CPU profiler (`perf_event_open`, ETW, `sample`): not integrated.
  Symbols would need LLVM debug metadata, which the backend does not emit.
- Allocation profiler with source spans: not implemented; see above.
- Flame graph: not implemented.

A `--sample` flag is reserved in the command table but the measurement path is
the counter profiler until native sampling lands. See
[the phase plan](superpowers/plans/2026-11-15-phase-02-developer-surface.md)
for the tracked gap.
