# Self-hosting Spike: a Yashy Lexer and Parser in Yashy

**Summary.** This record documents what a self-hosted Yashy front end would take, measured against a spike that reimplements the lexer and parser in Yashy itself (`selfhost/src/lexer.ya`, `selfhost/src/parser.ya`, `selfhost/src/ast.ya`). Two things came out of it:

1. **Measured numbers** for a bootstrap/codegen/perf/maintenance analysis (below).
2. **A real P0 compiler bug** found while probing `?`: a failed `?` in a function whose return type is a different shape of `Result`/`Option` compiled silently wrong — `gen_try` answered `unreachable` whenever the subject's LLVM repr differed from the return type's, so the run continued past a failed `?` with the wrong payload. Fixed by rebuilding the failure value in the return type (`crates/yashy-codegen/src/expr_gen.rs`, `gen_try`). That fix lands with this spike's commit because the spike surfaced it; it is not part of self-hosting per se.

## What the spike is

A Yashy program (`selfhost/main.ya`) that:

1. Lexes source bytes into the same token records the Rust lexer produces (`Kind,start,end;`).
2. Parses a **declared subset** of the grammar into a canonical digest string: nested s-expressions like `(program (fn main (params) (ret) (block ...)))`, plus a cursor threaded through as `"<cursor>;<digest>"` because Yashy arrays cannot hold structs/enums and tuples cannot hold `string` — there is no tree data structure available.
3. Compares its output with the Rust front end over the whole corpus (`crates/yashy-driver/tests/programs/*.ya` + `examples/*/main.ya`, 163 files).

The differential test (`crates/yashy-driver/tests/selfhost_diff.rs`) renders the Rust parser's real tree into the same shape and classifies each file as `agree` / `reproduced` / `unsupported` / `mismatch`. `yashy test selfhost\tests` is a cheaper self-check that needs neither Rust nor 30 seconds.

## The subset

Everything the spike parses parses identically to the Rust front end. Anything outside the subset is refused by name with a clear message, never guessed. Out of scope (and counted below as unsupported):

- `switch`, `if let`, casts, `for` over a slice, `?`, `move`
- struct/enum/object/trait/extern items, struct literals, empty `[]` literals
- `.await`, method calls on computed receivers, `Null`
- `fn f[T]` type parameter lists (the spike refuses them explicitly)

## Measured numbers

**Corpus:** 163 files / ~149 KB (`crates/yashy-driver/tests/programs` + `examples/*/main.ya`).

**Lexer differential:** 163 files agree, token for token (19.52 s). The lexer must agree on every file — it does.

**Parser differential:** 42 agree, 0 reproduced, 121 unsupported (0 of those the spike parsed anyway), 0 mismatched (33.83 s).

**Unsupported-cause tally:** `switch` ~25, struct/enum/object/trait/extern items ~25, empty `[]` literal 9, `.await` 13, struct literals 12, method call on a computed receiver 5, `if let` 4, casts 2, `for` over a slice 2, `?` 1, `move` 1, type parameter list 1, plus `Null`.

**Lines of code:** lexer 521, parser 940, ast 37, main 64, test 71.

**Build:** spike builds in 10.5 s with the debug compiler; the binary is 331,264 bytes.

**Performance:** spike lexes the whole corpus in 7.3 s vs `yashy` no-arg baseline 6.8 s (~45 ms/launch); `yashy outline` over the corpus takes 31.4 s vs baseline 30.7 s (~189 ms/launch). The front-end work is within noise — process start dominates.

## What self-hosting would take

### Bootstrap path

Yashy compiles to native binaries through LLVM, and there is no garbage collector, so a Yashy compiler binary is a self-contained native program. The bootstrap is therefore a **source-to-source** problem, not a cross-compiler problem:

1. Write the whole front end (lexer, parser, sema, codegen driver) in Yashy.
2. Compile it with the Rust `yashy` to get a Yashy compiler binary.
3. Compile that Yashy source with itself: the second binary must produce the same output as the first. Differential tests like the one in this spike are the check at every step.

The spike shows the front end *can* be written in Yashy and agrees with the Rust implementation on everything inside the subset. The remaining 121 unsupported files are mostly language features (structs, enums, objects, traits, `switch`, `if let`, async) rather than front-end architecture, so they fall out of the same work that implements those features in the compiler — not of a separate self-hosting effort.

### Codegen story

The spike is parse-only: no type checking, no verification, no codegen. A self-hosted compiler still needs an LLVM backend. The realistic path is the one the project already takes: the Yashy compiler front end emits textual LLVM IR, and `clang`/`lld` lower it to a native binary. A self-hosted front end would reuse that same emitter — the question is only *who runs it* (the Rust binary or a Yashy binary). Nothing in the spike changes that.

### Performance cost of a Yashy parser

The spike's corpus sweep is within noise of `yashy` itself; process start (~45–189 ms per launch) dominates, not front-end work. That measurement is for a debug compiler with no optimisation pass manager, so it is an upper bound on front-end cost, not a prediction of a self-hosted compiler's throughput. It does suggest a Yashy parser is not obviously slower at the source level — but it also does not measure memory, cache behaviour, or the verifier, which dominate real compile time.

### Maintenance cost of two implementations of one grammar

This is the strongest argument against self-hosting v1. Every grammar change touches the Rust parser **and** the Yashy parser, plus two differential tests, plus the desugar proof (`crates/yashy-sema/tests/desugar.rs`), plus the error catalog. The project's rule is "one fact, two renderings" for tooling — but the parser is not tooling, it is the single source of truth for what a program means. Two parsers are two sources of truth that must be kept in step forever.

## Recommendation for 2.0

**Do not self-host the parser in v1.** The spike proves it is feasible and that a Yashy parser can agree with the Rust one, but the maintenance cost of two grammars outweighs the benefit before 1.0 ships.

Revisit for 2.0, and if it happens, build it as:

1. The Rust parser stays the reference implementation.
2. A Yashy parser (this spike, extended to the full grammar) is checked against it by the differential test on every change — `agree` on every file inside the subset, `0 mismatched`.
3. The bootstrap is source-to-source: the Rust-built binary compiles the Yashy sources, and the Yashy-built binary must reproduce identical output.

The numbers above (42 agree / 121 unsupported / 0 mismatched, lexer 163/163) are the baseline to beat.

## Files

- `selfhost/src/lexer.ya` — the Yashy lexer (521 lines)
- `selfhost/src/parser.ya` — the Yashy parser, digest-based (940 lines)
- `selfhost/src/ast.ya` — `pack`/`packed_pos`/`packed_body` digest vocabulary (37 lines)
- `selfhost/main.ya` — `lex`/`parse` CLI (64 lines)
- `selfhost/tests/differential.ya` — self-check, `yashy test selfhost\tests` (71 lines)
- `crates/yashy-driver/tests/selfhost_diff.rs` — differential harness + Rust subset renderer
- `docs/superpowers/design/2027-selfhost-spike.md` — this record
