# Agent policy

An MCP session runs under a policy file at `<workspace>/.yashy/policy.toml`.
The file belongs to the workspace, not to the agent: an MCP session cannot edit
it, and an unknown key or verb refuses the whole session at start.

```toml
allow_verbs = ["check", "build", "test", "verify", "explain",
               "context", "index", "edit", "refactor", "format",
               "lint", "fix", "doc"]
protected_paths = ["yashy.lock", ".github/**"]
max_edits = 20
audit_max_bytes = 1048576
require_approval = ["edit", "refactor"]
```

- `allow_verbs` is the complete set of verbs the session may call. The names
  are the MCP tool names; there is no shell verb and no `run` verb.
- `protected_paths` is a list of globs. `*` matches within one path component,
  `**` across components, and `?` matches one character. The policy file itself
  is always protected, whether or not a pattern names it.
- `max_edits` caps the number of edits in one patch. The patch is refused
  before it is parsed or applied.
- `audit_max_bytes` is the size cap on the active audit segment.
- `require_approval` names mutating verbs that may not run until a token is
  spent. `check`, `build`, `test`, `verify`, `explain`, `context`, `index`,
  `lint` and `doc` cannot be named here.

A missing file is the strict default: `check`, `build`, `test`, `verify`,
`explain`, `context`, `index`, `lint` and `doc` are allowed; `edit`, `refactor`,
`format` and `fix` are refused. Verb names a policy does not know are an error,
not an empty grant.

A refusal is JSON-RPC `-32002` with `E1807` in `error.data.code`; a path outside
the workspace is `-32001`; an unwritable audit log is `-32005` with `E1804`.
Precedence is policy, then path containment, then the verb.

`yashyc agent policy [--json] [--root <dir>]` prints the effective policy.
`yashyc agent audit --verify [--json] [--root <dir>]` verifies the hash chain;
see [MCP](mcp.md) for the log's shape and rotation window.

## Approvals

`yashyc agent plan --patch <file> [--json] [--root <dir>]` reads a patch and
reports the files and edits it would make, the symbols named by its
replacements, the capabilities declared by `yashy.toml`, and the exact verb
set. It writes nothing.

`yashyc agent approve --patch <file> [--tree <hash>] [--verbs <a,b>] [--ttl
<seconds>] [--nonce <text>] [--json] [--root <dir>]` writes a short-lived token
under `.yashy/approvals/`. The token is bound to the tree hash, a canonical
patch fingerprint, the verb set and an expiry. The MCP session consumes it
once, before the verb runs; a token from another tree, another patch or another
verb is refused, and a replay is refused and logged. Supplying `--nonce` makes
a token byte-reproducible for tests; without it the current clock is used.

Still owed: token, wall-clock and output budgets counted by the harness;
session-key signatures on log entries.

## Budgets

`max_wall_ms`, `max_tokens` and `max_output_bytes` cap a session. Milliseconds
are spent inside tool calls; tokens are request-plus-response bytes divided by
four, rounded up; output bytes are the responses themselves. A limit is
refused with `E1807` before the next tool runs, and the refusal is an audit
entry. Budgets are session-wide, so an agent cannot raise its own by editing a
policy file (the file is protected).
