# 19. Tooling

## What you will learn

- `yashy index`, `outline`, `graph` and `search`: the compiler's
  symbol table in four shapes.
- `yashy context`: the token-budgeted pack an agent reads first.
- `yashy doc`: the public surface rendered from the compiler's tables.
- `yashy mcp`: the same verbs over JSON-RPC, sandboxed to the workspace.
- `run --sandbox`: a default-deny capability set for a program launch.
- `log`, `metrics` and `--deterministic`: program observability that a
  test can compare byte for byte.

A compiler that only a person can read is a compiler an agent cannot
drive. Phase 11 makes the compiler legible to a machine on the same
terms: one fact, two renderings. Every command below builds a JSON model
first and renders the human form from it, so the two cannot drift.

## The index

`yashy index` walks a directory and answers what the compiler knows about
its symbols: name, kind, file, span, signature, effects, contracts,
typestate, visibility and docs. A fact the compiler does not know is
`null`, never omitted and never invented.

```text
$ yashy index examples/hello --json
{
  "schema_version": 1,
  "tool": "yashy",
  "files": ["E:/.../examples/hello/main.ya"],
  "symbols": [
    {
      "name": "main",
      "kind": "fn",
      "signature": "fn main() -> void",
      "effects": { "io": true, "unsafe": false, "async": false },
      "docs": null
    }
  ],
  "imports": [],
  "tests": []
}
```

`outline` prints one file's symbols in source order;
`graph --dot` prints the module and call graph in Graphviz;
`search <query>` ranks exact-name before prefix before doc matches.

## Context

`yashy context --task "fix the bug in main"` assembles a pack — the
enclosing function, its callees' signatures, the tests that cover it —
in a fixed relevance order, adding each section only if the whole pack
still fits its token budget. `tokens`, `budget`, `included` and `dropped`
are all reported; a section that did not fit is named in `dropped`, never
silently truncated away. The estimator is documented: one token per four
bytes of ASCII, one per non-ASCII character, rounded up.

## Docs and the editor

`yashy doc` writes one Markdown file per module into `target/doc/` plus an
`index.md`. A private item appears only with `--private`; an undocumented
one still appears, marked `No documentation.` — an undocumented public
item is a fact worth printing, not a gap to hide. `--json` emits the same
model the Markdown renders from. The output is byte-identical across runs
and operating systems, so CI can assert the committed docs are current.

`yashy lsp` execs the sibling `yashy-lsp` binary on stdio, so an editor
needs one command. When the sibling is missing, `yashy lsp` exits 2
naming the path, and `yashy doctor` says whether it was found.

## MCP

`yashy mcp --root <dir>` speaks JSON-RPC 2.0 over stdio with seven tools:
`check`, `build`, `test`, `verify`, `explain`, `context`, `index`. Every
tool returns the schema-v1 payload the CLI returns. No tool accepts a path
outside `--root`, no tool accepts a shell string, and a malformed request
is answered with an error while the server stays alive.

## The sandbox

`yashy run --sandbox --allow-read <path> --allow-write <path>` runs the
program with a default-deny capability set: those paths, and nothing
else. A read or write outside the allowlist is `E1900` naming the
capability; a time, memory or output breach is `E1901` naming the limit.
Windows enforces with a Job Object, Linux with rlimits, and the runtime's
file entry points consult the allowlist on both.

## Deterministic runs

`--deterministic` fixes the clock at a given instant, seeds the RNG, and
pins the process id field, so `log_info`/`log_error` and
`metrics_add`/`metrics_set` — all JSON lines on stderr, all `io` effects,
so a `pure fn` cannot log — produce byte-identical output across runs:

```text
{"ts":1700000000000,"level":"info","message":"started"}
{"ts":1700000000000,"metric":"tasks","kind":"counter","value":3}
```
