# Lints

`yashyc lint --list --json` is the machine form of this page. A lint is
advice: it never changes whether a program builds. A lint can be promoted
to an error with `yashyc lint --deny <code> <file.ya>`, and filtered to one
group with `--group <correctness|style|complexity|perf|security|deprecated>`.

| Code | Group | Message | Fix |
|---|---|---|---|
| `LINT-UNUSED` | correctness | a binding is never used | remove the binding, or use `_` if the value is intentionally ignored |
| `LINT-SHADOW` | style | a binding shadows an earlier name in the same function | rename one of the bindings so both stay readable |
| `LINT-VAR` | correctness | `var` was removed: use `let` | `let` is the only mutable binding keyword |
