# RFCs

An RFC is the durable record of a language or compiler decision. A change that
touches syntax, HIR, verifier rules, diagnostics, determinism or the supported
targets needs one before it lands. A change that only fixes a bug or adds a
test does not.

## Index

| Number | Title | Status |
|---|---|---|
| [0000](0000-template.md) | RFC template | template, not a proposal |

An accepted RFC is numbered, kept in this directory, and linked from the phase
ledger that implemented it. A rejected RFC stays in the index with its reason;
the reason is part of the history.

## Statuses

- **draft** — being written; nothing is decided.
- **proposed** — open for review; the design is complete enough to argue with.
- **accepted** — approved; an implementation slice may start.
- **rejected** — considered and declined, with the reason.
- **superseded** — replaced by a later RFC, named in the header.

## Rule for acceptance

An RFC that changes a rule id, a diagnostic code or the determinism contract
cannot be marked accepted until its **Conformance** and **Migration** sections
are filled. A benchmark claim needs a row named under `docs/bench-policy.md`.
The decision is recorded in `docs/governance.md` with the date and the names of
the reviewers who approved it.
