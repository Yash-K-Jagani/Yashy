# MCP server

`yashyc mcp --root <workspace>` serves JSON-RPC 2.0 on stdin/stdout. The verbs
are the seven compiler verbs `check`, `build`, `test`, `verify`, `explain`,
`context`, `index`, the structured mutation verbs `edit` and `refactor`, and
`format`, `lint`, `fix` and `doc`. There is no shell verb, no environment
mutation, no network and no path outside the root.

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

## Policy and the audit log

A session may be governed by `.yashy/policy.toml`:

```toml
allow_verbs = ["check", "build", "edit"]
protected_paths = ["yashy.lock", ".github/**"]
max_edits = 20
audit_max_bytes = 1048576
```

A missing file is the strict default: the verbs that do not change the source
(`check`, `build`, `test`, `verify`, `explain`, `context`, `index`, `lint`,
`doc`) are allowed, and `edit`, `refactor`, `format` and `fix` are refused. A
file that names `allow_verbs` is the complete set. An unknown key or an unknown
tool name is refused when the server starts, so a misspelling cannot silently
widen the policy. `protected_paths` uses `*` (no `/`), `**` (any run) and `?`;
`.yashy/policy.toml` is always protected, so a session cannot edit the policy it
runs under. A request that names a protected path is refused with JSON-RPC
`-32002` and the diagnostic code `E1807`; so is a patch with more edits than
`max_edits`. The refusal happens before the tool runs.

Every call appends one line to `.yashy/audit/session.jsonl` with `seq`, `tool`,
`outcome`, `code`, `detail`, `prev_hash` and `hash`. The hash is SHA-256 over
the first five fields and the previous hash, so a removed or reordered entry
breaks the chain. Secret-looking words (`token=...`, `password: ...`) are
redacted before the entry is written. The active segment rotates at
`audit_max_bytes` into `session.1.jsonl` (up to five segments), and the chain
stays continuous across the rotation; when the oldest segment is pruned,
verification starts at the first entry still held and says so in its `entries`
count. If the log cannot be written, the call is refused with `-32005` and
`E1804`: no call is silent.

`yashyc agent audit --verify [--root <dir>] [--json]` recomputes the chain and
names the first bad sequence number; the human form prints
`N entries  chain ok  first bad: none`. `yashyc agent policy [--json]` prints
the effective policy. See [Agent policy](agent-policy.md) for the file's keys.
