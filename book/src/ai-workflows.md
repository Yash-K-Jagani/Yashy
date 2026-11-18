# AI-native workflows

The compiler exposes the same facts to a person at a terminal and to an agent
over a structured interface. The rule is one fact, two renderings: each tool
builds a JSON model first and renders the human form from it.

## The verbs

Seven compiler verbs are the whole public surface: `check`, `build`, `test`,
`verify`, `explain`, `context` and `index`. The MCP server offers exactly those
verbs — no shell execution, and no path outside the workspace root. The sandbox
is default-deny: `--allow-read` and `--allow-write` name the only paths.

```text
yashyc check --json examples/hello/main.ya
yashyc verify --json examples/hello/main.ya
yashyc explain --json E0100
yashyc index --json crates/yashy-stdlib/yashy
yashyc context --json crates/yashy-stdlib/yashy/io.ya
```

## Diagnostics are data

`check --json`, `verify --json` and `test --json` share schema v1. The envelope
carries `schema_version`, `tool`, `yashy_version`, `success`, `diagnostics` and
`summary`. A coded diagnostic ends with a `docs:` line in the human form, and
`yashyc explain <code>` prints the same page offline. A verifier diagnostic
also carries a `proof` object naming the obligation, the missing fact and the
remedy: it says what is missing, not just that verification failed.

For CI, `check --sarif` and `verify --sarif` render the diagnostics as a SARIF
2.1.0 log with rules, physical locations and machine-applicable fixes.

## Machine-readable documentation

`yashyc doc` is the documentation source of truth: the public surface rendered
from the compiler's own tables, byte-stable across runs and operating systems.
`yashyc doc --json` is the model; `yashyc doc` writes the Markdown. Edit the
source module's `///` comments, never the generated page.

`yashyc trace --json` emits the static rule matrix: design rule, enforcing
diagnostic code, covering test and conformance case. `trace --fail-orphans`
fails while a rule has no code, a code has no test, or a test names no rule.
`yashyc index` and `yashyc context` give an agent the module graph and the
surrounding facts before it edits code.

## A safe edit loop

1. Read `context` for the file you are about to change.
2. Make the edit, then run `check --json` and read `diagnostics[]`.
3. Take a `code`, run `explain --json <code>`, fix the cause and rerun `check`.
4. Run `verify --json`; `build` and `run` refuse an unsound program.
5. Run `test --json`, inspect `tests[]` and `summary`, and check `mutate` when
   the change touches behaviour.
