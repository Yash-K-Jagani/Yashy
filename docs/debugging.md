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
  metadata from HIR spans: one `DISubprogram` per function, one `DILocation`
  per checked statement scoped to its own function, one `DILexicalBlock` per
  nested `{ ... }` scope, and one `DILocalVariable` plus `llvm.dbg.declare`
  per parameter and `let` binding. A binding is scoped to the lexical block it
  is declared in and carries that statement's line, so a debugger shows it only
  while the block is live. The linked object has a real `.debug_line` section,
  so a debugger can break at a source line inside a function and `yashyc
  inspect --addr` can map an address back to the declaration's source span.
- `frame variable` shows a local as a typed value. `int`, `float`, `bool`,
  `char`, `string`, `*T`, user structs and user enums carry a DI type; a
  `string` is described as the runtime's `yashy_str` header reached through its
  pointer, which is what lets the summary providers print the text. A local of
  a type with no DI type yet (tuple, slice, map, task, closure) is left out
  rather than described wrongly.
- `yashyc debug` imports the value printers from
  `crates/yashy-runtime/debug/pretty_printers.py`; the compiler embeds the file,
  writes it beside the script, and `command script import`s it before the first
  breakpoint. The providers render `string`, arrays/slices, maps and
  `Option`/`Result` as values rather than addresses.
- Repeat `--break` to stop in several functions; arguments after `--` are
  passed to the debugged program.
- `inspect --human` prints a clickable `path:line:col: signature` line for the same source. `inspect --addr 0x… --binary <path> <source.ya>` resolves an address through `llvm-nm` and the compiler's own index to that function's source span.

## What is not there yet

- `Option[T]` and `Result[T, E]` locals are described by the synthetic structs
  their summary providers read, not by the checker's exact tag/payload layout;
  a scripted session that asserts a printed value is still missing.
- A scripted debugger session that asserts a typed value prints a particular
  string is not part of the test suite; LLDB is not required to build or test
  the compiler.
