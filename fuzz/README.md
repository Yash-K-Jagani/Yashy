# Fuzz targets

Two `cargo-fuzz` targets for the front end:

- `lex_parse` — lexes and parses arbitrary bytes.
- `check` — carries a program that parses through the type checker and the IR
  emitter.

## Running them

```sh
cargo install cargo-fuzz
cargo +nightly fuzz run lex_parse -- -max_total_time=120
cargo +nightly fuzz run check     -- -max_total_time=120
```

They need a nightly toolchain and `libfuzzer-sys`, which is why this package is
its own workspace and is deliberately **not** a member of the root workspace:
`cargo test --workspace` stays buildable on stable and offline.

## Where the seeds come from

`corpus/lex_parse/` holds one valid program and a few truncated ones, so the
fuzzer starts inside the grammar instead of spending its budget discovering the
letters of `fn`.

## When cargo-fuzz is not available

`crates/otter-driver/tests/never_panics.rs` covers the same invariant on stable,
with no dependencies: a table of malformed shapes (truncated tokens, unbalanced
delimiters, NUL bytes, absurd nesting) plus a seeded pseudo-random generator that
mutates real programs. If you are looking for a regression that used to crash,
look there first — a crash found by fuzzing should be added to that table as a
named case, because a table entry runs in every `cargo test` and a corpus entry
needs nightly.
