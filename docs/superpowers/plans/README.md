# Plans

A plan in this directory is the source of truth for one slice of work: it says
what to build, in what order, with a checkbox per step and an acceptance
command per gate. Do not start a later slice before the current slice's
acceptance commands pass. The ledger for a slice lives at
`.superpowers/sdd/<date>-<slice>/progress.md`.

## The 1.0 plan set is closed

The sixteen plans dated **2026-10-27** — the master plan and phases 1a through
13 — are complete and archived under [`history/`](history). `history/` also
holds the earlier Otter-era milestones and the 2026-10-23 rename record. Nothing
in `history/` is authoritative: read it for provenance, not for instructions.

The master plan remains the index of what 1.0 shipped:

- [`history/2026-10-27-yashy-master-plan.md`](history/2026-10-27-yashy-master-plan.md)
- [`history/2026-10-27-phase-01a-verifier-core.md`](history/2026-10-27-phase-01a-verifier-core.md)
  … through
  [`history/2026-10-27-phase-13-hardening.md`](history/2026-10-27-phase-13-hardening.md)

The design those plans implemented is still current:
[`docs/superpowers/specs/2026-10-27-yashy-master-design.md`](../specs/2026-10-27-yashy-master-design.md).

## The next plan

The next master plan has not been written yet. When it is agreed it lands in
this directory as `<date>-yashy-master-plan.md`, and each of its phases lands
beside it as `<date>-phase-<id>-<name>.md`. The rules for a plan are in
[`docs/gates.md`](../../gates.md) and in `AGENTS.md` under *Working with the
plans*.
