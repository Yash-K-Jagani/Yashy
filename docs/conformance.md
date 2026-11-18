# Conformance cases

A conformance case is a named, reproducible check that a rule holds. The table
below is the registry: every case names the rule it exercises and the test file
that runs it. `yashyc trace --json` reads this table, so a case with a rule id
that no diagnostic code enforces is an orphan just like a code with no test.

| Case | Rule | Test |
|---|---|---|
| `type-mismatch` | TYPE-1 | `crates/yashy-sema/tests/check.rs` |
| `non-exhaustive-switch` | PAT-1 | `crates/yashy-verify/tests/exhaustive.rs` |
| `type-after-name` | TYPE-10 | `crates/yashy-parser/tests/types.rs` |
| `lock-across-await` | ASYNC-3 | `crates/yashy-sema/tests/async_effect.rs` |
| `unknown-cfg-key` | CFG-3 | `crates/yashy-driver/tests/cfg.rs` |

Adding a case means adding a row here and a test that names it. Renaming a rule
id is a migration note, never an edit to a past release's case: the id is part
of the traceability contract. The registry is intentionally small; a rule with
no case at all is reported by `orphan_rules_without_code` until one is added.
