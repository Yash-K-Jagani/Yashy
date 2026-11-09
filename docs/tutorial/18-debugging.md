# 18. Debugging

## What you will learn

- `yashy run --debug`: the task table a panic prints.
- `yashy tasks`: the live-task table while the program runs.
- `yashy profile`: per-function counters, with the overhead measured.

A program that hangs silently is the worst kind of bug: nothing tells you
which piece is stuck. The async debugging surface answers exactly that, in
the vocabulary the executor already has: task id, function, resume index,
and whether the task is running or pending.

## The task table

`yashy run --debug` runs the child with `YASHY_DEBUG=1`. When the program
panics (or faults on arithmetic), the runtime prints the executor's live
tasks before the process ends:

```text
tasks: 2
task 1 main_task resume=0 state=running
task 2 blocked_on_channel resume=0 state=pending
```

`resume` is the resume index — the next suspension point the task's poll
dispatches on. `state` is `running` while the task is inside its poll and
`pending` while it is queued or suspended.

`yashy tasks` prints the same table while the program runs
(`YASHY_TASKS_DUMP=1`), so a program that never finishes still shows why.
It is debug builds only: `--release` is refused.

## The profiler

`yashy profile` measures a program with per-function counters — one
counter per function, incremented on entry and exit — rather than by
sampling. There is no sampling thread: a sampling profiler changes the
scheduling it measures. Self time is total minus children, and the report
prints the instrumentation overhead next to the table, because a number
without its measurement cost is the wrong number:

```text
$ yashy profile --report counters.json
program: workload
hot                      calls=1        self_ms=     0.009 total_ms=     0.010
small                    calls=10       self_ms=     0.001 total_ms=     0.002
instrumentation overhead: 12.50%
```

`yashy profile prog.ya` without `--report` builds and times the program;
the per-function table comes from the recorded report. `run --debug`
also writes `.yashy/debug.lldb`, a command file that breakpoints `main`
and dumps the task table on stop (launch it with `lldb -s
.yashy/debug.lldb run`); without a debugger, `YASHY_DEBUG=1` prints the
same table on a panic.
