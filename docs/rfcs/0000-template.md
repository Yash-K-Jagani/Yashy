# RFC 0000 — title

- **Status:** draft
- **Author:** name
- **Date:** YYYY-MM-DD
- **Phase / slice:** e.g. Phase 4 Task 8
- **Supersedes:** none
- **Superseded by:** none

## Motivation

What is broken or impossible today? Name a program the language cannot express
or a behaviour that is wrong, and the user who hits it. "It would be nice" is
not a motivation.

## Syntax

The exact source spelling, including what is *not* admitted. Show a minimal
program using the form and one the checker must refuse.

## HIR and desugaring

Which HIR nodes are added, and which existing nodes the form lowers to. A new
node needs a reason: sugar that desugars into existing HIR adds no node.

## Verifier rules

The rules the new form relies on, by ID (`OWN-3`, `ALIAS-2`, `DET-4`, ...). A
new rule needs a stable ID and a diagnostic that names it.

## Diagnostics

New codes, their catalog pages, their rule IDs and the span each one points at.
A malformed program must produce a diagnostic, never a compiler panic.

## Determinism

How the form behaves under a fixed seed, a fixed clock and a fixed target. Does
it read a capability? Does its output depend on map iteration order or wall
time? State it explicitly, even when the answer is "no".

## Migration

Which 1.0 programs change meaning, which keep it, and the diagnostic that tells
a reader what to do. A breaking change needs a release note and an edition
decision.

## Conformance

The test programs and the machine-readable assertions that prove the form. If
the RFC changes a rule id or a diagnostic code, this section is mandatory for
acceptance.

## Benchmark

A row under `docs/bench-policy.md` for any performance claim. "Faster" without
a row is not a claim this repository accepts.

## Documentation impact

Tutorial chapters, `docs/cli.md`, `docs/errors.md`, the generated stdlib page,
editor grammars and the semantic index. A new keyword touches all eleven
generated editor files.

## Decision

The choice made, the alternatives considered, and why they were declined. The
date, the approvers, and a link to the implementing commit or slice ledger.
