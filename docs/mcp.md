# MCP server

`yashyc mcp --root <workspace>` serves JSON-RPC 2.0 on stdin/stdout. The verbs
are the compiler verbs `check`, `build`, `test`, `verify`, `explain`, `context`,
`index`, plus the structured mutation verbs `edit` and `refactor`. There is no
shell verb, no environment mutation, no network and no path outside the root.

## Structured edits

`edit` takes a `patch`:

```json
{"name":"edit","arguments":{"patch":{
  "expected_tree_hash":"<tree hash>",
  "edits":[{"file":"main.ya",
            "span":{"start":10,"end":14},
            "expected_hash":"<file hash>",
            "replacement":"newName",
            "rationale":"rename"}]}}}
```

Every file hash and span is checked before a write. An overlapping edit, a
stale hash, a path containing `..`, or a patched tree that fails `check` is
refused, and the tree is byte-identical after a refusal. Hashes are FNV-1a over
bytes, printed as decimal strings; they are staleness checks, not signatures.

`refactor` takes `path`, `from` and `to`. It computes the edits from the lexer's
identifier tokens, so strings and comments carrying the same word are left
alone, then applies them through the same transaction. `format` takes `path`
and applies the canonical formatter through that transaction; `lint` takes an
optional `group` and returns diagnostics without writing anything. `fix` takes
`path` and applies only machine-applicable suggestions through the same
transaction. `doc` returns the compiler's JSON documentation model for the
workspace.

Mutation verbs return `{"applied":["main.ya"],"tree_hash":"<new tree hash>"}`.
