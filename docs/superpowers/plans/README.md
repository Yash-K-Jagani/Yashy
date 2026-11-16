# Plans

A plan in this directory is the source of truth for one slice of work: it says
what to build, in what order, with a checkbox per step and an acceptance
command per gate. Do not start a later slice before the current slice's
acceptance commands pass. The ledger for a slice lives at
`.superpowers/sdd/<date>-<slice>/progress.md`.

## The 1.0 plan set is closed

The sixteen plans dated **2026-10-27** — the master plan and phases 1a through
13 — are complete and archived under [`history/`](history). `history/` also
holds the earlier pre-rename milestones and the 2026-10-23 rename record. Nothing
in `history/` is authoritative: read it for provenance, not for instructions.

The master plan remains the index of what 1.0 shipped:

- [`history/2026-10-27-yashy-master-plan.md`](history/2026-10-27-yashy-master-plan.md)
- [`history/2026-10-27-phase-01a-verifier-core.md`](history/2026-10-27-phase-01a-verifier-core.md)
  … through
  [`history/2026-10-27-phase-13-hardening.md`](history/2026-10-27-phase-13-hardening.md)

The design those plans implemented is still the identity authority:
[`docs/superpowers/specs/2026-10-27-yashy-master-design.md`](../specs/2026-10-27-yashy-master-design.md).
2.0 does not contradict it; it extends it.

## The 2.0 plan set is active (proposed)

The **2026-11-15** set is the active proposal. It is one master index plus
sixteen phase files, and it is not started: no phase is in progress until the
set is approved.

- [`2026-11-15-yashy-2.0-master-plan.md`](2026-11-15-yashy-2.0-master-plan.md)
  — the index: global constraints, review focus, phase spine, gate protocol,
  cut order, first 30 days.
- Phases 1–14, with Phase 11 in three slices:

```text
2026-11-15-phase-01-instant-feedback.md
2026-11-15-phase-02-developer-surface.md
2026-11-15-phase-03-packages-distribution.md
2026-11-15-phase-04-language-power.md
2026-11-15-phase-05-core-data-text.md
2026-11-15-phase-06-network-web.md
2026-11-15-phase-07-data-persistence.md
2026-11-15-phase-08-application-stack.md
2026-11-15-phase-09-quality-stack.md
2026-11-15-phase-10-concurrency-parallelism.md
2026-11-15-phase-11a-representation.md
2026-11-15-phase-11b-codegen.md
2026-11-15-phase-11c-toolchain.md
2026-11-15-phase-12-reach-interop.md
2026-11-15-phase-13-ai-native.md
2026-11-15-phase-14-selfhost-spec-release.md
```

The design it implements is
[`../specs/2026-11-15-yashy-2.0-design.md`](../specs/2026-11-15-yashy-2.0-design.md);
the identity authority is still the 1.0 design
[`../specs/2026-10-27-yashy-master-design.md`](../specs/2026-10-27-yashy-master-design.md),
and [`docs/architecture.md`](../../architecture.md) is normative for the pipeline,
memory, aliasing and determinism contracts.

Detail falls off with distance on purpose: phases 1–3 are executable to the
task; 4–8 name deliverables, interfaces and gates; 9–14 name outcomes and gates.
A phase plan is revised at phase start, never rewritten from scratch, and a
phase does not start before its predecessor's gate passes. The rules for a plan
are in [`docs/gates.md`](../../gates.md) and in `AGENTS.md` under *Working with
the plans*.
