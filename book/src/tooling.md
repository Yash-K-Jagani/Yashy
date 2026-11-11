# The tools around the compiler

The compiler is one command. Around it are the commands that answer the
questions a compiler is asked: what is in this program, what does it depend
on, where did that time go, and is this source formatted the way the
repository wants it?

## The commands

| Command | What it answers |
|---|---|
| `yashy check <file>` | does this program type-check? |
| `yashy verify <file>` | is this program sound? |
| `yashy build <file>` | make me a binary |
| `yashy run <file>` | make me a binary and run it |
| `yashy test [dir]` | run every program under `tests/` |
| `yashy doctor` | is this machine able to compile at all? |
| `yashy fmt --check <files>` | is this source formatted the way the repository wants it? |
| `yashy index [dir]` | what symbols does this tree define? |
| `yashy outline <file>` | what is the shape of this program? |
| `yashy context --task "..."` | what do I need to read to do this? |
| `yashy doc [dir]` | the documentation, rendered from the doc blocks |
| `yashy explain <code>` | what does this diagnostic mean? |
| `yashy mcp --root <dir>` | the same facts, over the model context protocol |

## Every command has a JSON form

`--json` prints the same facts as a document, so a script does not have to
parse text:

```text
$ yashy check --json main.ya
{
  "schema_version": 1,
  "tool": "check",
  "success": true,
  "diagnostics": [],
  "summary": { ... }
}
```

The exit codes are the same everywhere: `0` success, `1` diagnostics or
failed tests, `2` a usage or configuration error, `3` an internal error.

## The format gate

`yashy fmt --check` writes nothing, prints one line per file that is not
already canonical, and exits `1` if there is at least one. It is the gate CI
enforces, so a pull request that reformats a file is caught by the machine
rather than by a reviewer:

```text
$ yashy fmt --check src/main.ya
src/main.ya
```

The comparison normalises line endings first, because `core.autocrlf=true` is
the default on Windows and a gate that fails on a clean checkout is a gate
people disable.

## The index

`yashy index` prints every symbol a tree defines — functions, structs,
enums, traits and methods — with its signature, its effects, its contracts
and its documentation. The output is sorted, so it is byte-identical across
runs:

```text
$ yashy index --json
{
  "schema_version": 1,
  "tool": "index",
  "symbols": [ ... ]
}
```

## The context pack

`yashy context --task "add pagination to the API"` reads the tree and returns
a pack: the functions the task touches, the tests that cover them, and
nothing else. The pack says what it dropped, so a reader knows what they are
not seeing.

## See also

- [Tooling in the tutorial](language-guide/19-tooling.md) — the same
  commands, with the language's own account of them.
- [The CLI reference](../cli.md) — every flag, in one place.

## Exercise

Write a program that prints its arguments, one per line, and run it with
three arguments. The standard library's `args` module has them; the point of
the exercise is that the arguments arrive in order and do not include the
program's own path.

The solution is `book/tests/exercise-args.ya`.
