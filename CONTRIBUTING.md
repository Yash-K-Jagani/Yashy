# Contributing

## Before you change anything

Read `AGENTS.md`. It states the language's identity and the current phase plan.
The active phase plan under `docs/superpowers/plans/` is the source of truth for
what to build next; a completed plan set lives under `history/` and is not
authoritative. Do not start a later slice before the current one's gate passes.

A language change needs an RFC in `docs/rfcs/` when it touches syntax, HIR,
verifier rules, diagnostics, determinism or supported targets. Read
`docs/governance.md` for how decisions are recorded.

## Everyday commands

```text
cargo build --release --workspace
cargo check --workspace --all-targets
cargo test --workspace
cargo fmt --all
yashyc verify --json examples/hello/main.ya
```

`cargo fmt --all --check` has known drift; format only the files you touched.
The end-to-end programs under `crates/yashy-driver/tests/programs/` are the
language's worked examples. A language change is not done until the tutorial,
examples and program suites still pass.

## What a change must carry

- a test that would fail without it, next to the crate it exercises;
- a diagnostic page and rule ID for every new failure mode;
- a docs update when the public surface changes (`docs/stdlib.md` is generated
  from doc blocks: edit the module, never the page);
- a benchmark row for any performance claim, per `docs/bench-policy.md`;
- an honest checkbox and a ledger under `.superpowers/sdd/` recording which
  acceptance commands actually ran. A step is checked only when the code proves
  it; otherwise it stays open with an inline `*Current:*` note.

## Review

A reviewer checks that the plan box matches the code, that malformed programs
still produce diagnostics rather than panics, and that the language's identity
is intact: no GC, borrow checker, exceptions, inheritance, operator overloading,
procedural macros, `dyn` or pluggable async runtime.

By contributing you agree to `CODE_OF_CONDUCT.md`. Security issues follow
`SECURITY.md`, not a public issue.
