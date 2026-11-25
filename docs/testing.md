# Testing Yashy

Phase 9 makes the testing loop ordinary and measurable.

## Packages

- `packages/test` provides value-rendering assertions, named tables, seeded
  properties with shrinking, LF-stable snapshots and explicit mocks. A test is
  still an ordinary program with `main`; the helpers only render failures
  better.
- `packages/fuzz/src/campaign.ya` runs a bounded, seeded campaign over a plain
  `fn(string) -> bool` target and shrinks the first failing input.

## CLI gates

| Command | Gate |
|---|---|
| `yashyc test --json` | one start and end event per test |
| `yashyc test --affected <paths>` | conservative selection with reasons |
| `yashyc bench --measure <file>` | warmup, N runs, median/p95, outlier rejection, interleaved A/B |
| `yashyc bench --save-baseline <file>` | a baseline the existing `--check-baseline` can read |
| `yashyc mutate --score killed:survived:timed_out:equivalent --fail-under <pct>` | mutation floor |
| `yashyc trace --json .` | generated rule matrix with orphan accounting |

## Mutation and differential testing

`yashy-mutate` generates stable AST mutants for arithmetic, relational and
boolean operators and integer literals. The report classifies killed,
survived, timed-out and equivalent mutants and computes a deterministic score.
`crates/yashy-driver/src/differential.rs` runs a corpus through two compiler
binaries and compares exit status and normalised output; paths and timing lines
are removed before the comparison.

The campaign runner that applies those mutants to a suite, the remaining
mutation kinds (`Option`/`Result` arms, ownership), and the second-toolchain
comparison are carried forward in `docs/gates.md`.
