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
