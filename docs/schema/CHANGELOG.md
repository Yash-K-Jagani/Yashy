# Schema changelog

## v1 (phase 11)

Frozen with the AI-native tooling phase. Payloads: `check`, `verify`,
`test`, `explain`, `doctor`, `expand`, `index`, `context`, `mcp`, `bench`.
Golden payloads live in `crates/yashy-driver/tests/schema_golden/`; the
diff gate is `cargo test -p yashy-driver --test schema_diff`.
