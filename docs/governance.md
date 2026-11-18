# Governance

## Where decisions live

Decisions live in this repository, not in a slide or a meeting that nobody can
read later. A language change goes through `docs/rfcs/`; a compiler change goes
through a phase plan and a slice ledger under `.superpowers/sdd/`. A decision
that affects users also gets a release note.

## Decision log

| Date | Decision | RFC / record | Approvers |
|---|---|---|---|
| 2026-10-27 | The 1.0 object model stays Rust-style: static dispatch, no `dyn`, no inheritance | `docs/superpowers/specs/2026-10-27-yashy-master-design.md` | project maintainers |
| 2026-11-15 | The 2.0 phase set and its ordering are adopted as the roadmap | `docs/superpowers/plans/2026-11-15-yashy-2.0-master-plan.md` | project maintainers |

An entry is appended, never edited, except to link the implementing commit once
it lands.

## Quarterly re-plan

At the start of each quarter the phase plans are reviewed. The diff of a
re-plan changes only the dates and the cut list: which slices move into the
next quarter and which move out. It never changes the project's identity, its
non-goals or the never-cut list below. A change to any of those is an RFC, not
a re-plan.

## Identity, non-goals and never-cut list

- **Identity:** a statically typed language compiled to native binaries through
  LLVM, with explicit ownership, errors as values, and one async executor.
- **Non-goals:** replacing Rust, being a scripting language, hosting a virtual
  machine.
- **Never cut:** no garbage collector, no borrow checker, no exceptions, no
  inheritance, no operator overloading, no procedural macros, no `dyn`, no
  pluggable async runtime. Removing a safety rule to make a benchmark faster
  is a regression, not an optimization.

## Review

A change to the language needs a parser test, a checker test, a diagnostic page
when it can fail, a formatter behavior, and a program under
`crates/yashy-driver/tests/programs/`. A change to the compiler needs a check
that runs without a toolchain where possible. A reviewer checks that the plan
box is ticked only when the code proves it, and that the ledger names the
acceptance commands that actually ran.
