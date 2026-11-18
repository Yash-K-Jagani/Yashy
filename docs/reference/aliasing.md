# Aliasing

The object model is Rust-style, not class-based. A type never gains a parent
and a trait never gains fields. Aliasing is decided by ownership, not by a
borrow checker: a value has one owner, and a copy of an owning pointer would
give it two.

## Enforced rules

**ALIAS-1 — owning pointers move.** A second copy of an owning `*mut T` without
`move` is [E1005](../errors/E1005.md).

**ALIAS-3 — a view has a proved source.** A borrowed view of an owned value
must name the owner it derives from. An unproved source is
[E1403](../errors/E1403.md).

**ALIAS-5 — assumptions are reported.** A proof that assumes an aliasing fact
must list that assumption in the verifier inventory. An unreported assumption
is [E1404](../errors/E1404.md).

## Rules not yet enforced

The design names four more aliasing rules that the current front end cannot
violate in a way it can detect, so they have no diagnostic code. They are listed
here so the reference does not claim they are checked. `yashyc trace
--fail-orphans` reports them as orphans rather than treating them as absent.

- **ALIAS-2** — a view cannot outlive the storage it views.
- **ALIAS-4** — a mutable view is exclusive for its lifetime.
- **DET-2**, **DET-3**, **DET-4**, **DET-5** — the determinism rules on the
  [determinism page](determinism.md).

A rule moves from this list to the enforced list only with a diagnostic, a
catalog page, a rule id in the traceability matrix and a test that fires it. A
code that can never fire is not enforcement.
