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
- A debug build (`yashyc run --debug` or `yashyc debug`) emits LLVM debug
  metadata from HIR spans: one `DISubprogram` per function and one `DILocation`
  per checked statement. The linked object has a real `.debug_line` section, so
  a debugger can break at a source line inside a function and `yashyc inspect
  --addr` can map an address back to the declaration's source span.
- Repeat `--break` to stop in several functions; arguments after `--` are
  passed to the debugged program.
- `inspect --human` prints a clickable `path:line:col: signature` line for the same source. `inspect --addr 0x… --binary <path> <source.ya>` resolves an address through `llvm-nm` and the compiler's own index to that function's source span.

## What is not there yet

The line table now covers checked statements, but there is still no type table:

- a debugger can stop at a source line, but local variables and user types do
  not appear as typed values;
- the LLDB summary providers are committed but cannot be attached without a
  type table.

Emitting type locations for locals and parameters is the next slice of work.
