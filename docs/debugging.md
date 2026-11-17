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
- Repeat `--break` to stop in several functions; arguments after `--` are
  passed to the debugged program.
- `yashyc inspect --human <file.ya> <symbol>` prints a clickable
  `path:line:col: signature` line for the same source.

## What is not there yet

The LLVM backend does not emit `!dbg` metadata, so the linked binary has no
line table or type table for Yashy source. That means:

- source-line breakpoints are not available; use symbol names;
- local variables and user types do not appear as typed values;
- the LLDB summary providers in
  `crates/yashy-runtime/debug/pretty_printers.py` are committed but not loaded
  automatically, because there is no type name to attach them to yet.

Emitting line and type tables from the HIR spans is the next slice of work. The
command is deliberately documented as it behaves, not as the plan hoped it
would behave.
