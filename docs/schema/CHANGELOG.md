# Schema changelog

## v1 (phase 11)

Frozen with the AI-native tooling phase. Payloads: `check`, `verify`,
`test`, `explain`, `doctor`, `expand`, `index`, `context`, `mcp`, `bench`.
Golden payloads live in `crates/yashy-driver/tests/schema_golden/`; the
diff gate is `cargo test -p yashy-driver --test schema_diff`.

`yashy_version` is a build value, not part of the schema: it tracks the
compiler's release version, so the goldens were regenerated when it
became `1.0.0` (2026-11-12). No field was added, removed or retyped —
the schema stays v1.

The `tool` field is the compiler's name, so the `context` and `index` goldens
were regenerated when the binary became `yashyc` (2026-11-15). It is declared as
a free string, so no field was added, removed or retyped and the schema stays
v1; `check`, `verify`, `test`, `explain` and `fmt` report their own verb in that
field and did not move.
