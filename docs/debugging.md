# Debugging

`yashyc debug <file.ya>` builds a program, writes a native debugger's command
file and starts the debugger. It never writes into the project: the binary and
the script live in a temporary directory that is removed when the session ends.

```text
yashyc debug main.ya
yashyc debug main.ya --break work -- --flag value
yashyc debug --binary target/release/app --break main
```

## Backend

LLDB is preferred on every platform. On Windows, the Microsoft Console Debugger
(`cdb`) is used when LLDB is absent. With neither on `PATH`, the command exits
`2` with an install hint. The chosen backend and the script path are printed on
stderr, so stdout stays available to the program being debugged.

## What works today

- The script sets `YASHY_DEBUG=1` in the child. On a fault the runtime prints
  the live task table: task id, function and resume index.
- Symbol breakpoints (`--break <fn>`) stop in named functions. With no
  `--break`, the entry point `main` is used.
- A debug build (`yashyc run --debug` or `yashyc debug`) also emits LLVM
  function-level debug metadata from HIR spans. The linked object has a real
  `.debug_line` section, so a debugger can set a source-line breakpoint at a
  function's entry line and `yashyc inspect --addr` can map an address back to
  the function's source span.
- Repeat `--break` to stop in several functions; arguments after `--` are
  passed to the debugged program.
- `inspect --human` prints a clickable `path:line:col: signature` line for the same source. `inspect --addr 0x… --binary <path> <source.ya>` resolves an address through `llvm-nm` and the compiler's own index to that function's source span.

## What is not there yet

The line table currently has function entries only, not a location on every
instruction, so:

- source-line breakpoints work at a function's entry line, not yet inside a
  function body;
- local variables and user types do not appear as typed values;
- the LLDB summary providers are committed but cannot be attached without a
  type table.

Emitting per-instruction locations from HIR spans is the next slice of work.
