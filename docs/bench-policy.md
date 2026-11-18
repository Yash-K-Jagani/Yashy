# Benchmark policy

A performance claim in this repository is a row in a benchmark file. Without a
row there is no claim, however plausible the reasoning sounded.

## What a row names

Every row records:

- the command and the input it was run on, so the row can be reproduced;
- the compiler version and target triple;
- the flags that were in force, including optimization and verification flags;
- the host (OS and CPU) and the date;
- the measured number and its unit, with the warmup and iteration count.

A row that leaves any of these out is not evidence and must not be cited in a
commit message, a design record or a release note.

## How a number is taken

1. Warm up once so a cold page cache or a lazy linker does not decide the run.
2. Run enough iterations to compute a median and a p95; report both, not just
   the best run.
3. Interleave A/B runs. Running every A first and every B second measures machine
   drift, not the change.
4. Reject outliers with a stated rule. Never delete a slow run merely because it
   was slow.
5. For an allocation claim, use the allocation report; for a binary-size claim,
   use the object file; for a link-time claim, time the link alone.

## What blocks a change

- A new compiler pass lands with a row in `benchmarks/` and an explanation in
  `--opt-report` of which proved fact it spent.
- A feature row that regresses beyond its recorded threshold blocks the phase
  gate until it is either fixed or the row is revised with the reason in the
  phase ledger.
- A claim about another operating system needs a row from that operating
  system. A Windows number is not a Linux number.

## Where the rows live

Existing rows live under `benchmarks/` (`run.ps1`, `ci.ps1`, `thresholds.bench`).
The ledger entry under `.superpowers/sdd/` for a slice names the rows it added.
`docs/optimization.md` explains the pass manager; this document is the rule that
governs the evidence a pass must bring.
