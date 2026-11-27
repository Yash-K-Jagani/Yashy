# Semantic review and impact

`yashyc index --json` is a snapshot of what the compiler knows: symbols with
spans, signatures, effects, contracts, typestate and visibility, the tests that
call them, and the capabilities declared by the root manifest. `yashyc review`
compares that snapshot with the workspace as it is now. It reads the graph,
not a line diff.

```console
$ yashyc index --json > baseline.json
$ yashyc review --baseline baseline.json --json
$ yashyc impact --baseline baseline.json --json
```

## What counts as a change

| Kind | Trigger | Breaks the public API when |
|---|---|---|
| `signature` | parameter/return spelling, deletion, visibility change | the symbol is `pub` |
| `effect` | the `io`/`unsafe`/`async` flags change | the symbol is `pub` |
| `contract` | `requires`/`ensures`/`invariant` clauses change | the symbol is `pub` |
| `ownership` | the typestate row changes | the symbol is `pub` |
| `behaviour` | a symbol was added | never |

A change to a `///` document is deliberately not a semantic change: a
comment-only edit reports no changes and `risk: low`. The report also carries
`capability_delta` (capability names the new manifest adds), `unsafe_delta`
(change in the number of symbols with the `unsafe` effect) and `test_delta`
(change in the number of test programs). Risk is `high` for a public break, a
new capability, a new unsafe effect or a dropped test; `medium` for other
changes; `low` for none.

Every change names the kind, symbol, file and byte span, whether the symbol is
public, and the old and new facts. The human and `--json` renderings come from
the same `Review` value.

## Impact

`yashyc impact` fans the changed symbol names out to:

- **tests** whose calls name a changed symbol, whose file declares one, or
  which existed in the baseline but are now gone;
- **packages** from the root manifest, plus workspace members with a manifest;
- **examples** and **docs** pages whose text names a changed symbol.

The report says what to re-run; it does not run it. An empty review gives an
empty impact.

The baseline must be an index snapshot from this compiler. Review does not
read git history, and a snapshot from another toolchain version can name
spans that no longer match. The SDK wrapper, MCP resources and prompts, and a
git-aware baseline are still owed.
