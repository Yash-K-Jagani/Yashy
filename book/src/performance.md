# Performance

Yashy treats optimization as a phase with evidence, not a habit of sprinkling
attributes. The compiler spends what the verifier proved, and every surviving
check or allocation is explained by `--opt-report`.

## Measure first

`yashyc bench` reads rows from `benchmarks/thresholds.bench`. A row names its
input, its flags and its target; a claim without a row is not a claim.

```text
yashyc bench --list
yashyc bench --json
yashyc bench --check-baseline benchmarks/baseline-linux.json
```

`--check-baseline` compares rows, applies each row's threshold, prints the
ratio and exits `1` on a regression. `--save-baseline` writes a JSON baseline
with the compiler version, target and flags. CI stores one baseline per OS.

## Find the hot path

`yashyc profile` reports wall-clock time and renders recorded counter tables;
`--alloc-report` reports the runtime's live-allocation total. `yashyc coverage`
is an explicit not-implemented error rather than a fake number, and
`coverage --fail-under` gates a floor when the source-based pipeline is
available.

Debug builds carry line tables. `yashyc inspect --layout --all <file.ya>`
prints size, alignment and field offsets from the same layout table a `sizeof`
query uses, so a layout claim can be checked without running a program.

## Narrow the loop

`yashyc test --affected <file>` selects tests that reach a changed module. A
changed type, trait, macro or `cfg` key widens the selection: any test naming
one is selected even without a direct import. An unknown changed file selects
everything. `--json` reports every selected and skipped test with its reason.

When a test passes for the wrong reason, `yashyc mutate --score
killed:survived:timed_out:equivalent` computes a deterministic score and
`--fail-under` gates it. Equivalent and timed-out mutants stay outside the
denominator, so the number measures what a suite can observe.
